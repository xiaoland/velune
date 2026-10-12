//! Transcript transfer records for presentation clients. Projections keep
//! complete messages; a delivered snapshot carries large payloads only by size,
//! and a client polling with a cursor receives only messages whose content changed.
use serde::{Deserialize, Serialize};
use velune_conversation::{
    ConversationActions, ConversationSummary, MessageIdentityConfirmation, MessageRole, RunState,
    RuntimeInteraction, ToolState, TranscriptItem, TranscriptMessageIdentity,
    TranscriptOutlineEntry, TranscriptTurn,
};

/// Position in one view's delivery history. A generation identifies the view
/// instance; a sequence advances whenever delivered content changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptCursor {
    pub generation: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MessagePayload {
    Inline {
        text: String,
    },
    /// Read with `message_payloads`; the size lets clients describe it without content.
    Deferred {
        bytes: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DeliveredBlock {
    Text {
        text: String,
    },
    /// Reasoning with a summary; `text` is what to show under it.
    Reasoning {
        summary: String,
        text: MessagePayload,
    },
    Tool {
        #[serde(rename = "toolID")]
        tool_id: Option<String>,
        title: String,
        state: ToolState,
        output: Option<MessagePayload>,
    },
    Notice {
        text: String,
    },
    Image {
        media_type: Option<String>,
        source: DeliveredImageSource,
    },
    /// A non-image file the user attached by path.
    File {
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DeliveredImageSource {
    Path {
        path: String,
    },
    /// Base64 content; usually large enough to be read with `message_payloads`.
    Data {
        data: MessagePayload,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveredMessage {
    /// Sequence at which this content was first delivered; unchanged content keeps it.
    pub stamp: u64,
    pub completed: bool,
    pub id: String,
    pub role: MessageRole,
    pub timestamp_unix_ms: Option<i64>,
    pub blocks: Vec<DeliveredBlock>,
}

/// A conversation projection as transferred to a presentation client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveredSnapshot {
    pub cursor: TranscriptCursor,
    /// None: `messages` is complete. Some: `messages` holds only messages
    /// changed after this cursor, and `message_ids` gives the complete order.
    pub base: Option<TranscriptCursor>,
    pub message_ids: Vec<String>,
    pub messages: Vec<DeliveredMessage>,
    pub revision: u64,
    pub conversation: ConversationSummary,
    pub context_runtime_id: String,
    pub resource_id: Option<String>,
    pub model_record_key: Option<String>,
    pub run_state: RunState,
    pub transcript_turns: Vec<TranscriptTurn>,
    pub transcript_items: Vec<TranscriptItem>,
    pub transcript_outline: Vec<TranscriptOutlineEntry>,
    pub transcript_message_identities: Vec<TranscriptMessageIdentity>,
    pub message_identity_confirmations: Vec<MessageIdentityConfirmation>,
    pub pending_interactions: Vec<RuntimeInteraction>,
    pub actions: ConversationActions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SnapshotUpdate {
    /// No conversation is loaded in the queried runtime context.
    Absent,
    /// The cursor is current.
    Unchanged,
    Changed {
        snapshot: Box<DeliveredSnapshot>,
    },
}
