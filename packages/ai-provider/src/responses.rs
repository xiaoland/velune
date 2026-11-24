//! What only the OpenAI Responses protocol needs: its business terminal and
//! the Codex subscription profile's stateless backend.
use crate::native::observe_sse;
use serde_json::Value;
use velune_ai::{InvalidContract, NativeErrorKind};

/// The Codex-compatible user agent for a resolved subscription client version.
pub fn codex_user_agent(version: &str) -> String {
    format!("codex_cli_rs/{version} Velune/0.1")
}

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
    let mut terminal = None;
    observe_sse(chunks, |event, value| {
        if let Some(value) = terminal_from_event(Some(event), value) {
            terminal = Some(value);
        }
    })
    .await?;
    terminal
        .map(|terminal| terminal == Terminal::Completed)
        .ok_or(NativeErrorKind::ProviderFailure)
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
    // Only fixed protocol names enter the summary; history contributes a count.
    let mut top_level_fields = Vec::new();
    let mut input_ids_removed = 0usize;
    for field in [
        "max_output_tokens",
        "max_completion_tokens",
        "temperature",
        "top_p",
        "user",
        "safety_identifier",
        // The ChatGPT subscription endpoint rejects API metadata, including
        // metadata forwarded from an Anthropic Messages request.
        "metadata",
        "prompt_cache_options",
        "prompt_cache_retention",
        "options",
        "previous_response_id",
        "conversation",
    ] {
        if object.remove(field).is_some() {
            top_level_fields.push(field);
        }
    }
    if object
        .get("service_tier")
        .is_some_and(|v| v != "priority" && v != "ultrafast")
    {
        object.remove("service_tier");
        top_level_fields.push("service_tier");
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
    } else if let Some(Value::Array(input)) = object.get_mut("input") {
        for item in input {
            if let Some(item) = item.as_object_mut() {
                if item.get("type").and_then(Value::as_str) == Some("item_reference") {
                    return Err(InvalidContract("Codex requires inline history"));
                }
                input_ids_removed += usize::from(item.remove("id").is_some());
                if item.get("role").and_then(Value::as_str) == Some("system") {
                    item.insert("role".into(), Value::String("developer".into()));
                }
            }
        }
    }
    if !top_level_fields.is_empty() || input_ids_removed > 0 {
        tracing::info!(
            event = "codex_request_field_omitted",
            ?top_level_fields,
            input_ids_removed
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        collections::BTreeMap,
        fmt,
        sync::{Arc, Mutex},
    };
    use tracing::{Event, Metadata, Subscriber, field::Field, span};

    // Capture the actual events, including any accidentally emitted per history item.
    #[derive(Default)]
    struct Events(Mutex<Vec<BTreeMap<&'static str, String>>>);

    impl Subscriber for Events {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
            span::Id::from_u64(1)
        }
        fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
        fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
        fn enter(&self, _: &span::Id) {}
        fn exit(&self, _: &span::Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut fields = BTreeMap::new();
            event.record(&mut |field: &Field, value: &dyn fmt::Debug| {
                fields.insert(field.name(), format!("{value:?}"));
            });
            self.0.lock().unwrap().push(fields);
        }
    }

    #[test]
    fn chatgpt_subscription_omits_unsupported_metadata_without_dropping_input() {
        let mut request = json!({"model":"synthetic", "stream":true,
            "metadata":{"user_id":"synthetic-user"},
            "input":[{"role":"user","content":"keep this input"}]});
        let input = request["input"].clone();
        codex_request(&mut request).unwrap();
        assert!(request.get("metadata").is_none());
        assert_eq!(request["input"], input);
        assert_eq!(request["store"], false);
    }

    #[test]
    fn codex_request_summarizes_omissions_once_and_preserves_inline_history() {
        let mut input =
            vec![json!({"role":"system","content":"synthetic instructions","id":null}); 256];
        input.push(json!({"type":"function_call_output","call_id":"keep-call","output":"synthetic result"}));
        let mut body = json!({
            "instructions":"caller instructions", "input":input,
            "temperature":0.5, "user":"synthetic user", "service_tier":"synthetic tier",
            "metadata":{"user_id":"synthetic-user"},
            "synthetic extension":{"id":"keep-nested-id"}
        });
        let events = Arc::new(Events::default());
        tracing::subscriber::with_default(events.clone(), || codex_request(&mut body)).unwrap();
        let mut expected_input =
            vec![json!({"role":"developer","content":"synthetic instructions"}); 256];
        expected_input.push(input.pop().unwrap());
        assert_eq!(
            body,
            json!({
                "instructions":"caller instructions","input":expected_input,
                "synthetic extension":{"id":"keep-nested-id"},"stream":true,"store":false
            })
        );
        let records = events.0.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0],
            BTreeMap::from([
                ("event", r#""codex_request_field_omitted""#.into()),
                (
                    "top_level_fields",
                    r#"["temperature", "user", "metadata", "service_tier"]"#.into()
                ),
                ("input_ids_removed", "256".into()),
            ])
        );
    }

    #[test]
    fn codex_request_only_reports_omissions_for_completed_conversions() {
        let events = Arc::new(Events::default());
        tracing::subscriber::with_default(events.clone(), || {
            for tier in ["priority", "ultrafast"] {
                let mut body = json!({"input":"synthetic text","service_tier":tier});
                codex_request(&mut body).unwrap();
                assert_eq!(
                    body,
                    json!({
                        "input":[{"role":"user","content":[{"type":"input_text","text":"synthetic text"}]}],
                        "service_tier":tier, "instructions":"", "store":false, "stream":true
                    })
                );
            }
            for (mut body, reason) in [
                (
                    json!({"conversation":"synthetic"}),
                    "Codex requires full history",
                ),
                (
                    json!({"temperature":0, "input":[{"id":"synthetic"},{"type":"item_reference"}]}),
                    "Codex requires inline history",
                ),
            ] {
                assert_eq!(codex_request(&mut body), Err(InvalidContract(reason)));
            }
        });
        assert!(events.0.lock().unwrap().is_empty());
    }
}
