//! Ordered, disposable message-list projection. Native snapshots remain authoritative.
use crate::conversation::{
    ConversationSnapshot, ExecutionKind, Executions, Message, MessageBlock,
    MessageIdentityConfirmation, MessageRole, RunState, TranscriptItem, TranscriptMessageIdentity,
    TranscriptOutlineEntry, TranscriptTurn, TurnOutcome, TurnSettings, TurnUsage,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

/// Forward consumption includes replacement: native branching and compaction are not append-only.
pub enum TranscriptEvent<'a> {
    /// Consume canonical content and its native presentation semantics together.
    ReplaceSnapshot(&'a ConversationSnapshot),
    ConfirmIdentities(&'a [MessageIdentityConfirmation]),
    Replace {
        messages: &'a [Message],
        executions: &'a Executions,
    },
    ExecutionState(RunState),
}

struct MessageAnchor {
    id: String,
    role: MessageRole,
    standalone: bool,
    work_notification: bool,
    compaction: bool,
    outcome: Option<TurnOutcome>,
    /// Index of the first block of the trailing visible text, which is the
    /// agent's reply; `None` when an assistant message does not end in text.
    reply_start: Option<usize>,
    /// What the native record says about the turn a user message starts.
    settings: Option<TurnSettings>,
    /// What the native record says the turn used.
    usage: Option<TurnUsage>,
}

#[derive(Default)]
pub struct TranscriptProjection {
    messages: Vec<MessageAnchor>,
    work_notification_ids: BTreeSet<String>,
    /// Closing replies recovered from native history before user tombstones.
    /// These delimit surviving user ranges without inventing another user.
    retained_reply_ids: BTreeSet<String>,
    running: bool,
    /// The native record shows the last turn unfinished outside Velune. It is
    /// presented as running but not timed: no execution was observed.
    elsewhere: bool,
    active: Option<(String, Instant)>,
    durations: BTreeMap<String, u64>,
    presentation_ids: BTreeMap<String, String>,
    /// What Velune asked for, by the user message of the turn it sent. Like
    /// durations, only for turns observed here.
    requested: BTreeMap<String, TurnSettings>,
    /// A request whose user message has not appeared yet, with the number of
    /// user messages before it.
    pending_request: Option<(TurnSettings, usize)>,
}
impl TranscriptProjection {
    pub fn retain_reply_boundaries(&mut self, ids: &[String]) {
        self.retained_reply_ids = ids.iter().cloned().collect();
    }

    pub fn apply(&mut self, event: TranscriptEvent<'_>) {
        match event {
            TranscriptEvent::ReplaceSnapshot(snapshot) => {
                self.work_notification_ids =
                    snapshot.work_notification_ids.iter().cloned().collect();
                self.apply(TranscriptEvent::Replace {
                    messages: &snapshot.messages,
                    executions: &snapshot.executions,
                });
            }
            TranscriptEvent::ConfirmIdentities(confirmations) => {
                let identities: BTreeMap<_, _> = confirmations
                    .iter()
                    .map(|confirmation| {
                        (
                            confirmation.previous_id.as_str(),
                            confirmation.current_id.as_str(),
                        )
                    })
                    .collect();
                for message in &mut self.messages {
                    if let Some(current) = identities.get(message.id.as_str()) {
                        message.id = (*current).into();
                    }
                }
                if let Some((id, _)) = &mut self.active
                    && let Some(current) = identities.get(id.as_str())
                {
                    *id = (*current).into();
                }
                for confirmation in confirmations {
                    if self.work_notification_ids.remove(&confirmation.previous_id) {
                        self.work_notification_ids
                            .insert(confirmation.current_id.clone());
                    }
                    if self.retained_reply_ids.remove(&confirmation.previous_id) {
                        self.retained_reply_ids
                            .insert(confirmation.current_id.clone());
                    }
                    if let Some(id) = self.presentation_ids.remove(&confirmation.previous_id) {
                        self.presentation_ids
                            .insert(confirmation.current_id.clone(), id);
                    }
                    if let Some(duration) = self.durations.remove(&confirmation.previous_id) {
                        self.durations
                            .insert(confirmation.current_id.clone(), duration);
                    }
                    if let Some(settings) = self.requested.remove(&confirmation.previous_id) {
                        self.requested
                            .insert(confirmation.current_id.clone(), settings);
                    }
                }
            }
            TranscriptEvent::Replace {
                messages,
                executions,
            } => {
                let membership: BTreeMap<_, _> = executions
                    .records
                    .iter()
                    .flat_map(|record| {
                        record
                            .message_ids
                            .iter()
                            .map(move |id| (id.as_str(), (record.kind, record.outcome)))
                    })
                    .collect();
                self.messages = messages
                    .iter()
                    .map(|message| MessageAnchor {
                        id: message.id.clone(),
                        role: message.role.clone(),
                        compaction: message
                            .blocks
                            .iter()
                            .any(|block| matches!(block, MessageBlock::ContextCompaction { .. })),
                        work_notification: self.work_notification_ids.contains(&message.id),
                        standalone: (message.role == MessageRole::Client
                            && !self.work_notification_ids.contains(&message.id))
                            || membership
                                .get(message.id.as_str())
                                .is_some_and(|(kind, _)| {
                                    *kind == ExecutionKind::ContextMaintenance
                                })
                            || (message.blocks.iter().any(|block| {
                                matches!(block, MessageBlock::ContextCompaction { .. })
                            }) && !membership.contains_key(message.id.as_str())),
                        outcome: membership
                            .get(message.id.as_str())
                            .and_then(|(_, outcome)| *outcome),
                        reply_start: (message.role == MessageRole::Assistant)
                            .then(|| trailing_text_start(&message.blocks))
                            .flatten(),
                        settings: message.turn_settings.clone(),
                        usage: message.turn_usage.clone(),
                    })
                    .collect();
                let identities: BTreeSet<_> = self
                    .messages
                    .iter()
                    .map(|message| message.id.as_str())
                    .collect();
                self.presentation_ids
                    .retain(|id, _| identities.contains(id.as_str()));
                for message in messages {
                    self.presentation_ids
                        .entry(message.id.clone())
                        .or_insert_with(|| format!("message:{}", message.id));
                }
                self.durations
                    .retain(|id, _| identities.contains(id.as_str()));
                self.requested
                    .retain(|id, _| identities.contains(id.as_str()));
                // An authoritative replacement can remove the in-progress branch.
                if self
                    .active
                    .as_ref()
                    .is_some_and(|(id, _)| !identities.contains(id.as_str()))
                {
                    self.active = None;
                }
                self.bind_request();
            }
            TranscriptEvent::ExecutionState(state) => {
                let running = matches!(state, RunState::Running | RunState::Stopping);
                let user = self
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == MessageRole::User)
                    .map(|m| m.id.clone());
                if running
                    && (!self.running || self.active.as_ref().map(|(id, _)| id) != user.as_ref())
                {
                    if let Some((id, started)) = self.active.take() {
                        self.durations.insert(id, elapsed_ms(started));
                    }
                    self.active = user.map(|id| (id, Instant::now()));
                } else if !running && let Some((id, started)) = self.active.take() {
                    self.durations.insert(id, elapsed_ms(started));
                }
                self.running = running;
                self.elsewhere = matches!(state, RunState::Elsewhere);
            }
        }
    }

    /// Velune's request for the turn it is about to send. It belongs to the
    /// next user message, whenever the runtime shows it.
    pub fn request(&mut self, settings: TurnSettings) {
        self.pending_request = Some((settings, self.user_messages().count()));
        self.bind_request();
    }

    /// The turn was not sent after all.
    pub fn withdraw_request(&mut self) {
        self.pending_request = None;
    }

    /// What Velune asked for, by user message, for turns it sent.
    pub fn requested(&self) -> &BTreeMap<String, TurnSettings> {
        &self.requested
    }

    /// What Velune recorded asking for a turn it sent before this
    /// projection; a request made here stays.
    pub fn recall_request(&mut self, user_message_id: &str, settings: TurnSettings) {
        self.requested
            .entry(user_message_id.to_owned())
            .or_insert(settings);
    }

    fn user_messages(&self) -> impl Iterator<Item = &MessageAnchor> {
        self.messages
            .iter()
            .filter(|message| message.role == MessageRole::User)
    }

    fn bind_request(&mut self) {
        let Some((_, before)) = &self.pending_request else {
            return;
        };
        let Some(id) = self
            .user_messages()
            .nth(*before)
            .map(|user| user.id.clone())
        else {
            return;
        };
        if let Some((settings, _)) = self.pending_request.take() {
            self.requested.insert(id, settings);
        }
    }

    /// Native record first, except the model: Velune's name for a model it
    /// chose reads better than the runtime's ID for it.
    fn settings(&self, user: &MessageAnchor) -> Option<TurnSettings> {
        let requested = self.requested.get(&user.id);
        let native = user.settings.as_ref();
        if requested.is_none() && native.is_none() {
            return None;
        }
        let pick =
            |native: Option<&String>, requested: Option<&String>| native.or(requested).cloned();
        Some(TurnSettings {
            runtime_id: pick(
                native.and_then(|n| n.runtime_id.as_ref()),
                requested.and_then(|r| r.runtime_id.as_ref()),
            ),
            model: pick(
                requested.and_then(|r| r.model.as_ref()),
                native.and_then(|n| n.model.as_ref()),
            ),
            effort: pick(
                native.and_then(|n| n.effort.as_ref()),
                requested.and_then(|r| r.effort.as_ref()),
            ),
            permission_mode: pick(
                native.and_then(|n| n.permission_mode.as_ref()),
                requested.and_then(|r| r.permission_mode.as_ref()),
            ),
            model_selection: requested
                .and_then(|r| r.model_selection.clone())
                .or_else(|| native.and_then(|n| n.model_selection.clone())),
        })
    }

    /// Fold the complete user range. Message completion is not an agent turn
    /// boundary: text progress can be followed by more tools and another answer.
    pub fn turns(&self) -> Vec<TranscriptTurn> {
        let starts: Vec<_> = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, message)| message.role == MessageRole::User)
            .map(|(i, _)| i)
            .collect();
        starts
            .iter()
            .enumerate()
            .map(|(ordinal, start)| {
                let next_user = starts
                    .get(ordinal + 1)
                    .copied()
                    .unwrap_or(self.messages.len());
                // A deleted prompt must not merge its preceding closing reply
                // into a later turn's work. Close at the first native retained
                // reply; orphaned messages following it stay individually
                // accessible in their original order until the next real user.
                let end = self.messages[start + 1..next_user]
                    .iter()
                    .position(|message| {
                        self.retained_reply_ids.contains(&message.id)
                            && message.reply_start.is_some()
                    })
                    .map_or(next_user, |offset| start + 1 + offset + 1);
                // An independent compaction ends the available ownership
                // evidence. Later notifications must not attach backwards.
                let mut independent_compaction = false;
                let responses: Vec<_> = self.messages[start + 1..end]
                    .iter()
                    .filter(|message| {
                        independent_compaction |= message.compaction && message.standalone;
                        !message.standalone
                            && !(message.work_notification && independent_compaction)
                    })
                    .collect();
                // The reply is the text at the end of the closing assistant
                // message. Everything before it, including that message's own
                // reasoning and tool blocks, is work; a turn that does not end
                // in text folds fully.
                let closing_index = responses
                    .iter()
                    .rposition(|message| {
                        message.role != MessageRole::System && !message.work_notification
                    })
                    .filter(|index| responses[*index].reply_start.is_some());
                let visible = closing_index.map(|index| responses[index]);
                let reply_start = visible.and_then(|message| message.reply_start).unwrap_or(0);
                let user = &self.messages[*start];
                let duration_ms = self
                    .active
                    .as_ref()
                    .filter(|(id, _)| id == &user.id)
                    .map(|(_, started)| elapsed_ms(*started))
                    .or_else(|| self.durations.get(&user.id).copied());
                TranscriptTurn {
                    id: format!("turn:{}", self.presentation_ids[&user.id]),
                    user_message_id: user.id.clone(),
                    work_message_ids: responses
                        .iter()
                        .enumerate()
                        .filter(|(index, message)| {
                            (reply_start > 0 || visible.is_none_or(|reply| reply.id != message.id))
                                // Truly late notifications remain standalone.
                                && !(message.work_notification
                                    && closing_index.is_some_and(|closing| *index > closing))
                                // Maintenance after a closing reply stays after
                                // it, even if an older adapter attached it to
                                // the preceding user execution.
                                && !(message.compaction
                                    && closing_index.is_some_and(|closing| *index > closing))
                        })
                        .map(|(_, message)| message.id.clone())
                        .collect(),
                    last_message_id: visible.map(|message| message.id.clone()),
                    reply_start_block: u32::try_from(reply_start).unwrap_or(u32::MAX),
                    duration_ms,
                    is_running: ordinal + 1 == starts.len() && (self.running || self.elsewhere),
                    can_fork: false,
                    outcome: user.outcome,
                    settings: self.settings(user),
                    usage: user.usage.clone(),
                }
            })
            .collect()
    }

    pub fn items(&self) -> Vec<TranscriptItem> {
        let turns = self.turns();
        let by_user: BTreeMap<_, _> = turns
            .iter()
            .map(|turn| (turn.user_message_id.as_str(), turn))
            .collect();
        let closing: BTreeMap<_, _> = turns
            .iter()
            .filter_map(|turn| {
                turn.last_message_id
                    .as_deref()
                    .map(|id| (id, turn.reply_start_block > 0))
            })
            .collect();
        let grouped: BTreeSet<_> = turns
            .iter()
            .flat_map(|turn| {
                turn.work_message_ids
                    .iter()
                    .chain(turn.last_message_id.iter())
            })
            .collect();
        let item = |id: &String, reply: bool| TranscriptItem::Message {
            id: format!(
                "{}{}",
                if reply { "reply:" } else { "" },
                self.presentation_ids[id]
            ),
            message_id: id.clone(),
        };
        let mut items = Vec::new();
        for message in &self.messages {
            if let Some(turn) = by_user.get(message.id.as_str()) {
                items.push(item(&message.id, false));
                if !turn.work_message_ids.is_empty() {
                    items.push(TranscriptItem::Work {
                        turn_id: turn.id.clone(),
                    });
                }
            } else if let Some(mixed) = closing.get(message.id.as_str()) {
                // A closing reply stays at its native position. Emitting it
                // with the user anchor moves intervening standalone notices
                // after the answer, despite their earlier source position.
                items.push(item(&message.id, *mixed));
            } else if !grouped.contains(&message.id) {
                items.push(item(&message.id, false));
            }
        }
        items
    }

    pub fn message_identities(&self) -> Vec<TranscriptMessageIdentity> {
        self.messages
            .iter()
            .map(|message| TranscriptMessageIdentity {
                id: self.presentation_ids[&message.id].clone(),
                message_id: message.id.clone(),
            })
            .collect()
    }

    pub fn outline(&self) -> Vec<TranscriptOutlineEntry> {
        self.messages
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .map(|message| TranscriptOutlineEntry {
                id: self.presentation_ids[&message.id].clone(),
                message_id: message.id.clone(),
            })
            .collect()
    }
}
fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// First block of the run of non-blank text that ends the message.
fn trailing_text_start(blocks: &[MessageBlock]) -> Option<usize> {
    let start = blocks
        .iter()
        .rposition(|block| !matches!(block, MessageBlock::Text { .. }))
        .map_or(0, |index| index + 1);
    blocks[start..]
        .iter()
        .any(|block| matches!(block, MessageBlock::Text { text } if !text.trim().is_empty()))
        .then_some(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compaction_remains_visible_without_hiding_the_reply() {
        let messages: Vec<Message> = serde_json::from_value(serde_json::json!([
            {"id":"user","role":"user","completed":true,"blocks":[{"kind":"text","text":"hello"}]},
            {"id":"answer","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"done"}]},
            {"id":"compact","role":"system","completed":false,"blocks":[{"kind":"contextCompaction","text":"正在压缩上下文"}]}
        ])).unwrap();
        let mut executions = Executions::default();
        executions.begin(ExecutionKind::UserTurn, false);
        for message in &messages[..2] {
            executions.attach(&message.id);
        }
        executions.finish(Some(TurnOutcome::Completed));
        executions.begin(ExecutionKind::ContextMaintenance, false);
        executions.attach("compact");
        let mut projection = TranscriptProjection::default();
        projection.apply(TranscriptEvent::Replace {
            messages: &messages,
            executions: &executions,
        });
        let turn = &projection.turns()[0];
        assert_eq!(turn.last_message_id.as_deref(), Some("answer"));
        assert!(turn.work_message_ids.is_empty());
        assert!(
            matches!(projection.items().last(), Some(TranscriptItem::Message { message_id, .. }) if message_id == "compact")
        );
    }

    #[test]
    fn client_notifications_stay_outside_work_and_preserve_the_final_reply() {
        let messages: Vec<Message> = serde_json::from_value(serde_json::json!([
            {"id":"user","role":"user","completed":true,"blocks":[{"kind":"text","text":"hello"}]},
            {"id":"thinking","role":"assistant","completed":true,"blocks":[{"kind":"reasoning","text":"checking"}]},
            {"id":"answer","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"done"}]},
            {"id":"notice","role":"client","completed":true,"blocks":[{"kind":"text","text":"background task complete"}]}
        ])).unwrap();
        let mut executions = Executions::default();
        executions.begin(ExecutionKind::UserTurn, false);
        for message in &messages {
            executions.attach(&message.id);
        }
        executions.finish(Some(TurnOutcome::Completed));
        let mut projection = TranscriptProjection::default();
        projection.apply(TranscriptEvent::Replace {
            messages: &messages,
            executions: &executions,
        });
        let turns = projection.turns();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].last_message_id.as_deref(), Some("answer"));
        assert_eq!(turns[0].work_message_ids, ["thinking"]);
        assert!(
            matches!(projection.items().last(), Some(TranscriptItem::Message { message_id, .. }) if message_id == "notice")
        );
        assert_eq!(projection.outline().len(), 1);
    }

    #[test]
    fn standalone_notifications_keep_their_native_position_before_closing_replies() {
        for (mixed, notice) in [
            (
                false,
                serde_json::json!({"id":"notice","role":"client","completed":true,"blocks":[{"kind":"text","text":"background task complete"}]}),
            ),
            (
                true,
                serde_json::json!({"id":"notice","role":"client","completed":true,"blocks":[{"kind":"text","text":"background task complete"}]}),
            ),
            (
                false,
                serde_json::json!({"id":"notice","role":"system","completed":false,"blocks":[{"kind":"contextCompaction","text":"compacting"}]}),
            ),
        ] {
            let blocks = if mixed {
                serde_json::json!([{"kind":"reasoning","text":"finishing"},{"kind":"text","text":"done"}])
            } else {
                serde_json::json!([{"kind":"text","text":"done"}])
            };
            let messages: Vec<Message> = serde_json::from_value(serde_json::json!([
                {"id":"user","role":"user","completed":true,"blocks":[{"kind":"text","text":"run background work"}]},
                {"id":"progress","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"checking"}]},
                notice,
                {"id":"notice2","role":"client","completed":true,"blocks":[{"kind":"text","text":"another background task complete"}]},
                {"id":"closing","role":"assistant","completed":true,"blocks":blocks},
                {"id":"late","role":"client","completed":true,"blocks":[{"kind":"text","text":"arrived after the reply"}]}
            ])).unwrap();
            let mut projection = TranscriptProjection::default();
            projection.apply(TranscriptEvent::Replace {
                messages: &messages,
                executions: &Executions::default(),
            });
            let turn = projection.turns().remove(0);
            assert_eq!(turn.last_message_id.as_deref(), Some("closing"));
            assert_eq!(
                turn.work_message_ids,
                if mixed {
                    vec!["progress", "closing"]
                } else {
                    vec!["progress"]
                }
            );
            let identities: BTreeMap<_, _> = projection
                .message_identities()
                .into_iter()
                .map(|identity| (identity.message_id, identity.id))
                .collect();
            let items = projection.items();
            assert!(matches!(&items[1], TranscriptItem::Work { turn_id } if turn_id == &turn.id));
            for item in &items {
                if let TranscriptItem::Message { id, message_id } = item {
                    let prefix = if mixed && message_id == "closing" {
                        "reply:"
                    } else {
                        ""
                    };
                    assert_eq!(id, &format!("{prefix}{}", identities[message_id]));
                }
            }
            let order: Vec<_> = projection
                .items()
                .into_iter()
                .map(|item| match item {
                    TranscriptItem::Message { message_id, .. } => message_id,
                    TranscriptItem::Work { .. } => "work".into(),
                })
                .collect();
            assert_eq!(
                order,
                ["user", "work", "notice", "notice2", "closing", "late"]
            );
        }
    }

    #[test]
    fn standalone_notifications_do_not_split_or_reorder_pure_work() {
        let messages: Vec<Message> = serde_json::from_value(serde_json::json!([
            {"id":"user","role":"user","completed":true,"blocks":[{"kind":"text","text":"run background work"}]},
            {"id":"before","role":"assistant","completed":true,"blocks":[{"kind":"reasoning","text":"checking"}]},
            {"id":"notice","role":"client","completed":true,"blocks":[{"kind":"text","text":"background task complete"}]},
            {"id":"after","role":"assistant","completed":true,"blocks":[{"kind":"reasoning","text":"still checking"}]},
            {"id":"notice2","role":"client","completed":true,"blocks":[{"kind":"text","text":"another background task complete"}]}
        ])).unwrap();
        let mut projection = TranscriptProjection::default();
        projection.apply(TranscriptEvent::Replace {
            messages: &messages,
            executions: &Executions::default(),
        });
        let turn = projection.turns().remove(0);
        assert_eq!(turn.last_message_id, None);
        assert_eq!(turn.work_message_ids, ["before", "after"]);
        assert_eq!(
            projection
                .items()
                .iter()
                .filter(|item| matches!(item, TranscriptItem::Work { .. }))
                .count(),
            1
        );
        let order: Vec<_> = projection
            .items()
            .into_iter()
            .map(|item| match item {
                TranscriptItem::Message { message_id, .. } => message_id,
                TranscriptItem::Work { turn_id } => {
                    assert_eq!(turn_id, turn.id);
                    "work".into()
                }
            })
            .collect();
        assert_eq!(order, ["user", "work", "notice", "notice2"]);
    }

    #[test]
    fn completed_errors_join_work_but_compaction_after_reply_keeps_its_position() {
        let messages: Vec<Message> = serde_json::from_value(serde_json::json!([
            {"id":"user","role":"user","completed":true,"blocks":[{"kind":"text","text":"hello"}]},
            {"id":"answer","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"done"}]},
            {"id":"error","role":"system","completed":true,"blocks":[{"kind":"notice","text":"serverOverloaded"}]},
            {"id":"compact","role":"system","completed":true,"blocks":[{"kind":"contextCompaction","text":"上下文已压缩"}]}
        ])).unwrap();
        let mut executions = Executions::default();
        executions.begin(ExecutionKind::UserTurn, false);
        for message in &messages {
            executions.attach(&message.id);
        }
        let mut projection = TranscriptProjection::default();
        projection.apply(TranscriptEvent::Replace {
            messages: &messages,
            executions: &executions,
        });
        executions.finish(Some(TurnOutcome::Cancelled));
        projection.apply(TranscriptEvent::Replace {
            messages: &messages,
            executions: &executions,
        });
        let turn = &projection.turns()[0];
        assert_eq!(turn.last_message_id.as_deref(), Some("answer"));
        assert_eq!(turn.work_message_ids, ["error"]);
        assert!(matches!(
            &projection.items()[1],
            TranscriptItem::Work { .. }
        ));
        assert!(
            matches!(&projection.items()[2], TranscriptItem::Message { message_id, .. } if message_id == "answer")
        );
        assert!(
            matches!(&projection.items()[3], TranscriptItem::Message { message_id, .. } if message_id == "compact")
        );
        projection.apply(TranscriptEvent::Replace {
            messages: &messages,
            executions: &executions,
        });
        assert_eq!(projection.turns()[0].outcome, Some(TurnOutcome::Cancelled));
        executions.begin(ExecutionKind::UserTurn, true);
        executions.finish(Some(TurnOutcome::Completed));
        projection.apply(TranscriptEvent::Replace {
            messages: &messages,
            executions: &executions,
        });
        assert_eq!(projection.turns()[0].outcome, Some(TurnOutcome::Completed));
        let mut continued = messages;
        continued.push(serde_json::from_value(serde_json::json!({"id":"more-work","role":"assistant","completed":false,"blocks":[{"kind":"tool","toolID":"call","title":"continuing","state":"running","output":null}]})).unwrap());
        projection.apply(TranscriptEvent::Replace {
            messages: &continued,
            executions: &executions,
        });
        assert!(projection.turns()[0].last_message_id.is_none());
    }
    #[test]
    fn steer_segments_share_result_and_manual_maintenance_never_joins_previous_work() {
        let messages: Vec<Message> = serde_json::from_value(serde_json::json!([
            {"id":"u1","role":"user","completed":true,"blocks":[]},
            {"id":"work","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"working"}]},
            {"id":"u2","role":"user","completed":true,"blocks":[]},
            {"id":"answer","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"done"}]},
            {"id":"manual","role":"system","completed":true,"blocks":[{"kind":"contextCompaction","text":"done"}]}
        ])).unwrap();
        for outcome in [
            TurnOutcome::Completed,
            TurnOutcome::Failed,
            TurnOutcome::Cancelled,
        ] {
            let mut executions = Executions::default();
            executions.begin(ExecutionKind::UserTurn, false);
            for m in &messages[..4] {
                executions.attach(&m.id);
            }
            executions.finish(Some(outcome));
            executions.begin(ExecutionKind::ContextMaintenance, false);
            executions.attach("manual");
            executions.finish(Some(TurnOutcome::Completed));
            let mut projection = TranscriptProjection::default();
            projection.apply(TranscriptEvent::Replace {
                messages: &messages,
                executions: &executions,
            });
            assert!(
                projection
                    .turns()
                    .iter()
                    .all(|t| t.outcome == Some(outcome))
            );
            assert!(
                projection
                    .turns()
                    .iter()
                    .all(|t| !t.work_message_ids.contains(&"manual".into()))
            );
            assert!(projection.items().iter().any(|i| matches!(i, TranscriptItem::Message { message_id, .. } if message_id == "manual")));
        }
    }
    #[test]
    fn deleted_users_preserve_original_closing_replies_in_visible_order() {
        let original: Vec<Message> = serde_json::from_value(serde_json::json!([
            {"id":"user1","role":"user","completed":true,"blocks":[{"kind":"text","text":"first"}]},
            {"id":"work1","role":"assistant","completed":true,"blocks":[{"kind":"reasoning","text":"checking"}]},
            {"id":"notice","role":"client","completed":true,"blocks":[{"kind":"text","text":"background complete"}]},
            {"id":"reply1","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"first answer"}]},
            {"id":"user2","role":"user","completed":true,"blocks":[{"kind":"text","text":"second"}]},
            {"id":"work2","role":"assistant","completed":true,"blocks":[{"kind":"reasoning","text":"checking"}]},
            {"id":"reply2","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"second answer"}]},
            {"id":"user3","role":"user","completed":true,"blocks":[{"kind":"text","text":"third"}]},
            {"id":"reply3","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"third answer"}]}
        ])).unwrap();
        for removed in [["user2", ""], ["user1", "user2"]] {
            let messages: Vec<_> = original
                .iter()
                .filter(|message| !removed.contains(&message.id.as_str()))
                .cloned()
                .collect();
            let hints = ["reply1".into(), "reply2".into(), "reply3".into()];
            let mut projection = TranscriptProjection::default();
            // Restore both orders used by cold decoration and live resync.
            projection.retain_reply_boundaries(&hints);
            for _ in 0..2 {
                projection.apply(TranscriptEvent::Replace {
                    messages: &messages,
                    executions: &Executions::default(),
                });
                let visible: Vec<_> = projection
                    .items()
                    .into_iter()
                    .filter_map(|item| match item {
                        TranscriptItem::Message { message_id, .. } => Some(message_id),
                        _ => None,
                    })
                    .collect();
                let replies: Vec<_> = visible
                    .iter()
                    .filter(|id| id.starts_with("reply"))
                    .map(String::as_str)
                    .collect();
                assert_eq!(replies, ["reply1", "reply2", "reply3"]);
                assert!(
                    visible.iter().position(|id| id == "notice").unwrap()
                        < visible.iter().position(|id| id == "reply1").unwrap(),
                    "retained closing boundary must not move a standalone notification"
                );
                assert_eq!(visible.iter().collect::<BTreeSet<_>>().len(), visible.len());
                assert!(
                    projection
                        .turns()
                        .iter()
                        .all(|turn| !turn.work_message_ids.contains(&"reply2".into()))
                );
            }
        }
    }
}
