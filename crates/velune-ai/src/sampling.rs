//! Sampling input/output semantics, shared where appropriate by the two service contracts.
use crate::{InvalidContract, Payload, ids::*, observation::Usage};
use serde_json::Value;
use std::{collections::HashSet, num::NonZeroU32};

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: ToolName,
    pub description: Payload<String>,
    // A JSON object, without interpreting any provider-specific schema dialect here.
    schema: Payload<Value>,
}
impl ToolDefinition {
    pub fn new(
        name: ToolName,
        description: Payload<String>,
        schema: Value,
    ) -> Result<Self, InvalidContract> {
        if !schema.is_object() {
            return Err(InvalidContract("tool schema must be an object"));
        }
        Ok(Self {
            name,
            description,
            schema: Payload::new(schema),
        })
    }
    pub fn schema(&self) -> &Value {
        self.schema.get()
    }
}
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: ToolName,
    pub arguments: Payload<Value>,
}
#[derive(Debug, Clone)]
pub enum Message {
    User(Payload<String>),
    Assistant {
        text: Option<Payload<String>>,
        tool_calls: Vec<ToolCall>,
    },
    ToolResult {
        call: ToolCallId,
        content: Payload<String>,
        is_error: bool,
    },
}
#[derive(Debug, Clone)]
pub struct SamplingInput {
    instructions: Option<Payload<String>>,
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
    max_output_tokens: NonZeroU32,
}
impl SamplingInput {
    pub fn new(
        instructions: Option<Payload<String>>,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        max_output_tokens: u32,
    ) -> Result<Self, InvalidContract> {
        if messages.is_empty() {
            return Err(InvalidContract("sampling messages must not be empty"));
        }
        let max_output_tokens = NonZeroU32::new(max_output_tokens)
            .ok_or(InvalidContract("max output tokens must be positive"))?;
        let mut names = HashSet::new();
        if tools.iter().any(|t| !names.insert(&t.name)) {
            return Err(InvalidContract("duplicate tool name"));
        }
        // Tool execution remains the caller's job. Validate correlation, not execution or JSON Schema.
        let mut calls = HashSet::new();
        let mut results = HashSet::new();
        for message in &messages {
            match message {
                Message::Assistant { tool_calls, .. } => {
                    for call in tool_calls {
                        if !calls.insert(&call.id) {
                            return Err(InvalidContract("duplicate tool call ID"));
                        }
                    }
                }
                Message::ToolResult { call, .. } => {
                    if !calls.contains(call) || !results.insert(call) {
                        return Err(InvalidContract(
                            "tool result requires a unique preceding call",
                        ));
                    }
                }
                Message::User(_) => {}
            }
        }
        Ok(Self {
            instructions,
            messages,
            tools,
            max_output_tokens,
        })
    }
    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_ref().map(|p| p.get().as_str())
    }
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }
    pub fn tools(&self) -> &[ToolDefinition] {
        &self.tools
    }
    pub fn max_output_tokens(&self) -> NonZeroU32 {
        self.max_output_tokens
    }
}
/// Explicit caller-selected target for this contract slice; no routing/fallback algorithm.
#[derive(Debug, Clone)]
pub struct SamplingRequest {
    pub call_id: CallId,
    pub provider: ProviderId,
    pub model: ModelId,
    pub input: SamplingInput,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Complete,
    OutputLimit,
    ToolCalls,
}
#[derive(Debug, Clone)]
pub struct SamplingOutput {
    pub text: Option<Payload<String>>,
    pub tool_calls: Vec<ToolCall>,
    pub finish: FinishReason,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingErrorKind {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionKnowledge {
    NotSent,
    Accepted,
    Unknown,
}
/// No raw provider error/body or automatic retry hint. Usage may exist even on failure.
#[derive(Debug, Clone)]
pub struct SamplingFailure {
    pub kind: SamplingErrorKind,
    pub execution: ExecutionKnowledge,
    pub partial: Option<PartialSamplingOutput>,
}

/// Incomplete tool arguments must not masquerade as a valid ToolCall or a completed output.
#[derive(Debug, Clone)]
pub struct PartialToolCall {
    pub index: u32,
    pub id: Option<ToolCallId>,
    pub name: Option<ToolName>,
    pub arguments: Payload<String>,
}
#[derive(Debug, Clone)]
pub struct PartialSamplingOutput {
    pub text: Option<Payload<String>>,
    pub tool_calls: Vec<PartialToolCall>,
}

/// Provider-independent streaming content. Index is local to this attempt; arguments are fragments,
/// not parsed JSON until completion. Finish may precede a final usage update.
#[derive(Debug, Clone)]
pub enum SamplingDelta {
    Text(Payload<String>),
    ToolIdentity {
        index: u32,
        id: Option<ToolCallId>,
        name: Option<ToolName>,
    },
    ToolArguments {
        index: u32,
        fragment: Payload<String>,
    },
    Finish(FinishReason),
    Usage(Usage),
}
#[derive(Debug, Clone)]
pub enum SamplingEventKind {
    Delta(SamplingDelta),
    Terminal { error: Option<SamplingErrorKind> },
}
#[derive(Debug, Clone)]
pub struct SamplingEvent {
    pub context: crate::observation::AttemptContext,
    pub sequence: u64,
    pub kind: SamplingEventKind,
}
pub type SamplingSink = Box<dyn FnMut(SamplingEvent) + Send>;

#[derive(Debug, Clone)]
pub struct SamplingCompletion {
    pub outcome: SamplingOutcome,
    pub call: crate::observation::CallObservation,
    /// Absent when the provider rejected the request before transport submission.
    pub attempt: Option<crate::observation::AttemptObservation>,
}
#[derive(Debug, Clone)]
pub struct SamplingOutcome {
    pub call_id: CallId,
    pub result: Result<SamplingOutput, SamplingFailure>,
    pub usage: Usage,
}
