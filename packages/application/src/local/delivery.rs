//! Delivery history behind `crate::transcript_delivery`: stamps changed
//! messages and answers cursor polls and payload reads.
use crate::transcript_delivery::*;
use std::collections::{BTreeSet, HashMap};
use std::hash::{BuildHasher, RandomState};
use std::io::Write;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use velune_conversation::{ConversationSnapshot, ImageSource, Message, MessageBlock, MessageRole};

/// Reasoning and tool output longer than this many UTF-8 bytes are read on demand.
const INLINE_PAYLOAD_LIMIT: usize = 2048;
/// Conversation views a client may show at once, such as several workspaces.
const RETAINED_VIEWS: usize = 8;
/// Delivery keeps full messages for cursor comparison and deferred payload
/// reads. Bound that duplicate cache independently from parked session views.
const RETAINED_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

// Share stamps across delivery owners in this process. A random starting value
// makes reuse negligible when a client reconnects to a restarted host.
static NEXT_PAYLOAD_STAMP: LazyLock<AtomicU64> =
    LazyLock::new(|| AtomicU64::new(RandomState::new().hash_one(())));

#[derive(Default)]
struct ByteCounter(usize);

impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A serialized-size estimate, counted without constructing a temporary JSON
/// buffer. Call only for a new or changed message, never for an unchanged poll.
fn message_size_estimate(message: &Message) -> usize {
    let mut counter = ByteCounter::default();
    if serde_json::to_writer(&mut counter, message).is_ok() {
        counter.0
    } else {
        0
    }
}

/// Delivery history of recently presented views, keyed by runtime context and conversation.
pub(super) struct TranscriptDelivery {
    next_generation: u64,
    views: Vec<View>,
    retained_message_bytes: usize,
}
struct View {
    context_runtime_id: String,
    conversation_id: String,
    generation: u64,
    sequence: u64,
    /// The delivered snapshot without messages, for change detection.
    header: ConversationSnapshot,
    order: Vec<String>,
    messages: HashMap<String, RetainedMessage>,
    message_bytes: usize,
}

