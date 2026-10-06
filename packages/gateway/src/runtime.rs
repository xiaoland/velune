//! Local LLM ingress. Same-protocol traffic remains native; cross-protocol
//! routing uses request-scoped best-effort translation. Fail-over is disabled.
use crate::{
    analytics::{
        AnalyticsOutcome, AnalyticsProtocol, AnalyticsRecord, RequestAnalytics,
        SharedAnalyticsSink, SharedTracker, TokenUsage, now_ms,
    },
    config::{GatewayConfig, GatewayProtocol},
    observation::Observation,
};
use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode},
    routing::post,
};
use futures_util::stream;
use reqwest::Client;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::TcpListener,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tracing::{Instrument, instrument::WithSubscriber};
use velune_ai::{
    DeliveryError, Payload,
    chat_completions::*,
    http::{Header, ResponseBody, ResponseMeta},
    ids::{ConfigRevision, ProviderId, ProviderModelId},
    messages::*,
    responses::*,
};
pub use velune_ai_provider::config::ResolvedCredential;

#[derive(Debug, Clone)]
pub struct CredentialTarget {
    pub protocol: GatewayProtocol,
    pub endpoint: String,
}
#[derive(Debug, Clone, Copy)]
pub enum CredentialResolutionError {
    Unavailable,
    UnauthorizedTarget,
    Timeout,
    InvalidContract,
    StaleBinding,
}
pub trait CredentialResolver: Send + Sync {
    fn resolve(
        &self,
        reference: String,
        target: CredentialTarget,
    ) -> velune_ai::OperationFuture<Result<ResolvedCredential, CredentialResolutionError>>;
}

use velune_ai_provider::{
    config::{
        ChatCompletionsConfig, CredentialRef, HttpEndpoint, MessagesConfig, ProtocolConfig,
        ProviderConfig, ResponsesConfig, Transport,
    },
    messages::Messages,
    openai::ChatCompletions,
    responses::OpenAiResponses,
};

