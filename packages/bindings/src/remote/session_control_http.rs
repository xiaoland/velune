//! The host's versioned HTTP projection for external session controllers.

use std::sync::Arc;

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{DefaultBodyLimit, Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use velune_application_api::conversation::{
    ConversationSummary, InteractionKind, InteractionOption, MessageRole, RunState,
    RuntimeInteraction, ToolState,
};
use velune_application_api::{DeliveredBlock, DeliveredMessage, Failure, FailureKind};

use super::session_control::{SessionControl, one_shot_options};

const MAX_BODY_BYTES: usize = 400 * 1024;
const MAX_HISTORY_MESSAGES: usize = 120;
const MAX_HISTORY_BYTES: usize = 128 * 1024;
const MAX_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_ID_BYTES: usize = 512;
const MAX_TITLE_BYTES: usize = 512;
const MAX_INTERACTIONS: usize = 16;
const MAX_INTERACTION_DETAIL_BYTES: usize = 4 * 1024;
const MAX_OPTION_LABEL_BYTES: usize = 256;
const MAX_OPTIONS: usize = 4;

struct Ingress {
    control: Arc<SessionControl>,
    token: String,
}

pub(super) fn router(control: Arc<SessionControl>, token: String) -> Router {
    let ingress = Arc::new(Ingress { control, token });
    Router::new()
        .route("/control/v1/sessions", get(list))
        .route("/control/v1/sessions/{id}", get(detail))
        .route("/control/v1/sessions/{id}/turns", post(send))
        .route("/control/v1/sessions/{id}/cancel", post(cancel))
        .route(
            "/control/v1/sessions/{id}/interactions/{request_id}",
            post(reply),
        )
        .route_layer(middleware::from_fn_with_state(ingress.clone(), authorize))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn(no_cache))
        .with_state(ingress)
}

async fn authorize(State(ingress): State<Arc<Ingress>>, request: Request, next: Next) -> Response {
    if !matches_token(request.headers(), &ingress.token) {
        return ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Invalid access token",
            "invalid_token",
        )
        .into_response();
    }
    next.run(request).await
}

fn matches_token(headers: &HeaderMap, expected: &str) -> bool {
    let values = headers.get_all(header::AUTHORIZATION);
    let mut values = values.iter();
    let Some(value) = values.next().and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let Some((scheme, token)) = value.split_once(' ') else {
        return false;
    };
    values.next().is_none()
        && scheme.eq_ignore_ascii_case("Bearer")
        && !expected.is_empty()
        && bool::from(expected.as_bytes().ct_eq(token.as_bytes()))
}

async fn no_cache(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

type ApiResult<T> = Result<Json<T>, ApiError>;

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
    code: &'static str,
}

struct ApiError {
    status: StatusCode,
    body: ErrorBody,
}

impl ApiError {
    const fn new(status: StatusCode, error: &'static str, code: &'static str) -> Self {
        Self {
            status,
            body: ErrorBody { error, code },
        }
    }

    fn invalid() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "Invalid request",
            "invalid_request",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(self.body)).into_response();
        if self.status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

impl From<Failure> for ApiError {
    fn from(error: Failure) -> Self {
        if error.code == "conversation_not_shared" {
            return Self::new(
                StatusCode::NOT_FOUND,
                "Session not found",
                "session_not_found",
            );
        }
        // Keep filesystem paths, provider responses and other application
        // diagnostics on the host.
        match error.kind {
            FailureKind::Invalid => Self::invalid(),
            FailureKind::Unsupported => Self::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Action unavailable",
                "unsupported",
            ),
            FailureKind::StaleInteraction => Self::new(
                StatusCode::CONFLICT,
                "Approval already resolved or expired",
                "stale_interaction",
            ),
            FailureKind::Conflict | FailureKind::OutstandingWork => {
                Self::new(StatusCode::CONFLICT, "Session state changed", "conflict")
            }
            FailureKind::Closed => Self::new(
                StatusCode::GONE,
                "Session sharing has ended",
                "sharing_closed",
            ),
            _ => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "Host temporarily unavailable",
                "unavailable",
            ),
        }
    }
}

impl From<PathRejection> for ApiError {
    fn from(_: PathRejection) -> Self {
        Self::invalid()
    }
}

impl From<JsonRejection> for ApiError {
    fn from(error: JsonRejection) -> Self {
        match error.status() {
            StatusCode::PAYLOAD_TOO_LARGE => Self::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Request is too large",
                "body_too_large",
            ),
            StatusCode::UNSUPPORTED_MEDIA_TYPE => Self::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Expected application/json",
                "expected_json",
            ),
            _ => Self::invalid(),
        }
    }
}

#[derive(Serialize)]
struct SessionList {
    sessions: Vec<Summary>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    id: String,
    title: String,
    updated_at: Option<i64>,
}

