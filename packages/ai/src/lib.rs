//! AI service contracts: native LLM protocol operations, their envelopes and
//! outcomes. No storage, global configuration, provider SDK, transport, or
//! tool execution lives here.
#![forbid(unsafe_code)]

pub mod http;
mod native;
pub use native::*;

use std::{fmt, future::Future, pin::Pin};

pub type OperationFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryError {
    Closed,
    Cancelled,
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