const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_IN_FLIGHT: usize = 16;
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub struct GatewayError(&'static str);
impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for GatewayError {}

#[derive(Clone)]
struct RouteTarget {
    max_output_tokens: Option<u32>,
    provider_slot: usize,
    model_slot: usize,
    model: ProviderModelId,
    protocol: GatewayProtocol,
    endpoint: String,
    config: ProviderConfig,
    reference: String,
    provider_id: String,
    provider_name: String,
    model_record_key: String,
}
struct Ingress {
    routes: Arc<RwLock<BTreeMap<String, RouteTarget>>>,
    token: String,
    resolver: Arc<dyn CredentialResolver>,
    client: Client,
    permits: Arc<Semaphore>,
    request_epoch: u64,
    next_request: AtomicU64,
    stopping: Arc<AtomicBool>,
    analytics: Option<SharedAnalyticsSink>,
}

pub struct Runner {
    config: GatewayConfig,
    routes: Arc<RwLock<BTreeMap<String, RouteTarget>>>,
    endpoint: String,
    token: String,
    stopping: Arc<AtomicBool>,
    stop: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}
impl Runner {
    /// Replace only runtime aliases. In-flight calls retain their captured target.
    /// Target configurations, credentials and the ingress token stay unchanged.
    pub fn replace_aliases(
        &self,
        aliases: BTreeMap<String, String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let next = build_routes(&self.config, &aliases)?;
        *self
            .routes
            .write()
            .map_err(|_| GatewayError("gateway alias lock"))? = next;
        Ok(())
    }
    pub fn start(
        config: GatewayConfig,
        credential_resolver: Arc<dyn CredentialResolver>,
        aliases: BTreeMap<String, String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::start_with_analytics(config, credential_resolver, aliases, None)
    }
    pub fn start_with_analytics(
        config: GatewayConfig,
        credential_resolver: Arc<dyn CredentialResolver>,
        aliases: BTreeMap<String, String>,
        analytics: Option<SharedAnalyticsSink>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        config.validate().map_err(GatewayError)?;
        let routes = Arc::new(RwLock::new(build_routes(&config, &aliases)?));
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let mut random = [0u8; 32];
        getrandom::fill(&mut random).map_err(|_| GatewayError("gateway token"))?;
        let token = random
            .iter()
            .map(|value| format!("{value:02x}"))
            .collect::<String>();
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .build()?;
        let mut epoch = [0u8; 8];
        getrandom::fill(&mut epoch).map_err(|_| GatewayError("gateway request identity"))?;
        let stopping = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Ingress {
            routes: routes.clone(),
            token: token.clone(),
            resolver: credential_resolver,
            client,
            permits: Arc::new(Semaphore::new(MAX_IN_FLIGHT)),
            request_epoch: u64::from_le_bytes(epoch),
            next_request: AtomicU64::new(1),
            stopping: stopping.clone(),
            analytics,
        });
        let (stop, stopped) = oneshot::channel();
        let dispatcher = tracing::dispatcher::get_default(Clone::clone);
        let parent = tracing::Span::current();
        let handle = thread::Builder::new().name("velune-ai-gateway".into()).spawn(move || {
            tracing::dispatcher::with_default(&dispatcher, || {
                let _entered = parent.enter();
                let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(runtime) => runtime, Err(_) => { tracing::error!(event="gateway_executor_failed"); return; }
                };
                runtime.block_on(async move {
                    let listener = match tokio::net::TcpListener::from_std(listener) { Ok(value)=>value,Err(_)=>return };
                    let router = Router::new().route("/v1/chat/completions",post(chat)).route("/v1/responses",post(responses)).route("/v1/messages",post(messages)).with_state(state);
                    tracing::info!(event="gateway_started");
                    // Dropping the server stops admission; dropping this executor aborts
                    // connection/dispatch tasks, including their kill-on-drop helpers.
                    tokio::select! { result = axum::serve(listener,router) => { if result.is_err() { tracing::warn!(event="gateway_listener_failed"); } }, _ = stopped => {} }
                    tracing::info!(event="gateway_stopped");
                });
            });
        })?;
        Ok(Self {
            config,
            routes,
            endpoint: format!("http://127.0.0.1:{}/v1", address.port()),
            token,
            stopping,
            stop: Some(stop),
            handle: Some(handle),
        })
    }
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub fn token(&self) -> &str {
        &self.token
    }
}
impl Drop for Runner {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn build_routes(
    config: &GatewayConfig,
    aliases: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, RouteTarget>, Box<dyn std::error::Error>> {
    let mut routes = BTreeMap::new();
    for (provider_slot, configured_provider) in config.providers.iter().enumerate() {
        for (model_slot, entry) in configured_provider.models.iter().enumerate() {
            let provider = config
                .validate_dispatch(&entry.record_key)
                .map_err(GatewayError)?;
            if !crate::config::credential_ready(provider) {
                return Err(Box::new(GatewayError("provider credential is required")));
            }
            let endpoint = parse_endpoint(&provider.endpoint)?;
            let protocol = match provider.protocol {
                GatewayProtocol::ChatCompletionsV1 => {
                    ProtocolConfig::ChatCompletions(ChatCompletionsConfig { endpoint })
                }
                GatewayProtocol::ResponsesV1 => {
                    ProtocolConfig::Responses(ResponsesConfig { endpoint })
                }
                GatewayProtocol::MessagesV1 => {
                    ProtocolConfig::Messages(MessagesConfig { endpoint })
                }
            };
            let binding = provider
                .models
                .iter()
                .find(|binding| binding.record_key == entry.record_key)
                .ok_or(GatewayError("provider binding missing"))?;
            let model = ProviderModelId::new(binding.provider_model_id.clone())?;
            let provider_config = ProviderConfig::new(
                ProviderId::new(provider.id.clone())?,
                ConfigRevision::new(1)?,
                protocol,
                CredentialRef::new(
                    provider
                        .credential_ref
                        .clone()
                        .unwrap_or_else(|| provider.id.clone()),
                )?,
            )?;
            routes.insert(
                entry.record_key.clone(),
                RouteTarget {
                    max_output_tokens: binding.max_output_tokens,
                    provider_slot: provider_slot + 1,
                    model_slot: model_slot + 1,
                    model,
                    protocol: provider.protocol.clone(),
                    endpoint: provider.endpoint.clone(),
                    config: provider_config,
                    reference: provider
                        .credential_ref
                        .clone()
                        .ok_or(GatewayError("authentication resource is required"))?,
                    provider_id: provider.id.clone(),
                    provider_name: provider.name.clone(),
                    model_record_key: entry.record_key.clone(),
                },
            );
        }
    }
    let targets = routes;
    let mut routes = BTreeMap::new();
    for entry in config
        .providers
        .iter()
        .flat_map(|provider| &provider.models)
    {
        let alias = format!("velune/model/{}", entry.record_key);
        routes.insert(
            alias,
            targets
                .get(&entry.record_key)
                .expect("built target")
                .clone(),
        );
    }
    for (alias, logical) in aliases {
        let target = targets
            .get(logical)
            .cloned()
            .ok_or(GatewayError("gateway route target"))?;
        if routes.insert(alias.clone(), target).is_some() {
            return Err(Box::new(GatewayError("gateway binding identity collision")));
        }
    }
    Ok(routes)
}
fn parse_endpoint(value: &str) -> Result<HttpEndpoint, Box<dyn std::error::Error>> {
    let url = reqwest::Url::parse(value).map_err(|_| GatewayError("provider endpoint"))?;
    let transport = match url.scheme() {
        "http" => Transport::Http,
        "https" => Transport::Https,
        _ => return Err(Box::new(GatewayError("provider endpoint scheme"))),
    };
    if url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Box::new(GatewayError("provider endpoint query")));
    }
    let host = url
        .host_str()
        .ok_or(GatewayError("provider endpoint host"))?;
    let port = url
        .port_or_known_default()
        .ok_or(GatewayError("provider endpoint port"))?;
    Ok(HttpEndpoint::new(transport, host, port, url.path())
        .map_err(|_| GatewayError("provider endpoint"))?)
}