impl From<ConversationSummary> for Summary {
    fn from(row: ConversationSummary) -> Self {
        let title = utf8_prefix(row.title.display_text(), MAX_TITLE_BYTES).to_owned();
        Self {
            id: row.id,
            title,
            updated_at: row.updated_at_unix_ms,
        }
    }
}

async fn list(State(ingress): State<Arc<Ingress>>) -> ApiResult<SessionList> {
    let sessions = ingress
        .control
        .list()
        .await?
        .into_iter()
        .map(Summary::from)
        .collect();
    Ok(Json(SessionList { sessions }))
}

#[derive(Serialize)]
struct SessionResponse {
    session: Session,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    #[serde(flatten)]
    summary: Summary,
    state: RunState,
    can_send: bool,
    can_cancel: bool,
    messages: Vec<Message>,
    interactions: Vec<Interaction>,
}

async fn detail(
    State(ingress): State<Arc<Ingress>>,
    path: Result<Path<String>, PathRejection>,
) -> ApiResult<SessionResponse> {
    let Path(id) = path?;
    validate_id(&id, MAX_ID_BYTES)?;
    let snapshot = ingress.control.detail(&id).await?;
    let session = Session {
        summary: snapshot.conversation.into(),
        state: snapshot.run_state,
        can_send: snapshot.actions.can_send,
        can_cancel: snapshot.actions.can_cancel,
        messages: project_messages(snapshot.messages),
        interactions: snapshot
            .pending_interactions
            .into_iter()
            .filter_map(project_interaction)
            .take(MAX_INTERACTIONS)
            .collect(),
    };
    Ok(Json(SessionResponse { session }))
}

#[derive(Serialize)]
struct Message {
    id: String,
    role: MessageRole,
    text: String,
    completed: bool,
}

fn project_messages(source: Vec<DeliveredMessage>) -> Vec<Message> {
    let mut budget = MAX_HISTORY_BYTES;
    let mut messages = Vec::new();
    for message in source.into_iter().rev() {
        if messages.len() == MAX_HISTORY_MESSAGES || budget < 4 {
            break;
        }
        if !valid_id(&message.id, MAX_ID_BYTES) {
            continue;
        }
        let text = project_text(&message, budget.min(MAX_MESSAGE_BYTES));
        if text.is_empty() {
            continue;
        }
        budget -= text.len();
        messages.push(Message {
            id: message.id,
            role: message.role,
            text,
            completed: message.completed,
        });
    }
    messages.reverse();
    messages
}

fn project_text(message: &DeliveredMessage, limit: usize) -> String {
    let mut result = String::new();
    for block in &message.blocks {
        let text = match block {
            DeliveredBlock::Text { text } if message.role != MessageRole::Tool => text.as_str(),
            // Some runtimes use the raw shell command as the tool title.
            // A status summary works without forwarding the command or output.
            DeliveredBlock::Tool { state, .. } => match state {
                ToolState::Pending => "工具等待执行",
                ToolState::Running => "工具执行中",
                ToolState::Completed => "工具已完成",
                ToolState::Failed => "工具执行失败",
            },
            _ => continue,
        };
        if text.trim().is_empty() {
            continue;
        }
        let newline = usize::from(!result.is_empty());
        let available = limit.saturating_sub(result.len() + newline);
        let prefix = utf8_prefix(text, available);
        if prefix.is_empty() {
            break;
        }
        if newline > 0 {
            result.push('\n');
        }
        result.push_str(prefix);
        if prefix.len() < text.len() {
            break;
        }
    }
    result
}

fn utf8_prefix(text: &str, max_bytes: usize) -> &str {
    &text[..text.floor_char_boundary(max_bytes)]
}

#[derive(Serialize)]
struct Interaction {
    id: String,
    title: String,
    detail: String,
    options: Vec<InteractionOption>,
}

fn project_interaction(request: RuntimeInteraction) -> Option<Interaction> {
    let InteractionKind::Approval {
        title,
        detail,
        options,
    } = request.kind
    else {
        return None;
    };
    // An approval must include all of the host's explanation, so an operator
    // can assess what the option does.
    if !valid_id(&request.id, MAX_ID_BYTES)
        || title.len() > MAX_TITLE_BYTES
        || detail.len() > MAX_INTERACTION_DETAIL_BYTES
    {
        return None;
    }
    let options = one_shot_options(&options)
        .filter(|option| option.label.len() <= MAX_OPTION_LABEL_BYTES)
        .take(MAX_OPTIONS)
        .cloned()
        .collect::<Vec<_>>();
    (!options.is_empty()).then_some(Interaction {
        id: request.id,
        title,
        detail,
        options,
    })
}

fn valid_id(id: &str, max_bytes: usize) -> bool {
    !id.is_empty() && id.len() <= max_bytes && !id.chars().any(char::is_control)
}

