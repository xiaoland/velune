//! The native HAPI v1 client contract over the existing application owner.
//! The HTTPS frontend must route this router at its origin, without a path prefix.

mod projection;

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::stream;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use velune_application_api::{Failure, FailureKind};

use super::session_control::SessionControl;
use projection::{Changes, History, MessageQuery};

const TOKEN_TTL: u64 = 4 * 60 * 60;
const JWT_HEADER: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
const SESSION_LIMIT: usize = 500;
const MAX_LOCAL_IDS: usize = 1000;
const MAX_LOCAL_ID_BYTES: usize = 128;

struct Hub {
    control: Arc<SessionControl>,
    access_token: String,
    signing_key: [u8; 32],
    epoch: u64,
    events: AtomicU64,
    streams: Arc<Semaphore>,
    history: History,
}

pub(super) fn router(control: Arc<SessionControl>, token: String, share_id: String) -> Router {
    let signing_key = sign(token.as_bytes(), b"velune/hapi/v1/client-auth");
    let epoch = now_ms();
    let state = Arc::new(Hub {
        control,
        access_token: token,
        signing_key,
        epoch,
        events: AtomicU64::new(0),
        streams: Arc::new(Semaphore::new(16)),
        history: History::new(epoch),
    });
    let protected = Router::new()
        .route("/api/sessions", get(list))
        .route("/api/sessions/{id}", get(detail))
        .route("/api/sessions/{id}/messages", get(messages).post(send))
        .route(
            "/api/sessions/{id}/messages/queued-state",
            post(queued_state),
        )
        .route("/api/sessions/{id}/abort", post(stop))
        .route(
            "/api/sessions/{id}/permissions/{request_id}/approve",
            post(approve),
        )
        .route(
            "/api/sessions/{id}/permissions/{request_id}/deny",
            post(deny),
        )
        .route(
            "/api/machines",
            get(|| async { Json(json!({"machines": []})) }),
        )
        .route("/api/visibility", post(|| async { ok() }))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_bearer,
        ));
    protected
        .route(
            "/health",
            get(|| async move {
                Json(json!({"status": "ok", "protocolVersion": 1, "capabilities": {}, "veluneShareId": share_id}))
            }),
        )
        .route("/api/auth", post(auth))
        .route("/api/events", get(events))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(middleware::from_fn(no_cache))
        .with_state(state)
}

async fn no_cache(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AuthRequest {
    access_token: String,
}

#[derive(Serialize, Deserialize)]
struct Claims {
    uid: u8,
    ns: String,
    iat: u64,
    exp: u64,
}

impl Hub {
    fn issue_bearer(&self, now: u64) -> String {
        let claims = Claims {
            uid: 1,
            ns: "velune".into(),
            iat: now,
            exp: now + TOKEN_TTL,
        };
        let claims =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("serializable claims"));
        let body = format!("{JWT_HEADER}.{claims}");
        let signature = URL_SAFE_NO_PAD.encode(sign(&self.signing_key, body.as_bytes()));
        format!("{body}.{signature}")
    }

    fn expiration(&self, token: &str, now: u64) -> Option<u64> {
        if !self.control.is_active() || token.len() > 1024 {
            return None;
        }
        let (body, signature) = token.rsplit_once('.')?;
        let claims = body.strip_prefix(JWT_HEADER)?.strip_prefix('.')?;
        let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
        let expected = sign(&self.signing_key, body.as_bytes());
        if !bool::from(expected.as_slice().ct_eq(&signature)) {
            return None;
        }
        let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims).ok()?).ok()?;
        (claims.exp > now && claims.uid == 1 && claims.ns == "velune").then_some(claims.exp)
    }

    fn session_event(&self, value: Value) -> Event {
        let number = self.events.fetch_add(1, Ordering::Relaxed) + 1;
        frame(value).id(format!("{:08x}:{number}:00000000", self.epoch as u32))
    }
}

