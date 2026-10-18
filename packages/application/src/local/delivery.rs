//! Delivery history behind `crate::transcript_delivery`: stamps changed
//! messages and answers cursor polls and payload reads.
use crate::transcript_delivery::*;
use std::collections::HashMap;
use velune_conversation::{ConversationSnapshot, ImageSource, Message, MessageBlock, MessageRole};

/// Reasoning and tool output longer than this many UTF-8 bytes are read on demand.
const INLINE_PAYLOAD_LIMIT: usize = 2048;
/// Conversation views a client may show at once, such as several workspaces.
const RETAINED_VIEWS: usize = 8;

/// Delivery history of recently presented views, keyed by runtime context and conversation.
pub(super) struct TranscriptDelivery {
    next_generation: u64,
    views: Vec<View>,
}
struct View {
    context_runtime_id: String,
    conversation_id: String,
    generation: u64,
    sequence: u64,
    /// The delivered snapshot without messages, for change detection.
    header: ConversationSnapshot,
    order: Vec<String>,
    messages: HashMap<String, (u64, Message)>,
}

impl TranscriptDelivery {
    pub(super) fn new() -> Self {
        // A client may outlive one core instance; start generations from the
        // clock so a cursor from an earlier instance cannot match a new view.
        let next_generation = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |elapsed| elapsed.as_nanos() as u64);
        Self {
            next_generation,
            views: Vec::new(),
        }
    }

    /// A complete delivered snapshot; it also becomes the base for later updates.
    pub(super) fn snapshot(&mut self, snapshot: ConversationSnapshot) -> DeliveredSnapshot {
        self.record(snapshot).deliver(None)
    }

    pub(super) fn update(
        &mut self,
        snapshot: Option<ConversationSnapshot>,
        since: TranscriptCursor,
    ) -> SnapshotUpdate {
        let Some(snapshot) = snapshot else {
            return SnapshotUpdate::Absent;
        };
        let view = self.record(snapshot);
        if since.generation == view.generation && since.sequence == view.sequence {
            return SnapshotUpdate::Unchanged;
        }
        let base = (since.generation == view.generation && since.sequence < view.sequence)
            .then_some(since);
        SnapshotUpdate::Changed {
            snapshot: Box::new(view.deliver(base)),
        }
    }

    /// Complete payloads of messages as last delivered in a view. Unknown
    /// identifiers are omitted; a newer delivery carries a newer stamp.
    pub(super) fn payloads(
        &self,
        context_runtime_id: &str,
        conversation_id: &str,
        message_ids: &[String],
    ) -> Vec<DeliveredMessage> {
        let Some(view) = self.views.iter().find(|view| {
            view.context_runtime_id == context_runtime_id && view.conversation_id == conversation_id
        }) else {
            return Vec::new();
        };
        message_ids
            .iter()
            .filter_map(|id| view.messages.get(id))
            .map(|(stamp, message)| delivered(*stamp, message, usize::MAX))
            .collect()
    }

    fn record(&mut self, mut snapshot: ConversationSnapshot) -> &View {
        let messages = std::mem::take(&mut snapshot.messages);
        let position = self.views.iter().position(|view| {
            view.context_runtime_id == snapshot.context_runtime_id
                && view.conversation_id == snapshot.conversation.id
        });
        let mut view = match position {
            Some(index) => self.views.remove(index),
            None => {
                self.next_generation += 1;
                View {
                    context_runtime_id: snapshot.context_runtime_id.clone(),
                    conversation_id: snapshot.conversation.id.clone(),
                    generation: self.next_generation,
                    sequence: 0,
                    header: snapshot.clone(),
                    order: Vec::new(),
                    messages: HashMap::new(),
                }
            }
        };
        let next = view.sequence + 1;
        let order = messages
            .iter()
            .map(|message| message.id.clone())
            .collect::<Vec<_>>();
        let mut changed = view.sequence == 0 || view.header != snapshot || view.order != order;
        let mut retained = HashMap::with_capacity(messages.len());
        for message in messages {
            let entry = match view.messages.remove(&message.id) {
                Some((stamp, previous)) if previous == message => (stamp, previous),
                _ => {
                    changed = true;
                    (next, message)
                }
            };
            retained.insert(entry.1.id.clone(), entry);
        }
        if changed {
            view.sequence = next;
        }
        view.header = snapshot;
        view.order = order;
        view.messages = retained;
        // Most recently presented first; the oldest view is forgotten.
        self.views.insert(0, view);
        self.views.truncate(RETAINED_VIEWS);
        &self.views[0]
    }
}

