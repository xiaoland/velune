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
use crate::service_error::{self, ServiceError};
use crate::{gemini, metrics::ModelTerminal, responses};
use eventsource_stream::{EventStreamError, Eventsource};
use futures_util::StreamExt;
use reqwest::{Response, header::HeaderValue};
use serde_json::Value;
use velune_ai::{
    InvalidContract, NativeCompletion, NativeErrorKind, NativeEvent, NativeFailure, NativeOutput,
    NativeProvider, NativeRequest, NativeSink, NativeSuccess, OperationFuture, Payload, Protocol,
    ProviderModelId,
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
        match (target.protocol(), authorization) {
            (Protocol::ChatCompletions | Protocol::Responses, HttpAuthorization::ApiKey(_)) => {
                return Err(InvalidContract(
                    "OpenAI protocols require bearer authentication",
                ));
            }
            (Protocol::Gemini, HttpAuthorization::Bearer(_)) => {
                return Err(InvalidContract("Gemini Developer API requires an API key"));
            }
            _ => {}
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

    /// Adapt the owned body and build the outgoing request before dispatch.
    fn prepare(&self, request: NativeRequest) -> Result<reqwest::Request, NativeErrorKind> {
        let mut body = request.body.into_inner();
        let protocol = self.target.protocol();
        let body_matches_protocol = if protocol == Protocol::Gemini {
            body.get("model").is_none() && body.get("stream").is_none()
        } else {
            body["model"].as_str() == Some(request.model.as_str())
        };
        if !body.is_object() || !body_matches_protocol {
            return Err(NativeErrorKind::InvalidInput);
        }
        if self.target.request_profile() == RequestProfile::CodexResponses {
            if !request.stream || responses::codex_request(&mut body).is_err() {
                return Err(NativeErrorKind::Unsupported);
            }
            return self.build(&request.model, &body, &request.headers, request.stream);
        }
        if self.credential.request_capabilities.explicit_output_cap == Some(false)
            && explicit_output_cap(protocol, &body)
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
        self.build(&request.model, &body, &request.headers, request.stream)
    }

    fn build(
        &self,
        model: &ProviderModelId,
        body: &Value,
        headers: &[Header],
        stream: bool,
    ) -> Result<reqwest::Request, NativeErrorKind> {
        let protocol = self.target.protocol();
        let profile = self.target.request_profile();
        let codex_client_version = self.target.codex_client_version();
        let base = self.target.endpoint();
        let endpoint = match protocol {
            Protocol::ChatCompletions => base.url("/chat/completions"),
            Protocol::Responses => base.url("/responses"),
            Protocol::Messages => base.url("/v1/messages"),
            Protocol::Gemini => gemini::url(base, model, stream)?,
        };
        let CredentialMaterial::Http {
            authorization,
            account_id,
        } = &self.credential.material
        else {
            unreachable!("validated HTTP credential")
        };
        let (name, value) = match authorization {
            HttpAuthorization::ApiKey(value) => (
                if protocol == Protocol::Gemini {
                    "x-goog-api-key"
                } else {
                    "x-api-key"
                },
                value.clone(),
            ),
            HttpAuthorization::Bearer(value) => ("authorization", format!("Bearer {value}")),
        };
        let mut auth = HeaderValue::from_str(&value).map_err(|_| NativeErrorKind::InvalidInput)?;
        auth.set_sensitive(true);
        let mut builder = self.client.inner.post(endpoint).header(name, auth).header(
            "accept",
            if stream {
                "text/event-stream"
            } else {
                "application/json"
            },
        );
        let own_agent = matches!(
            profile,
            RequestProfile::KimiCoding | RequestProfile::ArkCoding
        );
        for header in headers {
            let name = header.name.to_ascii_lowercase();
            let replaced = matches!(
                name.as_str(),
                "authorization"
                    | "x-api-key"
                    | "x-goog-api-key"
                    | "host"
                    | "content-length"
                    | "connection"
            ) || (own_agent && name == "user-agent")
                || (codex_client_version.is_some()
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
        if let Some(version) = codex_client_version {
            let account = account_id.as_deref().expect("validated Codex account");
            let mut account = HeaderValue::from_str(account).expect("validated account header");
            account.set_sensitive(true);
            builder = builder
                .header("chatgpt-account-id", account)
                .header("openai-beta", "responses=experimental")
                .header("originator", "codex_cli_rs")
                .header("version", version)
                .header("user-agent", responses::codex_user_agent(version));
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
        let stream = request.stream;
        let http = match self.prepare(request) {
            Ok(http) => http,
            Err(kind) => return Box::pin(async move { Err(NativeFailure::unsent(kind)) }),
        };
        let client = self.client.inner.clone();
        let protocol = self.target.protocol();
        let profile = self.target.request_profile();
        Box::pin(async move {
            let response = client
                .execute(http)
                .await
                .map_err(|error| NativeFailure::sent(transport_kind(&error), None))?;
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
                let body = match read_body(response).await {
                    Ok(body) => body,
                    Err(error) => {
                        return Err(NativeFailure::sent(transport_kind(&error), Some(meta)));
                    }
                };
                let completed = match protocol {
                    Protocol::Responses => responses::body_terminal(&body)
                        .map(|terminal| terminal == responses::Terminal::Completed),
                    Protocol::Gemini => {
                        let mut observed = gemini::ResponseObservation::default();
                        if let Ok(value) = serde_json::from_slice(&body) {
                            observed.observe(&value);
                        }
                        observed
                            .terminal()
                            .map(|terminal| terminal == ModelTerminal::Completed)
                    }
                    Protocol::ChatCompletions | Protocol::Messages => Some(false),
                };
                let Some(completed) = completed else {
                    return Err(NativeFailure {
                        kind: NativeErrorKind::ProviderFailure,
                        response: Some(meta),
                        body: Some(Payload::new(body)),
                        submitted: true,
                    });
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

fn explicit_output_cap(protocol: Protocol, body: &Value) -> bool {
    match protocol {
        Protocol::ChatCompletions => {
            body.get("max_tokens").is_some() || body.get("max_completion_tokens").is_some()
        }
        Protocol::Responses => body.get("max_output_tokens").is_some(),
        Protocol::Messages => body.get("max_tokens").is_some(),
        Protocol::Gemini => ["generationConfig", "generation_config"]
            .iter()
            .any(|config| {
                body[*config].get("maxOutputTokens").is_some()
                    || body[*config].get("max_output_tokens").is_some()
            }),
    }
}

/// Forward a successful stream's head and chunks as they arrive. Responses
/// and Gemini streams must also contain an explicit business terminal.
async fn relay(
    protocol: Protocol,
    profile: RequestProfile,
    response: Response,
    mut meta: ResponseMeta,
    mut events: NativeSink,
) -> NativeCompletion {
    if matches!(protocol, Protocol::Responses | Protocol::Gemini) {
        check_stream_type(protocol, profile, &response, &mut meta)?;
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
    let ended = match protocol {
        Protocol::Responses => responses::stream_terminal(chunks).await,
        Protocol::Gemini => {
            let mut observed = gemini::ResponseObservation::default();
            observe_sse(chunks, |_, value| observed.observe(value))
                .await
                .and_then(|()| {
                    observed
                        .terminal()
                        .map(|terminal| terminal == ModelTerminal::Completed)
                        .ok_or(NativeErrorKind::ProviderFailure)
                })
        }
        Protocol::ChatCompletions | Protocol::Messages => drain(chunks).await,
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

/// A successful stream must declare SSE. Codex alone may omit the media type,
/// which it does on its native SSE endpoint; an explicit wrong type never passes.
fn check_stream_type(
    protocol: Protocol,
    profile: RequestProfile,
    response: &reqwest::Response,
    meta: &mut ResponseMeta,
) -> Result<(), NativeFailure> {
    let header = |name| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    let content_type = header("content-type");
    let media_type = content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if media_type.is_some_and(|value| value.eq_ignore_ascii_case("text/event-stream")) {
        return Ok(());
    }
    if profile == RequestProfile::CodexResponses && !response.headers().contains_key("content-type")
    {
        meta.headers.push(Header {
            name: "content-type".into(),
            value: "text/event-stream".into(),
        });
        return Ok(());
    }
    tracing::warn!(
        event = if protocol == Protocol::Gemini {
            "gemini_stream_content_type_rejected"
        } else {
            "responses_stream_content_type_rejected"
        },
        upstream_status = meta.status,
        mime = media_type_class(media_type),
        encoding = encoding_class(header("content-encoding")),
    );
    Err(NativeFailure::sent(
        NativeErrorKind::Unsupported,
        Some(meta.clone()),
    ))
}

// Diagnostic labels must remain finite: upstream header values can contain secrets.
fn media_type_class(value: Option<&str>) -> &'static str {
    match value {
        None => "absent",
        Some(value) if value.eq_ignore_ascii_case("text/event-stream") => "sse",
        Some(value) if value.eq_ignore_ascii_case("application/json") => "json",
        Some(value) if value.eq_ignore_ascii_case("text/html") => "html",
        Some(value) if value.eq_ignore_ascii_case("text/plain") => "plain",
        Some(_) => "other",
    }
}
fn encoding_class(value: Option<&str>) -> &'static str {
    match value.map(str::trim) {
        None => "absent",
        Some(value) if value.eq_ignore_ascii_case("identity") => "identity",
        Some(value) if value.eq_ignore_ascii_case("gzip") => "gzip",
        Some(value) if value.eq_ignore_ascii_case("br") => "br",
        Some(value) if value.eq_ignore_ascii_case("deflate") => "deflate",
        Some(value) if value.eq_ignore_ascii_case("zstd") => "zstd",
        Some(_) => "other",
    }
}

/// Read all JSON SSE events. Native forwarding stays upstream of this observer.
pub(crate) async fn observe_sse<T: AsRef<[u8]>>(
    chunks: impl futures_util::Stream<Item = reqwest::Result<T>>,
    mut observe: impl FnMut(&str, &Value),
) -> Result<(), NativeErrorKind> {
    let events = chunks.eventsource();
    futures_util::pin_mut!(events);
    while let Some(event) = events.next().await {
        let event = event.map_err(|error| match error {
            EventStreamError::Transport(error) => transport_kind(&error),
            EventStreamError::Utf8(_) | EventStreamError::Parser(_) => {
                NativeErrorKind::ProviderFailure
            }
        })?;
        if event.data.is_empty() || event.data == "[DONE]" {
            continue;
        }
        let value =
            serde_json::from_str(&event.data).map_err(|_| NativeErrorKind::ProviderFailure)?;
        observe(&event.event, &value);
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CredentialRecovery, HttpEndpoint, RequestCapabilities};
    use serde_json::json;

    #[test]
    fn codex_subscription_uses_provider_client_version_and_preserves_model() {
        let provider = NativeHttp::new(
            HttpTarget::new(
                Protocol::Responses,
                HttpEndpoint::parse("https://chatgpt.com/backend-api/codex").unwrap(),
                RequestProfile::CodexResponses,
                Some("0.166.0"),
            )
            .unwrap(),
            HttpClient::new(reqwest::Client::builder()).unwrap(),
            ResolvedCredential {
                material: CredentialMaterial::Http {
                    authorization: HttpAuthorization::Bearer("test-token".into()),
                    account_id: Some("test-account".into()),
                },
                recovery: CredentialRecovery::None,
                request_capabilities: RequestCapabilities::default(),
            },
        )
        .unwrap();
        let model = "gpt-6.2-sol";
        let request = provider
            .prepare(NativeRequest {
                model: ProviderModelId::new(model).unwrap(),
                body: Payload::new(json!({"model": model, "input": "test prompt"})),
                headers: vec![
                    Header {
                        name: "Version".into(),
                        value: "0.162.0".into(),
                    },
                    Header {
                        name: "User-Agent".into(),
                        value: "codex_cli_rs/0.162.0".into(),
                    },
                ],
                stream: true,
            })
            .unwrap();

        let headers = request.headers();
        assert_eq!(headers.get_all("version").iter().count(), 1);
        assert_eq!(headers["version"], "0.166.0");
        assert_eq!(headers.get_all("user-agent").iter().count(), 1);
        assert_eq!(headers["user-agent"], "codex_cli_rs/0.166.0 Velune/0.1");
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body["model"], model);
    }
}
