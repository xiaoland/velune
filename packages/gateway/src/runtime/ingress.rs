//! Admission: authenticate the caller, read its request and pick its route.
use super::{
    ExecutionBackends, TelemetryContext,
    attempts::{Exchange, dispatch},
    credentials::CredentialResolver,
    routes::{Route, RouteTarget},
    wire::{
        DispatchGuard, WireEvent, error_response, forward_headers, rejected, response,
        scheduling_rejected,
    },
};
use crate::{
    analytics::{
        AnalyticsOutcome, AnalyticsRecord, RequestAnalytics, SharedAnalyticsSink, TokenUsage,
        now_ms,
    },
    config::{ExecutionBackend, Protocol},
    observation::Observation,
    scheduling::{Candidate, QuotaSource, ScheduleUnavailable, SchedulingState, SessionIdentity},
};
use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::State,
    http::{Request, Response, StatusCode},
    routing::post,
};
use futures_util::stream;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tracing::{Instrument, instrument::WithSubscriber};
use velune_ai_provider::{HttpClient, claude_code::Continuation};

const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(15);

/// The state every request on one Runner shares.
pub(super) struct Ingress {
    pub(super) telemetry: Arc<RwLock<TelemetryContext>>,
    pub(super) routes: Arc<RwLock<BTreeMap<String, Route>>>,
    pub(super) token: String,
    pub(super) resolver: Arc<dyn CredentialResolver>,
    pub(super) client: HttpClient,
    pub(super) request_epoch: u64,
    pub(super) next_request: AtomicU64,
    pub(super) stopping: Arc<AtomicBool>,
    pub(super) analytics: Option<SharedAnalyticsSink>,
    pub(super) health: SchedulingState,
    pub(super) gateway_id: String,
    pub(super) provider_epochs: BTreeMap<String, u64>,
    pub(super) quotas: Arc<dyn QuotaSource>,
    pub(super) session: Option<SessionIdentity>,
    pub(super) backends: ExecutionBackends,
    pub(super) backend_bindings: BTreeMap<String, String>,
}

pub(super) fn router(state: Arc<Ingress>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(chat))
        .route("/v1/responses", post(responses))
        .route("/v1/messages", post(messages))
        .with_state(state)
}

async fn chat(State(state): State<Arc<Ingress>>, request: Request<Body>) -> Response<Body> {
    ingress(state, request, Protocol::ChatCompletions).await
}
async fn messages(State(state): State<Arc<Ingress>>, request: Request<Body>) -> Response<Body> {
    ingress(state, request, Protocol::Messages).await
}
async fn responses(State(state): State<Arc<Ingress>>, request: Request<Body>) -> Response<Body> {
    ingress(state, request, Protocol::Responses).await
}

async fn ingress(
    state: Arc<Ingress>,
    request: Request<Body>,
    protocol: Protocol,
) -> Response<Body> {
    let request_id = format!(
        "{:x}-{:x}",
        state.request_epoch,
        state.next_request.fetch_add(1, Ordering::Relaxed)
    );
    let protocol_name = match protocol {
        Protocol::ChatCompletions => "chat_completions_v1",
        Protocol::Responses => "responses_v1",
        Protocol::Messages => "messages_v1",
    };
    let mut telemetry = state
        .telemetry
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let operation = telemetry.operation("gateway.request", velune_telemetry::SpanKind::Server);
    let dispatcher = operation
        .as_ref()
        .map(|op| op.dispatch())
        .unwrap_or_else(|| tracing::dispatcher::get_default(Clone::clone));
    let span = tracing::dispatcher::with_default(
        &dispatcher,
        || tracing::info_span!(parent: None, "gateway_request", request_id, operation_id = tracing::field::Empty, protocol = protocol_name, stream = tracing::field::Empty),
    );
    telemetry.parent = operation.as_ref().and_then(|op| op.carrier());
    telemetry.parent_revision = operation.as_ref().and_then(|op| op.revision());
    let observation = Observation::new(
        "gateway_request_finished",
        span.clone(),
        state.stopping.clone(),
    )
    .with_telemetry(operation, telemetry.owner.clone());
    let started_at_ms = now_ms();
    ingress_observed(
        state,
        request_id,
        started_at_ms,
        request,
        protocol,
        observation,
        telemetry,
    )
    .instrument(span)
    .with_subscriber(dispatcher)
    .await
}

