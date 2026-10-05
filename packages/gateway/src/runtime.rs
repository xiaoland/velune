//! Loopback OpenAI protocol ingress and explicit provider routing.
//!
//! This is a deliberately narrow ingress: one explicit model route, no
//! fallback, and no LAN listener. Credential references are resolved only by
//! the host-provided platform helper at dispatch time.
use crate::config::{GatewayConfig, GatewayProtocol};
use reqwest::Client;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use velune_ai::{
    AiService, Payload,
    direct::DirectAiService,
    ids::{AttemptId, CallId, ConfigRevision, ModelId, ProviderId},
    observation::Quantity,
    provider::ProviderBinding,
    responses::{
        DirectResponsesService, ResponsesBinding, ResponsesEvent, ResponsesOutput,
        ResponsesProvider, ResponsesRequest,
    },
    sampling::{
        FinishReason, Message, SamplingDelta, SamplingEvent, SamplingEventKind, SamplingInput,
        ToolCall, ToolDefinition,
    },
};
use velune_ai_provider::{
    config::{
        ChatCompletionsConfig, CredentialRef, HttpEndpoint, ProtocolConfig, ProviderConfig,
        ResponsesConfig, Transport,
    },
    openai::ChatCompletions,
    responses::OpenAiResponses,
};

const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_HEADERS_BYTES: usize = 64 * 1024;
const MAX_REASONING_LEVEL_BYTES: usize = 64;

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
    logical_model_id: String,
    model: crate::config::ModelDefinition,
    provider_id: String,
    native_responses_reasoning_efforts: Option<Vec<String>>,
    service: RouteService,
}

#[derive(Clone)]
enum RouteService {
    Chat(Arc<dyn AiService>),
    Responses(Arc<dyn ResponsesProvider>),
}

pub struct Runner {
    endpoint: String,
    token: String,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Runner {
    pub fn start(
        config: GatewayConfig,
        credential_resolver: Option<PathBuf>,
        route_aliases: BTreeMap<String, String>,
        native_responses_constraints: BTreeMap<String, Vec<String>>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        config.validate().map_err(GatewayError)?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let token = ephemeral_token()?;
        let routes = build_routes(
            &config,
            credential_resolver,
            &route_aliases,
            &native_responses_constraints,
        )?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_token = token.clone();
        let dispatcher = tracing::dispatcher::get_default(Clone::clone);
        let parent = tracing::Span::current();
        let handle = thread::Builder::new()
            .name("velune-ai-gateway".into())
            .spawn(move || tracing::dispatcher::with_default(&dispatcher, || {
                let _entered = parent.enter();
                tracing::info!(event = "gateway_started");
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(_) => { tracing::error!(event = "gateway_executor_failed"); return; },
                };
                while !thread_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let result = handle_connection(
                                stream,
                                &thread_token,
                                &routes,
                                &runtime,
                                &thread_stop,
                            );
                            if result.is_err() { tracing::warn!(event = "gateway_connection_failed"); }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => { tracing::warn!(event = "gateway_listener_failed", io_kind = ?error.kind()); break; },
                    }
                }
                tracing::info!(event = "gateway_stopped");
            }))?;
        Ok(Self {
            endpoint: format!("http://127.0.0.1:{}/v1", address.port()),
            token,
            stop,
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
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

static TOKEN: AtomicU64 = AtomicU64::new(1);

fn ephemeral_token() -> Result<String, Box<dyn std::error::Error>> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| GatewayError("gateway token entropy"))?;
    let mut token = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut token, "{byte:02x}").expect("string write");
    }
    Ok(token)
}