async fn chat(State(state): State<Arc<Ingress>>, request: Request<Body>) -> Response<Body> {
    ingress(state, request, GatewayProtocol::ChatCompletionsV1).await
}
async fn messages(State(state): State<Arc<Ingress>>, request: Request<Body>) -> Response<Body> {
    ingress(state, request, GatewayProtocol::MessagesV1).await
}
async fn responses(State(state): State<Arc<Ingress>>, request: Request<Body>) -> Response<Body> {
    ingress(state, request, GatewayProtocol::ResponsesV1).await
}

pub(super) enum WireEvent {
    Headers(ResponseMeta),
    Body(Vec<u8>),
    Failed,
}
struct DispatchGuard {
    task: tokio::task::JoinHandle<()>,
    _permit: OwnedSemaphorePermit,
    observation: Observation,
}
impl Drop for DispatchGuard {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn ingress(
    state: Arc<Ingress>,
    request: Request<Body>,
    protocol: GatewayProtocol,
) -> Response<Body> {
    let request_id = format!(
        "{:x}-{:x}",
        state.request_epoch,
        state.next_request.fetch_add(1, Ordering::Relaxed)
    );
    let protocol_name = match protocol {
        GatewayProtocol::ChatCompletionsV1 => "chat_completions_v1",
        GatewayProtocol::ResponsesV1 => "responses_v1",
        GatewayProtocol::MessagesV1 => "messages_v1",
    };
    let span = tracing::info_span!(parent: None, "gateway_request", request_id, protocol = protocol_name, stream = tracing::field::Empty);
    let dispatcher = tracing::dispatcher::get_default(Clone::clone);
    let observation = Observation::new(
        "gateway_request_finished",
        span.clone(),
        state.stopping.clone(),
    );
    let started_at_ms = now_ms();
    ingress_observed(
        state,
        request_id,
        started_at_ms,
        request,
        protocol,
        observation,
    )
    .instrument(span)
    .with_subscriber(dispatcher)
    .await
}

async fn ingress_observed(
    state: Arc<Ingress>,
    request_id: String,
    started_at_ms: i64,
    request: Request<Body>,
    protocol: GatewayProtocol,
    observation: Observation,
) -> Response<Body> {
    tracing::info!(event = "gateway_request_received");
    let bearer_valid = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        == Some(format!("Bearer {}", state.token).as_str());
    let key_valid = protocol == GatewayProtocol::MessagesV1
        && request
            .headers()
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            == Some(state.token.as_str());
    if !bearer_valid && !key_valid {
        return rejected(
            observation,
            StatusCode::UNAUTHORIZED,
            "gateway authentication failed",
        );
    }
    let permit = match state.permits.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return rejected(
                observation,
                StatusCode::SERVICE_UNAVAILABLE,
                "gateway concurrency limit reached",
            );
        }
    };
    let (parts, body) = request.into_parts();
    let headers = forward_headers(&parts.headers);
    let bytes =
        match tokio::time::timeout(REQUEST_READ_TIMEOUT, to_bytes(body, MAX_REQUEST_BYTES)).await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(_)) => {
                return rejected(
                    observation,
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "gateway request body limit",
                );
            }
            Err(_) => {
                return rejected(
                    observation,
                    StatusCode::REQUEST_TIMEOUT,
                    "gateway request body deadline",
                );
            }
        };
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(Value::Object(value)) => Value::Object(value),
        _ => {
            return rejected(
                observation,
                StatusCode::BAD_REQUEST,
                "gateway requires a JSON object",
            );
        }
    };
    let Some(model) = body["model"].as_str() else {
        return rejected(
            observation,
            StatusCode::BAD_REQUEST,
            "gateway model is required",
        );
    };
    let target = match state.routes.read() {
        Ok(routes) => routes.get(model).cloned(),
        Err(_) => {
            return rejected(
                observation,
                StatusCode::INTERNAL_SERVER_ERROR,
                "gateway alias state unavailable",
            );
        }
    };
    let Some(target) = target else {
        return rejected(
            observation,
            StatusCode::BAD_REQUEST,
            "gateway model route is missing",
        );
    };
    let stream = match body.get("stream") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return rejected(
                observation,
                StatusCode::BAD_REQUEST,
                "stream must be a boolean",
            );
        }
    };
    let translation = if target.protocol == protocol {
        None
    } else {
        match velune_ai_provider::translation::prepare(
            crate::translation::protocol(&protocol),
            crate::translation::protocol(&target.protocol),
            &body,
            target.max_output_tokens,
        ) {
            Ok(plan) => {
                tracing::info!(
                    event = "gateway_translation_prepared",
                    source_protocol = ?protocol,
                    target_protocol = ?target.protocol,
                );
                Some(plan)
            }
            Err(error) => {
                return rejected(
                    observation,
                    StatusCode::BAD_REQUEST,
                    &format!("protocol conversion: {error}"),
                );
            }
        }
    };
    let body = translation
        .as_ref()
        .map_or(body.clone(), |plan| plan.request().clone());
    let headers = if translation.is_some() {
        crate::translation::upstream_headers(headers, &target.protocol)
    } else {
        headers
    };
    tracing::Span::current().record("stream", stream);
    tracing::info!(
        event = "gateway_route_selected",
        provider_slot = target.provider_slot,
        model_slot = target.model_slot
    );
    let (sender, mut receiver) = mpsc::channel(1);
    let request_span = tracing::Span::current();
    let dispatcher = tracing::dispatcher::get_default(Clone::clone);
    let analytics = state.analytics.clone().map(|sink| {
        RequestAnalytics::new(
            sink,
            AnalyticsRecord {
                request_id,
                provider_id: target.provider_id.clone(),
                provider_name: target.provider_name.clone(),
                model_record_key: target.model_record_key.clone(),
                provider_model_id: target.model.as_str().to_owned(),
                protocol: match target.protocol {
                    GatewayProtocol::ChatCompletionsV1 => AnalyticsProtocol::ChatCompletions,
                    GatewayProtocol::ResponsesV1 => AnalyticsProtocol::Responses,
                    GatewayProtocol::MessagesV1 => AnalyticsProtocol::Messages,
                },
                started_at_ms,
                terminal_at_ms: started_at_ms,
                elapsed_ms: 0,
                first_output_ms: None,
                terminal_elapsed_ms: None,
                status: None,
                outcome: AnalyticsOutcome::Cancelled,
                usage: TokenUsage::default(),
                usage_reported: false,
                usage_complete: false,
            },
            state.stopping.clone(),
        )
    });
    let task = tokio::spawn(async move {
        // Future polls retain the application's scoped diagnostic dispatcher.
        dispatch(
            state,
            analytics,
            target,
            body,
            headers,
            stream,
            translation,
            sender,
        )
        .instrument(request_span)
        .with_subscriber(dispatcher)
        .await;
    });
    let mut guard = DispatchGuard {
        task,
        _permit: permit,
        observation,
    };
    let Some(WireEvent::Headers(meta)) = receiver.recv().await else {
        guard.observation.finish("upstream_unavailable");
        return error_response(
            StatusCode::BAD_GATEWAY,
            "upstream ended before response headers",
        );
    };
    tracing::info!(event = "gateway_response_ready", http_status = meta.status);
    let body_stream = stream::unfold((receiver, guard), |(mut receiver, mut guard)| async move {
        let Some(next) = receiver.recv().await else {
            guard.observation.finish("transport_completed");
            return None;
        };
        let item = match next {
            WireEvent::Body(bytes) => Ok(Bytes::from(bytes)),
            WireEvent::Failed | WireEvent::Headers(_) => {
                guard.observation.finish("upstream_failed");
                Err(std::io::Error::other("upstream response interrupted"))
            }
        };
        Some((item, (receiver, guard)))
    });
    response(meta, Body::from_stream(body_stream))
}