/// Rejection path shared by every authenticated admission failure. Records
/// the exchange in request analytics with whatever route identity is known at
/// that point, so gateway-side rejections appear next to dispatched requests
/// instead of only in diagnostic logs. Unauthenticated requests are not usage
/// and stay out of analytics.
struct Admission<'a> {
    state: &'a Ingress,
    request_id: &'a str,
    started_at_ms: i64,
    protocol: Protocol,
}
impl Admission<'_> {
    fn reject(
        &self,
        observation: Observation,
        status: StatusCode,
        message: &str,
        target: Option<&RouteTarget>,
    ) -> Response<Body> {
        self.record(status, target);
        rejected(observation, status, message)
    }
    fn reject_scheduling(
        &self,
        observation: Observation,
        unavailable: ScheduleUnavailable,
        target: Option<&RouteTarget>,
    ) -> Response<Body> {
        let response = scheduling_rejected(observation, unavailable);
        self.record(response.status(), target);
        response
    }
    fn record(&self, status: StatusCode, target: Option<&RouteTarget>) {
        let Some(sink) = &self.state.analytics else {
            return;
        };
        let terminal_at_ms = now_ms();
        sink.record(AnalyticsRecord {
            request_id: self.request_id.to_owned(),
            provider_id: target.map_or_else(String::new, |target| target.provider_id.clone()),
            provider_name: target.map_or_else(String::new, |target| target.provider_name.clone()),
            model_record_key: target
                .map_or_else(String::new, |target| target.model_record_key.clone()),
            provider_model_id: target
                .map_or_else(String::new, |target| target.model.as_str().to_owned()),
            protocol: target.map_or(self.protocol, |target| target.protocol),
            started_at_ms: self.started_at_ms,
            terminal_at_ms,
            elapsed_ms: u64::try_from(terminal_at_ms.saturating_sub(self.started_at_ms))
                .unwrap_or(0),
            first_output_ms: None,
            terminal_elapsed_ms: None,
            status: Some(status.as_u16()),
            outcome: AnalyticsOutcome::Rejected,
            usage: TokenUsage::default(),
            usage_reported: false,
            usage_complete: false,
        });
    }
}