struct RetainedMessage {
    changed_at: u64,
    stamp: u64,
    message: Message,
    bytes: usize,
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
            retained_message_bytes: 0,
        }
    }

    /// A complete delivered snapshot; it also becomes the base for later updates.
    pub(super) fn snapshot(
        &mut self,
        snapshot: ConversationSnapshot,
        observed: &BTreeSet<(String, String)>,
    ) -> DeliveredSnapshot {
        let result = self.record(snapshot).deliver(None);
        self.trim(observed, true);
        result
    }

    pub(super) fn update(
        &mut self,
        snapshot: Option<ConversationSnapshot>,
        since: TranscriptCursor,
        observed: &BTreeSet<(String, String)>,
    ) -> SnapshotUpdate {
        let Some(snapshot) = snapshot else {
            return SnapshotUpdate::Absent;
        };
        let view = self.record(snapshot);
        let unchanged = since.generation == view.generation && since.sequence == view.sequence;
        if unchanged {
            self.trim(observed, true);
            return SnapshotUpdate::Unchanged;
        }
        let base = (since.generation == view.generation && since.sequence < view.sequence)
            .then_some(since);
        let result = SnapshotUpdate::Changed {
            snapshot: view.deliver(base),
        };
        self.trim(observed, true);
        result
    }

    /// Latest delivered header; quota reads do not launch a runtime or scan history.
    pub(super) fn quota_record(&self, conversation_id: &str) -> Option<&ConversationSnapshot> {
        self.views
            .iter()
            .find(|view| view.conversation_id == conversation_id)
            .map(|view| &view.header)
    }

    /// Complete payloads of messages as last delivered in a view. Unknown
    /// identifiers are omitted; new content or a rebuilt view uses a different stamp.
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
            .map(|entry| delivered(entry.stamp, &entry.message, usize::MAX))
            .collect()
    }

    fn record(&mut self, mut snapshot: ConversationSnapshot) -> &View {
        let messages = std::mem::take(&mut snapshot.messages);
        let work_notifications: BTreeSet<_> = snapshot
            .work_notification_ids
            .iter()
            .map(String::as_str)
            .collect();
        let position = self.views.iter().position(|view| {
            view.context_runtime_id == snapshot.context_runtime_id
                && view.conversation_id == snapshot.conversation.id
        });
        let mut view = match position {
            Some(index) => {
                let view = self.views.remove(index);
                self.retained_message_bytes = self
                    .retained_message_bytes
                    .saturating_sub(view.message_bytes);
                view
            }
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
                    message_bytes: 0,
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
        let mut message_bytes = 0usize;
        let mut next_stamp = None;
        for mut message in messages {
            // Use the same native classification as transcript grouping. The
            // authoritative role and blocks remain available to history checks.
            if message.role == MessageRole::Client
                && work_notifications.contains(message.id.as_str())
            {
                message.role = MessageRole::Tool;
            }
            let entry = match view.messages.remove(&message.id) {
                Some(entry) if entry.message == message => entry,
                _ => {
                    changed = true;
                    RetainedMessage {
                        changed_at: next,
                        stamp: *next_stamp.get_or_insert_with(|| {
                            NEXT_PAYLOAD_STAMP.fetch_add(1, Ordering::Relaxed)
                        }),
                        bytes: message_size_estimate(&message),
                        message,
                    }
                }
            };
            message_bytes = message_bytes.saturating_add(entry.bytes);
            retained.insert(entry.message.id.clone(), entry);
        }
        if changed {
            view.sequence = next;
        }
        view.header = snapshot;
        view.order = order;
        view.messages = retained;
        view.message_bytes = message_bytes;
        self.retained_message_bytes = self.retained_message_bytes.saturating_add(message_bytes);
        // Most recently presented first. The later trim preserves observed views.
        self.views.insert(0, view);
        &self.views[0]
    }

    /// Reapply soft count and byte targets after a view's observation lease changes.
    pub(super) fn trim_for_observations(&mut self, observed: &BTreeSet<(String, String)>) {
        self.trim_to(RETAINED_MESSAGE_BYTES, observed, false);
    }

    /// Keep the most recent delivery for an immediate payload read. A single
    /// oversized current view may exceed the estimate budget by itself.
    fn trim(&mut self, observed: &BTreeSet<(String, String)>, preserve_current: bool) {
        self.trim_to(RETAINED_MESSAGE_BYTES, observed, preserve_current);
    }

    fn trim_to(
        &mut self,
        budget: usize,
        observed: &BTreeSet<(String, String)>,
        preserve_current: bool,
    ) {
        while self.views.len() > RETAINED_VIEWS || self.retained_message_bytes > budget {
            let candidate = self
                .views
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, view)| {
                    let current = preserve_current && index == 0;
                    let visible = observed.contains(&(
                        view.context_runtime_id.clone(),
                        view.conversation_id.clone(),
                    ));
                    (!current && !visible).then_some(index)
                });
            let Some(index) = candidate else {
                // Visible views may exceed the soft targets; preserve them
                // until a view releases or its observation lease expires. The
                // sole oversized recent view is retained only on the snapshot
                // path, never after its observation has been released.
                break;
            };
            let removed = self.views.remove(index);
            self.retained_message_bytes = self
                .retained_message_bytes
                .saturating_sub(removed.message_bytes);
        }
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
                .filter(|entry| base.is_none() || entry.changed_at > after)
                .map(|entry| delivered(entry.stamp, &entry.message, INLINE_PAYLOAD_LIMIT))
                .collect(),
            revision: header.revision,
            conversation: header.conversation.clone(),
            context_runtime_id: header.context_runtime_id.clone(),
            resource_id: header.resource_id.clone(),
            model_record_key: header.model_record_key.clone(),
            run_state: header.run_state.clone(),
            transcript_turns: header.transcript_turns.clone(),
            usage: ConversationUsage::of(&header.transcript_turns),
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
        MessageBlock::RuntimeFailure {
            failure,
            detail,
            retry,
        } => DeliveredBlock::RuntimeFailure {
            failure: *failure,
            detail: detail.clone(),
            retry: retry.clone(),
        },
        MessageBlock::ContextCompaction { text } => {
            DeliveredBlock::ContextCompaction { text: text.clone() }
        }
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