fn sign(key: &[u8], payload: &[u8]) -> [u8; 32] {
    let mut signer = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    signer.update(payload);
    signer.finalize().into_bytes().into()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn auth(State(hub): State<Arc<Hub>>, Json(body): Json<AuthRequest>) -> ApiResult {
    if !hub.control.is_active()
        || !bool::from(
            hub.access_token
                .as_bytes()
                .ct_eq(body.access_token.as_bytes()),
        )
    {
        return Err(unauthorized());
    }
    Ok(Json(json!({
        "token": hub.issue_bearer(now_ms() / 1000),
        "user": {"id": 1, "firstName": "Velune"}
    })))
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let (scheme, value) = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .split_once(' ')?;
    scheme.eq_ignore_ascii_case("Bearer").then_some(value)
}

async fn require_bearer(State(hub): State<Arc<Hub>>, request: Request, next: Next) -> Response {
    if bearer(request.headers())
        .and_then(|token| hub.expiration(token, now_ms() / 1000))
        .is_none()
    {
        return unauthorized().into_response();
    }
    next.run(request).await
}

type ApiResult = Result<Json<Value>, ApiError>;

struct ApiError {
    status: StatusCode,
    message: &'static str,
    code: &'static str,
}

impl ApiError {
    const fn new(status: StatusCode, message: &'static str, code: &'static str) -> Self {
        Self {
            status,
            message,
            code,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"error": self.message, "code": self.code})),
        )
            .into_response()
    }
}

impl From<Failure> for ApiError {
    fn from(failure: Failure) -> Self {
        // Application errors can carry local paths or backend details. Only
        // expose the action's public category here.
        if failure.code == "conversation_not_shared" {
            return Self::new(
                StatusCode::NOT_FOUND,
                "Session not found",
                "session_not_found",
            );
        }
        match failure.kind {
            FailureKind::Invalid => Self::new(
                StatusCode::BAD_REQUEST,
                "Invalid request",
                "invalid_request",
            ),
            FailureKind::Unsupported => Self::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Action unavailable",
                "unsupported",
            ),
            FailureKind::Conflict | FailureKind::OutstandingWork => Self::new(
                StatusCode::CONFLICT,
                "Conversation state changed",
                "conflict",
            ),
            FailureKind::StaleInteraction => Self::new(
                StatusCode::NOT_FOUND,
                "Permission request already resolved",
                "stale_interaction",
            ),
            _ => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "Velune is temporarily unavailable",
                "unavailable",
            ),
        }
    }
}

fn unauthorized() -> ApiError {
    ApiError::new(StatusCode::UNAUTHORIZED, "Invalid token", "invalid_token")
}

fn invalid_request() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "Invalid request",
        "invalid_request",
    )
}

fn unsupported() -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "Action unavailable",
        "unsupported",
    )
}

fn session_error(error: Failure) -> ApiError {
    if error.kind == FailureKind::Invalid {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "Session not found",
            "session_not_found",
        )
    } else {
        error.into()
    }
}

fn ok() -> Json<Value> {
    Json(json!({"ok": true}))
}

#[derive(Default, Deserialize)]
struct ListQuery {
    limit: Option<usize>,
    order: Option<String>,
}

async fn list(State(hub): State<Arc<Hub>>, Query(query): Query<ListQuery>) -> ApiResult {
    let limit = query.limit.unwrap_or(SESSION_LIMIT);
    if !(1..=SESSION_LIMIT).contains(&limit) {
        return Err(invalid_request());
    }
    let mut rows = hub.control.list().await?;
    if query.order.as_deref() == Some("updatedAt") {
        rows.sort_by_key(|row| std::cmp::Reverse(row.updated_at_unix_ms));
    }
    let sessions = rows
        .iter()
        .take(limit)
        .map(|row| hub.history.summary(row))
        .collect::<Vec<_>>();
    Ok(Json(json!({"sessions": sessions})))
}

