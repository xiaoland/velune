//! Generic Anthropic Messages v1 provider adapter.
//!
//! This adapter accepts an already assembled client and credential. It does not
//! read provider configuration, resolve credentials, retry, or choose a model.
//! Request version/beta headers and all native JSON/SSE fields remain caller-owned;
//! the resolved API key replaces caller authentication using x-api-key.
use crate::config::{ProtocolConfig, ProviderConfig, ResolvedCredential, Transport};
use reqwest::{Client, Response, header::HeaderValue};
use serde_json::Value;
use std::sync::Arc;
use velune_ai::{InvalidContract, OperationFuture, Payload};
use velune_ai::{
    http::{Header, ResponseBody, ResponseMeta},
    messages::*,
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

pub struct Messages {
    config: ProviderConfig,
    client: Client,
    credential: Arc<dyn Fn() -> Option<String> + Send + Sync>,
}

impl Messages {
    pub fn new(
        config: ProviderConfig,
        client: Client,
        credential: Payload<String>,
    ) -> Result<Self, InvalidContract> {
        let ProtocolConfig::Messages(protocol) = config.protocol() else {
            return Err(InvalidContract("provider requires Messages v1"));
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
        let ProtocolConfig::Messages(protocol) = config.protocol() else {
            return Err(InvalidContract("provider requires Messages v1"));
        };
        if !matches!(
            protocol.endpoint.transport(),
            Transport::Http | Transport::Https
        ) {
            return Err(InvalidContract("unsupported provider transport"));
        }
        if credential.subscription {
            return Err(InvalidContract(
                "subscription credential does not support Messages",
            ));
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

    fn native_request_json(
        body: &Value,
        provider_model_id: &str,
    ) -> Result<Value, InvalidContract> {
        if !body.is_object() || body["model"].as_str() != Some(provider_model_id) {
            return Err(InvalidContract(
                "Messages provider model does not match request body",
            ));
        }
        let body = body.clone();
        if serde_json::to_vec(&body)
            .map_err(|_| InvalidContract("Messages body encoding"))?
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

impl MessagesProvider for Messages {
    fn messages(
        &self,
        request: MessagesRequest,
        mut events: MessagesSink,
    ) -> OperationFuture<MessagesCompletion> {
        let has_version = request.headers.iter().any(|header| {
            header.name.eq_ignore_ascii_case("anthropic-version")
                && !header.value.trim().is_empty()
                && HeaderValue::from_str(&header.value).is_ok()
        });
        let body = match Self::native_request_json(request.body.get(), request.model.as_str())
            .and_then(|body| {
                if has_version {
                    Ok(body)
                } else {
                    Err(InvalidContract("Messages requires anthropic-version"))
                }
            }) {
            Ok(body) => body,
            Err(_) => {
                return Box::pin(async {
                    MessagesCompletion {
                        result: Err(MessagesFailure {
                            kind: MessagesErrorKind::InvalidInput,
                            response: None,
                            body: None,
                            submitted: false,
                        }),
                    }
                });
            }
        };
        let Some(protocol) = (match self.config.protocol() {
            ProtocolConfig::Messages(value) => Some(value),
            _ => None,
        }) else {
            return Box::pin(async {
                MessagesCompletion {
                    result: Err(MessagesFailure {
                        kind: MessagesErrorKind::Unsupported,
                        response: None,
                        body: None,
                        submitted: false,
                    }),
                }
            });
        };
        let endpoint = protocol.endpoint.url("/v1/messages");
        let client = self.client.clone();
        let credential = Arc::clone(&self.credential);
        let headers = request.headers;
        let stream_requested = request.stream;
        Box::pin(async move {
            let Some(credential) = credential() else {
                return MessagesCompletion {
                    result: Err(MessagesFailure {
                        kind: MessagesErrorKind::Authentication,
                        response: None,
                        body: None,
                        submitted: false,
                    }),
                };
            };
            let mut auth = match HeaderValue::from_str(&credential) {
                Ok(value) => value,
                Err(_) => {
                    return MessagesCompletion {
                        result: Err(MessagesFailure {
                            kind: MessagesErrorKind::InvalidInput,
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
                .header("x-api-key", auth)
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
                    "authorization" | "x-api-key" | "host" | "content-length" | "connection"
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
                    return MessagesCompletion {
                        result: Err(MessagesFailure {
                            kind: MessagesErrorKind::InvalidInput,
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
                    return MessagesCompletion {
                        result: Err(MessagesFailure {
                            kind: if error.is_timeout() {
                                MessagesErrorKind::Timeout
                            } else {
                                MessagesErrorKind::Transport
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
                return MessagesCompletion {
                    result: Err(MessagesFailure {
                        kind: match status {
                            401 => MessagesErrorKind::Authentication,
                            403 => MessagesErrorKind::Permission,
                            429 => MessagesErrorKind::RateLimited,
                            _ => MessagesErrorKind::ProviderFailure,
                        },
                        response: Some(meta),
                        body: body.map(Payload::new),
                        submitted: true,
                    }),
                };
            }
            if stream_requested {
                if (events)(MessagesEvent::Headers(meta.clone()))
                    .await
                    .is_err()
                {
                    return MessagesCompletion {
                        result: Err(MessagesFailure {
                            kind: MessagesErrorKind::Cancelled,
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
                            if (events)(MessagesEvent::Body(Payload::new(chunk.to_vec())))
                                .await
                                .is_err()
                            {
                                return MessagesCompletion {
                                    result: Err(MessagesFailure {
                                        kind: MessagesErrorKind::Cancelled,
                                        response: Some(meta),
                                        body: None,
                                        submitted: true,
                                    }),
                                };
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            return MessagesCompletion {
                                result: Err(MessagesFailure {
                                    kind: if error.is_timeout() {
                                        MessagesErrorKind::Timeout
                                    } else {
                                        MessagesErrorKind::Transport
                                    },
                                    response: Some(meta),
                                    body: None,
                                    submitted: true,
                                }),
                            };
                        }
                    }
                }
                return MessagesCompletion {
                    result: Ok(MessagesOutput::Stream(meta)),
                };
            }
            match bounded_body(response).await {
                Ok(body) => MessagesCompletion {
                    result: Ok(MessagesOutput::Json(ResponseBody {
                        meta,
                        body: Payload::new(body),
                    })),
                },
                Err(_) => MessagesCompletion {
                    result: Err(MessagesFailure {
                        kind: MessagesErrorKind::ProviderFailure,
                        response: None,
                        body: None,
                        submitted: true,
                    }),
                },
            }
        })
    }
}