#[cfg(test)]
mod quota_tests {
    use super::*;
    fn header(runtime: &str) -> ConversationSnapshot {
        serde_json::from_value(serde_json::json!({
            "revision":1,"conversation":{"id":"conversation","title":{"source":"untitled"},"runtimeId":"origin"},
            "contextRuntimeId":runtime,"resourceId":null,"modelRecordKey":null,"runState":"idle",
            "messages":[],"transcriptTurns":[],"transcriptItems":[],"transcriptOutline":[],"transcriptMessageIdentities":[],
            "actions":{"canSend":true,"canCancel":false,"canSwitch":true}
        })).unwrap()
    }
    #[test]
    fn quota_uses_the_latest_delivered_context_of_a_logical_conversation() {
        let mut delivery = TranscriptDelivery::new();
        let observed = BTreeSet::new();
        delivery.snapshot(header("codex"), &observed);
        delivery.snapshot(header("pi"), &observed);
        assert_eq!(
            delivery
                .quota_record("conversation")
                .unwrap()
                .context_runtime_id,
            "pi"
        );
        assert!(delivery.quota_record("missing").is_none());
    }

    fn text_message(id: &str, text: &str) -> Message {
        Message {
            completed: true,
            id: id.into(),
            role: MessageRole::Assistant,
            timestamp_unix_ms: None,
            blocks: vec![MessageBlock::Text { text: text.into() }],
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
        }
    }

    #[test]
    fn rebuilding_a_view_at_the_same_sequence_changes_its_payload_stamp() {
        let tool_message = |output: &str| Message {
            blocks: vec![MessageBlock::Tool {
                tool_id: "tool".into(),
                title: "Generate a report".into(),
                state: velune_conversation::ToolState::Completed,
                output: Some(output.into()),
                conversation_id: None,
            }],
            ..text_message("message", "")
        };
        let mut snapshot = header("runtime");
        let old_output = "a".repeat(INLINE_PAYLOAD_LIMIT + 1);
        let new_output = "b".repeat(old_output.len());
        snapshot.messages = vec![
            tool_message(&old_output),
            text_message("unchanged", "reply"),
        ];
        let mut delivery = TranscriptDelivery::new();
        let observed = BTreeSet::new();
        let message_ids = ["message".into(), "unchanged".into()];
        let first = delivery.snapshot(snapshot.clone(), &observed);
        assert!(matches!(
            first.messages[0].blocks.as_slice(),
            [DeliveredBlock::Tool {
                output: Some(MessagePayload::Deferred { .. }),
                ..
            }]
        ));
        assert!(matches!(
            delivery.update(Some(snapshot.clone()), first.cursor, &observed),
            SnapshotUpdate::Unchanged
        ));
        let first_payload = delivery.payloads("runtime", "conversation", &message_ids);
        assert_eq!(first_payload[0].stamp, first.messages[0].stamp);

        delivery.trim_to(0, &observed, false);
        snapshot.messages[0] = tool_message(&new_output);
        let SnapshotUpdate::Changed { snapshot: rebuilt } =
            delivery.update(Some(snapshot.clone()), first.cursor, &observed)
        else {
            panic!("a cursor from an evicted view must receive a complete replacement");
        };
        assert_eq!(rebuilt.base, None);
        assert_eq!(rebuilt.cursor.sequence, first.cursor.sequence);
        assert_ne!(rebuilt.cursor.generation, first.cursor.generation);
        assert_eq!(rebuilt.messages[0].id, first.messages[0].id);
        assert_eq!(rebuilt.messages[0].blocks, first.messages[0].blocks);
        let latest_payload = delivery.payloads("runtime", "conversation", &message_ids);
        assert_eq!(latest_payload[0].stamp, rebuilt.messages[0].stamp);
        assert_ne!(latest_payload[0].stamp, first.messages[0].stamp);
        assert_ne!(latest_payload[0].blocks, first_payload[0].blocks);

        snapshot.messages[0] = tool_message(&old_output);
        let SnapshotUpdate::Changed { snapshot: delta } =
            delivery.update(Some(snapshot), rebuilt.cursor, &observed)
        else {
            panic!("a subsequent edit must be delivered as an incremental change");
        };
        assert_eq!(delta.base, Some(rebuilt.cursor));
        assert_eq!(delta.message_ids, message_ids);
        assert_eq!(delta.messages.len(), 1);
        assert_eq!(delta.messages[0].id, rebuilt.messages[0].id);
        assert_ne!(delta.messages[0].stamp, rebuilt.messages[0].stamp);
        let final_payload = delivery.payloads("runtime", "conversation", &message_ids);
        assert_eq!(final_payload[0].stamp, delta.messages[0].stamp);
        assert_eq!(final_payload[1].stamp, rebuilt.messages[1].stamp);
    }