async fn detail(State(hub): State<Arc<Hub>>, Path(id): Path<String>) -> ApiResult {
    let snapshot = hub.control.detail(&id).await.map_err(session_error)?;
    let projected = hub.history.project(&id, snapshot);
    Ok(Json(json!({"session": projected.session})))
}

async fn messages(
    State(hub): State<Arc<Hub>>,
    Path(id): Path<String>,
    Query(query): Query<MessageQuery>,
) -> ApiResult {
    if !query.valid() {
        return Err(invalid_request());
    }
    let snapshot = hub.control.detail(&id).await.map_err(session_error)?;
    Ok(Json(hub.history.project(&id, snapshot).page(query)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SendMessage {
    text: String,
    local_id: Option<String>,
    attachments: Option<Vec<Value>>,
    scheduled_at: Option<u64>,
    delivery_mode: Option<String>,
}

fn valid_local_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_LOCAL_ID_BYTES && !id.chars().any(char::is_control)
}

async fn send(
    State(hub): State<Arc<Hub>>,
    Path(id): Path<String>,
    Json(body): Json<SendMessage>,
) -> ApiResult {
    if body.text.trim().is_empty()
        || body
            .local_id
            .as_deref()
            .is_some_and(|id| !valid_local_id(id))
    {
        return Err(invalid_request());
    }
    if body
        .attachments
        .as_ref()
        .is_some_and(|rows| !rows.is_empty())
        || body.scheduled_at.is_some()
        || body
            .delivery_mode
            .as_deref()
            .is_some_and(|mode| mode != "queue")
    {
        return Err(unsupported());
    }
    // Complete the projection receipt even when the HTTP client disconnects
    // after the application has admitted the turn.
    tokio::spawn(async move {
        let message_id = hub
            .control
            .send(&id, body.text, body.local_id.clone())
            .await?;
        if let Some(local_id) = body.local_id {
            hub.history.note_send(&id, local_id, message_id);
        }
        Ok(ok())
    })
    .await
    .map_err(|_| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Velune is temporarily unavailable",
            "unavailable",
        )
    })?
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueuedState {
    local_ids: Vec<String>,
}

async fn queued_state(
    State(hub): State<Arc<Hub>>,
    Path(id): Path<String>,
    Json(body): Json<QueuedState>,
) -> ApiResult {
    if body.local_ids.len() > MAX_LOCAL_IDS || body.local_ids.iter().any(|id| !valid_local_id(id)) {
        return Err(invalid_request());
    }
    let snapshot = hub.control.detail(&id).await.map_err(session_error)?;
    let projected = hub.history.project(&id, snapshot);
    Ok(Json(projected.queued_state(&body.local_ids)))
}

async fn stop(State(hub): State<Arc<Hub>>, Path(id): Path<String>) -> ApiResult {
    hub.control.stop(&id).await?;
    Ok(ok())
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Decision {
    mode: Option<String>,
    allow_tools: Option<Vec<String>>,
    decision: Option<String>,
    answers: Option<Value>,
}

async fn approve(
    State(hub): State<Arc<Hub>>,
    Path((id, request_id)): Path<(String, String)>,
    body: Option<Json<Decision>>,
) -> ApiResult {
    decide(hub, id, request_id, true, body).await
}

async fn deny(
    State(hub): State<Arc<Hub>>,
    Path((id, request_id)): Path<(String, String)>,
    body: Option<Json<Decision>>,
) -> ApiResult {
    decide(hub, id, request_id, false, body).await
}

async fn decide(
    hub: Arc<Hub>,
    id: String,
    request_id: String,
    allow: bool,
    body: Option<Json<Decision>>,
) -> ApiResult {
    let body = body.map(|Json(body)| body).unwrap_or_default();
    let expected = if allow { "approved" } else { "denied" };
    if body.mode.is_some()
        || body.answers.is_some()
        || body
            .allow_tools
            .as_ref()
            .is_some_and(|tools| !tools.is_empty())
        || body
            .decision
            .as_deref()
            .is_some_and(|decision| decision != expected)
    {
        return Err(unsupported());
    }
    hub.control.decide(&id, &request_id, allow).await?;
    Ok(ok())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventQuery {
    session_id: Option<String>,
    machine_id: Option<String>,
    token: Option<String>,
}

enum Subscription {
    All(Option<Vec<velune_application_api::conversation::ConversationSummary>>),
    Session(String, Changes),
}

struct Feed {
    hub: Arc<Hub>,
    _permit: OwnedSemaphorePermit,
    subscription: Subscription,
    expiration: u64,
    queue: VecDeque<Event>,
    interval: tokio::time::Interval,
    last_heartbeat: tokio::time::Instant,
}

async fn events(
    State(hub): State<Arc<Hub>>,
    headers: HeaderMap,
    Query(query): Query<EventQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let token = if headers.contains_key(header::AUTHORIZATION) {
        bearer(&headers)
    } else {
        query.token.as_deref()
    };
    let expiration = token
        .and_then(|token| hub.expiration(token, now_ms() / 1000))
        .ok_or_else(unauthorized)?;
    let permit = hub.streams.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Too many event streams",
            "stream_limit",
        )
    })?;
    if query.machine_id.is_some() {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "Machine not found",
            "machine_not_found",
        ));
    }
    let (subscription, period) = match query.session_id {
        Some(id) => {
            hub.control.detail(&id).await.map_err(session_error)?;
            (
                Subscription::Session(id, Changes::default()),
                Duration::from_millis(500),
            )
        }
        None => (Subscription::All(None), Duration::from_secs(3)),
    };
    let subscription_id = format!("velune-{}", hub.events.fetch_add(1, Ordering::Relaxed));
    let handshake = frame(json!({
        "type": "connection-changed",
        "data": {"status": "connected", "subscriptionId": subscription_id, "resume": "gap"}
    }));
    // Each connection starts with a REST resync. There is no replay promise
    // for the disposable poll projection.
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let feed = Feed {
        hub,
        _permit: permit,
        subscription,
        expiration,
        queue: VecDeque::from([handshake]),
        interval,
        last_heartbeat: tokio::time::Instant::now(),
    };
    let sse = Sse::new(stream::unfold(feed, Feed::next));
    Ok((
        [(header::HeaderName::from_static("x-accel-buffering"), "no")],
        sse,
    ))
}

