//! Authoritative AI service contracts. Sampling is the first operation, not a separate service.
//! No storage, global configuration, provider SDK, transport, or tool execution lives here.
#![forbid(unsafe_code)]

pub mod chat_completions;
pub mod direct;
pub mod http;
pub mod ids;
pub mod messages;
pub mod observation;
pub mod provider;
pub mod responses;
pub mod sampling;

use std::{fmt, future::Future, pin::Pin};

pub type OperationFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryError {
    Closed,
    Cancelled,
}

/// Caller-facing contract. Capture the binding before returning the future.
pub trait SamplingService: Send + Sync {
    fn sampling(
        &self,
        request: sampling::SamplingRequest,
        attempt: ids::AttemptId,
        events: sampling::SamplingSink,
    ) -> Result<OperationFuture<sampling::SamplingCompletion>, InvalidContract>;
}

/// Contains a static field/reason only; never includes rejected input or provider response bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidContract(pub &'static str);
impl fmt::Display for InvalidContract {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for InvalidContract {}

/// Explicit access to request/response content; Debug cannot accidentally dump bodies.
/// This is not encryption or a secret store. Callers can still explicitly access the content.
#[derive(Clone, PartialEq, Eq)]
pub struct Payload<T>(T);
impl<T> Payload<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }
    pub fn get(&self) -> &T {
        &self.0
    }
    pub fn into_inner(self) -> T {
        self.0
    }
}
impl<T> fmt::Debug for Payload<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Payload([redacted])")
    }
}