    #[test]
    fn work_notifications_are_delivered_as_tools_in_full_updates_and_payloads() {
        let mut snapshot = header("pi");
        let mut parent = text_message("directive", "parent asks child to check");
        parent.role = MessageRole::User;
        let mut returned = text_message("returned", "**child report**");
        returned.role = MessageRole::Client;
        let mut background = text_message("background", "task finished");
        background.role = MessageRole::Client;
        background.blocks = vec![MessageBlock::Notice {
            text: "task finished".into(),
        }];
        let mut extension = text_message("independent", "other extension");
        extension.role = MessageRole::Client;
        snapshot.messages = vec![parent, returned, background, extension];
        snapshot.work_notification_ids = vec!["returned".into()];

        let mut delivery = TranscriptDelivery::new();
        let observed = BTreeSet::new();
        let full = delivery.snapshot(snapshot.clone(), &observed);
        assert_eq!(
            full.message_ids,
            ["directive", "returned", "background", "independent"]
        );
        assert_eq!(
            full.messages
                .iter()
                .map(|m| m.role.clone())
                .collect::<Vec<_>>(),
            [
                MessageRole::User,
                MessageRole::Tool,
                MessageRole::Client,
                MessageRole::Client,
            ]
        );
        assert_eq!(
            full.messages[1].blocks,
            [DeliveredBlock::Text {
                text: "**child report**".into()
            }]
        );

        snapshot.work_notification_ids.push("background".into());
        let SnapshotUpdate::Changed { snapshot: delta } =
            delivery.update(Some(snapshot), full.cursor, &observed)
        else {
            panic!("a change to the delivered role must update the cached message");
        };
        assert_eq!(delta.message_ids, full.message_ids);
        assert_eq!(delta.messages.len(), 1);
        assert_eq!(delta.messages[0].id, "background");
        assert_eq!(delta.messages[0].role, MessageRole::Tool);
        assert_eq!(
            delta.messages[0].blocks,
            [DeliveredBlock::Notice {
                text: "task finished".into()
            }]
        );
        assert_eq!(
            delivery.payloads("pi", "conversation", &["background".into()]),
            delta.messages
        );
    }

