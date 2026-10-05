//! Native OpenAI Chat Completions operation.
//!
//! The body and stream bytes remain protocol data. Routing may replace `model`,
//! but this operation does not project the request into sampling messages.

use crate::{
    DeliveryError, InvalidContract, OperationFuture, Payload,
    http::{Header, ResponseMeta},
    ids::{ModelId, ProviderId},
};
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};

#[derive(Debug, Clone)]
pub struct ChatCompletionsRequest {
    pub model: ModelId,
    pub body: Payload<Value>,
    pub headers: Vec<Header>,
    pub stream: bool,
}

#[derive(Debug, Clone)]
pub enum ChatCompletionsEvent {
    Headers(ResponseMeta),
    Body(Payload<Vec<u8>>),
}

#[derive(Debug, Clone)]
pub enum ChatCompletionsOutput {
    Json(crate::http::ResponseBody),
    Stream(ResponseMeta),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatCompletionsErrorKind {
    InvalidInput,
    Unsupported,
    Authentication,
    Permission,
    RateLimited,
    Timeout,
    Transport,
    ProviderFailure,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct ChatCompletionsFailure {
    pub kind: ChatCompletionsErrorKind,
    pub response: Option<ResponseMeta>,
    pub body: Option<Payload<Vec<u8>>>,
    pub submitted: bool,
}

#[derive(Debug, Clone)]
pub struct ChatCompletionsCompletion {
    pub result: Result<ChatCompletionsOutput, ChatCompletionsFailure>,
}

pub type ChatCompletionsSink =
    Box<dyn FnMut(ChatCompletionsEvent) -> OperationFuture<Result<(), DeliveryError>> + Send>;

pub trait ChatCompletionsProvider: Send + Sync {
    fn chat_completions(
        &self,
        request: ChatCompletionsRequest,
        events: ChatCompletionsSink,
    ) -> OperationFuture<ChatCompletionsCompletion>;
}

pub struct ChatCompletionsBinding {
    provider: ProviderId,
    models: HashSet<ModelId>,
    adapter: Arc<dyn ChatCompletionsProvider>,
}

impl ChatCompletionsBinding {
    pub fn new(
        provider: ProviderId,
        models: Vec<ModelId>,
        adapter: Arc<dyn ChatCompletionsProvider>,
    ) -> Result<Self, InvalidContract> {
        if models.is_empty() {
            return Err(InvalidContract("chat completions provider needs a model"));
        }
        let count = models.len();
        let models = models.into_iter().collect::<HashSet<_>>();
        if models.len() != count {
            return Err(InvalidContract("duplicate chat completions provider model"));
        }
        Ok(Self {
            provider,
            models,
            adapter,
        })
    }
}

pub struct DirectChatCompletionsService {
    binding: Arc<ChatCompletionsBinding>,
}

impl DirectChatCompletionsService {
    pub fn new(binding: Arc<ChatCompletionsBinding>) -> Self {
        Self { binding }
    }
}

impl ChatCompletionsProvider for DirectChatCompletionsService {
    fn chat_completions(
        &self,
        request: ChatCompletionsRequest,
        events: ChatCompletionsSink,
    ) -> OperationFuture<ChatCompletionsCompletion> {
        if !self.binding.models.contains(&request.model) {
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
        let _provider = &self.binding.provider;
        self.binding.adapter.chat_completions(request, events)
    }
}
