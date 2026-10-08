//! Native OpenAI Responses v1 provider adapter.
//!
//! The adapter only replaces the logical model with its provider mapping. All
//! other request JSON and upstream response bytes remain native Responses data.
use crate::config::{ProtocolConfig, ProviderConfig, ResolvedCredential, Transport};
use reqwest::{Client, Response, header::HeaderValue};
use serde_json::Value;
use std::sync::Arc;
use velune_ai::http::{Header, ResponseBody, ResponseMeta};
use velune_ai::{InvalidContract, OperationFuture, Payload, responses::*};

async fn bounded_body(response: Response) -> Result<Vec<u8>, ()> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ())?;
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub struct OpenAiResponses {
    config: ProviderConfig,
    client: Client,
    credential: Arc<dyn Fn() -> Option<ResolvedCredential> + Send + Sync>,
}

impl OpenAiResponses {
    pub fn with_resolved_credential(
        config: ProviderConfig,
        client: Client,
        credential: ResolvedCredential,
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
        if credential.token.is_empty() {
            return Err(InvalidContract("provider credential is empty"));
        }
        Ok(Self {
            config,
            client,
            credential: Arc::new(move || {
                Some(ResolvedCredential {
                    token: credential.token.clone(),
                    explicit_output_cap: credential.explicit_output_cap,
                    subscription: credential.subscription,
                })
            }),
        })
    }

    fn request_json(body: &Value, provider_model_id: &str) -> Result<Value, InvalidContract> {
        if !body.is_object() || body["model"].as_str() != Some(provider_model_id) {
            return Err(InvalidContract(
                "Responses provider model does not match request body",
            ));
        }
        let body = body.clone();
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
                headers: Vec::new(),
                body: body.map(Payload::new),
                submitted,
                terminal,
            }),
        }
    }
}

fn response_meta(response: &reqwest::Response) -> ResponseMeta {
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            Some(Header {
                name: name.as_str().to_owned(),
                value: value.to_str().ok()?.to_owned(),
            })
        })
        .collect();
    ResponseMeta {
        status: response.status().as_u16(),
        headers,
    }
}

fn failure_with_meta(
    kind: ResponsesErrorKind,
    meta: ResponseMeta,
    body: Option<Vec<u8>>,
    submitted: bool,
) -> ResponsesCompletion {
    ResponsesCompletion {
        result: Err(ResponsesFailure {
            kind,
            status: Some(meta.status),
            headers: meta.headers,
            body: body.map(Payload::new),
            submitted,
            terminal: None,
        }),
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

fn observe_event(
    event: Option<&str>,
    data: &str,
    terminal: &mut Option<ResponsesTerminal>,
) -> Result<(), ()> {
    if data.is_empty() || data == "[DONE]" {
        return Ok(());
    }
    let value: Value = serde_json::from_str(data).map_err(|_| ())?;
    if let Some(value) = terminal_from_event(event, &value) {
        *terminal = Some(value);
    }
    Ok(())
}

impl ResponsesProvider for OpenAiResponses {
    fn responses(
        &self,
        request: ResponsesRequest,
        mut events: ResponsesSink,
    ) -> OperationFuture<ResponsesCompletion> {
        let body = match Self::request_json(request.body.get(), request.model.as_str()) {
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
        let headers = request.headers;
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
            let mut builder = client
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
                .header("connection", "close");
            for header in &headers {
                let name = header.name.to_ascii_lowercase();
                if matches!(
                    name.as_str(),
                    "authorization" | "host" | "content-length" | "connection"
                ) {
                    continue;
                }
                let Ok(value) = HeaderValue::from_str(&header.value) else {
                    continue;
                };
                builder = builder.header(&header.name, value);
            }
            let request = match builder.json(&body).build() {
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
            let meta = response_meta(&response);
            let status = meta.status;
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            if !(200..300).contains(&status) {
                let body = bounded_body(response).await.ok();
                return failure_with_meta(
                    match status {
                        401 => ResponsesErrorKind::Authentication,
                        403 => ResponsesErrorKind::Permission,
                        429 => ResponsesErrorKind::RateLimited,
                        _ => ResponsesErrorKind::ProviderFailure,
                    },
                    meta,
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
                if (events)(ResponsesEvent::Headers(meta.clone()))
                    .await
                    .is_err()
                {
                    return Self::failure(ResponsesErrorKind::Cancelled, Some(status), None, true);
                }
                let delivery_failed =
                    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let delivery_flag = std::sync::Arc::clone(&delivery_failed);
                let raw = futures_util::stream::unfold(
                    (response.bytes_stream(), events, delivery_flag),
                    |(mut chunks, mut events, delivery_failed)| async move {
                        use futures_util::StreamExt;
                        match chunks.next().await {
                            Some(Ok(chunk)) => {
                                if !chunk.is_empty()
                                    && (events)(ResponsesEvent::Body(Payload::new(chunk.to_vec())))
                                        .await
                                        .is_err()
                                {
                                    delivery_failed
                                        .store(true, std::sync::atomic::Ordering::Release);
                                    None
                                } else {
                                    Some((Ok(chunk), (chunks, events, delivery_failed)))
                                }
                            }
                            Some(Err(error)) => {
                                Some((Err(error), (chunks, events, delivery_failed)))
                            }
                            None => None,
                        }
                    },
                );
                use eventsource_stream::Eventsource;
                let chunks = raw.eventsource();
                futures_util::pin_mut!(chunks);
                let mut terminal = None;
                use futures_util::StreamExt;
                while let Some(event) = chunks.next().await {
                    let event = match event {
                        Ok(event) => event,
                        Err(error) => {
                            if delivery_failed.load(std::sync::atomic::Ordering::Acquire) {
                                return Self::failure(
                                    ResponsesErrorKind::Cancelled,
                                    Some(status),
                                    None,
                                    true,
                                );
                            }
                            let kind = match error {
                                eventsource_stream::EventStreamError::Transport(_) => {
                                    ResponsesErrorKind::Transport
                                }
                                eventsource_stream::EventStreamError::Utf8(_)
                                | eventsource_stream::EventStreamError::Parser(_) => {
                                    ResponsesErrorKind::ProviderFailure
                                }
                            };
                            return Self::failure(kind, Some(status), None, true);
                        }
                    };
                    if observe_event(Some(&event.event), &event.data, &mut terminal).is_err() {
                        return Self::failure(
                            ResponsesErrorKind::ProviderFailure,
                            Some(status),
                            None,
                            true,
                        );
                    }
                }
                if delivery_failed.load(std::sync::atomic::Ordering::Acquire) {
                    return Self::failure(ResponsesErrorKind::Cancelled, Some(status), None, true);
                }
                match terminal {
                    Some(ResponsesTerminal::Completed) => ResponsesCompletion {
                        result: Ok(ResponsesOutput::Stream(meta, ResponsesTerminal::Completed)),
                    },
                    Some(terminal) => ResponsesCompletion {
                        result: Ok(ResponsesOutput::Stream(meta, terminal)),
                    },
                    None => Self::failure(
                        ResponsesErrorKind::ProviderFailure,
                        Some(status),
                        None,
                        true,
                    ),
                }
            } else {
                let body = match bounded_body(response).await {
                    Ok(body) => body,
                    Err(_) => {
                        return Self::failure(ResponsesErrorKind::Transport, None, None, true);
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
                ResponsesCompletion {
                    result: Ok(ResponsesOutput::Json(
                        ResponseBody {
                            meta,
                            body: Payload::new(body),
                        },
                        terminal,
                    )),
                }
            }
        })
    }
}
