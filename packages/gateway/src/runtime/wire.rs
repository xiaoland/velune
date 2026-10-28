//! What travels back to the caller: response heads, bodies and failures.
use crate::{observation::Observation, scheduling::ScheduleUnavailable};
use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode},
};
use serde_json::json;
use std::time::Duration;
use tokio::sync::mpsc;
use velune_ai::{
    Payload,
    http::{Header, ResponseBody, ResponseMeta},
};

pub(crate) enum WireEvent {
    Headers(ResponseMeta),
    Body(Vec<u8>),
    Failed,
}
pub(super) struct DispatchGuard {
    pub(super) task: tokio::task::JoinHandle<()>,
    pub(super) observation: Observation,
}
impl Drop for DispatchGuard {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) fn forward_headers(headers: &HeaderMap) -> Vec<Header> {
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
pub(super) fn response(meta: ResponseMeta, body: Body) -> Response<Body> {
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
pub(super) fn rejected(
    mut observation: Observation,
    status: StatusCode,
    message: &str,
) -> Response<Body> {
    observation.http_response(status.as_u16());
    observation.finish("rejected");
    tracing::warn!(
        event = "gateway_request_rejected",
        http_status = status.as_u16()
    );
    error_response(status, message)
}
pub(super) fn scheduling_rejected(
    observation: Observation,
    unavailable: ScheduleUnavailable,
) -> Response<Body> {
    let mut response = rejected(
        observation,
        if unavailable.retry_after.is_some() {
            StatusCode::TOO_MANY_REQUESTS
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        "no eligible provider: quota or health observations do not permit a request",
    );
    if let Some(delay) = unavailable.retry_after {
        response.headers_mut().insert(
            "retry-after",
            HeaderValue::from_str(&retry_seconds(delay).to_string()).expect("numeric retry delay"),
        );
    }
    response
}
pub(super) fn retry_seconds(delay: Duration) -> u64 {
    delay
        .as_secs()
        .saturating_add(u64::from(delay.subsec_nanos() > 0))
        .max(1)
}
pub(super) fn error_response(status: StatusCode, message: &str) -> Response<Body> {
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
pub(crate) async fn send_json(sender: &mpsc::Sender<WireEvent>, value: ResponseBody) -> bool {
    sender.send(WireEvent::Headers(value.meta)).await.is_ok()
        && sender
            .send(WireEvent::Body(value.body.into_inner()))
            .await
            .is_ok()
}
pub(super) async fn send_failure(
    sender: &mpsc::Sender<WireEvent>,
    meta: Option<ResponseMeta>,
    body: Option<Payload<Vec<u8>>>,
    status: u16,
    message: &str,
) {
    let mut meta = meta.unwrap_or(ResponseMeta {
        status,
        headers: Vec::new(),
    });
    let mut body = body;
    if (200..300).contains(&meta.status) {
        // An upstream HTTP success can still fail the protocol contract before
        // commitment. Never label our replacement error body as success.
        meta = ResponseMeta {
            status: 502,
            headers: Vec::new(),
        };
        body = None;
    }
    if body.is_none() {
        meta.headers.retain(|header| {
            !matches!(
                header.name.to_ascii_lowercase().as_str(),
                "content-type"
                    | "content-length"
                    | "content-encoding"
                    | "content-range"
                    | "etag"
                    | "content-md5"
                    | "digest"
                    | "last-modified"
                    | "accept-ranges"
            )
        });
        meta.headers.push(Header {
            name: "content-type".into(),
            value: "application/json".into(),
        });
    }
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