fn forward_headers(headers: &HeaderMap) -> Vec<Header> {
    let connection_fields = headers
        .get_all("connection")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| {
            value
                .split(',')
                .map(|value| value.trim().to_ascii_lowercase())
        })
        .collect::<Vec<_>>();
    headers
        .iter()
        .filter_map(|(name, value)| {
            let name = name.as_str();
            if [
                "connection",
                "keep-alive",
                "proxy-authenticate",
                "proxy-authorization",
                "te",
                "trailer",
                "transfer-encoding",
                "upgrade",
                "host",
                "content-length",
                "authorization",
                "x-api-key",
                "cookie",
                "set-cookie",
            ]
            .contains(&name)
                || connection_fields.iter().any(|field| field == name)
            {
                return None;
            }
            Some(Header {
                name: name.to_owned(),
                value: value.to_str().ok()?.to_owned(),
            })
        })
        .collect()
}
fn response(meta: ResponseMeta, body: Body) -> Response<Body> {
    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::from_u16(meta.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut raw = HeaderMap::new();
    for header in meta.headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::try_from(header.name),
            HeaderValue::try_from(header.value),
        ) {
            raw.append(name, value);
        }
    }
    for header in forward_headers(&raw) {
        if let (Ok(name), Ok(value)) = (
            HeaderName::try_from(header.name),
            HeaderValue::try_from(header.value),
        ) {
            response.headers_mut().append(name, value);
        }
    }
    response
}
fn rejected(mut observation: Observation, status: StatusCode, message: &str) -> Response<Body> {
    observation.finish("rejected");
    tracing::warn!(
        event = "gateway_request_rejected",
        http_status = status.as_u16()
    );
    error_response(status, message)
}
fn error_response(status: StatusCode, message: &str) -> Response<Body> {
    response(
        ResponseMeta {
            status: status.as_u16(),
            headers: vec![Header {
                name: "content-type".into(),
                value: "application/json".into(),
            }],
        },
        Body::from(json!({"error":{"message":message}}).to_string()),
    )
}
pub(super) async fn send_json(sender: &mpsc::Sender<WireEvent>, value: ResponseBody) {
    if sender.send(WireEvent::Headers(value.meta)).await.is_ok() {
        let _ = sender.send(WireEvent::Body(value.body.into_inner())).await;
    }
}
async fn send_failure(
    sender: &mpsc::Sender<WireEvent>,
    meta: Option<ResponseMeta>,
    body: Option<Payload<Vec<u8>>>,
    status: u16,
    message: &str,
) {
    let meta = meta.unwrap_or(ResponseMeta {
        status,
        headers: vec![Header {
            name: "content-type".into(),
            value: "application/json".into(),
        }],
    });
    let bytes = body.map(Payload::into_inner).unwrap_or_else(|| {
        json!({"error":{"message":message}})
            .to_string()
            .into_bytes()
    });
    send_json(
        sender,
        ResponseBody {
            meta,
            body: Payload::new(bytes),
        },
    )
    .await;
}

