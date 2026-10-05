//! Content-free observation types; no persistence or logging backend.
use crate::{
    ids::*,
    sampling::{ExecutionKnowledge, SamplingErrorKind},
};
use std::time::Duration;

use crate::sampling::Usage;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Sampling,
}
#[derive(Debug, Clone)]
pub struct CallObservation {
    pub call: CallId,
    pub operation: Operation,
    pub revision: ConfigRevision,
    pub elapsed: Duration,
    /// Count only actual submitted attempts, never infer from a logical operation count.
    pub attempts: u32,
}
#[derive(Debug, Clone)]
pub struct AttemptContext {
    pub call: CallId,
    pub attempt: AttemptId,
    pub provider: ProviderId,
    pub model: ProviderModelId,
    pub revision: ConfigRevision,
}
#[derive(Debug, Clone)]
pub struct AttemptObservation {
    pub context: AttemptContext,
    pub execution: ExecutionKnowledge,
    pub error: Option<SamplingErrorKind>,
    pub usage: Usage,
    pub elapsed: Duration,
}