impl Feed {
    async fn next(mut self) -> Option<(Result<Event, Infallible>, Self)> {
        loop {
            if now_ms() / 1000 >= self.expiration {
                return None;
            }
            if let Some(event) = self.queue.pop_front() {
                return Some((Ok(event), self));
            }
            self.interval.tick().await;
            let changes = match &mut self.subscription {
                Subscription::All(previous) => {
                    let mut sessions = self.hub.control.list().await.ok()?;
                    sessions.truncate(SESSION_LIMIT);
                    let changes = projection::list_changes(previous.as_deref(), &sessions);
                    *previous = Some(sessions);
                    changes
                }
                Subscription::Session(id, previous) => {
                    let snapshot = self.hub.control.detail(id).await.ok()?;
                    previous.observe(id, self.hub.history.project(id, snapshot))
                }
            };
            self.queue.extend(
                changes
                    .into_iter()
                    .map(|change| self.hub.session_event(change)),
            );
            if self.last_heartbeat.elapsed() >= Duration::from_secs(15) {
                self.last_heartbeat = tokio::time::Instant::now();
                self.queue.push_back(frame(
                    json!({"type": "heartbeat", "data": {"timestamp": now_ms()}}),
                ));
            }
        }
    }
}

fn frame(value: Value) -> Event {
    Event::default().data(value.to_string())
}
