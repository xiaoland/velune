//! MiniMax Chat Completions adapter. All vendor JSON and SSE stay on this side of the contract.
//! The app supplies the already-configured HTTP client and resolved in-memory credential.
mod capture;
mod framing;
pub use capture::fixture_is_safe;
mod mapping;
use crate::config::{ProtocolConfig, ProviderConfig, Transport};
use eventsource_stream::{EventStreamError, Eventsource};
pub use framing::FramingEvidence;
use futures_util::StreamExt;
pub use mapping::Decoder;
use reqwest::{Client, header::HeaderValue};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use velune_ai::{InvalidContract, OperationFuture, Payload, provider::*, sampling::*};

pub const ENDPOINT: &str = "https://api.minimax.cn/v1/chat/completions";
pub const MODEL: &str = "MiniMax-M3";
/// Only projected JSON crosses this optional explicit capture boundary. No default logging.
/// Request content must be separately authorized for capture by the composition root.
#[derive(Clone)]
pub enum ProtocolRecord {
    Request(Payload<Value>),
    Status(u16),
    Chunk(Payload<Value>),
    Done,
    StreamEnd {
        classification: StreamEnd,
        framing: FramingEvidence,
    },
}
#[derive(Debug, Clone, Copy)]
pub enum StreamEnd {
    DoneDecoded,
    CleanEofWithoutDone,
    Transport,
    Timeout,
    Body,
    Decode,
    SizeLimit,
    SseParser,
    Utf8,
}
pub type Capture = Arc<dyn Fn(ProtocolRecord) + Send + Sync>;

pub struct MiniMax {
    config: ProviderConfig,
    client: Client,
    credential: Payload<String>,
    capture: Option<Capture>,
}
impl MiniMax {
    /// Client must be assembled with redirects/retries disabled, normal proxy/TLS validation,
    /// and a finite whole-request timeout. Client construction (including env proxy resolution)
    /// belongs to app main, not this library.
    pub fn new(
        config: ProviderConfig,
        client: Client,
        credential: Payload<String>,
        capture: Option<Capture>,
    ) -> Result<Self, InvalidContract> {
        let ProtocolConfig::ChatCompletions(protocol) = config.protocol() else {
            return Err(InvalidContract("MiniMax requires Chat Completions"));
        };
        let ep = &protocol.endpoint;
        if ep.transport() != Transport::Https
            || ep.host() != "api.minimax.cn"
            || ep.port().get() != 443
            || ep.base_path() != "/v1"
        {
            return Err(InvalidContract(
                "MiniMax endpoint is outside this bounded adapter",
            ));
        }
        if config.models().len() != 1 || config.models()[0].external_name() != MODEL {
            return Err(InvalidContract(
                "MiniMax adapter supports only the reviewed model",
            ));
        }
        if credential.get().is_empty()
            || HeaderValue::from_str(&format!("Bearer {}", credential.get())).is_err()
        {
            return Err(InvalidContract("invalid credential header"));
        }
        Ok(Self {
            config,
            client,
            credential,
            capture,
        })
    }
}
impl SamplingProvider for MiniMax {
    fn sampling(
        &self,
        request: ProviderSamplingRequest,
        mut events: ProviderSamplingSink,
    ) -> OperationFuture<ProviderSamplingOutcome> {
        let valid_target = request.context.provider == *self.config.id()
            && request.context.revision == self.config.revision()
            && request.context.model == *self.config.models()[0].id();
        let body = if valid_target {
            request_json(&request.input)
        } else {
            Err(InvalidContract("request binding mismatch"))
        };
        let client = self.client.clone();
        let credential = self.credential.clone();
        let capture = self.capture.clone();
        Box::pin(async move {
            let mut decoder = Decoder::new();
            let body = match body {
                Ok(body) => body,
                Err(_) => {
                    return decoder.failure(
                        SamplingErrorKind::Unsupported,
                        ExecutionKnowledge::NotSent,
                        false,
                    );
                }
            };
            if let Some(capture) = &capture {
                capture(ProtocolRecord::Request(Payload::new(body.clone())));
            }
            let mut auth = match HeaderValue::from_str(&format!("Bearer {}", credential.get())) {
                Ok(value) => value,
                Err(_) => {
                    return decoder.failure(
                        SamplingErrorKind::InvalidInput,
                        ExecutionKnowledge::NotSent,
                        false,
                    );
                }
            };
            auth.set_sensitive(true);
            let request = match client
                .post(ENDPOINT)
                .header("authorization", auth)
                .header("accept", "text/event-stream")
                .json(&body)
                .build()
            {
                Ok(request) => request,
                Err(_) => {
                    return decoder.failure(
                        SamplingErrorKind::InvalidInput,
                        ExecutionKnowledge::NotSent,
                        false,
                    );
                }
            };
            // Exactly one execute, no loop/retry. Errors are classified, never formatted.
            let response = match client.execute(request).await {
                Ok(response) => response,
                Err(error) => {
                    return decoder.failure(
                        if error.is_timeout() {
                            SamplingErrorKind::Timeout
                        } else {
                            SamplingErrorKind::Transport
                        },
                        ExecutionKnowledge::Unknown,
                        true,
                    );
                }
            };
            let status = response.status().as_u16();
            if let Some(capture) = &capture {
                capture(ProtocolRecord::Status(status));
            }
            if status != 200 {
                let kind = match status {
                    401 => SamplingErrorKind::Authentication,
                    403 => SamplingErrorKind::Permission,
                    429 => SamplingErrorKind::RateLimited,
                    _ => SamplingErrorKind::ProviderFailure,
                };
                return decoder.failure(kind, ExecutionKnowledge::Unknown, true);
            }
            let sse = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.split(';').next() == Some("text/event-stream"));
            if !sse {
                return decoder.failure(
                    SamplingErrorKind::Unsupported,
                    ExecutionKnowledge::Accepted,
                    true,
                );
            }
            // Bound the whole stream before SSE buffering, not just each completed event.
            let observer = Arc::new(Mutex::new(framing::FramingObserver::default()));
            let wire_observer = Arc::clone(&observer);
            let record_end = |classification| {
                if let Some(capture) = &capture {
                    capture(ProtocolRecord::StreamEnd {
                        classification,
                        framing: observer.lock().expect("framing").evidence(),
                    });
                }
            };
            let mut bytes = 0_usize;
            let body = response.bytes_stream().map(move |chunk| {
                let chunk = chunk.map_err(|e| {
                    if e.is_timeout() {
                        StreamEnd::Timeout
                    } else if e.is_body() {
                        StreamEnd::Body
                    } else if e.is_decode() {
                        StreamEnd::Decode
                    } else {
                        StreamEnd::Transport
                    }
                })?;
                bytes = bytes.saturating_add(chunk.len());
                if bytes > 262_144 {
                    return Err(StreamEnd::SizeLimit);
                }
                wire_observer.lock().expect("framing").push(&chunk);
                Ok(chunk)
            });
            let mut stream = body.eventsource();
            while let Some(event) = stream.next().await {
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        let classification = match error {
                            EventStreamError::Transport(kind) => kind,
                            EventStreamError::Utf8(_) => StreamEnd::Utf8,
                            EventStreamError::Parser(_) => StreamEnd::SseParser,
                        };
                        record_end(classification);
                        return decoder.failure(
                            if matches!(classification, StreamEnd::Timeout) {
                                SamplingErrorKind::Timeout
                            } else {
                                SamplingErrorKind::Transport
                            },
                            ExecutionKnowledge::Accepted,
                            true,
                        );
                    }
                };
                if event.event != "message" && !event.event.is_empty() {
                    return decoder.failure(
                        SamplingErrorKind::Unsupported,
                        ExecutionKnowledge::Accepted,
                        true,
                    );
                }
                if event.data == "[DONE]" {
                    if let Some(capture) = &capture {
                        capture(ProtocolRecord::Done);
                    }
                    record_end(StreamEnd::DoneDecoded);
                    return decoder.complete();
                }
                let raw: Value = match serde_json::from_str(&event.data) {
                    Ok(value) => value,
                    Err(_) => {
                        return decoder.failure(
                            SamplingErrorKind::ProviderFailure,
                            ExecutionKnowledge::Accepted,
                            true,
                        );
                    }
                };
                // Do not capture any record that reflects the injected credential/placeholder.
                if event.data.contains(credential.get()) {
                    return decoder.failure(
                        SamplingErrorKind::ProviderFailure,
                        ExecutionKnowledge::Accepted,
                        true,
                    );
                }
                let projected = mapping::project(&raw);
                if let Some(capture) = &capture {
                    capture(ProtocolRecord::Chunk(Payload::new(projected.clone())));
                }
                if let Err(kind) = decoder.push(&projected, &mut events) {
                    return decoder.failure(kind, ExecutionKnowledge::Accepted, true);
                }
            }
            record_end(StreamEnd::CleanEofWithoutDone);
            let framing = observer.lock().expect("framing").evidence();
            decoder.clean_eof(framing)
        })
    }
}