#[allow(clippy::too_many_arguments)] // Captured request, delivery and analytics have distinct lifetimes.
async fn dispatch(
    state: Arc<Ingress>,
    analytics: Option<RequestAnalytics>,
    target: RouteTarget,
    body: Value,
    headers: Vec<Header>,
    stream: bool,
    translation: Option<velune_ai_provider::translation::PreparedTranslation>,
    sender: mpsc::Sender<WireEvent>,
) {
    let tracker = analytics.as_ref().map(|guard| guard.tracker.clone());
    let attempt_span = tracing::info_span!("gateway_attempt", attempt = 1);
    let mut observation = Observation::new(
        "gateway_attempt_finished",
        attempt_span.clone(),
        state.stopping.clone(),
    );
    if let Some(plan) = translation {
        let model = target.model.as_str().to_owned();
        let (upstream, receiver) = mpsc::channel(1);
        // Both futures belong to the ingress task. Downstream drop aborts both;
        // bounded channels retain native backpressure and cancellation.
        let native = dispatch_observed(
            state,
            tracker,
            target,
            body,
            headers,
            stream,
            upstream,
            &mut observation,
        )
        .instrument(attempt_span);
        let conversion = crate::translation::deliver(plan, model, stream, receiver, sender);
        tokio::join!(native, conversion);
    } else {
        dispatch_observed(
            state,
            tracker,
            target,
            body,
            headers,
            stream,
            sender,
            &mut observation,
        )
        .instrument(attempt_span)
        .await;
    }
    drop(analytics);
}