async fn ingress_observed(
    state: Arc<Ingress>,
    request_id: String,
    started_at_ms: i64,
    request: Request<Body>,
    protocol: Protocol,
    observation: Observation,
    telemetry: TelemetryContext,
) -> Response<Body> {
    tracing::info!(event = "gateway_request_received");
    let bearer_valid = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        == Some(format!("Bearer {}", state.token).as_str());
    let key_valid = protocol == Protocol::Messages
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
    // Authenticated admission failures carry usage-relevant diagnostics; the
    // admission context records them in request analytics at each rejection.
    let admission = Admission {
        state: &state,
        request_id: &request_id,
        started_at_ms,
        protocol,
    };
    let (parts, body) = request.into_parts();
    let headers = forward_headers(&parts.headers);
    let bytes = match tokio::time::timeout(REQUEST_READ_TIMEOUT, to_bytes(body, usize::MAX)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(error)) => {
            tracing::warn!(
                event = "gateway_request_body_rejected",
                reason = "read_failed",
                read_error = %error,
            );
            return admission.reject(
                observation,
                StatusCode::BAD_REQUEST,
                "gateway request body read failed",
                None,
            );
        }
        Err(_) => {
            tracing::warn!(
                event = "gateway_request_body_rejected",
                reason = "read_deadline",
            );
            return admission.reject(
                observation,
                StatusCode::REQUEST_TIMEOUT,
                "gateway request body deadline",
                None,
            );
        }
    };
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(Value::Object(value)) => Value::Object(value),
        Ok(_) => {
            tracing::warn!(
                event = "gateway_request_body_rejected",
                reason = "not_an_object",
                body_bytes = bytes.len(),
                opens_with_brace = bytes.first() == Some(&b'{'),
            );
            return admission.reject(
                observation,
                StatusCode::BAD_REQUEST,
                "gateway requires a JSON object",
                None,
            );
        }
        Err(error) => {
            // Length, first-byte shape and the parser position distinguish an
            // empty, truncated or misaligned body without logging any payload.
            tracing::warn!(
                event = "gateway_request_body_rejected",
                reason = "invalid_json",
                body_bytes = bytes.len(),
                opens_with_brace = bytes.first() == Some(&b'{'),
                parse_error = %error,
            );
            return admission.reject(
                observation,
                StatusCode::BAD_REQUEST,
                "gateway requires a JSON object",
                None,
            );
        }
    };
    let Some(model) = body["model"].as_str() else {
        return admission.reject(
            observation,
            StatusCode::BAD_REQUEST,
            "gateway model is required",
            None,
        );
    };
    let route = match state.routes.read() {
        Ok(routes) => routes.get(model).cloned(),
        Err(_) => {
            return admission.reject(
                observation,
                StatusCode::INTERNAL_SERVER_ERROR,
                "gateway alias state unavailable",
                None,
            );
        }
    };
    let Some(route) = route else {
        return admission.reject(
            observation,
            StatusCode::BAD_REQUEST,
            "gateway model route is missing",
            None,
        );
    };
    if route[0].aggregate && has_upstream_state(&body) {
        return admission.reject(
            observation,
            StatusCode::BAD_REQUEST,
            "model routes require full history; previous_response_id and conversation are unsupported",
            Some(&route[0]),
        );
    }
    if let Some(session) = state.backend_session()
        && state
            .backends
            .claude_code
            .validate_result_ids(&session, &tail_tool_result_ids(protocol, &body))
            .is_err()
    {
        return admission.reject(
            observation,
            StatusCode::CONFLICT,
            "tool results belong to a cancelled or invalidated Claude interaction",
            Some(&route[0]),
        );
    }
    let route = match state.continuation_route(&route) {
        Ok(route) => route,
        Err(()) => {
            return admission.reject(
                observation,
                StatusCode::CONFLICT,
                "Claude tool continuation is unavailable for this session and provider binding",
                Some(&route[0]),
            );
        }
    };
    let stream = match body.get("stream") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return admission.reject(
                observation,
                StatusCode::BAD_REQUEST,
                "stream must be a boolean",
                Some(&route[0]),
            );
        }
    };
    tracing::Span::current().record("stream", stream);
    let candidates = match state.candidates(&route) {
        Ok(candidates) => candidates,
        Err(unavailable) => {
            return admission.reject_scheduling(observation, unavailable, Some(&route[0]));
        }
    };
    let (sender, mut receiver) = mpsc::channel(1);
    let request_span = tracing::Span::current();
    let dispatcher = tracing::dispatcher::get_default(Clone::clone);
    let task = tokio::spawn(async move {
        // Future polls retain the application's scoped diagnostic dispatcher.
        let exchange = Exchange {
            request_id,
            started_at_ms,
            protocol,
            body,
            headers,
            stream,
        };
        dispatch(state, exchange, candidates, sender, telemetry)
            .instrument(request_span)
            .with_subscriber(dispatcher)
            .await;
    });
    let mut guard = DispatchGuard { task, observation };
    let Some(WireEvent::Headers(meta)) = receiver.recv().await else {
        guard.observation.http_response(502);
        guard.observation.finish("upstream_unavailable");
        return error_response(
            StatusCode::BAD_GATEWAY,
            "upstream ended before response headers",
        );
    };
    guard.observation.http_response(meta.status);
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

fn has_upstream_state(body: &Value) -> bool {
    ["previous_response_id", "conversation"]
        .iter()
        .any(|field| body.get(*field).is_some_and(|value| !value.is_null()))
}