impl View {
    fn deliver(&self, base: Option<TranscriptCursor>) -> DeliveredSnapshot {
        let after = base.map_or(0, |cursor| cursor.sequence);
        let header = &self.header;
        DeliveredSnapshot {
            cursor: TranscriptCursor {
                generation: self.generation,
                sequence: self.sequence,
            },
            base,
            message_ids: self.order.clone(),
            messages: self
                .order
                .iter()
                .map(|id| &self.messages[id])
                .filter(|(stamp, _)| base.is_none() || *stamp > after)
                .map(|(stamp, message)| delivered(*stamp, message, INLINE_PAYLOAD_LIMIT))
                .collect(),
            revision: header.revision,
            conversation: header.conversation.clone(),
            context_runtime_id: header.context_runtime_id.clone(),
            resource_id: header.resource_id.clone(),
            model_record_key: header.model_record_key.clone(),
            run_state: header.run_state.clone(),
            transcript_turns: header.transcript_turns.clone(),
            transcript_items: header.transcript_items.clone(),
            transcript_outline: header.transcript_outline.clone(),
            transcript_message_identities: header.transcript_message_identities.clone(),
            message_identity_confirmations: header.message_identity_confirmations.clone(),
            pending_interactions: header.pending_interactions.clone(),
            actions: header.actions.clone(),
            follow_up: header.follow_up.clone(),
            sub_agents: header.sub_agents.clone(),
            collaborators: header.collaborators.clone(),
        }
    }
}

fn delivered(stamp: u64, message: &Message, inline_limit: usize) -> DeliveredMessage {
    let payload = |text: &str| {
        if text.len() <= inline_limit {
            MessagePayload::Inline { text: text.into() }
        } else {
            MessagePayload::Deferred {
                bytes: text.len() as u64,
            }
        }
    };
    DeliveredMessage {
        stamp,
        completed: message.completed,
        id: message.id.clone(),
        role: message.role.clone(),
        timestamp_unix_ms: message.timestamp_unix_ms,
        blocks: message
            .blocks
            .iter()
            .flat_map(|block| match block {
                // The paths Velune appended to a user's text are shown as files.
                // So is the line naming the agent that sent the text.
                MessageBlock::Text { text } if message.role == MessageRole::User => {
                    let (text, sender) = match velune_conversation::agent_message::split(text) {
                        Some((text, sender)) => (
                            text,
                            Some(DeliveredBlock::AgentSender {
                                conversation_id: sender.conversation_id,
                                runtime_name: sender.runtime_name,
                                title: sender.title,
                            }),
                        ),
                        None => (text.as_str(), None),
                    };
                    let (text, files) = velune_conversation::attachment_list::split(text);
                    sender
                        .into_iter()
                        .chain(
                            (!text.is_empty()).then(|| DeliveredBlock::Text { text: text.into() }),
                        )
                        .chain(
                            files
                                .into_iter()
                                .map(|path| DeliveredBlock::File { path: path.into() }),
                        )
                        .collect::<Vec<_>>()
                }
                block => delivered_block(block, &payload).into_iter().collect(),
            })
            .collect(),
    }
}

/// None for reasoning without a summary, which has nothing to show.
fn delivered_block(
    block: &MessageBlock,
    payload: &impl Fn(&str) -> MessagePayload,
) -> Option<DeliveredBlock> {
    Some(match block {
        MessageBlock::Text { text } => DeliveredBlock::Text { text: text.clone() },
        MessageBlock::Notice { text } => DeliveredBlock::Notice { text: text.clone() },
        MessageBlock::Reasoning { text } => {
            let (summary, text) = velune_conversation::reasoning::summary(text)?;
            DeliveredBlock::Reasoning {
                summary: summary.into(),
                text: payload(text),
            }
        }
        MessageBlock::Tool {
            tool_id,
            title,
            state,
            output,
            conversation_id,
        } => DeliveredBlock::Tool {
            tool_id: tool_id.clone(),
            title: title.clone(),
            state: state.clone(),
            output: output.as_deref().map(payload),
            linked_conversation_id: conversation_id
                .clone()
                .or_else(|| output.as_deref().and_then(collaboration_conversation)),
        },
        MessageBlock::Image { media_type, source } => DeliveredBlock::Image {
            media_type: media_type.clone(),
            source: match source {
                ImageSource::Path { path } => DeliveredImageSource::Path { path: path.clone() },
                ImageSource::Data { data } => DeliveredImageSource::Data {
                    data: payload(data),
                },
            },
        },
    })
}

/// The conversation a collaboration tool reported on: its result is a status
/// object naming the conversation and its state.
pub(super) fn collaboration_conversation(output: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(output.trim()).ok()?;
    value["state"].as_str()?;
    value["runtime_id"].as_str()?;
    value["conversation_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}
