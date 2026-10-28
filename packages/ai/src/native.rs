//! Native protocol operations: one request to one provider model in its own
//! protocol. The body and stream bytes remain protocol data; routing may
//! replace `model`, but nothing here projects the request into another shape.
use crate::{
    DeliveryError, InvalidContract, OperationFuture, Payload,
    http::{Header, ResponseBody, ResponseMeta},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The LLM wire protocols Velune speaks natively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Protocol {
    /// OpenAI Chat Completions v1.
    #[serde(rename = "chatCompletionsV1")]
    ChatCompletions,
    /// OpenAI Responses v1.
    #[serde(rename = "responsesV1")]
    Responses,
    /// Anthropic Messages.
    #[serde(rename = "messagesV1")]
    Messages,
}

/// Exact identifier supplied by the provider, never a gateway alias or application record key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderModelId(String);
impl ProviderModelId {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidContract> {
        let value = value.into();
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(InvalidContract(
                "provider model identifier is empty or contains control characters",
            ));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct NativeRequest {
    pub model: ProviderModelId,
    pub body: Payload<Value>,
    pub headers: Vec<Header>,
    pub stream: bool,
}

#[derive(Debug, Clone)]
pub enum NativeEvent {
    /// A successful upstream response has begun. Body chunks follow for streams.
    Headers(ResponseMeta),
    Body(Payload<Vec<u8>>),
}

pub type NativeSink =
    Box<dyn FnMut(NativeEvent) -> OperationFuture<Result<(), DeliveryError>> + Send>;

/// Transport completion only: a forwarded stream can contain native error
/// events, which protocol consumers interpret; nothing here reconstructs one.
#[derive(Debug, Clone)]
pub enum NativeOutput {
    Json(ResponseBody),
    /// Already delivered through the sink.
    Stream(ResponseMeta),
}

#[derive(Debug, Clone)]
pub struct NativeSuccess {
    pub output: NativeOutput,
    /// The response itself reported completion. Only Responses reports it;
    /// incomplete, failed or cancelled terminals still succeed in transport.
    pub completed: bool,
}

/// What the provider boundary established, independently of the HTTP status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeErrorKind {
    InvalidInput,
    Unsupported,
    Authentication,
    Permission,
    RateLimited,
    /// Explicit account usage exhaustion, independent of its HTTP status.
    QuotaExhausted,
    /// Temporary service capacity exhaustion.
    Overloaded,
    Timeout,
    Transport,
    ProviderFailure,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct NativeFailure {
    pub kind: NativeErrorKind,
    /// The upstream response, when one arrived.
    pub response: Option<ResponseMeta>,
    pub body: Option<Payload<Vec<u8>>>,
    /// The request was handed to the transport; not proof the upstream received it.
    pub submitted: bool,
}
impl NativeFailure {
    /// Rejected before anything was sent.
    pub fn unsent(kind: NativeErrorKind) -> Self {
        Self {
            kind,
            response: None,
            body: None,
            submitted: false,
        }
    }
    /// Sent, without a response or with only its head.
    pub fn sent(kind: NativeErrorKind, response: Option<ResponseMeta>) -> Self {
        Self {
            kind,
            response,
            body: None,
            submitted: true,
        }
    }
}

pub type NativeCompletion = Result<NativeSuccess, NativeFailure>;

/// One attempt against one immutable provider binding in its own protocol.
/// No protocol conversion, routing or retry.
pub trait NativeProvider: Send + Sync {
    fn send(&self, request: NativeRequest, events: NativeSink)
    -> OperationFuture<NativeCompletion>;
}
