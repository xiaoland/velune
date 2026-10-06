//! Native Anthropic Messages operation.
//!
//! The body and stream bytes remain protocol data. Routing may replace `model`,
//! but this operation does not project the request into sampling messages.

use crate::{
    DeliveryError, InvalidContract, OperationFuture, Payload,
    http::{Header, ResponseMeta},
    ids::{ProviderId, ProviderModelId},
};
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};

#[derive(Debug, Clone)]
pub struct MessagesRequest {
    pub model: ProviderModelId,
    pub body: Payload<Value>,
    pub headers: Vec<Header>,
    pub stream: bool,
}

#[derive(Debug, Clone)]
pub enum MessagesEvent {
    Headers(ResponseMeta),
    Body(Payload<Vec<u8>>),
}

#[derive(Debug, Clone)]
/// Transport completion only: a forwarded SSE stream can contain native error events.
/// Protocol consumers interpret those events; this API never reconstructs a message.
pub enum MessagesOutput {
    Json(crate::http::ResponseBody),
    Stream(ResponseMeta),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagesErrorKind {
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
pub struct MessagesFailure {
    pub kind: MessagesErrorKind,
    pub response: Option<ResponseMeta>,
    pub body: Option<Payload<Vec<u8>>>,
    pub submitted: bool,
}

#[derive(Debug, Clone)]
pub struct MessagesCompletion {
    pub result: Result<MessagesOutput, MessagesFailure>,
}

pub type MessagesSink =
    Box<dyn FnMut(MessagesEvent) -> OperationFuture<Result<(), DeliveryError>> + Send>;

/// Native /v1/messages operation. Callers supply anthropic-version in request headers.
/// Authentication is resolved by the composition root, not read from these headers.
pub trait MessagesProvider: Send + Sync {
    fn messages(
        &self,
        request: MessagesRequest,
        events: MessagesSink,
    ) -> OperationFuture<MessagesCompletion>;
}

pub struct MessagesBinding {
    provider: ProviderId,
    models: HashSet<ProviderModelId>,
    adapter: Arc<dyn MessagesProvider>,
}

impl MessagesBinding {
    pub fn new(
        provider: ProviderId,
        models: Vec<ProviderModelId>,
        adapter: Arc<dyn MessagesProvider>,
    ) -> Result<Self, InvalidContract> {
        if models.is_empty() {
            return Err(InvalidContract("messages provider needs a model"));
        }
        let count = models.len();
        let models = models.into_iter().collect::<HashSet<_>>();
        if models.len() != count {
            return Err(InvalidContract("duplicate messages provider model"));
        }
        Ok(Self {
            provider,
            models,
            adapter,
        })
    }
}

pub struct DirectMessagesService {
    binding: Arc<MessagesBinding>,
}

impl DirectMessagesService {
    pub fn new(binding: Arc<MessagesBinding>) -> Self {
        Self { binding }
    }
}

impl MessagesProvider for DirectMessagesService {
    fn messages(
        &self,
        request: MessagesRequest,
        events: MessagesSink,
    ) -> OperationFuture<MessagesCompletion> {
        if !self.binding.models.contains(&request.model) {
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
        let _provider = &self.binding.provider;
        self.binding.adapter.messages(request, events)
    }
}
