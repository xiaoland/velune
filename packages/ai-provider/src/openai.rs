//! Generic OpenAI Chat Completions v1 provider adapter.
//!
//! This adapter accepts an already assembled client and credential. It does not
//! read provider configuration, resolve credentials, retry, or choose a model.
use crate::HttpClient;
use crate::config::{
    CredentialMaterial, HttpAuthorization, ProtocolConfig, ProviderConfig, ResolvedCredential,
    Transport,
};
use reqwest::{Response, header::HeaderValue};
use serde_json::Value;
use std::sync::Arc;
use velune_ai::{InvalidContract, OperationFuture, Payload};
use velune_ai::{
    chat_completions::*,
    http::{Header, ResponseBody, ResponseMeta},
};

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

pub struct ChatCompletions {
    config: ProviderConfig,
    client: HttpClient,
    explicit_output_cap: Option<bool>,
    credential: Arc<dyn Fn() -> Option<String> + Send + Sync>,
}

impl ChatCompletions {
    pub fn new(
        config: ProviderConfig,
        client: HttpClient,
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
            explicit_output_cap: None,
            credential: Arc::new(move || Some(value.clone())),
        })
    }
    pub fn with_resolved_credential(
        config: ProviderConfig,
        client: HttpClient,
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
        let CredentialMaterial::Http {
            authorization: HttpAuthorization::Bearer(token),
            ..
        } = credential.material
        else {
            return Err(InvalidContract(
                "Chat Completions requires HTTP bearer authentication",
            ));
        };
        if token.is_empty() {
            return Err(InvalidContract("provider credential is empty"));
        }
        Ok(Self {
            config,
            client,
            explicit_output_cap: credential.request_capabilities.explicit_output_cap,
            credential: Arc::new(move || Some(token.clone())),
        })
    }

    fn native_request_json(
        body: &Value,
        provider_model_id: &str,
    ) -> Result<Value, InvalidContract> {
        if !body.is_object() || body["model"].as_str() != Some(provider_model_id) {
            return Err(InvalidContract(
                "Chat Completions provider model does not match request body",
            ));
        }
        let body = body.clone();
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
        if self.explicit_output_cap == Some(false)
            && ["max_tokens", "max_completion_tokens"]
                .iter()
                .any(|field| request.body.get().get(*field).is_some())
        {
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
        }
        let body = match Self::native_request_json(request.body.get(), request.model.as_str()) {
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
        let request_profile = self.config.request_profile();
        let client = self.client.inner.clone();
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
                if request_profile == crate::config::RequestProfile::KimiCoding
                    && name == "user-agent"
                {
                    continue;
                }
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
            if request_profile == crate::config::RequestProfile::KimiCoding {
                // Set a single genuine client identity, never impersonate an allowed agent.
                builder = builder.header("user-agent", crate::service_error::USER_AGENT);
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
                        kind: match crate::service_error::classify(
                            request_profile,
                            status,
                            body.as_deref(),
                        ) {
                            Some(crate::service_error::ServiceError::Authentication) => {
                                ChatCompletionsErrorKind::Authentication
                            }
                            Some(crate::service_error::ServiceError::Permission) => {
                                ChatCompletionsErrorKind::Permission
                            }
                            Some(crate::service_error::ServiceError::QuotaExhausted) => {
                                ChatCompletionsErrorKind::QuotaExhausted
                            }
                            Some(crate::service_error::ServiceError::RateLimited) => {
                                ChatCompletionsErrorKind::RateLimited
                            }
                            Some(crate::service_error::ServiceError::Overloaded) => {
                                ChatCompletionsErrorKind::Overloaded
                            }
                            None => match status {
                                401 => ChatCompletionsErrorKind::Authentication,
                                403 => ChatCompletionsErrorKind::Permission,
                                429 => ChatCompletionsErrorKind::RateLimited,
                                _ => ChatCompletionsErrorKind::ProviderFailure,
                            },
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
