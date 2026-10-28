//! One native protocol request over HTTP to an already resolved target.
//!
//! The adapter accepts an assembled client and a resolved credential. It does
//! not read provider configuration, resolve credentials, retry, or choose a
//! model. Request fields, headers and response bytes stay native; only the
//! Codex profile adapts Responses to its backend's stateless contract.
use crate::HttpClient;
use crate::config::{
    CredentialMaterial, HttpAuthorization, HttpTarget, RequestProfile, ResolvedCredential,
};
use crate::responses;
use crate::service_error::{self, ServiceError};
use futures_util::StreamExt;
use reqwest::{Response, header::HeaderValue};
use serde_json::Value;
use velune_ai::{
    InvalidContract, NativeCompletion, NativeErrorKind, NativeEvent, NativeFailure, NativeOutput,
    NativeProvider, NativeRequest, NativeSink, NativeSuccess, OperationFuture, Payload, Protocol,
    http::{Header, ResponseBody, ResponseMeta},
};

pub struct NativeHttp {
    target: HttpTarget,
    client: HttpClient,
    credential: ResolvedCredential,
}

impl NativeHttp {
    pub fn new(
        target: HttpTarget,
        client: HttpClient,
        credential: ResolvedCredential,
    ) -> Result<Self, InvalidContract> {
        let CredentialMaterial::Http {
            authorization,
            account_id,
        } = &credential.material
        else {
            return Err(InvalidContract(
                "HTTP providers require HTTP authentication",
            ));
        };
        if authorization.secret().is_empty() {
            return Err(InvalidContract("provider credential is empty"));
        }
        // Only Messages has a native API key header.
        if matches!(authorization, HttpAuthorization::ApiKey(_))
            && target.protocol() != Protocol::Messages
        {
            return Err(InvalidContract(
                "OpenAI protocols require bearer authentication",
            ));
        }
        if target.request_profile() == RequestProfile::CodexResponses
            && account_id
                .as_ref()
                .is_none_or(|value| value.is_empty() || HeaderValue::from_str(value).is_err())
        {
            return Err(InvalidContract(
                "Codex request profile requires an account identity",
            ));
        }
        Ok(Self {
            target,
            client,
            credential,
        })
    }

    /// The body to send, or why it may not be sent.
    fn prepare(&self, request: &NativeRequest) -> Result<Value, NativeErrorKind> {
        let mut body = request.body.get().clone();
        if !body.is_object() || body["model"].as_str() != Some(request.model.as_str()) {
            return Err(NativeErrorKind::InvalidInput);
        }
        if self.target.request_profile() == RequestProfile::CodexResponses {
            if !request.stream || responses::codex_request(&mut body).is_err() {
                return Err(NativeErrorKind::Unsupported);
            }
            return Ok(body);
        }
        let output_caps: &[&str] = match self.target.protocol() {
            Protocol::ChatCompletions => &["max_tokens", "max_completion_tokens"],
            Protocol::Responses => &["max_output_tokens"],
            Protocol::Messages => &["max_tokens"],
        };
        if self.credential.request_capabilities.explicit_output_cap == Some(false)
            && output_caps.iter().any(|field| body.get(*field).is_some())
        {
            return Err(NativeErrorKind::Unsupported);
        }
        // The caller chooses the Messages version; it is never assumed.
        if self.target.protocol() == Protocol::Messages
            && !request.headers.iter().any(|header| {
                header.name.eq_ignore_ascii_case("anthropic-version")
                    && !header.value.trim().is_empty()
                    && HeaderValue::from_str(&header.value).is_ok()
            })
        {
            return Err(NativeErrorKind::InvalidInput);
        }
        Ok(body)
    }

