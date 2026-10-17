//! Native OpenAI Responses v1 provider adapter.
//!
//! Standard profile preserves native requests and response bytes. The explicit
//! Codex subscription profile adapts the backend's stateless streaming contract.
use crate::config::{
    CredentialMaterial, HttpAuthorization, ProtocolConfig, ProviderConfig, RequestProfile,
    ResolvedCredential, Transport,
};
use reqwest::{Client, ClientBuilder, Response, header::HeaderValue};
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

/// A Responses transport that never redirects credentials or repeats a request
/// behind the gateway's explicit recovery policy.
#[derive(Clone)]
pub struct ResponsesClient(Client);
impl ResponsesClient {
    pub fn new(builder: ClientBuilder) -> Result<Self, reqwest::Error> {
        builder
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map(Self)
    }
}

pub struct OpenAiResponses {
    config: ProviderConfig,
    client: ResponsesClient,
    credential: Arc<ResolvedCredential>,
}

impl OpenAiResponses {
    pub fn with_resolved_credential(
        config: ProviderConfig,
        client: ResponsesClient,
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
        let CredentialMaterial::Http {
            authorization: HttpAuthorization::Bearer(token),
            account_id,
        } = &credential.material
        else {
            return Err(InvalidContract(
                "Responses requires HTTP bearer authentication",
            ));
        };
        if token.is_empty() {
            return Err(InvalidContract("provider credential is empty"));
        }
        if config.request_profile() == RequestProfile::CodexResponses
            && account_id
                .as_ref()
                .is_none_or(|value| value.is_empty() || HeaderValue::from_str(value).is_err())
        {
            return Err(InvalidContract(
                "Codex request profile requires an account identity",
            ));
        }
        Ok(Self {
            config,
            client,
            credential: Arc::new(credential),
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

fn terminal_from_status(status: &str) -> Option<ResponsesTerminal> {
    match status {
        "completed" => Some(ResponsesTerminal::Completed),
        "incomplete" => Some(ResponsesTerminal::Incomplete),
        "failed" => Some(ResponsesTerminal::Failed),
        "cancelled" | "canceled" => Some(ResponsesTerminal::Cancelled),
        _ => None,
    }
}

// Codex keeps no upstream response state; retain the caller's complete input and
// instructions, adapting only backend fields without inventing calls or results.
fn codex_request(body: &mut Value) -> Result<(), InvalidContract> {
    let object = body
        .as_object_mut()
        .ok_or(InvalidContract("Responses body"))?;
    if ["previous_response_id", "conversation"]
        .iter()
        .any(|key| object.get(*key).is_some_and(|v| !v.is_null()))
    {
        return Err(InvalidContract("Codex requires full history"));
    }
    for field in [
        "max_output_tokens",
        "max_completion_tokens",
        "temperature",
        "top_p",
        "user",
        "safety_identifier",
        "prompt_cache_options",
        "prompt_cache_retention",
        "options",
        "previous_response_id",
        "conversation",
    ] {
        if object.remove(field).is_some() {
            tracing::info!(event = "codex_request_field_omitted", field);
        }
    }
    if object
        .get("service_tier")
        .is_some_and(|v| v != "priority" && v != "ultrafast")
    {
        object.remove("service_tier");
        tracing::info!(
            event = "codex_request_field_omitted",
            field = "service_tier"
        );
    }
    object
        .entry("instructions")
        .or_insert(Value::String(String::new()));
    object.insert("store".into(), Value::Bool(false));
    object.insert("stream".into(), Value::Bool(true));
    if let Some(Value::String(text)) = object.get("input") {
        object.insert(
            "input".into(),
            serde_json::json!([{"role":"user","content":[{"type":"input_text","text":text}]}]),
        );
    }
    if let Some(Value::Array(input)) = object.get_mut("input") {
        for item in input {
            if let Some(item) = item.as_object_mut() {
                if item.get("type").and_then(Value::as_str) == Some("item_reference") {
                    return Err(InvalidContract("Codex requires inline history"));
                }
                if item.remove("id").is_some() {
                    tracing::info!(event = "codex_request_field_omitted", field = "input.id");
                }
                if item.get("role").and_then(Value::as_str) == Some("system") {
                    item.insert("role".into(), Value::String("developer".into()));
                }
            }
        }
    }
    Ok(())
}

fn unsupported_output_cap(body: &Value, credential: &ResolvedCredential) -> bool {
    credential.request_capabilities.explicit_output_cap == Some(false)
        && body.get("max_output_tokens").is_some()
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
        let mut body = match Self::request_json(request.body.get(), request.model.as_str()) {
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
        let client = self.client.0.clone();
        let credential = Arc::clone(&self.credential);
        let profile = self.config.request_profile();
        let headers = request.headers;
        let stream = request.stream;
        Box::pin(async move {
            let codex = profile == RequestProfile::CodexResponses;
            if codex {
                if !stream || codex_request(&mut body).is_err() {
                    return Self::failure(ResponsesErrorKind::Unsupported, Some(422), None, false);
                }
            } else if unsupported_output_cap(&body, &credential) {
                return Self::failure(ResponsesErrorKind::Unsupported, Some(422), None, false);
            }
            let mut auth = match HeaderValue::from_str(&format!(
                "Bearer {}",
                credential.token().expect("validated HTTP credential")
            )) {
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
                ) || (codex
                    && matches!(
                        name.as_str(),
                        "accept"
                            | "cookie"
                            | "x-api-key"
                            | "chatgpt-account-id"
                            | "originator"
                            | "user-agent"
                            | "openai-beta"
                            | "version"
                    ))
                {
                    continue;
                }
                let Ok(value) = HeaderValue::from_str(&header.value) else {
                    continue;
                };
                builder = builder.header(&header.name, value);
            }
            if codex {
                let CredentialMaterial::Http {
                    account_id: Some(account_id),
                    ..
                } = &credential.material
                else {
                    unreachable!("validated Codex credential")
                };
                let mut account =
                    HeaderValue::from_str(account_id).expect("validated account header");
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
            let mut meta = response_meta(&response);
            let status = meta.status;
            let content_type_present = response.headers().contains_key("content-type");
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
                let media_type = content_type
                    .as_deref()
                    .and_then(|value| value.split(';').next())
                    .map(str::trim);
                let is_sse =
                    media_type.is_some_and(|value| value.eq_ignore_ascii_case("text/event-stream"));
                let codex_without_type = codex && !content_type_present;
                if !is_sse && !codex_without_type {
                    let encoding = encoding_class(
                        response
                            .headers()
                            .get("content-encoding")
                            .and_then(|value| value.to_str().ok()),
                    );
                    tracing::warn!(
                        event = "responses_stream_content_type_rejected",
                        upstream_status = status,
                        mime = media_type_class(media_type),
                        encoding
                    );
                    return Self::failure(
                        ResponsesErrorKind::Unsupported,
                        Some(status),
                        None,
                        true,
                    );
                }
                if codex_without_type {
                    // Codex can omit MIME on its native SSE endpoint. This is
                    // limited to an absent header, not a contradictory one.
                    meta.headers.push(Header {
                        name: "content-type".into(),
                        value: "text/event-stream".into(),
                    });
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
