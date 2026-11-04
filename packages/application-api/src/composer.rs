//! Shared unfinished input. Client navigation and execution targets are separate.
use crate::TurnChoice;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, uniffi::Enum)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ComposerContextKey {
    Conversation {
        owner_id: String,
        conversation_id: String,
    },
    NewConversation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComposerAttachment {
    pub id: String,
    pub name: String,
    pub bytes: u64,
}

#[derive(Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComposerAttachmentContent {
    pub attachment: ComposerAttachment,
    #[serde(with = "crate::bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComposerRuntimeChoice {
    pub owner_id: String,
    pub runtime_id: String,
    pub choice: TurnChoice,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComposerInputContent {
    pub text: String,
    pub attachments: Vec<ComposerAttachment>,
}

/// Execution choices belong to a target, independently of borrowed input content.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComposerTargetChoices {
    pub runtime_id: Option<String>,
    pub runtime_owner_id: Option<String>,
    pub choices: Vec<ComposerRuntimeChoice>,
    pub choice_edited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComposerTarget {
    pub context: ComposerContextKey,
    pub revision: u64,
    pub choices: ComposerTargetChoices,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComposerInput {
    pub id: String,
    pub revision: u64,
    pub content: ComposerInputContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComposerInputBinding {
    pub context: ComposerContextKey,
    pub input_id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComposerCreation {
    pub input_id: String,
    pub owner_id: String,
    pub runtime_id: String,
    pub conversation_id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComposerInputState {
    /// Stable execution-application identity, independent of the viewing client.
    pub owner_id: String,
    pub revision: u64,
    pub inputs: Vec<ComposerInput>,
    pub bindings: Vec<ComposerInputBinding>,
    pub targets: Vec<ComposerTarget>,
    /// Native identity handoffs, including completed operations, for idempotent recovery.
    pub creations: Vec<ComposerCreation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComposerInputUpdate {
    pub revision: u64,
    /// None means the caller already has this exact directory revision.
    pub state: Option<ComposerInputState>,
}

#[derive(Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(
    tag = "operation",
    content = "parameters",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ComposerInputOperation {
    Read {
        since_revision: Option<u64>,
    },
    /// Resolve once. Reopening a context must not reinitialize its content.
    Ensure {
        context: ComposerContextKey,
        input_id: String,
        seed: ComposerInputContent,
    },
    /// None creates a new identity; Some updates exactly the observed revision.
    Save {
        input_id: String,
        expected_revision: Option<u64>,
        content: ComposerInputContent,
    },
    EnsureTarget {
        context: ComposerContextKey,
        seed: ComposerTargetChoices,
    },
    SaveTarget {
        context: ComposerContextKey,
        expected_revision: Option<u64>,
        choices: ComposerTargetChoices,
    },
    Bind {
        context: ComposerContextKey,
        input_id: String,
        expected_revision: Option<u64>,
    },
    /// The old new-conversation input becomes this native session's input.
    Promote {
        input_id: String,
        owner_id: String,
        conversation_id: String,
        runtime_id: String,
    },
    Forget {
        context: ComposerContextKey,
    },
}
