//! Native OpenAI Responses transport contract.
//!
//! Responses requests are intentionally opaque JSON.  The gateway may validate
//! the selected model and bounded fields, while the provider adapter preserves
//! native `input`, `tools`, `reasoning`, and terminal event semantics.
use crate::{
    DeliveryError, InvalidContract, OperationFuture, Payload,
    http::{Header, ResponseBody, ResponseMeta},
    ids::{ModelId, ProviderId},
};
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};

#[derive(Debug, Clone)]
pub struct ResponsesRequest {
    pub model: ModelId,
    pub body: Payload<Value>,
    pub headers: Vec<Header>,
    pub stream: bool,
}

#[derive(Debug, Clone)]
pub enum ResponsesEvent {
    /// A successful upstream response has begun. Body chunks follow for streams.
    Headers(ResponseMeta),
    Body(Payload<Vec<u8>>),
}

#[derive(Debug, Clone)]
pub enum ResponsesOutput {
    Json(ResponseBody, ResponsesTerminal),
    Stream(ResponseMeta, ResponsesTerminal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponsesTerminal {
    Completed,
    Incomplete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponsesErrorKind {
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
pub struct ResponsesFailure {
    pub kind: ResponsesErrorKind,
    pub status: Option<u16>,
    pub headers: Vec<Header>,
    pub body: Option<Payload<Vec<u8>>>,
    pub submitted: bool,
    pub terminal: Option<ResponsesTerminal>,
}

#[derive(Debug, Clone)]
pub struct ResponsesCompletion {
    pub result: Result<ResponsesOutput, ResponsesFailure>,
}

pub type ResponsesSink =
    Box<dyn FnMut(ResponsesEvent) -> OperationFuture<Result<(), DeliveryError>> + Send>;

/// One immutable native Responses provider binding. No protocol conversion or retry.
pub trait ResponsesProvider: Send + Sync {
    fn responses(
        &self,
        request: ResponsesRequest,
        events: ResponsesSink,
    ) -> OperationFuture<ResponsesCompletion>;
}

/// Immutable service binding for one provider's native Responses route.
/// The gateway cannot retarget an in-flight operation by replacing app config.
pub struct ResponsesBinding {
    provider: ProviderId,
    models: HashSet<ModelId>,
    adapter: Arc<dyn ResponsesProvider>,
}

impl ResponsesBinding {
    pub fn new(
        provider: ProviderId,
        models: Vec<ModelId>,
        adapter: Arc<dyn ResponsesProvider>,
    ) -> Result<Self, InvalidContract> {
        if models.is_empty() {
            return Err(InvalidContract(
                "responses provider needs at least one model",
            ));
        }
        let model_count = models.len();
        let models = models.into_iter().collect::<HashSet<_>>();
        if models.len() != model_count {
            return Err(InvalidContract("duplicate responses provider model"));
        }
        Ok(Self {
            provider,
            models,
            adapter,
        })
    }
}

pub struct DirectResponsesService {
    binding: Arc<ResponsesBinding>,
}

impl DirectResponsesService {
    pub fn new(binding: Arc<ResponsesBinding>) -> Self {
        Self { binding }
    }
}

impl ResponsesProvider for DirectResponsesService {
    fn responses(
        &self,
        request: ResponsesRequest,
        events: ResponsesSink,
    ) -> OperationFuture<ResponsesCompletion> {
        if !self.binding.models.contains(&request.model) {
            return Box::pin(async {
                ResponsesCompletion {
                    result: Err(ResponsesFailure {
                        kind: ResponsesErrorKind::InvalidInput,
                        status: None,
                        headers: Vec::new(),
                        body: None,
                        submitted: false,
                        terminal: None,
                    }),
                }
            });
        }
        let _provider = &self.binding.provider;
        self.binding.adapter.responses(request, events)
    }
}