    #[test]
    fn delivery_budget_evicts_oldest_complete_messages_but_keeps_newest_payloads() {
        let mut first = header("synthetic");
        first.conversation.id = "first".into();
        first.messages = vec![text_message("message", "old payload")];
        let mut latest = header("synthetic");
        latest.conversation.id = "latest".into();
        latest.messages = vec![text_message("message", "new payload")];
        let mut delivery = TranscriptDelivery::new();
        let observed = BTreeSet::new();
        delivery.snapshot(first, &observed);
        delivery.snapshot(latest, &observed);
        delivery.views[0].message_bytes = 40;
        delivery.views[1].message_bytes = 40;
        delivery.retained_message_bytes = 80;

        delivery.trim_to(64, &observed, true);

        assert_eq!(delivery.views.len(), 1);
        assert_eq!(delivery.views[0].conversation_id, "latest");
        let payloads = delivery.payloads("synthetic", "latest", &["message".into()]);
        assert_eq!(payloads.len(), 1);
        assert_eq!(
            payloads[0].blocks,
            vec![DeliveredBlock::Text {
                text: "new payload".into()
            }]
        );
        assert!(
            delivery
                .payloads("synthetic", "first", &["message".into()])
                .is_empty()
        );
    }

    #[test]
    fn visible_delivery_views_exceed_soft_count_and_byte_targets_until_released() {
        let mut first = header("runtime");
        first.conversation.id = "first".into();
        let mut second = header("runtime");
        second.conversation.id = "second".into();
        let visible = BTreeSet::from([
            ("runtime".into(), "first".into()),
            ("runtime".into(), "second".into()),
        ]);
        let mut delivery = TranscriptDelivery::new();
        delivery.snapshot(first, &visible);
        delivery.snapshot(second, &visible);
        delivery.views[0].message_bytes = 40;
        delivery.views[1].message_bytes = 40;
        delivery.retained_message_bytes = 80;

        delivery.trim_to(64, &visible, false);
        assert_eq!(delivery.views.len(), 2);
        assert_eq!(delivery.retained_message_bytes, 80);

        let remaining = BTreeSet::from([("runtime".into(), "second".into())]);
        delivery.trim_to(64, &remaining, false);
        assert_eq!(delivery.views.len(), 1);
        assert_eq!(delivery.views[0].conversation_id, "second");
        assert_eq!(delivery.retained_message_bytes, 40);

        let mut all_visible = BTreeSet::new();
        for index in 0..RETAINED_VIEWS + 1 {
            all_visible.insert(("runtime".into(), format!("visible-{index}")));
        }
        let mut many = TranscriptDelivery::new();
        for index in 0..RETAINED_VIEWS + 1 {
            let mut snapshot = header("runtime");
            snapshot.conversation.id = format!("visible-{index}");
            many.snapshot(snapshot, &all_visible);
        }
        assert_eq!(many.views.len(), RETAINED_VIEWS + 1);
    }

    #[test]
    fn last_oversized_view_is_reclaimed_on_release_but_current_payload_is_retained() {
        let mut snapshot = header("runtime");
        snapshot.conversation.id = "large".into();
        let observed = BTreeSet::from([("runtime".into(), "large".into())]);
        let mut delivery = TranscriptDelivery::new();
        delivery.snapshot(snapshot.clone(), &observed);
        delivery.views[0].message_bytes = RETAINED_MESSAGE_BYTES + 1;
        delivery.retained_message_bytes = RETAINED_MESSAGE_BYTES + 1;

        delivery.trim_for_observations(&observed);
        assert_eq!(
            delivery.views.len(),
            1,
            "visible oversized view is protected"
        );
        delivery.trim_for_observations(&BTreeSet::new());
        assert!(
            delivery.views.is_empty(),
            "released oversized view is reclaimed"
        );

        delivery.snapshot(snapshot, &BTreeSet::new());
        delivery.views[0].message_bytes = RETAINED_MESSAGE_BYTES + 1;
        delivery.retained_message_bytes = RETAINED_MESSAGE_BYTES + 1;
        delivery.trim(&BTreeSet::new(), true);
        assert_eq!(
            delivery.views.len(),
            1,
            "a just-read payload stays available"
        );
    }
}