    fn build(
        &self,
        body: &Value,
        headers: &[Header],
        stream: bool,
    ) -> Result<reqwest::Request, NativeErrorKind> {
        let protocol = self.target.protocol();
        let profile = self.target.request_profile();
        let codex = profile == RequestProfile::CodexResponses;
        let endpoint = self.target.endpoint().url(match protocol {
            Protocol::ChatCompletions => "/chat/completions",
            Protocol::Responses => "/responses",
            Protocol::Messages => "/v1/messages",
        });
        let CredentialMaterial::Http {
            authorization,
            account_id,
        } = &self.credential.material
        else {
            unreachable!("validated HTTP credential")
        };
        let (name, value) = match authorization {
            HttpAuthorization::ApiKey(value) => ("x-api-key", value.clone()),
            HttpAuthorization::Bearer(value) => ("authorization", format!("Bearer {value}")),
        };
        let mut auth = HeaderValue::from_str(&value).map_err(|_| NativeErrorKind::InvalidInput)?;
        auth.set_sensitive(true);
        let mut builder = self
            .client
            .inner
            .post(endpoint)
            .header(name, auth)
            .header(
                "accept",
                if stream {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            )
            .header("connection", "close");
        let own_agent = matches!(
            profile,
            RequestProfile::KimiCoding | RequestProfile::ArkCoding
        );
        for header in headers {
            let name = header.name.to_ascii_lowercase();
            let replaced = matches!(
                name.as_str(),
                "authorization" | "x-api-key" | "host" | "content-length" | "connection"
            ) || (own_agent && name == "user-agent")
                || (codex
                    && matches!(
                        name.as_str(),
                        "accept"
                            | "cookie"
                            | "chatgpt-account-id"
                            | "originator"
                            | "user-agent"
                            | "openai-beta"
                            | "version"
                    ));
            if replaced {
                continue;
            }
            if let Ok(value) = HeaderValue::from_str(&header.value) {
                builder = builder.header(&header.name, value);
            }
        }
        if own_agent {
            // A single genuine client identity, never an impersonated agent.
            builder = builder.header("user-agent", service_error::USER_AGENT);
        }
        if codex {
            let account = account_id.as_deref().expect("validated Codex account");
            let mut account = HeaderValue::from_str(account).expect("validated account header");
            account.set_sensitive(true);
            builder = builder
                .header("chatgpt-account-id", account)
                .header("openai-beta", "responses=experimental")
                .header("originator", "codex_cli_rs")
                .header("version", "0.162.0")
                .header("user-agent", "codex_cli_rs/0.162.0 Velune/0.1");
            if let Some(key) = body["prompt_cache_key"].as_str()
                && let Ok(value) = HeaderValue::from_str(key)
            {
                builder = builder
                    .header("session-id", value.clone())
                    .header("thread-id", value);
            }
        }
        builder
            .json(body)
            .build()
            .map_err(|_| NativeErrorKind::InvalidInput)
    }
}

impl NativeProvider for NativeHttp {
    fn send(
        &self,
        request: NativeRequest,
        events: NativeSink,
    ) -> OperationFuture<NativeCompletion> {
        let prepared = self
            .prepare(&request)
            .and_then(|body| self.build(&body, &request.headers, request.stream));
        let http = match prepared {
            Ok(http) => http,
            Err(kind) => return Box::pin(async move { Err(NativeFailure::unsent(kind)) }),
        };
        let client = self.client.inner.clone();
        let protocol = self.target.protocol();
        let profile = self.target.request_profile();
        let stream = request.stream;
        Box::pin(async move {
            let response = client.execute(http).await.map_err(|error| {
                NativeFailure::sent(
                    if error.is_timeout() {
                        NativeErrorKind::Timeout
                    } else {
                        NativeErrorKind::Transport
                    },
                    None,
                )
            })?;
            let meta = response_meta(&response);
            if !(200..300).contains(&meta.status) {
                let body = read_body(response).await.ok();
                return Err(NativeFailure {
                    kind: failure_kind(profile, meta.status, body.as_deref()),
                    response: Some(meta),
                    body: body.map(Payload::new),
                    submitted: true,
                });
            }
            if stream {
                relay(protocol, profile, response, meta, events).await
            } else {
                let body = read_body(response)
                    .await
                    .map_err(|_| NativeFailure::sent(NativeErrorKind::Transport, None))?;
                let completed = match protocol {
                    Protocol::Responses => match responses::body_terminal(&body) {
                        Some(terminal) => terminal == responses::Terminal::Completed,
                        None => {
                            return Err(NativeFailure {
                                kind: NativeErrorKind::ProviderFailure,
                                response: Some(meta),
                                body: Some(Payload::new(body)),
                                submitted: true,
                            });
                        }
                    },
                    Protocol::ChatCompletions | Protocol::Messages => false,
                };
                Ok(NativeSuccess {
                    output: NativeOutput::Json(ResponseBody {
                        meta,
                        body: Payload::new(body),
                    }),
                    completed,
                })
            }
        })
    }
}

/// Forward a successful stream's head and chunks as they arrive. Responses
/// streams must also end in an explicit business terminal.
async fn relay(
    protocol: Protocol,
    profile: RequestProfile,
    response: Response,
    mut meta: ResponseMeta,
    mut events: NativeSink,
) -> NativeCompletion {
    if protocol == Protocol::Responses {
        responses::check_stream_type(profile, &response, &mut meta)?;
    }
    if events(NativeEvent::Headers(meta.clone())).await.is_err() {
        return Err(NativeFailure::sent(NativeErrorKind::Cancelled, Some(meta)));
    }
    let delivered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let delivery = delivered.clone();
    let chunks = futures_util::stream::unfold(
        (response.bytes_stream(), events, delivery),
        |(mut chunks, mut events, delivered)| async move {
            let chunk = chunks.next().await?;
            if let Ok(chunk) = &chunk
                && !chunk.is_empty()
                && events(NativeEvent::Body(Payload::new(chunk.to_vec())))
                    .await
                    .is_err()
            {
                delivered.store(false, std::sync::atomic::Ordering::Release);
                return None;
            }
            Some((chunk, (chunks, events, delivered)))
        },
    );
    let ended = if protocol == Protocol::Responses {
        responses::stream_terminal(chunks).await
    } else {
        drain(chunks).await
    };
    if !delivered.load(std::sync::atomic::Ordering::Acquire) {
        return Err(NativeFailure::sent(NativeErrorKind::Cancelled, Some(meta)));
    }
    match ended {
        Ok(completed) => Ok(NativeSuccess {
            output: NativeOutput::Stream(meta),
            completed,
        }),
        Err(kind) => Err(NativeFailure::sent(kind, Some(meta))),
    }
}

async fn drain<T>(
    chunks: impl futures_util::Stream<Item = reqwest::Result<T>>,
) -> Result<bool, NativeErrorKind> {
    futures_util::pin_mut!(chunks);
    while let Some(chunk) = chunks.next().await {
        chunk.map_err(|error| transport_kind(&error))?;
    }
    Ok(false)
}

pub(crate) fn transport_kind(error: &reqwest::Error) -> NativeErrorKind {
    if error.is_timeout() {
        NativeErrorKind::Timeout
    } else {
        NativeErrorKind::Transport
    }
}

fn failure_kind(profile: RequestProfile, status: u16, body: Option<&[u8]>) -> NativeErrorKind {
    match service_error::classify(profile, status, body) {
        Some(ServiceError::Authentication) => NativeErrorKind::Authentication,
        Some(ServiceError::Permission) => NativeErrorKind::Permission,
        Some(ServiceError::QuotaExhausted) => NativeErrorKind::QuotaExhausted,
        Some(ServiceError::RateLimited) => NativeErrorKind::RateLimited,
        Some(ServiceError::Overloaded) => NativeErrorKind::Overloaded,
        None => match status {
            401 => NativeErrorKind::Authentication,
            403 => NativeErrorKind::Permission,
            429 => NativeErrorKind::RateLimited,
            _ => NativeErrorKind::ProviderFailure,
        },
    }
}

async fn read_body(response: Response) -> Result<Vec<u8>, reqwest::Error> {
    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        body.extend_from_slice(&chunk?);
    }
    Ok(body)
}

fn response_meta(response: &Response) -> ResponseMeta {
    ResponseMeta {
        status: response.status().as_u16(),
        headers: response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                Some(Header {
                    name: name.as_str().to_owned(),
                    value: value.to_str().ok()?.to_owned(),
                })
            })
            .collect(),
    }
}
