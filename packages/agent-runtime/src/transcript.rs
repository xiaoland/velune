//! Ordered, disposable message-list projection. Native snapshots remain authoritative.
use crate::conversation::{
    Message, MessageBlock, MessageIdentityConfirmation, MessageRole, RunState, TranscriptItem,
    TranscriptMessageIdentity, TranscriptOutlineEntry, TranscriptTurn,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

/// Forward consumption includes replacement: native branching and compaction are not append-only.
pub enum TranscriptEvent<'a> {
    ConfirmIdentities(&'a [MessageIdentityConfirmation]),
    ReplaceMessages(&'a [Message]),
    ExecutionState(RunState),
}

struct MessageAnchor {
    id: String,
    role: MessageRole,
    pure_text: bool,
    completed: bool,
}

#[derive(Default)]
pub struct TranscriptProjection {
    messages: Vec<MessageAnchor>,
    running: bool,
    active: Option<(String, Instant)>,
    durations: BTreeMap<String, u64>,
    presentation_ids: BTreeMap<String, String>,
}
impl TranscriptProjection {
    pub fn apply(&mut self, event: TranscriptEvent<'_>) {
        match event {
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
                    if let Some(id) = self.presentation_ids.remove(&confirmation.previous_id) {
                        self.presentation_ids
                            .insert(confirmation.current_id.clone(), id);
                    }
                    if let Some(duration) = self.durations.remove(&confirmation.previous_id) {
                        self.durations
                            .insert(confirmation.current_id.clone(), duration);
                    }
                }
            }
            TranscriptEvent::ReplaceMessages(messages) => {
                self.messages = messages
                    .iter()
                    .map(|message| MessageAnchor {
                        id: message.id.clone(),
                        role: message.role.clone(),
                        pure_text: message.role == MessageRole::Assistant
                            && message.blocks.iter().any(|block| matches!(block, MessageBlock::Text { text } if !text.trim().is_empty()))
                            && message
                                .blocks
                                .iter()
                                .all(|block| matches!(block, MessageBlock::Text { .. })),
                        completed: message.completed,
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
                // An authoritative replacement can remove the in-progress branch.
                if self
                    .active
                    .as_ref()
                    .is_some_and(|(id, _)| !identities.contains(id.as_str()))
                {
                    self.active = None;
                }
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
            }
        }
    }

    /// A completed text-only assistant closes the range; streaming text may remain
    /// visible but cannot yet determine where the work interval ends.
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
                let responses = &self.messages[start + 1..next_user];
                let closed = responses
                    .iter()
                    .position(|message| message.pure_text && message.completed);
                let end = closed.map_or(responses.len(), |index| index + 1);
                let responses = &responses[..end];
                let visible = responses.last().filter(|message| message.pure_text);
                let work_end = responses.len() - usize::from(visible.is_some());
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
                    work_message_ids: responses[..work_end]
                        .iter()
                        .map(|message| message.id.clone())
                        .collect(),
                    last_message_id: visible.map(|message| message.id.clone()),
                    duration_ms,
                    is_running: ordinal + 1 == starts.len() && self.running && closed.is_none(),
                }
            })
            .collect()
    }

    pub fn items(&self) -> Vec<TranscriptItem> {
        let turns = self.turns();
        let work: BTreeMap<_, _> = turns
            .iter()
            .flat_map(|turn| {
                turn.work_message_ids
                    .iter()
                    .map(move |id| (id.as_str(), turn.id.as_str()))
            })
            .collect();
        let mut emitted = BTreeSet::new();
        self.messages
            .iter()
            .filter_map(|message| {
                if let Some(turn_id) = work.get(message.id.as_str()) {
                    return emitted.insert(*turn_id).then(|| TranscriptItem::Work {
                        turn_id: (*turn_id).into(),
                    });
                }
                Some(TranscriptItem::Message {
                    id: self.presentation_ids[&message.id].clone(),
                    message_id: message.id.clone(),
                })
            })
            .collect()
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
