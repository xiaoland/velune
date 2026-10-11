//! Request-scoped RelayKit protocol conversion. Rust owns transport and authentication.
mod bridge;
use bridge::{Session, call};
use serde_json::{Value, json};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmProtocol {
    ChatCompletions,
    Responses,
    Messages,
}
impl LlmProtocol {
    fn wire(self) -> &'static str {
        match self {
            Self::ChatCompletions => "openai",
            Self::Responses => "openai_responses",
            Self::Messages => "claude",
        }
    }
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
fn invalid(field: &'static str) -> TranslationError {
    TranslationError {
        kind: TranslationErrorKind::Invalid,
        field,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionNote {
    pub field: String,
    pub code: String,
}

fn append_notes(notes: &mut Vec<ConversionNote>, result: &Value) {
    if let Some(values) = result["notes"].as_array() {
        for value in values {
            if let (Some(field), Some(code)) = (value["field"].as_str(), value["code"].as_str()) {
                let note = ConversionNote {
                    field: field.into(),
                    code: code.into(),
                };
                if !notes.contains(&note) {
                    notes.push(note);
                }
            }
        }
    }
}

pub struct PreparedTranslation {
    session: Session,
    request: Value,
    notes: Vec<ConversionNote>,
}
impl fmt::Debug for PreparedTranslation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedTranslation")
            .finish_non_exhaustive()
    }
}
impl PreparedTranslation {
    pub fn request(&self) -> &Value {
        &self.request
    }
    pub fn notes(&self) -> &[ConversionNote] {
        &self.notes
    }
    pub fn translate_json(&mut self, body: &Value) -> Result<Value, TranslationError> {
        let mut result = call(self.session.handle, json!({"op":"response", "body":body}))?;
        append_notes(&mut self.notes, &result);
        Ok(result["body"].take())
    }
    pub fn stream(self, model: String) -> Result<StreamTranslator, TranslationError> {
        call(self.session.handle, json!({"op":"stream", "model":model}))?;
        Ok(StreamTranslator {
            session: self.session,
            notes: self.notes,
        })
    }
}

/// Native protocols bypass conversion. Messages uses the configured output limit only when absent.
pub fn prepare(
    from: LlmProtocol,
    to: LlmProtocol,
    body: &Value,
    target_output_limit: Option<u32>,
) -> Result<PreparedTranslation, TranslationError> {
    if from == to {
        return Err(invalid("translation_requires_different_protocols"));
    }
    if !body.is_object() {
        return Err(invalid("request.object"));
    }
    if to == LlmProtocol::Messages
        && target_output_limit.is_none()
        && ["max_tokens", "max_completion_tokens", "max_output_tokens"]
            .iter()
            .all(|key| body[*key].is_null())
    {
        return Err(invalid("max_tokens_required"));
    }
    let mut result = call(
        0,
        json!({"op":"prepare", "from":from.wire(), "to":to.wire(), "body":body, "limit":target_output_limit.unwrap_or(0)}),
    )?;
    let handle = result["handle"]
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v != 0)
        .ok_or_else(|| invalid("relaykit.handle"))?;
    let mut notes = vec![];
    append_notes(&mut notes, &result);
    Ok(PreparedTranslation {
        session: Session::new(handle),
        request: result["body"].take(),
        notes,
    })
}

pub struct StreamTranslator {
    session: Session,
    notes: Vec<ConversionNote>,
}
impl fmt::Debug for StreamTranslator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamTranslator").finish_non_exhaustive()
    }
}
impl StreamTranslator {
    pub fn notes(&self) -> &[ConversionNote] {
        &self.notes
    }
    pub fn push_event(&mut self, event: &str, data: &str) -> Result<Vec<u8>, TranslationError> {
        self.command(json!({"op":"push", "event":event, "data":data}))
    }
    pub fn finish(&mut self) -> Result<Vec<u8>, TranslationError> {
        self.command(json!({"op":"finish"}))
    }
    fn command(&mut self, command: Value) -> Result<Vec<u8>, TranslationError> {
        let result = call(self.session.handle, command)?;
        append_notes(&mut self.notes, &result);
        let mut bytes = Vec::new();
        if let Some(events) = result["events"].as_array() {
            for event in events {
                if let Some(kind) = event["type"].as_str() {
                    bytes.extend_from_slice(format!("event: {kind}\n").as_bytes());
                }
                bytes.extend_from_slice(format!("data: {event}\n\n").as_bytes());
            }
        }
        if result["done"] == true {
            bytes.extend_from_slice(b"data: [DONE]\n\n");
        }
        Ok(bytes)
    }
}
