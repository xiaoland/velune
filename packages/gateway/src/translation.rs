//! Cross-protocol delivery. Native transport remains in the provider adapters.
use crate::{
    config::GatewayProtocol,
    runtime::{WireEvent, send_json},
};
use eventsource_stream::Eventsource;
use futures_util::{StreamExt, stream};
use serde_json::json;
use tokio::sync::mpsc;
use velune_ai::{
    Payload,
    http::{Header, ResponseBody, ResponseMeta},
};
use velune_ai_provider::translation::{LlmProtocol, PreparedTranslation};

const MAX_TRANSLATED_BODY: usize = 16 * 1024 * 1024;

pub(super) fn protocol(value: &GatewayProtocol) -> LlmProtocol {
    match value {
        GatewayProtocol::ChatCompletionsV1 => LlmProtocol::ChatCompletions,
        GatewayProtocol::ResponsesV1 => LlmProtocol::Responses,
        GatewayProtocol::MessagesV1 => LlmProtocol::Messages,
    }
}

pub(super) fn upstream_headers(mut headers: Vec<Header>, target: &GatewayProtocol) -> Vec<Header> {
    headers.retain(|header| {
        let name = header.name.to_ascii_lowercase();
        !matches!(
            name.as_str(),
            "content-type" | "accept" | "content-encoding"
        ) && if *target == GatewayProtocol::MessagesV1 {
            !name.starts_with("openai-")
        } else {
            !name.starts_with("anthropic-")
        }
    });
    if *target == GatewayProtocol::MessagesV1
        && !headers
            .iter()
            .any(|header| header.name.eq_ignore_ascii_case("anthropic-version"))
    {
        // Only synthesized for a cross-protocol Messages request. Native
        // Messages callers keep their explicit API version unchanged.
        headers.push(Header {
            name: "anthropic-version".into(),
            value: "2023-06-01".into(),
        });
    }
    headers
}

fn converted_meta(mut meta: ResponseMeta, streaming: bool) -> ResponseMeta {
    meta.headers.retain(|header| {
        !matches!(
            header.name.to_ascii_lowercase().as_str(),
            "content-type"
                | "content-length"
                | "content-encoding"
                | "etag"
                | "content-md5"
                | "digest"
        )
    });
    meta.headers.push(Header {
        name: "content-type".into(),
        value: if streaming {
            "text/event-stream"
        } else {
            "application/json"
        }
        .into(),
    });
    meta
}

async fn interrupted(sender: &mpsc::Sender<WireEvent>, committed: bool, code: &'static str) {
    tracing::warn!(event = "gateway_translation_failed", code, committed);
    if committed {
        let _ = sender.send(WireEvent::Failed).await;
    } else {
        send_json(sender, ResponseBody {
            meta: ResponseMeta { status: 502, headers: vec![Header { name: "content-type".into(), value: "application/json".into() }] },
            body: Payload::new(json!({"error":{"message":"upstream protocol conversion interrupted", "code":code}}).to_string().into_bytes()),
        }).await;
    }
}

pub(super) async fn deliver(
    mut plan: PreparedTranslation,
    model: String,
    streaming: bool,
    mut receiver: mpsc::Receiver<WireEvent>,
    sender: mpsc::Sender<WireEvent>,
) {
    for note in plan.notes() {
        tracing::info!(
            event = "gateway_translation_note",
            code = note.code,
            field = note.field,
            phase = "request"
        );
    }
    let Some(WireEvent::Headers(meta)) = receiver.recv().await else {
        interrupted(&sender, false, "upstream_headers_missing").await;
        return;
    };
    // HTTP failures retain the upstream status and body. They are not a
    // successful LLM output to be translated into another business protocol.
    if !(200..300).contains(&meta.status) || !streaming {
        let mut bytes = Vec::new();
        while let Some(event) = receiver.recv().await {
            match event {
                WireEvent::Body(chunk)
                    if bytes.len().saturating_add(chunk.len()) <= MAX_TRANSLATED_BODY =>
                {
                    bytes.extend(chunk)
                }
                _ => {
                    interrupted(&sender, false, "upstream_body_interrupted").await;
                    return;
                }
            }
        }
        if !(200..300).contains(&meta.status) {
            send_json(
                &sender,
                ResponseBody {
                    meta,
                    body: Payload::new(bytes),
                },
            )
            .await;
            return;
        }
        let body = match serde_json::from_slice(&bytes)
            .ok()
            .and_then(|value| plan.translate_json(&value).ok())
        {
            Some(value) => value,
            None => {
                interrupted(&sender, false, "invalid_upstream_json").await;
                return;
            }
        };
        for note in plan.notes() {
            tracing::info!(
                event = "gateway_translation_note",
                code = note.code,
                field = note.field,
                phase = "response"
            );
        }
        send_json(
            &sender,
            ResponseBody {
                meta: converted_meta(meta, false),
                body: Payload::new(body.to_string().into_bytes()),
            },
        )
        .await;
        return;
    }

    let bytes = stream::unfold((receiver, 0usize), |(mut receiver, total)| async move {
        let event = receiver.recv().await?;
        let result = match event {
            WireEvent::Body(bytes) if total.saturating_add(bytes.len()) <= MAX_TRANSLATED_BODY => {
                Ok(bytes)
            }
            _ => Err(std::io::Error::other("upstream stream interrupted")),
        };
        let total = total.saturating_add(result.as_ref().map_or(0, Vec::len));
        Some((result, (receiver, total)))
    });
    let events = bytes.eventsource();
    futures_util::pin_mut!(events);
    let mut translator = plan.stream(model);
    let mut committed = false;
    let mut meta = Some(converted_meta(meta, true));
    while let Some(event) = events.next().await {
        let output = match event {
            Ok(event) => translator.push_event(&event.event, &event.data),
            Err(_) => {
                interrupted(&sender, committed, "upstream_stream_interrupted").await;
                return;
            }
        };
        let output = match output {
            Ok(bytes) => bytes,
            Err(_) => {
                interrupted(&sender, committed, "invalid_upstream_event").await;
                return;
            }
        };
        if !output.is_empty() {
            if let Some(meta) = meta.take() {
                if sender.send(WireEvent::Headers(meta)).await.is_err() {
                    return;
                }
                committed = true;
            }
            if sender.send(WireEvent::Body(output)).await.is_err() {
                return;
            }
        }
    }
    let output = match translator.finish() {
        Ok(bytes) => bytes,
        Err(_) => {
            interrupted(&sender, committed, "upstream_terminal_missing").await;
            return;
        }
    };
    for note in translator.notes() {
        tracing::info!(
            event = "gateway_translation_note",
            code = note.code,
            field = note.field,
            phase = "stream"
        );
    }
    if let Some(meta) = meta.take()
        && sender.send(WireEvent::Headers(meta)).await.is_err()
    {
        return;
    }
    if !output.is_empty() {
        let _ = sender.send(WireEvent::Body(output)).await;
    }
}
