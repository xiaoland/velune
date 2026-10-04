//! Generic OpenAI Chat Completions v1 provider adapter.
//!
//! This adapter accepts an already assembled client and credential. It does not
//! read provider configuration, resolve credentials, retry, or choose a model.
use crate::{
    config::{ProtocolConfig, ProviderConfig, Transport},
    minimax::Decoder,
    minimax::mapping,
};
use eventsource_stream::{EventStreamError, Eventsource};
use futures_util::StreamExt;
use reqwest::{Client, header::HeaderValue};
use serde_json::{Value, json};
use std::sync::Arc;
use velune_ai::{InvalidContract, OperationFuture, Payload, provider::*, sampling::*};

type CredentialResolver = dyn Fn(&str) -> Option<String> + Send + Sync;

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

    pub fn with_resolver(
        config: ProviderConfig,
        client: Client,
        resolver: Arc<CredentialResolver>,
    ) -> Result<Self, InvalidContract> {
        let ProtocolConfig::ChatCompletions(protocol) = config.protocol() else {
            return Err(InvalidContract("provider requires Chat Completions v1"));
        };
        if protocol.endpoint.transport() != Transport::Http
            && protocol.endpoint.transport() != Transport::Https
        {
            return Err(InvalidContract("unsupported provider transport"));
        }
        let reference = config.credential().as_str().to_owned();
        Ok(Self {
            config,
            client,
            credential: Arc::new(move || resolver(&reference)),
        })
    }

    fn request_json(input: &SamplingInput, external_model: &str) -> Result<Value, InvalidContract> {
        if input.max_output_tokens().get() > 1_048_576 {
            return Err(InvalidContract("provider output limit is too large"));
        }
        let mut messages = Vec::new();
        if let Some(instructions) = input.instructions() {
            messages.push(json!({"role":"system","content":instructions}));
        }
        for message in input.messages() {
            messages.push(match message {
                Message::User(text) => json!({"role":"user","content":text.get()}),
                Message::Assistant { text, tool_calls } => {
                    let mut value = json!({
                        "role":"assistant",
                        "content":text.as_ref().map(Payload::get)
                    });
                    if !tool_calls.is_empty() {
                        value["tool_calls"] = tool_calls
                            .iter()
                            .map(|tool| {
                                json!({
                                    "id":tool.id.as_str(),
                                    "type":"function",
                                    "function":{
                                        "name":tool.name.as_str(),
                                    "arguments":tool.arguments.get().to_string()
                                    }
                                })
                            })
                            .collect();
                    }
                    value
                }
                Message::ToolResult {
                    call,
                    content,
                    is_error,
                } => {
                    if *is_error {
                        return Err(InvalidContract("error tool result is unsupported"));
                    }
                    json!({"role":"tool","tool_call_id":call.as_str(),"content":content.get()})
                }
            });
        }
        let mut body = json!({
            "model": external_model,
            "messages": messages,
            "max_completion_tokens": input.max_output_tokens().get(),
            "stream": true,
            "stream_options": {"include_usage": true}
        });
        if let Some(level) = input.options().reasoning_level() {
            body["reasoning_effort"] = Value::String(level.to_owned());
        }
        if !input.tools().is_empty() {
            body["tools"] = input
                .tools()
                .iter()
                .map(|tool| {
                    json!({
                        "type":"function",
                        "function":{
                            "name":tool.name.as_str(),
                            "description":tool.description.get(),
                            "parameters":tool.schema()
                        }
                    })
                })
                .collect();
            body["tool_choice"] = json!("auto");
        }
        if body.to_string().len() > 256 * 1024 {
            return Err(InvalidContract("provider request is too large"));
        }
        Ok(body)
    }
}

impl SamplingProvider for ChatCompletions {
    fn sampling(
        &self,
        request: ProviderSamplingRequest,
        mut events: ProviderSamplingSink,
    ) -> OperationFuture<ProviderSamplingOutcome> {
        let Some(mapping) = self.config.model(&request.context.model) else {
            return Box::pin(async {
                Decoder::new().failure(
                    SamplingErrorKind::InvalidInput,
                    ExecutionKnowledge::NotSent,
                    false,
                )
            });
        };
        let body = match Self::request_json(&request.input, mapping.external_name()) {
            Ok(body) => body,
            Err(_) => {
                return Box::pin(async {
                    Decoder::new().failure(
                        SamplingErrorKind::InvalidInput,
                        ExecutionKnowledge::NotSent,
                        false,
                    )
                });
            }
        };
        let Some(protocol) = (match self.config.protocol() {
            ProtocolConfig::ChatCompletions(value) => Some(value),
            _ => None,
        }) else {
            return Box::pin(async {
                Decoder::new().failure(
                    SamplingErrorKind::Unsupported,
                    ExecutionKnowledge::NotSent,
                    false,
                )
            });
        };
        let endpoint = protocol.endpoint.url("/chat/completions");
        let client = self.client.clone();
        let credential = Arc::clone(&self.credential);
        Box::pin(async move {
            let mut decoder = Decoder::new();
            let Some(credential) = credential() else {
                return decoder.failure(
                    SamplingErrorKind::Authentication,
                    ExecutionKnowledge::NotSent,
                    false,
                );
            };
            let mut auth = match HeaderValue::from_str(&format!("Bearer {credential}")) {
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
                .post(endpoint)
                .header("authorization", auth)
                .header("accept", "text/event-stream")
                .header("connection", "close")
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
            if status != 200 {
                return decoder.failure(
                    match status {
                        401 => SamplingErrorKind::Authentication,
                        403 => SamplingErrorKind::Permission,
                        429 => SamplingErrorKind::RateLimited,
                        _ => SamplingErrorKind::ProviderFailure,
                    },
                    ExecutionKnowledge::Accepted,
                    true,
                );
            }
            let is_sse = response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.split(';').next() == Some("text/event-stream"));
            if !is_sse {
                return decoder.failure(
                    SamplingErrorKind::Unsupported,
                    ExecutionKnowledge::Accepted,
                    true,
                );
            }
            let mut stream = response.bytes_stream().eventsource();
            while let Some(event) = stream.next().await {
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        return decoder.failure(
                            match error {
                                EventStreamError::Transport(_) => SamplingErrorKind::Transport,
                                EventStreamError::Utf8(_) | EventStreamError::Parser(_) => {
                                    SamplingErrorKind::ProviderFailure
                                }
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
                let projected = mapping::project(&raw);
                if let Err(kind) = decoder.push(&projected, &mut events) {
                    return decoder.failure(kind, ExecutionKnowledge::Accepted, true);
                }
            }
            decoder.failure(
                SamplingErrorKind::ProviderFailure,
                ExecutionKnowledge::Accepted,
                true,
            )
        })
    }
}
