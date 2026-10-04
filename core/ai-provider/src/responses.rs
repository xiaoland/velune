//! Native OpenAI Responses v1 provider adapter.
//!
//! The adapter only replaces the logical model with its provider mapping. All
//! other request JSON and upstream response bytes remain native Responses data.
use crate::config::{
    ProtocolConfig, ProviderConfig, ResolvedCredential, Transport, parse_resolved_credential,
};
use reqwest::{Client, header::HeaderValue};
use serde_json::Value;
use std::sync::Arc;
use velune_ai::{InvalidContract, OperationFuture, Payload, responses::*};

type CredentialResolver = dyn Fn(&str, Option<&str>) -> Option<String> + Send + Sync;

pub struct OpenAiResponses {
    config: ProviderConfig,
    client: Client,
    credential: Arc<dyn Fn() -> Option<ResolvedCredential> + Send + Sync>,
}

impl OpenAiResponses {
    pub fn with_resolver(
        config: ProviderConfig,
        client: Client,
        resolver: Arc<CredentialResolver>,
    ) -> Result<Self, InvalidContract> {
        let ProtocolConfig::Responses(protocol) = config.protocol() else {
            return Err(InvalidContract("provider requires OpenAI Responses v1"));
        };
        if !matches!(
            protocol.endpoint.transport(),
            Transport::Http | Transport::Https
        ) {
            return Err(InvalidContract("unsupported provider transport"));
        }
        let reference = config.credential().as_str().to_owned();
        let source = config.credential_source().map(str::to_owned);
        let source_present = source.is_some();
        Ok(Self {
            config,
            client,
            credential: Arc::new(move || {
                resolver(&reference, source.as_deref()).and_then(|value| {
                    parse_resolved_credential(&value, source_present, "responsesV1")
                })
            }),
        })
    }

    fn request_json(body: &Value, external_model: &str) -> Result<Value, InvalidContract> {
        let mut body = body
            .as_object()
            .cloned()
            .ok_or(InvalidContract("Responses body must be an object"))?;
        body.insert("model".into(), Value::String(external_model.to_owned()));
        let body = Value::Object(body);
        if serde_json::to_vec(&body)
            .map_err(|_| InvalidContract("Responses body encoding"))?
            .len()
            > 256 * 1024
        {
            return Err(InvalidContract("provider request is too large"));
        }
        Ok(body)
    }

    fn failure(
        kind: ResponsesErrorKind,
        status: Option<u16>,
        body: Option<Vec<u8>>,
        submitted: bool,
    ) -> ResponsesCompletion {
        Self::terminal_failure(kind, status, body, submitted, None)
    }

    fn terminal_failure(
        kind: ResponsesErrorKind,
        status: Option<u16>,
        body: Option<Vec<u8>>,
        submitted: bool,
        terminal: Option<ResponsesTerminal>,
    ) -> ResponsesCompletion {
        ResponsesCompletion {
            result: Err(ResponsesFailure {
                kind,
                status,
                body: body.map(Payload::new),
                submitted,
                terminal,
            }),
        }
    }
}

fn terminal_from_status(status: &str) -> Option<ResponsesTerminal> {
    match status {
        "completed" => Some(ResponsesTerminal::Completed),
        "incomplete" => Some(ResponsesTerminal::Incomplete),
        "failed" => Some(ResponsesTerminal::Failed),
        "cancelled" | "canceled" => Some(ResponsesTerminal::Cancelled),
        _ => None,
    }
}

fn unsupported_subscription_field(body: &Value, credential: &ResolvedCredential) -> bool {
    if credential.explicit_output_cap != Some(false) {
        return false;
    }
    [
        "max_output_tokens",
        "temperature",
        "prompt_cache_retention",
        "prompt_cache_options",
        "prompt_cache_key",
        "options",
    ]
    .iter()
    .any(|field| body.get(*field).is_some())
}

fn violates_subscription_contract(
    body: &Value,
    stream: bool,
    credential: &ResolvedCredential,
) -> bool {
    if !credential.subscription {
        return false;
    }
    !stream
        || body.get("stream") != Some(&Value::Bool(true))
        || body.get("store") != Some(&Value::Bool(false))
        || body.get("previous_response_id").is_some()
}

fn terminal_from_event(event: Option<&str>, value: &Value) -> Option<ResponsesTerminal> {
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

fn response_terminal(body: &[u8]) -> Result<ResponsesTerminal, ()> {
    let value: Value = serde_json::from_slice(body).map_err(|_| ())?;
    value["status"]
        .as_str()
        .and_then(terminal_from_status)
        .ok_or(())
}

fn observe_sse(buffer: &mut Vec<u8>, terminal: &mut Option<ResponsesTerminal>) -> Result<(), ()> {
    while let Some((position, delimiter)) = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| (position, 2))
        .or_else(|| {
            buffer
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|position| (position, 4))
        })
    {
        let frame = buffer.drain(..position + delimiter).collect::<Vec<_>>();
        let frame = std::str::from_utf8(&frame).map_err(|_| ())?;
        let mut event = None;
        let mut data = String::new();
        for line in frame.lines() {
            let line = line.trim_end_matches('\r');
            if let Some(value) = line.strip_prefix("event:") {
                event = Some(value.trim());
            } else if let Some(value) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value.trim_start());
            }
        }
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let value: Value = serde_json::from_str(&data).map_err(|_| ())?;
        if let Some(value) = terminal_from_event(event, &value) {
            *terminal = Some(value);
        }
    }
    Ok(())
}

