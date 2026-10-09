//! Runtime-neutral, replaceable conversation projection contracts.
//! Harness adapters own native event parsing and durable sessions.

use serde::{Deserialize, Serialize};

/// Native names are authoritative. Derived titles may be refreshed from the first user message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", content = "text", rename_all = "camelCase")]
pub enum ConversationTitle {
    Native(String),
    FirstMessage(String),
    Untitled,
}
impl ConversationTitle {
    pub fn display_text(&self) -> &str {
        match self {
            Self::Native(text) | Self::FirstMessage(text) => text,
            Self::Untitled => "未命名会话",
        }
    }
    pub fn refresh(&mut self, first_user_text: Option<&str>) {
        if !matches!(self, Self::Native(_)) {
            *self = conversation_title(None, first_user_text);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: String,
    pub title: ConversationTitle,
    /// Unix epoch milliseconds; None means the source did not provide a timestamp.
    pub updated_at_unix_ms: Option<i64>,
    /// Native creation time, when available; never inferred from modification time.
    pub created_at_unix_ms: Option<i64>,
    pub runtime_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub can_rename: bool,
    #[serde(default)]
    pub can_delete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Idle,
    Running,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationActions {
    pub can_send: bool,
    pub can_cancel: bool,
    pub can_switch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    /// Content is no longer streaming; completion does not imply execution success.
    pub completed: bool,
    pub id: String,
    pub role: MessageRole,
    /// Native message time in Unix epoch milliseconds, when available.
    pub timestamp_unix_ms: Option<i64>,
    pub blocks: Vec<MessageBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolState {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MessageBlock {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    Tool {
        #[serde(rename = "toolID")]
        tool_id: Option<String>,
        title: String,
        state: ToolState,
        output: Option<String>,
    },
    Notice {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSnapshot {
    /// Revision within the current projection lifecycle. History and execution
    /// projections may restart it; it is not a global ordering key across preparation.
    pub revision: u64,
    pub conversation: ConversationSummary,
    /// The current native context; independent of the logical origin used in lists.
    pub context_runtime_id: String,
    pub resource_id: Option<String>,
    pub model_record_key: Option<String>,
    pub run_state: RunState,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub transcript_turns: Vec<TranscriptTurn>,
    pub transcript_items: Vec<TranscriptItem>,
    pub transcript_outline: Vec<TranscriptOutlineEntry>,
    pub transcript_message_identities: Vec<TranscriptMessageIdentity>,
    /// Idempotent live-to-native confirmations within this projection lifecycle.
    #[serde(default)]
    pub message_identity_confirmations: Vec<MessageIdentityConfirmation>,
    #[serde(default)]
    pub pending_interactions: Vec<RuntimeInteraction>,
    pub actions: ConversationActions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SettingOption {
    pub id: String,
    pub label: String,
}

/// Platform executable discovery hint; the selected absolute path remains application configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExecutableDiscovery {
    pub command: String,
    pub minimum_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SettingField {
    pub key: String,
    pub label: String,
    pub kind: String,
    pub required: bool,
    pub value: String,
    pub options: Vec<SettingOption>,
    pub help: Option<String>,
    pub executable_discovery: Option<ExecutableDiscovery>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SettingAction {
    pub id: String,
    pub label: String,
}

/// A runtime request awaiting an explicit user response. IDs belong to the active session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInteraction {
    pub id: String,
    pub kind: InteractionKind,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InteractionKind {
    Approval {
        title: String,
        detail: String,
        options: Vec<InteractionOption>,
    },
    UserInput {
        questions: Vec<InteractionQuestion>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InteractionOption {
    pub id: String,
    pub label: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InteractionQuestion {
    pub id: String,
    pub text: String,
    pub options: Vec<InteractionOption>,
    pub secret: bool,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RuntimeInteractionReply {
    Decision {
        #[serde(rename = "optionId")]
        option_id: String,
    },
    Answers {
        answers: Vec<InteractionAnswer>,
    },
    Cancel,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InteractionAnswer {
    pub question_id: String,
    pub values: Vec<String>,
}

/// A readable projection title. Native names have priority; fallback text is a
/// bounded single line from the first user message, without an additional model call.
pub fn conversation_title(
    native_name: Option<&str>,
    first_user_text: Option<&str>,
) -> ConversationTitle {
    fn readable(text: &str) -> Option<String> {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!text.is_empty()).then(|| text.chars().take(80).collect())
    }
    if let Some(text) = native_name.filter(|text| !text.trim().is_empty()) {
        ConversationTitle::Native(text.trim().into())
    } else if let Some(text) = first_user_text.and_then(readable) {
        ConversationTitle::FirstMessage(text)
    } else {
        ConversationTitle::Untitled
    }
}

pub fn first_user_text(messages: &[Message]) -> Option<&str> {
    messages
        .iter()
        .filter(|m| m.role == MessageRole::User)
        .flat_map(|m| &m.blocks)
        .find_map(|block| match block {
            MessageBlock::Text { text } if !text.trim().is_empty() => Some(text.as_str()),
            _ => None,
        })
}

/// Disposable presentation range; references canonical messages without copying content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptTurn {
    pub id: String,
    pub user_message_id: String,
    pub work_message_ids: Vec<String>,
    /// The text-only assistant currently visible outside the work group. It may
    /// still be streaming; execution lifecycle determines whether work is running.
    pub last_message_id: Option<String>,
    /// Observed execution elapsed time. None for history without lifecycle evidence.
    pub duration_ms: Option<u64>,
    pub is_running: bool,
}

/// A transient identity confirmation; native messages remain the durable authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MessageIdentityConfirmation {
    pub previous_id: String,
    pub current_id: String,
}

/// Complete ordered presentation, emitted atomically with canonical content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TranscriptItem {
    Message { id: String, message_id: String },
    Work { turn_id: String },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptOutlineEntry {
    pub id: String,
    pub message_id: String,
}

/// Stable disposable row identity, including rows currently inside a work group.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptMessageIdentity {
    pub id: String,
    pub message_id: String,
}
