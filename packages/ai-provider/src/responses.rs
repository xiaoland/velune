//! What only the OpenAI Responses protocol needs: its business terminal, its
//! stream media type, and the Codex subscription profile's stateless backend.
use crate::config::RequestProfile;
use crate::native::transport_kind;
use futures_util::StreamExt;
use serde_json::Value;
use velune_ai::{
    InvalidContract, NativeErrorKind, NativeFailure,
    http::{Header, ResponseMeta},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Terminal {
    Completed,
    Incomplete,
    Failed,
    Cancelled,
}

fn terminal_from_status(status: &str) -> Option<Terminal> {
    match status {
        "completed" => Some(Terminal::Completed),
        "incomplete" => Some(Terminal::Incomplete),
        "failed" => Some(Terminal::Failed),
        "cancelled" | "canceled" => Some(Terminal::Cancelled),
        _ => None,
    }
}

fn terminal_from_event(event: Option<&str>, value: &Value) -> Option<Terminal> {
    value
        .get("status")
        .and_then(Value::as_str)
        .and_then(terminal_from_status)
        .or_else(|| event.and_then(|name| terminal_from_status(name.strip_prefix("response.")?)))
        .or_else(|| {
            value["type"]
                .as_str()
                .and_then(|name| terminal_from_status(name.strip_prefix("response.")?))
        })
}

/// A JSON response's terminal, which it must state.
pub(crate) fn body_terminal(body: &[u8]) -> Option<Terminal> {
    let value: Value = serde_json::from_slice(body).ok()?;
    value["status"].as_str().and_then(terminal_from_status)
}

/// Read an SSE stream to its end; whether its explicit terminal is completion.
pub(crate) async fn stream_terminal<T: AsRef<[u8]>>(
    chunks: impl futures_util::Stream<Item = reqwest::Result<T>>,
) -> Result<bool, NativeErrorKind> {
    use eventsource_stream::{EventStreamError, Eventsource};
    let events = chunks.eventsource();
    futures_util::pin_mut!(events);
    let mut terminal = None;
    while let Some(event) = events.next().await {
        let event = event.map_err(|error| match error {
            EventStreamError::Transport(error) => transport_kind(&error),
            EventStreamError::Utf8(_) | EventStreamError::Parser(_) => {
                NativeErrorKind::ProviderFailure
            }
        })?;
        if event.data.is_empty() || event.data == "[DONE]" {
            continue;
        }
        let value: Value =
            serde_json::from_str(&event.data).map_err(|_| NativeErrorKind::ProviderFailure)?;
        if let Some(value) = terminal_from_event(Some(&event.event), &value) {
            terminal = Some(value);
        }
    }
    terminal
        .map(|terminal| terminal == Terminal::Completed)
        .ok_or(NativeErrorKind::ProviderFailure)
}

/// A successful stream must declare SSE. Codex alone may omit the media type,
/// which it does on its native SSE endpoint; an explicit wrong type never passes.
pub(crate) fn check_stream_type(
    profile: RequestProfile,
    response: &reqwest::Response,
    meta: &mut ResponseMeta,
) -> Result<(), NativeFailure> {
    let header = |name| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    let content_type = header("content-type");
    let media_type = content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if media_type.is_some_and(|value| value.eq_ignore_ascii_case("text/event-stream")) {
        return Ok(());
    }
    if profile == RequestProfile::CodexResponses && !response.headers().contains_key("content-type")
    {
        meta.headers.push(Header {
            name: "content-type".into(),
            value: "text/event-stream".into(),
        });
        return Ok(());
    }
    tracing::warn!(
        event = "responses_stream_content_type_rejected",
        upstream_status = meta.status,
        mime = media_type_class(media_type),
        encoding = encoding_class(header("content-encoding")),
    );
    Err(NativeFailure::sent(
        NativeErrorKind::Unsupported,
        Some(meta.clone()),
    ))
}

// Diagnostic labels must remain finite: upstream header values can contain secrets.
fn media_type_class(value: Option<&str>) -> &'static str {
    match value {
        None => "absent",
        Some(value) if value.eq_ignore_ascii_case("text/event-stream") => "sse",
        Some(value) if value.eq_ignore_ascii_case("application/json") => "json",
        Some(value) if value.eq_ignore_ascii_case("text/html") => "html",
        Some(value) if value.eq_ignore_ascii_case("text/plain") => "plain",
        Some(_) => "other",
    }
}
fn encoding_class(value: Option<&str>) -> &'static str {
    match value.map(str::trim) {
        None => "absent",
        Some(value) if value.eq_ignore_ascii_case("identity") => "identity",
        Some(value) if value.eq_ignore_ascii_case("gzip") => "gzip",
        Some(value) if value.eq_ignore_ascii_case("br") => "br",
        Some(value) if value.eq_ignore_ascii_case("deflate") => "deflate",
        Some(value) if value.eq_ignore_ascii_case("zstd") => "zstd",
        Some(_) => "other",
    }
}

// Codex keeps no upstream response state; retain the caller's complete input and
// instructions, adapting only backend fields without inventing calls or results.
pub(crate) fn codex_request(body: &mut Value) -> Result<(), InvalidContract> {
    let object = body
        .as_object_mut()
        .ok_or(InvalidContract("Responses body"))?;
    if ["previous_response_id", "conversation"]
        .iter()
        .any(|key| object.get(*key).is_some_and(|v| !v.is_null()))
    {
        return Err(InvalidContract("Codex requires full history"));
    }
    for field in [
        "max_output_tokens",
        "max_completion_tokens",
        "temperature",
        "top_p",
        "user",
        "safety_identifier",
        "prompt_cache_options",
        "prompt_cache_retention",
        "options",
        "previous_response_id",
        "conversation",
    ] {
        if object.remove(field).is_some() {
            tracing::info!(event = "codex_request_field_omitted", field);
        }
    }
    if object
        .get("service_tier")
        .is_some_and(|v| v != "priority" && v != "ultrafast")
    {
        object.remove("service_tier");
        tracing::info!(
            event = "codex_request_field_omitted",
            field = "service_tier"
        );
    }
    object
        .entry("instructions")
        .or_insert(Value::String(String::new()));
    object.insert("store".into(), Value::Bool(false));
    object.insert("stream".into(), Value::Bool(true));
    if let Some(Value::String(text)) = object.get("input") {
        object.insert(
            "input".into(),
            serde_json::json!([{"role":"user","content":[{"type":"input_text","text":text}]}]),
        );
    }
    if let Some(Value::Array(input)) = object.get_mut("input") {
        for item in input {
            if let Some(item) = item.as_object_mut() {
                if item.get("type").and_then(Value::as_str) == Some("item_reference") {
                    return Err(InvalidContract("Codex requires inline history"));
                }
                if item.remove("id").is_some() {
                    tracing::info!(event = "codex_request_field_omitted", field = "input.id");
                }
                if item.get("role").and_then(Value::as_str) == Some("system") {
                    item.insert("role".into(), Value::String("developer".into()));
                }
            }
        }
    }
    Ok(())
}