fn validate_id(id: &str, max_bytes: usize) -> Result<(), ApiError> {
    valid_id(id, max_bytes)
        .then_some(())
        .ok_or_else(ApiError::invalid)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SendTurn {
    text: String,
    operation_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplyToInteraction {
    option_id: String,
}

#[derive(Serialize)]
struct Acceptance {
    state: &'static str,
}

fn accepted() -> Json<Acceptance> {
    Json(Acceptance { state: "accepted" })
}

async fn send(
    State(ingress): State<Arc<Ingress>>,
    path: Result<Path<String>, PathRejection>,
    body: Result<Json<SendTurn>, JsonRejection>,
) -> ApiResult<Acceptance> {
    let Path(id) = path?;
    let Json(body) = body?;
    validate_id(&id, MAX_ID_BYTES)?;
    validate_id(&body.operation_id, 128)?;
    ingress
        .control
        .send(&id, body.text, Some(body.operation_id))
        .await?;
    Ok(accepted())
}

async fn cancel(
    State(ingress): State<Arc<Ingress>>,
    path: Result<Path<String>, PathRejection>,
    body: Result<Json<Empty>, JsonRejection>,
) -> ApiResult<Acceptance> {
    let Path(id) = path?;
    let Json(Empty {}) = body?;
    validate_id(&id, MAX_ID_BYTES)?;
    ingress.control.stop(&id).await?;
    Ok(accepted())
}

async fn reply(
    State(ingress): State<Arc<Ingress>>,
    path: Result<Path<(String, String)>, PathRejection>,
    body: Result<Json<ReplyToInteraction>, JsonRejection>,
) -> ApiResult<Acceptance> {
    let Path((id, request_id)) = path?;
    let Json(body) = body?;
    validate_id(&id, MAX_ID_BYTES)?;
    validate_id(&request_id, MAX_ID_BYTES)?;
    validate_id(&body.option_id, 128)?;
    ingress
        .control
        .reply(&id, &request_id, body.option_id)
        .await?;
    Ok(accepted())
}

#[cfg(test)]
mod tests {
    use super::*;
    use velune_application_api::MessagePayload;

    #[test]
    fn bearer_is_exact_and_rejects_ambiguous_headers() {
        let mut headers = HeaderMap::new();
        assert!(!matches_token(&headers, "test-token"));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bearer test-token"),
        );
        assert!(matches_token(&headers, "test-token"));
        assert!(!matches_token(&headers, "test-tokem"));
        headers.append(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer test-token"),
        );
        assert!(!matches_token(&headers, "test-token"));
    }

    #[test]
    fn transcript_limits_preserve_utf8_and_exclude_tool_input_and_output() {
        let text = |id: &str, count: usize| DeliveredMessage {
            stamp: 1,
            completed: true,
            id: id.into(),
            role: MessageRole::Assistant,
            timestamp_unix_ms: None,
            blocks: vec![DeliveredBlock::Text {
                text: "中".repeat(count),
            }],
        };
        let mut source = (0..150)
            .map(|n| text(&n.to_string(), 1))
            .collect::<Vec<_>>();
        source.push(text("large", MAX_MESSAGE_BYTES));
        source.push(DeliveredMessage {
            stamp: 2,
            completed: true,
            id: "tool".into(),
            role: MessageRole::Tool,
            timestamp_unix_ms: None,
            blocks: vec![DeliveredBlock::Tool {
                tool_id: Some("tool-id".into()),
                title: "shell command".into(),
                state: ToolState::Completed,
                output: Some(MessagePayload::Inline {
                    text: "tool output".into(),
                }),
                linked_conversation_id: None,
            }],
        });
        let projected = project_messages(source);
        assert_eq!(projected.len(), MAX_HISTORY_MESSAGES);
        assert_eq!(projected.first().unwrap().id, "32");
        assert_eq!(projected.last().unwrap().text, "工具已完成");
        assert_eq!(
            projected[MAX_HISTORY_MESSAGES - 2].text.len(),
            MAX_MESSAGE_BYTES - 1
        );
    }

    #[test]
    fn approval_projection_preserves_only_host_allowed_options() {
        let approval = RuntimeInteraction {
            id: "request-1".into(),
            kind: InteractionKind::Approval {
                title: "Approval".into(),
                detail: "Full details".into(),
                options: [
                    (
                        "vendorPermitOnce",
                        velune_application_api::conversation::ApprovalOptionKind::AllowOnce,
                    ),
                    (
                        "allow",
                        velune_application_api::conversation::ApprovalOptionKind::AllowAlways,
                    ),
                    (
                        "deny",
                        velune_application_api::conversation::ApprovalOptionKind::RejectOnce,
                    ),
                ]
                .into_iter()
                .map(|(id, kind)| InteractionOption {
                    id: id.into(),
                    label: id.into(),
                    approval_kind: Some(kind),
                })
                .collect(),
            },
        };
        let projected = project_interaction(approval).unwrap();
        assert_eq!(
            projected
                .options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>(),
            ["vendorPermitOnce", "deny"]
        );
    }
}
