//! Generic OpenAI Chat Completions v1 provider adapter.
//!
//! This adapter accepts an already assembled client and credential. It does not
//! read provider configuration, resolve credentials, retry, or choose a model.
use crate::config::{ProtocolConfig, ProviderConfig, ResolvedCredential, Transport};
use reqwest::{Client, Response, header::HeaderValue};
use serde_json::Value;
use std::sync::Arc;
use velune_ai::{InvalidContract, OperationFuture, Payload};
use velune_ai::{
    chat_completions::*,
    http::{Header, ResponseBody, ResponseMeta},
};

const MAX_RESPONSE_BODY_BYTES: usize = 16 * 1024 * 1024;

async fn bounded_body(response: Response) -> Result<Vec<u8>, ()> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ())?;
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BODY_BYTES {
            return Err(());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub struct ChatCompletions {
    config: ProviderConfig,
    client: Client,
    credential: Arc<dyn Fn() -> Option<String> + Send + Sync>,
}

impl ChatCompletions {
    pub fn new(
        config: ProviderConfig,
        client: Client,
        credential: Payload<String>,
    ) -> Result<Self, InvalidContract> {
        let ProtocolConfig::ChatCompletions(protocol) = config.protocol() else {
            return Err(InvalidContract("provider requires Chat Completions v1"));
        };
        if protocol.endpoint.transport() != Transport::Http
            && protocol.endpoint.transport() != Transport::Https
        {
            return Err(InvalidContract("unsupported provider transport"));
        }
        if credential.get().is_empty() {
            return Err(InvalidContract("provider credential is empty"));
        }
        let value = credential.into_inner();
        Ok(Self {
            config,
            client,
            credential: Arc::new(move || Some(value.clone())),
        })
    }
    pub fn with_resolved_credential(
        config: ProviderConfig,
        client: Client,
        credential: ResolvedCredential,
    ) -> Result<Self, InvalidContract> {
        let ProtocolConfig::ChatCompletions(protocol) = config.protocol() else {
            return Err(InvalidContract("provider requires Chat Completions v1"));
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
        let token = credential.token;
        Ok(Self {
            config,
            client,
            credential: Arc::new(move || Some(token.clone())),
        })
    }

    fn native_request_json(body: &Value, external_model: &str) -> Result<Value, InvalidContract> {
        let mut object = body
            .as_object()
            .cloned()
            .ok_or(InvalidContract("Chat Completions body must be an object"))?;
        object.insert("model".into(), Value::String(external_model.to_owned()));
        let body = Value::Object(object);
        if serde_json::to_vec(&body)
            .map_err(|_| InvalidContract("Chat Completions body encoding"))?
            .len()
            > 256 * 1024
        {
            return Err(InvalidContract("provider request is too large"));
        }
        Ok(body)
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

impl ChatCompletionsProvider for ChatCompletions {
    fn chat_completions(
        &self,
        request: ChatCompletionsRequest,
        mut events: ChatCompletionsSink,
    ) -> OperationFuture<ChatCompletionsCompletion> {
        let Some(mapping) = self.config.model(&request.model) else {
            return Box::pin(async {
                ChatCompletionsCompletion {
                    result: Err(ChatCompletionsFailure {
                        kind: ChatCompletionsErrorKind::InvalidInput,
                        response: None,
                        body: None,
                        submitted: false,
                    }),
                }
            });
        };
        let body = match Self::native_request_json(request.body.get(), mapping.external_name()) {
            Ok(body) => body,
            Err(_) => {
                return Box::pin(async {
                    ChatCompletionsCompletion {
                        result: Err(ChatCompletionsFailure {
                            kind: ChatCompletionsErrorKind::InvalidInput,
                            response: None,
                            body: None,
                            submitted: false,
                        }),
                    }
                });
            }
        };
        let Some(protocol) = (match self.config.protocol() {
            ProtocolConfig::ChatCompletions(value) => Some(value),
            _ => None,
        }) else {
            return Box::pin(async {
                ChatCompletionsCompletion {
                    result: Err(ChatCompletionsFailure {
                        kind: ChatCompletionsErrorKind::Unsupported,
                        response: None,
                        body: None,
                        submitted: false,
                    }),
                }
            });
        };
        let endpoint = protocol.endpoint.url("/chat/completions");
        let client = self.client.clone();
        let credential = Arc::clone(&self.credential);
        let headers = request.headers;
        let stream_requested = request.stream;
        Box::pin(async move {
            let Some(credential) = credential() else {
                return ChatCompletionsCompletion {
                    result: Err(ChatCompletionsFailure {
                        kind: ChatCompletionsErrorKind::Authentication,
                        response: None,
                        body: None,
                        submitted: false,
                    }),
                };
            };
            let mut auth = match HeaderValue::from_str(&format!("Bearer {credential}")) {
                Ok(value) => value,
                Err(_) => {
                    return ChatCompletionsCompletion {
                        result: Err(ChatCompletionsFailure {
                            kind: ChatCompletionsErrorKind::InvalidInput,
                            response: None,
                            body: None,
                            submitted: false,
                        }),
                    };
                }
            };
            auth.set_sensitive(true);
            let mut builder = client
                .post(endpoint)
                .header("authorization", auth)
                .header(
                    "accept",
                    if stream_requested {
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
                    return ChatCompletionsCompletion {
                        result: Err(ChatCompletionsFailure {
                            kind: ChatCompletionsErrorKind::InvalidInput,
                            response: None,
                            body: None,
                            submitted: false,
                        }),
                    };
                }
            };
            let response = match client.execute(request).await {
                Ok(response) => response,
                Err(error) => {
                    return ChatCompletionsCompletion {
                        result: Err(ChatCompletionsFailure {
                            kind: if error.is_timeout() {
                                ChatCompletionsErrorKind::Timeout
                            } else {
                                ChatCompletionsErrorKind::Transport
                            },
                            response: None,
                            body: None,
                            submitted: true,
                        }),
                    };
                }
            };
            let meta = response_meta(&response);
            let status = meta.status;
            if !(200..300).contains(&status) {
                let body = bounded_body(response).await.ok();
                return ChatCompletionsCompletion {
                    result: Err(ChatCompletionsFailure {
                        kind: match status {
                            401 => ChatCompletionsErrorKind::Authentication,
                            403 => ChatCompletionsErrorKind::Permission,
                            429 => ChatCompletionsErrorKind::RateLimited,
                            _ => ChatCompletionsErrorKind::ProviderFailure,
                        },
                        response: Some(meta),
                        body: body.map(Payload::new),
                        submitted: true,
                    }),
                };
            }
            if stream_requested {
                if (events)(ChatCompletionsEvent::Headers(meta.clone()))
                    .await
                    .is_err()
                {
                    return ChatCompletionsCompletion {
                        result: Err(ChatCompletionsFailure {
                            kind: ChatCompletionsErrorKind::Cancelled,
                            response: Some(meta),
                            body: None,
                            submitted: true,
                        }),
                    };
                }
                let mut chunks = response.bytes_stream();
                use futures_util::StreamExt;
                while let Some(chunk) = chunks.next().await {
                    match chunk {
                        Ok(chunk) if !chunk.is_empty() => {
                            if (events)(ChatCompletionsEvent::Body(Payload::new(chunk.to_vec())))
                                .await
                                .is_err()
                            {
                                return ChatCompletionsCompletion {
                                    result: Err(ChatCompletionsFailure {
                                        kind: ChatCompletionsErrorKind::Cancelled,
                                        response: Some(meta),
                                        body: None,
                                        submitted: true,
                                    }),
                                };
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            return ChatCompletionsCompletion {
                                result: Err(ChatCompletionsFailure {
                                    kind: if error.is_timeout() {
                                        ChatCompletionsErrorKind::Timeout
                                    } else {
                                        ChatCompletionsErrorKind::Transport
                                    },
                                    response: Some(meta),
                                    body: None,
                                    submitted: true,
                                }),
                            };
                        }
                    }
                }
                return ChatCompletionsCompletion {
                    result: Ok(ChatCompletionsOutput::Stream(meta)),
                };
            }
            match bounded_body(response).await {
                Ok(body) => ChatCompletionsCompletion {
                    result: Ok(ChatCompletionsOutput::Json(ResponseBody {
                        meta,
                        body: Payload::new(body),
                    })),
                },
                Err(_) => ChatCompletionsCompletion {
                    result: Err(ChatCompletionsFailure {
                        kind: ChatCompletionsErrorKind::ProviderFailure,
                        response: None,
                        body: None,
                        submitted: true,
                    }),
                },
            }
        })
    }
}
