//! Transcript transfer records for presentation clients. Projections keep
//! complete messages; a delivered snapshot carries large payloads only by size,
//! and a client polling with a cursor receives only messages whose content changed.
use serde::{Deserialize, Serialize};
use velune_conversation::{
    ConversationActions, ConversationSummary, MessageIdentityConfirmation, MessageRole, RateLimit,
    RunState, RuntimeInteraction, ToolState, TranscriptItem, TranscriptMessageIdentity,
    TranscriptOutlineEntry, TranscriptTurn,
};

/// Position in one view's delivery history. A generation identifies the view
/// instance; a sequence advances whenever delivered content changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptCursor {
    pub generation: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
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
        /// The conversation this call started or addressed, when it was one of
        /// Velune's collaboration tools; clients offer to open it.
        #[serde(rename = "linkedConversationID")]
        linked_conversation_id: Option<String>,
    },
    Notice {
        text: String,
    },
    RuntimeFailure {
        failure: velune_conversation::RuntimeFailureKind,
        detail: String,
        retry: Option<velune_conversation::RuntimeRetry>,
    },
    ContextCompaction {
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
    /// The conversation whose agent sent this user-side message through
    /// Velune; it precedes the message text.
    AgentSender {
        #[serde(rename = "conversationID")]
        conversation_id: String,
        runtime_name: String,
        title: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
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
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
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
    /// What the turns' native records say the conversation used.
    pub usage: ConversationUsage,
    pub transcript_items: Vec<TranscriptItem>,
    pub transcript_outline: Vec<TranscriptOutlineEntry>,
    pub transcript_message_identities: Vec<TranscriptMessageIdentity>,
    pub message_identity_confirmations: Vec<MessageIdentityConfirmation>,
    pub pending_interactions: Vec<RuntimeInteraction>,
    pub actions: ConversationActions,
    pub follow_up: velune_conversation::FollowUp,
    pub sub_agents: Vec<velune_conversation::SubAgent>,
    pub collaborators: Vec<velune_conversation::Collaborator>,
}

/// Where a conversation stands by its turns' native records: context is
/// the latest recorded turn's, counts are summed over the turns that
/// recorded them (absent when none did), and cost and account limits are
/// the latest reported. Nothing is estimated.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ConversationUsage {
    #[uniffi(default = 0)]
    pub ended_turns: u32,
    /// Ended turns whose native record has no usage.
    #[uniffi(default = 0)]
    pub unrecorded_turns: u32,
    #[uniffi(default = None)]
    pub context_tokens: Option<u64>,
    #[uniffi(default = None)]
    pub context_window: Option<u64>,
    #[uniffi(default = None)]
    pub input_tokens: Option<u64>,
    #[uniffi(default = None)]
    pub cache_read_tokens: Option<u64>,
    #[uniffi(default = None)]
    pub output_tokens: Option<u64>,
    #[uniffi(default = None)]
    pub reasoning_tokens: Option<u64>,
    /// Input and output together, when either is recorded.
    #[uniffi(default = None)]
    pub session_tokens: Option<u64>,
    #[uniffi(default = None)]
    pub session_cost_micro_usd: Option<u64>,
    #[uniffi(default = [])]
    pub rate_limits: Vec<RateLimit>,
}

impl ConversationUsage {
    pub fn of(turns: &[TranscriptTurn]) -> Self {
        let recorded = || turns.iter().filter_map(|turn| turn.usage.as_ref());
        let total = |count: fn(&velune_conversation::TurnUsage) -> Option<u64>| {
            recorded()
                .filter_map(count)
                .reduce(|a, b| a.saturating_add(b))
        };
        let latest = recorded().next_back();
        let input_tokens = total(|usage| usage.input_tokens);
        let output_tokens = total(|usage| usage.output_tokens);
        let ended = || turns.iter().filter(|turn| !turn.is_running);
        Self {
            ended_turns: ended().count() as u32,
            unrecorded_turns: ended().filter(|turn| turn.usage.is_none()).count() as u32,
            context_tokens: latest.and_then(|usage| usage.context_tokens),
            context_window: latest.and_then(|usage| usage.context_window),
            input_tokens,
            cache_read_tokens: total(|usage| usage.cache_read_tokens),
            output_tokens,
            reasoning_tokens: total(|usage| usage.reasoning_tokens),
            session_tokens: (input_tokens.is_some() || output_tokens.is_some()).then(|| {
                input_tokens
                    .unwrap_or(0)
                    .saturating_add(output_tokens.unwrap_or(0))
            }),
            session_cost_micro_usd: recorded()
                .rev()
                .find_map(|usage| usage.session_cost_micro_usd),
            rate_limits: recorded()
                .rev()
                .map(|usage| &usage.rate_limits)
                .find(|limits| !limits.is_empty())
                .cloned()
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[allow(clippy::large_enum_variant)]
pub enum SnapshotUpdate {
    /// No conversation is loaded in the queried runtime context.
    Absent,
    /// The cursor is current.
    Unchanged,
    Changed {
        snapshot: DeliveredSnapshot,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use velune_conversation::TurnUsage;

    fn turn(running: bool, usage: Option<TurnUsage>) -> TranscriptTurn {
        TranscriptTurn {
            id: String::new(),
            user_message_id: String::new(),
            work_message_ids: Vec::new(),
            last_message_id: None,
            reply_start_block: 0,
            duration_ms: None,
            is_running: running,
            outcome: None,
            settings: None,
            usage,
        }
    }

    #[test]
    fn usage_sums_recorded_counts_and_keeps_latest_reports() {
        let limit = RateLimit {
            window_minutes: Some(300),
            used_percent: 40,
            resets_at_unix_ms: None,
        };
        let turns = [
            turn(
                false,
                Some(TurnUsage {
                    input_tokens: Some(100),
                    output_tokens: Some(10),
                    session_cost_micro_usd: Some(5),
                    rate_limits: vec![limit.clone()],
                    ..Default::default()
                }),
            ),
            turn(false, None),
            turn(
                false,
                Some(TurnUsage {
                    input_tokens: Some(200),
                    reasoning_tokens: Some(3),
                    context_tokens: Some(250),
                    context_window: Some(1000),
                    ..Default::default()
                }),
            ),
            turn(true, None),
        ];
        let usage = ConversationUsage::of(&turns);
        assert_eq!(usage.ended_turns, 3);
        assert_eq!(usage.unrecorded_turns, 1);
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens
            ),
            (Some(300), Some(10), None)
        );
        assert_eq!(usage.reasoning_tokens, Some(3));
        assert_eq!(usage.session_tokens, Some(310));
        assert_eq!(
            (usage.context_tokens, usage.context_window),
            (Some(250), Some(1000))
        );
        // The latest turn reported neither; the earlier reports stand.
        assert_eq!(usage.session_cost_micro_usd, Some(5));
        assert_eq!(usage.rate_limits, vec![limit]);
        assert_eq!(ConversationUsage::of(&[]), ConversationUsage::default());
    }
}