/// Pure request mapping, available to a manual composition root for preflight review/replay.
pub fn request_json(input: &SamplingInput) -> Result<Value, InvalidContract> {
    if input.max_output_tokens().get() > 1024 {
        return Err(InvalidContract("bounded adapter output limit"));
    }
    let mut messages = Vec::new();
    if let Some(instructions) = input.instructions() {
        messages.push(json!({"role":"system","content":instructions}));
    }
    for message in input.messages() {
        messages.push(match message {
            Message::User(text) => json!({"role":"user","content":text.get()}),
            Message::Assistant { text, tool_calls } => {
                let mut value = json!({"role":"assistant","content":text.as_ref().map(Payload::get)});
                if !tool_calls.is_empty() {
                    value["tool_calls"] = tool_calls.iter().map(|tool| json!({"id":tool.id.as_str(),"type":"function","function":{"name":tool.name.as_str(),"arguments":tool.arguments.get().to_string()}})).collect();
                }
                value
            }
            Message::ToolResult { call, content, is_error } => {
                // No exact protocol representation for the generic error flag in this slice.
                if *is_error { return Err(InvalidContract("error tool result is unsupported")); }
                json!({"role":"tool","tool_call_id":call.as_str(),"content":content.get()})
            }
        });
    }
    let mut body = json!({"model":MODEL,"messages":messages,"max_tokens":input.max_output_tokens().get(),"thinking":{"type":"disabled"},"stream":true,"stream_options":{"include_usage":true},"service_tier":"standard"});
    if !input.tools().is_empty() {
        body["tools"] = input.tools().iter().map(|tool| json!({"type":"function","function":{"name":tool.name.as_str(),"description":tool.description.get(),"parameters":tool.schema()}})).collect();
        body["tool_choice"] = json!("auto");
    }
    if body.to_string().len() > 4096 {
        return Err(InvalidContract("bounded adapter request size"));
    }
    Ok(body)
}
