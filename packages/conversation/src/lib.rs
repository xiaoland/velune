//! Runtime-neutral, replaceable conversation projection contracts.
//! Harness adapters own native event parsing and durable sessions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub updated_at: Option<String>,
    pub runtime_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
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
    pub id: String,
    pub role: String,
    pub blocks: Vec<MessageBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MessageBlock {
    Text {
        text: String,
    },
    Tool {
        #[serde(rename = "toolID")]
        tool_id: Option<String>,
        title: String,
        state: Option<String>,
    },
    Notice {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSnapshot {
    pub revision: u64,
    pub conversation: ConversationSummary,
    pub resource_id: Option<String>,
    pub model_record_key: Option<String>,
    pub run_state: RunState,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub pending_interactions: Vec<RuntimeInteraction>,
    pub actions: ConversationActions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub state: String,
    pub capabilities: Vec<String>,
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