// Only the request tail introduces results for this exchange. Older results in
// full conversation history must not resurrect cancelled execution ownership.
fn tail_tool_result_ids(protocol: Protocol, body: &Value) -> Vec<String> {
    match protocol {
        Protocol::Messages => body["messages"]
            .as_array()
            .and_then(|messages| messages.last())
            .filter(|message| message["role"] == "user")
            .and_then(|message| message["content"].as_array())
            .into_iter()
            .flatten()
            .filter(|block| block["type"] == "tool_result")
            .filter_map(|block| block["tool_use_id"].as_str().map(str::to_owned))
            .collect(),
        Protocol::ChatCompletions => body["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .rev()
            .take_while(|message| message["role"] == "tool")
            .filter_map(|message| message["tool_call_id"].as_str().map(str::to_owned))
            .collect(),
        Protocol::Responses => body["input"]
            .as_array()
            .into_iter()
            .flatten()
            .rev()
            .take_while(|item| {
                matches!(
                    item["type"].as_str(),
                    Some("function_call_output" | "custom_tool_call_output")
                )
            })
            .filter_map(|item| item["call_id"].as_str().map(str::to_owned))
            .collect(),
    }
}

impl Ingress {
    pub(super) fn backend_session(&self) -> Option<String> {
        self.session
            .as_ref()
            .map(|session| ExecutionBackends::session_key(&self.gateway_id, session))
    }
    pub(super) fn continuation_route(&self, route: &Route) -> Result<Route, ()> {
        let Some(session) = self.backend_session() else {
            return Ok(route.clone());
        };
        match self.backends.claude_code.continuation(&session) {
            Continuation::None => Ok(route.clone()),
            Continuation::Invalidated => Err(()),
            Continuation::Pinned { binding } => {
                let mut target = route
                    .iter()
                    .find(|target| {
                        matches!(target.execution_backend, ExecutionBackend::ClaudeCli { .. })
                            && self.backend_bindings.get(&target.provider_id) == Some(&binding)
                    })
                    .cloned()
                    .ok_or(())?;
                target.continuation_required = true;
                Ok(Route::from([target]))
            }
        }
    }
    pub(super) fn candidate<'a>(&self, target: &'a RouteTarget) -> Candidate<'a> {
        Candidate {
            provider: &target.provider_id,
            model: &target.model_record_key,
            model_id: target.model.as_str(),
            epoch: self.provider_epochs[&target.provider_id],
            limits: &target.quota_limits,
            weight: target.weight,
        }
    }
    pub(super) fn candidates(
        &self,
        route: &Route,
    ) -> Result<Vec<RouteTarget>, ScheduleUnavailable> {
        let first = &route[0];
        if first.continuation_required {
            return Ok(route.to_vec());
        }
        let candidates: Vec<_> = route.iter().map(|target| self.candidate(target)).collect();
        self.health
            .rank(
                &self.gateway_id,
                &first.route_key,
                first.strategy,
                first.sticky,
                self.session.as_ref(),
                &candidates,
                self.quotas.as_ref(),
            )
            .map(|indices| {
                let mut selected: Vec<_> =
                    indices.iter().map(|index| route[*index].clone()).collect();
                // Keep paused members for a later eligibility recheck. Admission
                // still rejects them until their quota/health facts permit use.
                selected.extend(
                    route
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| !indices.contains(index))
                        .map(|(_, target)| target.clone()),
                );
                selected
            })
    }
    pub(super) fn analytics(
        &self,
        request_id: String,
        started_at_ms: i64,
        target: &RouteTarget,
    ) -> Option<RequestAnalytics> {
        self.analytics.clone().map(|sink| {
            RequestAnalytics::new(
                sink,
                AnalyticsRecord {
                    request_id,
                    provider_id: target.provider_id.clone(),
                    provider_name: target.provider_name.clone(),
                    model_record_key: target.model_record_key.clone(),
                    provider_model_id: target.model.as_str().to_owned(),
                    protocol: target.protocol,
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
                self.stopping.clone(),
            )
        })
    }
}