#[allow(clippy::too_many_arguments)] // Same native dispatch for both delivery modes.
async fn dispatch_observed(
    state: Arc<Ingress>,
    tracker: Option<SharedTracker>,
    target: RouteTarget,
    mut body: Value,
    headers: Vec<Header>,
    stream: bool,
    sender: mpsc::Sender<WireEvent>,
    observation: &mut Observation,
) {
    tracing::info!(event = "gateway_attempt_started");
    // The gateway alone resolves ingress aliases to the exact provider identifier.
    body["model"] = Value::String(target.model.as_str().to_owned());
    let credential = match state
        .resolver
        .resolve(
            target.reference.clone(),
            CredentialTarget {
                protocol: target.protocol.clone(),
                endpoint: target.endpoint.clone(),
            },
        )
        .await
    {
        Ok(credential) => credential,
        Err(error) => {
            if let Some(tracker) = &tracker {
                tracker
                    .lock()
                    .expect("owned analytics tracker")
                    .finish(AnalyticsOutcome::Failed);
            }
            observation.finish("credential_failed");
            tracing::warn!(event="gateway_credential_failed",kind=?error);
            send_failure(
                &sender,
                None,
                None,
                503,
                "认证资源不可用；请在认证设置中检查认证或重新导入来源。",
            )
            .await;
            return;
        }
    };
    if let Some(tracker) = &tracker {
        tracker.lock().expect("owned analytics tracker").begin();
    }
    match target.protocol {
        GatewayProtocol::ChatCompletionsV1 => {
            let adapter = match ChatCompletions::with_resolved_credential(
                target.config,
                state.client.clone(),
                credential,
            ) {
                Ok(value) => value,
                Err(_) => {
                    if let Some(tracker) = &tracker {
                        tracker
                            .lock()
                            .expect("owned analytics tracker")
                            .finish(AnalyticsOutcome::Failed);
                    }
                    observation.finish("configuration_failed");
                    send_failure(
                        &sender,
                        None,
                        None,
                        502,
                        "provider configuration is invalid",
                    )
                    .await;
                    return;
                }
            };
            let sink_sender = sender.clone();
            let sent = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let sink_sent = sent.clone();
            let sink_tracker = tracker.clone();
            let completion = adapter
                .chat_completions(
                    ChatCompletionsRequest {
                        model: target.model,
                        body: Payload::new(body),
                        stream,
                        headers,
                    },
                    Box::new(move |event| {
                        let sender = sink_sender.clone();
                        let sent = sink_sent.clone();
                        let tracker = sink_tracker.clone();
                        Box::pin(async move {
                            let event = match event {
                                ChatCompletionsEvent::Headers(meta) => {
                                    if let Some(tracker) = &tracker {
                                        tracker
                                            .lock()
                                            .expect("owned analytics tracker")
                                            .headers(meta.status);
                                    }
                                    tracing::info!(
                                        event = "gateway_upstream_headers",
                                        http_status = meta.status
                                    );
                                    sent.store(true, std::sync::atomic::Ordering::Release);
                                    WireEvent::Headers(meta)
                                }
                                ChatCompletionsEvent::Body(bytes) => {
                                    if let Some(tracker) = &tracker {
                                        tracker
                                            .lock()
                                            .expect("owned analytics tracker")
                                            .chunk(bytes.get());
                                    }
                                    WireEvent::Body(bytes.into_inner())
                                }
                            };
                            sender.send(event).await.map_err(|_| DeliveryError::Closed)
                        })
                    }),
                )
                .await;
            let headers_sent = sent.load(std::sync::atomic::Ordering::Acquire);
            if let Some(tracker) = &tracker {
                tracker
                    .lock()
                    .expect("owned analytics tracker")
                    .finish(match &completion.result {
                        Ok(_) => AnalyticsOutcome::Completed,
                        Err(error) if matches!(error.kind, ChatCompletionsErrorKind::Cancelled) => {
                            AnalyticsOutcome::Cancelled
                        }
                        Err(_) => AnalyticsOutcome::Failed,
                    });
            }
            observation.finish(match &completion.result {
                Ok(_) => "transport_completed",
                Err(error) if matches!(error.kind, ChatCompletionsErrorKind::Cancelled) => {
                    "downstream_closed"
                }
                Err(_) => "upstream_failed",
            });
            match completion.result {
                Ok(ChatCompletionsOutput::Json(body)) => {
                    tracing::info!(
                        event = "gateway_upstream_headers",
                        http_status = body.meta.status
                    );
                    if let Some(tracker) = &tracker {
                        tracker
                            .lock()
                            .expect("owned analytics tracker")
                            .json(body.meta.status, body.body.get());
                    }
                    send_json(&sender, body).await;
                }
                Ok(ChatCompletionsOutput::Stream(_)) => {}
                Err(error) => {
                    if matches!(error.kind, ChatCompletionsErrorKind::Cancelled) {
                        tracing::info!(event = "gateway_dispatch_canceled");
                    } else {
                        tracing::warn!(event="gateway_dispatch_failed",protocol="chat_completions",kind=?error.kind);
                    }
                    if let Some(tracker) = &tracker {
                        if let Some(body) = &error.body {
                            tracker.lock().expect("owned analytics tracker").json(
                                error.response.as_ref().map_or(502, |m| m.status),
                                body.get(),
                            );
                        } else if let Some(meta) = &error.response {
                            tracker
                                .lock()
                                .expect("owned analytics tracker")
                                .headers(meta.status);
                        }
                    }
                    if let Some(meta) = &error.response {
                        tracing::info!(
                            event = "gateway_upstream_headers",
                            http_status = meta.status
                        );
                    }
                    if headers_sent {
                        let _ = sender.send(WireEvent::Failed).await;
                    } else {
                        let status = match error.kind {
                            ChatCompletionsErrorKind::Authentication => 401,
                            ChatCompletionsErrorKind::Permission => 403,
                            ChatCompletionsErrorKind::RateLimited => 429,
                            ChatCompletionsErrorKind::Timeout => 504,
                            _ => 502,
                        };
                        send_failure(
                            &sender,
                            error.response,
                            error.body,
                            status,
                            "upstream request failed",
                        )
                        .await;
                    }
                }
            }
        }
        GatewayProtocol::ResponsesV1 => {
            let adapter = match OpenAiResponses::with_resolved_credential(
                target.config,
                state.client.clone(),
                credential,
            ) {
                Ok(value) => value,
                Err(_) => {
                    if let Some(tracker) = &tracker {
                        tracker
                            .lock()
                            .expect("owned analytics tracker")
                            .finish(AnalyticsOutcome::Failed);
                    }
                    observation.finish("configuration_failed");
                    send_failure(
                        &sender,
                        None,
                        None,
                        502,
                        "provider configuration is invalid",
                    )
                    .await;
                    return;
                }
            };
            let sink_sender = sender.clone();
            let sent = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let sink_sent = sent.clone();
            let sink_tracker = tracker.clone();
            let completion = adapter
                .responses(
                    ResponsesRequest {
                        model: target.model,
                        body: Payload::new(body),
                        stream,
                        headers,
                    },
                    Box::new(move |event| {
                        let sender = sink_sender.clone();
                        let sent = sink_sent.clone();
                        let tracker = sink_tracker.clone();
                        Box::pin(async move {
                            let event = match event {
                                ResponsesEvent::Headers(meta) => {
                                    if let Some(tracker) = &tracker {
                                        tracker
                                            .lock()
                                            .expect("owned analytics tracker")
                                            .headers(meta.status);
                                    }
                                    tracing::info!(
                                        event = "gateway_upstream_headers",
                                        http_status = meta.status
                                    );
                                    sent.store(true, std::sync::atomic::Ordering::Release);
                                    WireEvent::Headers(meta)
                                }
                                ResponsesEvent::Body(bytes) => {
                                    if let Some(tracker) = &tracker {
                                        tracker
                                            .lock()
                                            .expect("owned analytics tracker")
                                            .chunk(bytes.get());
                                    }
                                    WireEvent::Body(bytes.into_inner())
                                }
                            };
                            sender.send(event).await.map_err(|_| DeliveryError::Closed)
                        })
                    }),
                )
                .await;
            if let Some(tracker) = &tracker {
                tracker
                    .lock()
                    .expect("owned analytics tracker")
                    .finish(match &completion.result {
                        Ok(_) => AnalyticsOutcome::Completed,
                        Err(error) if matches!(error.kind, ResponsesErrorKind::Cancelled) => {
                            AnalyticsOutcome::Cancelled
                        }
                        Err(_) => AnalyticsOutcome::Failed,
                    });
            }
            observation.finish(match &completion.result {
                Ok(_) => "transport_completed",
                Err(error) if matches!(error.kind, ResponsesErrorKind::Cancelled) => {
                    "downstream_closed"
                }
                Err(_) => "upstream_failed",
            });
            match completion.result {
                Ok(ResponsesOutput::Json(body, _)) => {
                    tracing::info!(
                        event = "gateway_upstream_headers",
                        http_status = body.meta.status
                    );
                    if let Some(tracker) = &tracker {
                        tracker
                            .lock()
                            .expect("owned analytics tracker")
                            .json(body.meta.status, body.body.get());
                    }
                    send_json(&sender, body).await;
                }
                Ok(ResponsesOutput::Stream(..)) => {}
                Err(error) => {
                    if matches!(error.kind, ResponsesErrorKind::Cancelled) {
                        tracing::info!(event = "gateway_dispatch_canceled");
                    } else {
                        tracing::warn!(event="gateway_dispatch_failed",protocol="responses",kind=?error.kind);
                    }
                    if let Some(tracker) = &tracker {
                        if let Some(body) = &error.body {
                            tracker
                                .lock()
                                .expect("owned analytics tracker")
                                .json(error.status.unwrap_or(502), body.get());
                        } else if let Some(status) = error.status {
                            tracker
                                .lock()
                                .expect("owned analytics tracker")
                                .headers(status);
                        }
                    }
                    if let Some(status) = error.status {
                        tracing::info!(event = "gateway_upstream_headers", http_status = status);
                    }
                    if sent.load(std::sync::atomic::Ordering::Acquire) {
                        let _ = sender.send(WireEvent::Failed).await;
                    } else {
                        let status = error.status.unwrap_or(match error.kind {
                            ResponsesErrorKind::Authentication => 401,
                            ResponsesErrorKind::Permission => 403,
                            ResponsesErrorKind::RateLimited => 429,
                            ResponsesErrorKind::Timeout => 504,
                            _ => 502,
                        });
                        let meta = Some(ResponseMeta {
                            status,
                            headers: error.headers,
                        });
                        send_failure(&sender, meta, error.body, status, "upstream request failed")
                            .await;
                    }
                }
            }
        }
        GatewayProtocol::MessagesV1 => {
            let adapter = match Messages::with_resolved_credential(
                target.config,
                state.client.clone(),
                credential,
            ) {
                Ok(value) => value,
                Err(_) => {
                    if let Some(tracker) = &tracker {
                        tracker
                            .lock()
                            .expect("owned analytics tracker")
                            .finish(AnalyticsOutcome::Failed);
                    }
                    observation.finish("configuration_failed");
                    send_failure(
                        &sender,
                        None,
                        None,
                        502,
                        "provider configuration is invalid",
                    )
                    .await;
                    return;
                }
            };
            let sink_sender = sender.clone();
            let sent = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let sink_sent = sent.clone();
            let sink_tracker = tracker.clone();
            let completion = adapter
                .messages(
                    MessagesRequest {
                        model: target.model,
                        body: Payload::new(body),
                        stream,
                        headers,
                    },
                    Box::new(move |event| {
                        let sender = sink_sender.clone();
                        let sent = sink_sent.clone();
                        let tracker = sink_tracker.clone();
                        Box::pin(async move {
                            let event = match event {
                                MessagesEvent::Headers(meta) => {
                                    if let Some(tracker) = &tracker {
                                        tracker
                                            .lock()
                                            .expect("owned analytics tracker")
                                            .headers(meta.status);
                                    }
                                    tracing::info!(
                                        event = "gateway_upstream_headers",
                                        http_status = meta.status
                                    );
                                    sent.store(true, std::sync::atomic::Ordering::Release);
                                    WireEvent::Headers(meta)
                                }
                                MessagesEvent::Body(bytes) => {
                                    if let Some(tracker) = &tracker {
                                        tracker
                                            .lock()
                                            .expect("owned analytics tracker")
                                            .chunk(bytes.get());
                                    }
                                    WireEvent::Body(bytes.into_inner())
                                }
                            };
                            sender.send(event).await.map_err(|_| DeliveryError::Closed)
                        })
                    }),
                )
                .await;
            let headers_sent = sent.load(std::sync::atomic::Ordering::Acquire);
            if let Some(tracker) = &tracker {
                tracker
                    .lock()
                    .expect("owned analytics tracker")
                    .finish(match &completion.result {
                        Ok(_) => AnalyticsOutcome::Completed,
                        Err(error) if matches!(error.kind, MessagesErrorKind::Cancelled) => {
                            AnalyticsOutcome::Cancelled
                        }
                        Err(_) => AnalyticsOutcome::Failed,
                    });
            }
            observation.finish(match &completion.result {
                Ok(_) => "transport_completed",
                Err(error) if matches!(error.kind, MessagesErrorKind::Cancelled) => {
                    "downstream_closed"
                }
                Err(_) => "upstream_failed",
            });
            match completion.result {
                Ok(MessagesOutput::Json(body)) => {
                    tracing::info!(
                        event = "gateway_upstream_headers",
                        http_status = body.meta.status
                    );
                    if let Some(tracker) = &tracker {
                        tracker
                            .lock()
                            .expect("owned analytics tracker")
                            .json(body.meta.status, body.body.get());
                    }
                    send_json(&sender, body).await;
                }
                Ok(MessagesOutput::Stream(_)) => {}
                Err(error) => {
                    if matches!(error.kind, MessagesErrorKind::Cancelled) {
                        tracing::info!(event = "gateway_dispatch_canceled");
                    } else {
                        tracing::warn!(event="gateway_dispatch_failed",protocol="messages",kind=?error.kind);
                    }
                    if let Some(tracker) = &tracker {
                        if let Some(body) = &error.body {
                            tracker.lock().expect("owned analytics tracker").json(
                                error.response.as_ref().map_or(502, |m| m.status),
                                body.get(),
                            );
                        } else if let Some(meta) = &error.response {
                            tracker
                                .lock()
                                .expect("owned analytics tracker")
                                .headers(meta.status);
                        }
                    }
                    if let Some(meta) = &error.response {
                        tracing::info!(
                            event = "gateway_upstream_headers",
                            http_status = meta.status
                        );
                    }
                    if headers_sent {
                        let _ = sender.send(WireEvent::Failed).await;
                    } else {
                        let status = match error.kind {
                            MessagesErrorKind::InvalidInput => 400,
                            MessagesErrorKind::Authentication => 401,
                            MessagesErrorKind::Permission => 403,
                            MessagesErrorKind::RateLimited => 429,
                            MessagesErrorKind::Timeout => 504,
                            _ => 502,
                        };
                        send_failure(
                            &sender,
                            error.response,
                            error.body,
                            status,
                            "upstream request failed",
                        )
                        .await;
                    }
                }
            }
        }
    }
}
