//! Explicit, request-scoped translation between LLM wire protocols.
//!
//! Native requests bypass this module. Best-effort projections report static
//! conversion notes; malformed structures never become successful outputs.
mod request;
mod response;
mod stream;

use serde_json::Value;
use std::{collections::BTreeMap, fmt};
pub use stream::StreamTranslator;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmProtocol {
    ChatCompletions,
    Responses,
    Messages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslationErrorKind {
    Invalid,
    Truncated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranslationError {
    pub kind: TranslationErrorKind,
    pub field: &'static str,
}
impl fmt::Display for TranslationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.field)
    }
}
impl std::error::Error for TranslationError {}
pub(super) fn invalid(field: &'static str) -> TranslationError {
    TranslationError {
        kind: TranslationErrorKind::Invalid,
        field,
    }
}
/// Immutable conversion plan for one request. Tool names and custom-tool codecs
/// belong to this plan, not global gateway state.
pub struct PreparedTranslation {
    from: LlmProtocol,
    to: LlmProtocol,
    request: Value,
    tools: BTreeMap<String, ToolIdentity>,
    notes: Vec<ConversionNote>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversionNote {
    pub field: &'static str,
    pub code: &'static str,
}
impl fmt::Debug for PreparedTranslation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedTranslation")
            .field("from", &self.from)
            .field("to", &self.to)
            .finish_non_exhaustive()
    }
}
#[derive(Clone)]
pub(super) struct ToolIdentity {
    original: String,
    namespace: Option<String>,
    custom: bool,
}
impl PreparedTranslation {
    pub fn request(&self) -> &Value {
        &self.request
    }
    pub fn notes(&self) -> &[ConversionNote] {
        &self.notes
    }
    pub fn translate_json(&mut self, body: &Value) -> Result<Value, TranslationError> {
        request::note(&mut self.notes, "response", "best_effort_projection");
        response::translate(self.to, self.from, body, &self.tools, &mut self.notes)
    }
    pub fn stream(&self, model: String) -> StreamTranslator {
        StreamTranslator::new(self.to, self.from, model, self.tools.clone())
    }
}

/// Prepares an asynchronous operation without reading configuration or secrets.
/// Explicit request output limits take precedence over `target_output_limit`.
/// The latter is used only when Messages requires a missing `max_tokens`.
pub fn prepare(
    from: LlmProtocol,
    to: LlmProtocol,
    body: &Value,
    target_output_limit: Option<u32>,
) -> Result<PreparedTranslation, TranslationError> {
    if from == to {
        return Err(invalid("translation_requires_different_protocols"));
    }
    let (request, tools, notes) = request::translate(from, to, body, target_output_limit)?;
    Ok(PreparedTranslation {
        from,
        to,
        request,
        tools,
        notes,
    })
}

pub(super) fn object<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a serde_json::Map<String, Value>, TranslationError> {
    value.as_object().ok_or_else(|| invalid(field))
}
pub(super) fn string<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a str, TranslationError> {
    value.as_str().ok_or_else(|| invalid(field))
}
pub(super) fn array<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a Vec<Value>, TranslationError> {
    value.as_array().ok_or_else(|| invalid(field))
}

/// Local conversion identity, never a reference to upstream server-side state.
pub(super) fn local_response_id() -> String {
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    format!(
        "velune_translation_{:x}_{now:x}_{:x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
