//! One bounded text generation, for application features without agent tools.
//! The caller resolves provider, model and credentials; this boundary owns
//! the native request and response shapes. No routing or hidden retries.
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use velune_ai::{
    DeliveryError, NativeErrorKind, NativeEvent, NativeOutput, NativeProvider, NativeRequest,
    Payload, Protocol, ProviderModelId, http::Header,
};

pub async fn generate(
    provider: Box<dyn NativeProvider>,
    protocol: Protocol,
    model: ProviderModelId,
    prompt: String,
    stream: bool,
) -> Result<String, NativeErrorKind> {
    let mut body = match protocol {
        Protocol::ChatCompletions => json!({"messages":[{"role":"user","content":prompt}]}),
        Protocol::Responses => json!({"input":[{"role":"user","content":prompt}]}),
        Protocol::Messages => {
            json!({"messages":[{"role":"user","content":prompt}],"max_tokens":1024})
        }
    };
    body["model"] = json!(model.as_str());
    body["stream"] = json!(stream);
    let received = Arc::new(Mutex::new(Vec::<u8>::new()));
    let sink = received.clone();
    let success = provider
        .send(
            NativeRequest {
                model,
                body: Payload::new(body),
                headers: if protocol == Protocol::Messages {
                    vec![Header {
                        name: "anthropic-version".into(),
                        value: "2023-06-01".into(),
                    }]
                } else {
                    Vec::new()
                },
                stream,
            },
            Box::new(move |event| {
                let sink = sink.clone();
                Box::pin(async move {
                    if let NativeEvent::Body(bytes) = event {
                        let mut sink = sink.lock().expect("text response buffer");
                        if sink.len() + bytes.get().len() > 2 * 1024 * 1024 {
                            return Err(DeliveryError::Closed);
                        }
                        sink.extend(bytes.into_inner());
                    }
                    Ok(())
                })
            }),
        )
        .await
        .map_err(|failure| failure.kind)?;
    if protocol == Protocol::Responses && !success.completed {
        return Err(NativeErrorKind::ProviderFailure);
    }
    let value = match success.output {
        NativeOutput::Json(body) => {
            serde_json::from_slice(body.body.get()).map_err(|_| NativeErrorKind::ProviderFailure)?
        }
        NativeOutput::Stream(_) => {
            // Streamed Responses includes its final native response. Do not
            // accept partial deltas as a completed application result.
            if protocol != Protocol::Responses {
                return Err(NativeErrorKind::ProviderFailure);
            }
            let bytes = std::mem::take(&mut *received.lock().expect("text response buffer"));
            let mut events = futures_util::stream::once(async { Ok::<_, std::io::Error>(bytes) })
                .boxed()
                .eventsource();
            let mut response = None;
            while let Some(event) = events.next().await {
                let event = event.map_err(|_| NativeErrorKind::ProviderFailure)?;
                if event.data == "[DONE]" {
                    continue;
                }
                let value: Value = serde_json::from_str(&event.data)
                    .map_err(|_| NativeErrorKind::ProviderFailure)?;
                if value["type"] == "response.completed" {
                    response = Some(value["response"].clone());
                }
            }
            response.ok_or(NativeErrorKind::ProviderFailure)?
        }
    };
    let text = match protocol {
        Protocol::ChatCompletions if value["choices"][0]["finish_reason"] == "stop" => {
            value["choices"][0]["message"]["content"]
                .as_str()
                .map(str::to_owned)
        }
        Protocol::Messages
            if matches!(
                value["stop_reason"].as_str(),
                Some("end_turn" | "stop_sequence")
            ) =>
        {
            value["content"].as_array().map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b["type"] == "text")
                    .filter_map(|b| b["text"].as_str())
                    .collect::<String>()
            })
        }
        Protocol::Responses => value["output"].as_array().map(|items| {
            items
                .iter()
                .filter(|item| item["type"] == "message")
                .flat_map(|item| item["content"].as_array().into_iter().flatten())
                .filter(|block| block["type"] == "output_text")
                .filter_map(|block| block["text"].as_str())
                .collect::<String>()
        }),
        Protocol::ChatCompletions | Protocol::Messages => None,
    }
    .ok_or(NativeErrorKind::ProviderFailure)?;
    if text.trim().is_empty() {
        return Err(NativeErrorKind::ProviderFailure);
    }
    Ok(text)
}