fn build_routes(
    config: &GatewayConfig,
    credential_resolver: Option<PathBuf>,
    route_aliases: &BTreeMap<String, String>,
    native_responses_constraints: &BTreeMap<String, Vec<String>>,
) -> Result<BTreeMap<String, RouteTarget>, Box<dyn std::error::Error>> {
    let revision = ConfigRevision::new(1).map_err(|_| GatewayError("gateway revision"))?;
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .build()?;
    let required_providers: BTreeSet<_> = config
        .routes
        .iter()
        .map(|route| route.provider_id.as_str())
        .collect();
    let mut services = BTreeMap::<String, RouteService>::new();
    for provider in &config.providers {
        if !required_providers.contains(provider.id.as_str()) {
            continue;
        }
        if !crate::config::credential_ready(provider) {
            return Err(Box::new(GatewayError("provider credential is required")));
        }
        if matches!(provider.protocol, GatewayProtocol::MessagesV1) {
            return Err(Box::new(GatewayError("provider protocol is unsupported")));
        }
        let endpoint = parse_endpoint(&provider.endpoint)?;
        let model_ids = provider
            .models
            .iter()
            .map(|binding| ModelId::new(binding.model_id.clone()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| GatewayError("provider model id"))?;
        let protocol = match provider.protocol {
            GatewayProtocol::ChatCompletionsV1 => {
                ProtocolConfig::ChatCompletions(ChatCompletionsConfig { endpoint })
            }
            GatewayProtocol::ResponsesV1 => ProtocolConfig::Responses(ResponsesConfig { endpoint }),
            GatewayProtocol::MessagesV1 => unreachable!("unsupported protocol checked above"),
        };
        let credential_ref = provider
            .credential_ref
            .clone()
            .unwrap_or_else(|| provider.id.clone());
        let credential = CredentialRef::new(credential_ref)
            .map_err(|_| GatewayError("provider credential reference"))?;
        let source_json = provider
            .credential_source
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| GatewayError("provider credential source"))?;
        let provider_config = ProviderConfig::new(
            ProviderId::new(provider.id.clone()).map_err(|_| GatewayError("provider id"))?,
            revision,
            protocol,
            credential,
            provider
                .models
                .iter()
                .zip(model_ids.iter())
                .map(|(binding, id)| {
                    velune_ai_provider::config::ModelMapping::new(
                        id.clone(),
                        binding.external_model_id.clone(),
                    ).map(|mapping| {
                        use velune_ai_provider::config::ChatCompletionsOutputLimitField as WireField;
                        let field = match binding.chat_completions_output_limit_field {
                            crate::config::ChatCompletionsOutputLimitField::MaxTokens => WireField::MaxTokens,
                            crate::config::ChatCompletionsOutputLimitField::MaxCompletionTokens => WireField::MaxCompletionTokens,
                        };
                        mapping.with_chat_completions_output_limit_field(field)
                    })
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| GatewayError("provider model mapping"))?,
        )
        .map_err(|_| GatewayError("provider configuration"))?
        .with_credential_source(source_json);
        let resolver = credential_resolver.clone();
        let expected_protocol = match provider.protocol {
            GatewayProtocol::ChatCompletionsV1 => "chatCompletionsV1",
            GatewayProtocol::ResponsesV1 => "responsesV1",
            GatewayProtocol::MessagesV1 => unreachable!("unsupported protocol checked above"),
        };
        let expected_endpoint = provider.endpoint.clone();
        let resolver = Arc::new(move |reference: &str, source: Option<&str>| {
            let path = resolver.as_ref()?;
            let mut command = Command::new(path);
            if let Some(source) = source {
                command.arg("--source-json").arg(source);
            } else {
                command.arg(reference);
            }
            let output = command.output().ok()?;
            if !output.status.success() {
                return None;
            }
            let value = String::from_utf8(output.stdout).ok()?;
            let value = value.trim();
            if source.is_some() {
                let value: Value = serde_json::from_str(value).ok()?;
                if value["contractVersion"] != 1 {
                    return None;
                }
                if value["capabilities"]["protocol"] != expected_protocol
                    || value["capabilities"]["endpoint"] != expected_endpoint
                {
                    return None;
                }
                let bearer = value["bearer"].as_str()?.trim();
                (!bearer.is_empty()).then(|| value.to_string())
            } else {
                (!value.is_empty()).then(|| value.to_owned())
            }
        });
        let service = match provider.protocol {
            GatewayProtocol::ChatCompletionsV1 => {
                let adapter =
                    ChatCompletions::with_resolver(provider_config, client.clone(), resolver)
                        .map_err(|_| GatewayError("provider adapter"))?;
                let binding = ProviderBinding::new(
                    ProviderId::new(provider.id.clone())
                        .map_err(|_| GatewayError("provider id"))?,
                    revision,
                    model_ids,
                    Arc::new(adapter),
                )
                .map_err(|_| GatewayError("provider binding"))?;
                RouteService::Chat(Arc::new(DirectAiService::new(Arc::new(binding))))
            }
            GatewayProtocol::ResponsesV1 => {
                let adapter =
                    OpenAiResponses::with_resolver(provider_config, client.clone(), resolver)
                        .map_err(|_| GatewayError("provider adapter"))?;
                let binding = ResponsesBinding::new(
                    ProviderId::new(provider.id.clone())
                        .map_err(|_| GatewayError("provider id"))?,
                    model_ids,
                    Arc::new(adapter),
                )
                .map_err(|_| GatewayError("provider binding"))?;
                RouteService::Responses(Arc::new(DirectResponsesService::new(Arc::new(binding))))
            }
            GatewayProtocol::MessagesV1 => unreachable!("unsupported protocol checked above"),
        };
        services.insert(provider.id.clone(), service);
    }
    let mut routes = BTreeMap::new();
    for route in &config.routes {
        let model = config
            .model(&route.model_id)
            .ok_or(GatewayError("model route is missing"))?;
        let provider_id = route.provider_id.clone();
        let native_responses_reasoning_efforts =
            native_responses_constraints.get(&model.id).cloned();
        let service = services
            .get(&provider_id)
            .ok_or(GatewayError("route provider is missing"))?;
        let target = RouteTarget {
            logical_model_id: model.id.clone(),
            model: model.clone(),
            provider_id,
            native_responses_reasoning_efforts,
            service: match service {
                RouteService::Chat(value) => RouteService::Chat(Arc::clone(value)),
                RouteService::Responses(value) => RouteService::Responses(Arc::clone(value)),
            },
        };
        if routes.insert(model.id.clone(), target).is_some() {
            return Err(Box::new(GatewayError("gateway binding identity collision")));
        }
    }
    for (alias, logical_model_id) in route_aliases {
        let target = routes
            .get(logical_model_id)
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

fn handle_connection(
    mut stream: TcpStream,
    token: &str,
    routes: &BTreeMap<String, RouteTarget>,
    runtime: &tokio::runtime::Runtime,
    stop: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Accepted sockets can inherit the listener's nonblocking mode on macOS.
    // Request parsing uses bounded blocking reads, not a readiness loop.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err(error) => {
            write_json(
                &mut stream,
                "400 Bad Request",
                json!({"error":{"message":error.to_string()}}),
            )?;
            return Ok(());
        }
    };
    if request.method != "POST"
        || !matches!(
            request.path.as_str(),
            "/v1/chat/completions" | "/v1/responses"
        )
    {
        write_json(
            &mut stream,
            "404 Not Found",
            json!({"error":{"message":"unsupported gateway path"}}),
        )?;
        return Ok(());
    }
    if request.headers.get("authorization").map(String::as_str) != Some(&format!("Bearer {token}"))
    {
        write_json(
            &mut stream,
            "401 Unauthorized",
            json!({"error":{"message":"gateway authorization failed"}}),
        )?;
        return Ok(());
    }
    let body: Value = match serde_json::from_slice(&request.body) {
        Ok(value) => value,
        Err(_) => {
            write_json(
                &mut stream,
                "400 Bad Request",
                json!({"error":{"message":"gateway request JSON"}}),
            )?;
            return Ok(());
        }
    };
    let Some(model_id) = body["model"].as_str() else {
        write_json(
            &mut stream,
            "400 Bad Request",
            json!({"error":{"message":"gateway model"}}),
        )?;
        return Ok(());
    };
    let Some(target) = routes.get(model_id) else {
        write_json(
            &mut stream,
            "422 Unprocessable Entity",
            json!({"error":{"message":"gateway model route"}}),
        )?;
        return Ok(());
    };
    match (request.path.as_str(), &target.service) {
        ("/v1/chat/completions", RouteService::Chat(service)) => {
            handle_chat_request(stream, token, stop, runtime, target, &body, service)
        }
        ("/v1/responses", RouteService::Responses(service)) => {
            handle_responses_request(stream, token, stop, runtime, target, &body, service)
        }
        _ => {
            write_json(
                &mut stream,
                "422 Unprocessable Entity",
                json!({"error":{"message":"gateway provider protocol does not match endpoint"}}),
            )?;
            Ok(())
        }
    }
}

fn handle_chat_request(
    mut stream: TcpStream,
    _token: &str,
    stop: &AtomicBool,
    runtime: &tokio::runtime::Runtime,
    target: &RouteTarget,
    body: &Value,
    service: &Arc<dyn AiService>,
) -> Result<(), Box<dyn std::error::Error>> {
    let model_id = target.logical_model_id.as_str();
    let input = sampling_input(
        body,
        target.model.max_output_tokens,
        &target.model.reasoning_levels,
    )
    .map_err(|error| {
        let _ = write_json(
            &mut stream,
            "400 Bad Request",
            json!({"error":{"message":error.to_string()}}),
        );
        GatewayError("gateway chat request")
    })?;
    let call = CallId::new(format!(
        "gateway-call-{}",
        TOKEN.fetch_add(1, Ordering::Relaxed)
    ))
    .map_err(|_| GatewayError("gateway call id"))?;
    let attempt = AttemptId::new(format!(
        "gateway-attempt-{}",
        TOKEN.fetch_add(1, Ordering::Relaxed)
    ))
    .map_err(|_| GatewayError("gateway attempt id"))?;
    let request = velune_ai::sampling::SamplingRequest {
        call_id: call,
        provider: ProviderId::new(target.provider_id.clone())
            .map_err(|_| GatewayError("gateway provider id"))?,
        model: ModelId::new(model_id).map_err(|_| GatewayError("gateway model id"))?,
        input,
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
    )?;
    stream.flush()?;
    let monitor = stream.try_clone()?;
    monitor.set_read_timeout(Some(Duration::from_millis(20)))?;
    let output = Arc::new(Mutex::new(stream));
    let sink_output = Arc::clone(&output);
    let cancelled = Arc::new(AtomicBool::new(false));
    let sink_cancelled = Arc::clone(&cancelled);
    let prepared = service
        .sampling(
            request,
            attempt,
            Box::new(move |event| {
                let mut stream = sink_output.lock().expect("gateway stream");
                if write_event(&mut stream, event).is_err() {
                    sink_cancelled.store(true, Ordering::Release);
                }
            }),
        )
        .map_err(|_| GatewayError("gateway request preparation"))?;
    let completion = runtime.block_on(async {
        tokio::pin!(prepared);
        loop {
            tokio::select! {
                result = &mut prepared => break Some(result),
                _ = tokio::time::sleep(Duration::from_millis(25)) => {
                    if stop.load(Ordering::Acquire)
                        || cancelled.load(Ordering::Acquire)
                        || socket_closed(&monitor)
                    {
                        break None;
                    }
                }
            }
        }
    });
    let Some(completion) = completion else {
        return Ok(());
    };
    if let Err(failure) = completion.outcome.result {
        let mut stream = output.lock().expect("gateway stream");
        let body =
            json!({"error":{"message":format!("provider request failed: {:?}", failure.kind)}});
        let _ = write_sse(&mut stream, &body);
    }
    Ok(())
}

fn handle_responses_request(
    mut stream: TcpStream,
    _token: &str,
    stop: &AtomicBool,
    runtime: &tokio::runtime::Runtime,
    target: &RouteTarget,
    body: &Value,
    service: &Arc<dyn ResponsesProvider>,
) -> Result<(), Box<dyn std::error::Error>> {
    let stream_response = match validate_responses_body(
        body,
        target.model.max_output_tokens,
        target
            .native_responses_reasoning_efforts
            .as_deref()
            .unwrap_or(&target.model.reasoning_levels),
    ) {
        Ok(stream) => stream,
        Err(error) => {
            write_json(
                &mut stream,
                "400 Bad Request",
                json!({"error":{"message":error.to_string()}}),
            )?;
            return Ok(());
        }
    };
    let model =
        ModelId::new(target.model.id.clone()).map_err(|_| GatewayError("gateway model id"))?;
    let request = ResponsesRequest {
        model,
        body: Payload::new(body.clone()),
        stream: stream_response,
    };
    let monitor = stream.try_clone()?;
    monitor.set_read_timeout(Some(Duration::from_millis(20)))?;
    let output = Arc::new(Mutex::new(stream));
    let sink_output = Arc::clone(&output);
    let headers_written = Arc::new(AtomicBool::new(false));
    let sink_headers_written = Arc::clone(&headers_written);
    let cancelled = Arc::new(AtomicBool::new(false));
    let sink_cancelled = Arc::clone(&cancelled);
    let prepared = service.responses(
        request,
        Box::new(move |event| {
            let mut stream = sink_output.lock().expect("gateway stream");
            let result = match event {
                ResponsesEvent::Headers {
                    status,
                    content_type,
                } => {
                    let content_type = content_type
                        .as_deref()
                        .unwrap_or("text/event-stream");
                    let result = write!(
                        stream,
                        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
                        status,
                        status_reason(status),
                        content_type
                    )
                    .and_then(|_| stream.flush());
                    if result.is_ok() {
                        sink_headers_written.store(true, Ordering::Release);
                    }
                    result
                }
                ResponsesEvent::Body(body) => stream.write_all(body.get()),
            };
            if result.is_err() {
                sink_cancelled.store(true, Ordering::Release);
            }
        }),
    );
    let completion = runtime.block_on(async {
        tokio::pin!(prepared);
        loop {
            tokio::select! {
                result = &mut prepared => break Some(result),
                _ = tokio::time::sleep(Duration::from_millis(25)) => {
                    if stop.load(Ordering::Acquire)
                        || cancelled.load(Ordering::Acquire)
                        || socket_closed(&monitor)
                    {
                        break None;
                    }
                }
            }
        }
    });
    let Some(completion) = completion else {
        return Ok(());
    };
    match completion.result {
        Ok(ResponsesOutput::Json(body, _terminal)) => {
            let mut stream = output.lock().expect("gateway stream");
            let body = body.get();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(body)?;
        }
        Ok(ResponsesOutput::Stream(_terminal)) => {
            if !headers_written.load(Ordering::Acquire) {
                let mut stream = output.lock().expect("gateway stream");
                write_json(
                    &mut stream,
                    "502 Bad Gateway",
                    json!({"error":{"message":"Responses stream ended without headers"}}),
                )?;
            }
        }
        Err(failure) => {
            if !headers_written.load(Ordering::Acquire) {
                let mut stream = output.lock().expect("gateway stream");
                if let (Some(status), Some(body)) = (failure.status, failure.body) {
                    let body = body.get();
                    write!(
                        stream,
                        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        status,
                        status_reason(status),
                        body.len()
                    )?;
                    stream.write_all(body)?;
                } else {
                    write_json(
                        &mut stream,
                        responses_status(failure.kind),
                        json!({"error":{"message":format!("provider request failed: {:?}", failure.kind)}}),
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn validate_responses_body(
    body: &Value,
    max_output_tokens: u32,
    reasoning_levels: &[String],
) -> Result<bool, Box<dyn std::error::Error>> {
    if !body.is_object() || body.get("input").is_none() {
        return Err(Box::new(GatewayError("Responses input is required")));
    }
    if body.get("background").and_then(Value::as_bool) == Some(true) {
        return Err(Box::new(GatewayError(
            "Responses background is unsupported",
        )));
    }
    let stream = match body.get("stream") {
        None => false,
        Some(value) => value
            .as_bool()
            .ok_or(GatewayError("Responses stream must be boolean"))?,
    };
    if let Some(value) = body.get("max_output_tokens") {
        let requested = value
            .as_u64()
            .ok_or(GatewayError("Responses output limit"))?;
        if requested == 0 || requested > u64::from(max_output_tokens) {
            return Err(Box::new(GatewayError("Responses output limit")));
        }
    }
    if let Some(reasoning) = body.get("reasoning") {
        if !reasoning.is_object() {
            return Err(Box::new(GatewayError("Responses reasoning")));
        }
        if let Some(effort) = reasoning.get("effort") {
            let effort = effort.as_str().ok_or(GatewayError("Responses reasoning"))?;
            if effort.len() > MAX_REASONING_LEVEL_BYTES
                || !reasoning_levels.iter().any(|level| level == effort)
            {
                return Err(Box::new(GatewayError("Responses reasoning")));
            }
        }
    }
    Ok(stream)
}

fn responses_status(kind: velune_ai::responses::ResponsesErrorKind) -> &'static str {
    use velune_ai::responses::ResponsesErrorKind;
    match kind {
        ResponsesErrorKind::InvalidInput => "400 Bad Request",
        ResponsesErrorKind::Authentication => "401 Unauthorized",
        ResponsesErrorKind::Permission => "403 Forbidden",
        ResponsesErrorKind::RateLimited => "429 Too Many Requests",
        ResponsesErrorKind::Timeout => "504 Gateway Timeout",
        ResponsesErrorKind::Unsupported => "422 Unprocessable Entity",
        ResponsesErrorKind::Cancelled => "499 Client Closed Request",
        ResponsesErrorKind::Transport | ResponsesErrorKind::ProviderFailure => "502 Bad Gateway",
    }
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        499 => "Client Closed Request",
        502 => "Bad Gateway",
        504 => "Gateway Timeout",
        _ => "Upstream Response",
    }
}

fn socket_closed(stream: &TcpStream) -> bool {
    let mut byte = [0_u8; 1];
    match stream.peek(&mut byte) {
        Ok(0) => true,
        Ok(_) => false,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            false
        }
        Err(_) => true,
    }
}

fn write_event(stream: &mut TcpStream, event: SamplingEvent) -> std::io::Result<()> {
    match event.kind {
        SamplingEventKind::Delta(delta) => match delta {
            SamplingDelta::Text(text) => write_sse(
                stream,
                &json!({
                    "id":"velune",
                    "object":"chat.completion.chunk",
                    "choices":[{"index":0,"delta":{"role":"assistant","content":text.get()},"finish_reason":null}]
                }),
            ),
            SamplingDelta::ToolIdentity { index, id, name } => write_sse(
                stream,
                &json!({
                    "id":"velune",
                    "object":"chat.completion.chunk",
                    "choices":[{"index":0,"delta":{"tool_calls":[{"index":index,"id":id.as_ref().map(|v|v.as_str()),"type":"function","function":{"name":name.as_ref().map(|v|v.as_str())}}]},"finish_reason":null}]
                }),
            ),
            SamplingDelta::ToolArguments { index, fragment } => write_sse(
                stream,
                &json!({
                    "id":"velune",
                    "object":"chat.completion.chunk",
                    "choices":[{"index":0,"delta":{"tool_calls":[{"index":index,"function":{"arguments":fragment.get()}}]},"finish_reason":null}]
                }),
            ),
            SamplingDelta::Finish(reason) => write_sse(
                stream,
                &json!({
                    "id":"velune",
                    "object":"chat.completion.chunk",
                    "choices":[{"index":0,"delta":{},"finish_reason":finish_name(reason)}]
                }),
            ),
            SamplingDelta::Usage(usage) => write_sse(
                stream,
                &json!({
                    "id":"velune",
                    "object":"chat.completion.chunk",
                    "choices":[],
                    "usage":usage_json(usage)
                }),
            ),
        },
        SamplingEventKind::Terminal { error: None } => stream.write_all(b"data: [DONE]\n\n"),
        SamplingEventKind::Terminal { error: Some(error) } => write_sse(
            stream,
            &json!({"error":{"message":format!("provider request failed: {error:?}")}}),
        ),
    }
}

fn finish_name(reason: FinishReason) -> &'static str {
    match reason {
        FinishReason::Complete => "stop",
        FinishReason::OutputLimit => "length",
        FinishReason::ToolCalls => "tool_calls",
    }
}

fn usage_json(usage: velune_ai::observation::Usage) -> Value {
    let mut value = serde_json::Map::new();
    if let Quantity::Reported(input) = usage.input_tokens {
        value.insert("prompt_tokens".into(), json!(input));
    }
    if let Quantity::Reported(output) = usage.output_tokens {
        value.insert("completion_tokens".into(), json!(output));
    }
    Value::Object(value)
}

fn write_sse(stream: &mut TcpStream, body: &Value) -> std::io::Result<()> {
    let body = serde_json::to_string(body).unwrap_or_else(|_| "{}".into());
    write!(stream, "data: {body}\n\n")?;
    stream.flush()
}

struct HttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, Box<dyn std::error::Error>> {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while head.len() < MAX_HEADERS_BYTES {
        stream.read_exact(&mut byte)?;
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let split = head
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(GatewayError("gateway headers"))?;
    let header_text =
        std::str::from_utf8(&head[..split]).map_err(|_| GatewayError("gateway headers"))?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or(GatewayError("gateway request line"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or(GatewayError("gateway method"))?
        .to_owned();
    let path = parts.next().ok_or(GatewayError("gateway path"))?.to_owned();
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or(GatewayError("gateway content length"))?;
    if length > MAX_REQUEST_BYTES {
        return Err(Box::new(GatewayError("gateway request too large")));
    }
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body)?;
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn write_json(stream: &mut TcpStream, status: &str, body: Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)
}

fn sampling_input(
    body: &Value,
    max: u32,
    reasoning_levels: &[String],
) -> Result<SamplingInput, Box<dyn std::error::Error>> {
    if body["stream"].as_bool() != Some(true) {
        return Err(Box::new(GatewayError("gateway requires streaming")));
    }
    if body["store"].as_bool() == Some(true) {
        return Err(Box::new(GatewayError("gateway store is unsupported")));
    }
    let requested = body["max_completion_tokens"]
        .as_u64()
        .or_else(|| body["max_tokens"].as_u64())
        .ok_or(GatewayError("gateway output limit"))?;
    let requested = u32::try_from(requested).map_err(|_| GatewayError("gateway output limit"))?;
    if requested == 0 || requested > max {
        return Err(Box::new(GatewayError("gateway output limit")));
    }
    let mut instructions = None;
    let mut messages = Vec::new();
    for message in body["messages"]
        .as_array()
        .ok_or(GatewayError("gateway messages"))?
    {
        let role = message["role"]
            .as_str()
            .ok_or(GatewayError("gateway message role"))?;
        match role {
            "system" | "developer" => {
                instructions = Some(Payload::new(message_text(
                    &message["content"],
                    "gateway instructions",
                )?));
            }
            "user" => messages.push(Message::User(Payload::new(message_text(
                &message["content"],
                "gateway user content",
            )?))),
            "assistant" => {
                let text = if message["content"].is_null() {
                    None
                } else {
                    Some(Payload::new(message_text(
                        &message["content"],
                        "gateway assistant content",
                    )?))
                };
                let mut calls = Vec::new();
                if let Some(tool_values) = message["tool_calls"].as_array() {
                    for tool in tool_values {
                        let id = velune_ai::ids::ToolCallId::new(
                            tool["id"]
                                .as_str()
                                .ok_or(GatewayError("gateway tool id"))?
                                .to_owned(),
                        )
                        .map_err(|_| GatewayError("gateway tool id"))?;
                        let name = velune_ai::ids::ToolName::new(
                            tool.pointer("/function/name")
                                .and_then(Value::as_str)
                                .ok_or(GatewayError("gateway tool name"))?
                                .to_owned(),
                        )
                        .map_err(|_| GatewayError("gateway tool name"))?;
                        let arguments = tool
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .ok_or(GatewayError("gateway tool arguments"))?;
                        let arguments: Value = serde_json::from_str(arguments)
                            .map_err(|_| GatewayError("gateway tool arguments"))?;
                        calls.push(ToolCall {
                            id,
                            name,
                            arguments: Payload::new(arguments),
                        });
                    }
                }
                messages.push(Message::Assistant {
                    text,
                    tool_calls: calls,
                });
            }
            "tool" => messages.push(Message::ToolResult {
                call: velune_ai::ids::ToolCallId::new(
                    message["tool_call_id"]
                        .as_str()
                        .ok_or(GatewayError("gateway tool result id"))?
                        .to_owned(),
                )
                .map_err(|_| GatewayError("gateway tool result id"))?,
                content: Payload::new(message_text(&message["content"], "gateway tool result")?),
                is_error: false,
            }),
            _ => return Err(Box::new(GatewayError("gateway message role"))),
        }
    }
    if messages.is_empty() {
        return Err(Box::new(GatewayError("gateway messages")));
    }
    let mut input = SamplingInput::new(
        instructions,
        messages,
        parse_tools(&body["tools"])?,
        requested,
    )?;
    if let Some(reasoning) = body["reasoning_effort"].as_str() {
        if reasoning.len() > MAX_REASONING_LEVEL_BYTES {
            return Err(Box::new(GatewayError("gateway reasoning level")));
        }
        if !reasoning_levels.iter().any(|level| level == reasoning) {
            return Err(Box::new(GatewayError("gateway reasoning level")));
        }
        input = input
            .with_reasoning_level(Some(reasoning.to_owned()))
            .map_err(|_| GatewayError("gateway reasoning level"))?;
    }
    Ok(input)
}

fn message_text(value: &Value, error: &'static str) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_owned());
    }
    let Some(parts) = value.as_array() else {
        return Err(Box::new(GatewayError(error)));
    };
    let mut output = String::new();
    for part in parts {
        if part["type"] != "text" {
            return Err(Box::new(GatewayError(
                "gateway content part is unsupported",
            )));
        }
        output.push_str(
            part["text"]
                .as_str()
                .ok_or(GatewayError("gateway text part"))?,
        );
    }
    Ok(output)
}

fn parse_tools(value: &Value) -> Result<Vec<ToolDefinition>, Box<dyn std::error::Error>> {
    let Some(tools) = value.as_array() else {
        return Ok(Vec::new());
    };
    tools
        .iter()
        .map(|tool| {
            let function = &tool["function"];
            let name = velune_ai::ids::ToolName::new(
                function["name"]
                    .as_str()
                    .ok_or(GatewayError("gateway tool name"))?
                    .to_owned(),
            )
            .map_err(|_| GatewayError("gateway tool name"))?;
            ToolDefinition::new(
                name,
                Payload::new(
                    function["description"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                ),
                function["parameters"].clone(),
            )
            .map_err(|_| GatewayError("gateway tool schema").into())
        })
        .collect()
}