impl ResponsesProvider for OpenAiResponses {
    fn responses(
        &self,
        request: ResponsesRequest,
        mut events: ResponsesSink,
    ) -> OperationFuture<ResponsesCompletion> {
        let Some(mapping) = self.config.model(&request.model) else {
            return Box::pin(async {
                Self::failure(ResponsesErrorKind::InvalidInput, None, None, false)
            });
        };
        let body = match Self::request_json(request.body.get(), mapping.external_name()) {
            Ok(body) => body,
            Err(_) => {
                return Box::pin(async {
                    Self::failure(ResponsesErrorKind::InvalidInput, None, None, false)
                });
            }
        };
        let Some(protocol) = (match self.config.protocol() {
            ProtocolConfig::Responses(value) => Some(value),
            _ => None,
        }) else {
            return Box::pin(async {
                Self::failure(ResponsesErrorKind::Unsupported, None, None, false)
            });
        };
        let endpoint = protocol.endpoint.url("/responses");
        let client = self.client.clone();
        let credential = Arc::clone(&self.credential);
        let stream = request.stream;
        Box::pin(async move {
            let Some(credential) = credential() else {
                return Self::failure(ResponsesErrorKind::Authentication, None, None, false);
            };
            if violates_subscription_contract(&body, stream, &credential)
                || unsupported_subscription_field(&body, &credential)
            {
                return Self::failure(ResponsesErrorKind::Unsupported, Some(422), None, false);
            }
            let mut auth = match HeaderValue::from_str(&format!("Bearer {}", credential.token)) {
                Ok(value) => value,
                Err(_) => {
                    return Self::failure(ResponsesErrorKind::InvalidInput, None, None, false);
                }
            };
            auth.set_sensitive(true);
            let request = match client
                .post(endpoint)
                .header("authorization", auth)
                .header(
                    "accept",
                    if stream {
                        "text/event-stream"
                    } else {
                        "application/json"
                    },
                )
                .header("connection", "close")
                .json(&body)
                .build()
            {
                Ok(request) => request,
                Err(_) => {
                    return Self::failure(ResponsesErrorKind::InvalidInput, None, None, false);
                }
            };
            let response = match client.execute(request).await {
                Ok(response) => response,
                Err(error) => {
                    return Self::failure(
                        if error.is_timeout() {
                            ResponsesErrorKind::Timeout
                        } else {
                            ResponsesErrorKind::Transport
                        },
                        None,
                        None,
                        true,
                    );
                }
            };
            let status = response.status().as_u16();
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            if !(200..300).contains(&status) {
                let body = response.bytes().await.ok().map(|value| value.to_vec());
                return Self::failure(
                    match status {
                        401 => ResponsesErrorKind::Authentication,
                        403 => ResponsesErrorKind::Permission,
                        429 => ResponsesErrorKind::RateLimited,
                        _ => ResponsesErrorKind::ProviderFailure,
                    },
                    Some(status),
                    body,
                    true,
                );
            }
            if stream {
                let is_sse = content_type
                    .as_deref()
                    .and_then(|value| value.split(';').next())
                    == Some("text/event-stream");
                if !is_sse {
                    return Self::failure(
                        ResponsesErrorKind::Unsupported,
                        Some(status),
                        None,
                        true,
                    );
                }
                events(ResponsesEvent::Headers {
                    status,
                    content_type,
                });
                let mut chunks = response.bytes_stream();
                let mut sse_buffer = Vec::new();
                let mut terminal = None;
                use futures_util::StreamExt;
                while let Some(chunk) = chunks.next().await {
                    let chunk = match chunk {
                        Ok(chunk) => chunk,
                        Err(_) => {
                            return Self::failure(
                                ResponsesErrorKind::Transport,
                                Some(status),
                                None,
                                true,
                            );
                        }
                    };
                    if !chunk.is_empty() {
                        let chunk = chunk.to_vec();
                        events(ResponsesEvent::Body(Payload::new(chunk.clone())));
                        sse_buffer.extend_from_slice(&chunk);
                        if observe_sse(&mut sse_buffer, &mut terminal).is_err() {
                            return Self::failure(
                                ResponsesErrorKind::ProviderFailure,
                                Some(status),
                                None,
                                true,
                            );
                        }
                        if let Some(terminal) = terminal {
                            if terminal == ResponsesTerminal::Completed {
                                return ResponsesCompletion {
                                    result: Ok(ResponsesOutput::Stream(terminal)),
                                };
                            }
                            return Self::terminal_failure(
                                ResponsesErrorKind::ProviderFailure,
                                Some(status),
                                None,
                                true,
                                Some(terminal),
                            );
                        }
                    }
                }
                Self::failure(
                    ResponsesErrorKind::ProviderFailure,
                    Some(status),
                    None,
                    true,
                )
            } else {
                let body = match response.bytes().await {
                    Ok(body) => body.to_vec(),
                    Err(_) => {
                        return Self::failure(
                            ResponsesErrorKind::Transport,
                            Some(status),
                            None,
                            true,
                        );
                    }
                };
                let terminal = match response_terminal(&body) {
                    Ok(terminal) => terminal,
                    Err(_) => {
                        return Self::failure(
                            ResponsesErrorKind::ProviderFailure,
                            Some(status),
                            Some(body),
                            true,
                        );
                    }
                };
                if terminal != ResponsesTerminal::Completed {
                    return Self::terminal_failure(
                        ResponsesErrorKind::ProviderFailure,
                        Some(status),
                        Some(body),
                        true,
                        Some(terminal),
                    );
                }
                ResponsesCompletion {
                    result: Ok(ResponsesOutput::Json(Payload::new(body), terminal)),
                }
            }
        })
    }
}
