//! Ordered, disposable message-list projection. Native snapshots remain authoritative.
use crate::conversation::{
    Message, MessageIdentityConfirmation, MessageRole, RunState, TranscriptTurn,
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
}

#[derive(Default)]
pub struct TranscriptProjection {
    messages: Vec<MessageAnchor>,
    running: bool,
    active: Option<(String, Instant)>,
    durations: BTreeMap<String, u64>,
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
                    })
                    .collect();
                let identities: BTreeSet<_> = self
                    .messages
                    .iter()
                    .map(|message| message.id.as_str())
                    .collect();
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
                if running && (!self.running || self.active.is_none()) {
                    self.active = user.map(|id| (id, Instant::now()));
                } else if !running && let Some((id, started)) = self.active.take() {
                    self.durations.insert(id, elapsed_ms(started));
                }
                self.running = running;
            }
        }
    }

    /// User messages are persistent outline anchors. The latest result remains visible
    /// even when execution failed or ended with a tool result instead of an answer.
    pub fn turns(&self) -> Vec<TranscriptTurn> {
        let starts: Vec<_> = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == MessageRole::User)
            .map(|(i, _)| i)
            .collect();
        starts
            .iter()
            .enumerate()
            .map(|(ordinal, start)| {
                let end = starts
                    .get(ordinal + 1)
                    .copied()
                    .unwrap_or(self.messages.len());
                let user = &self.messages[*start];
                let responses = &self.messages[start + 1..end];
                // A handoff or status notice after the result must not hide that result.
                let last_result = responses
                    .iter()
                    .rposition(|message| message.role != MessageRole::System)
                    .unwrap_or_else(|| responses.len().saturating_sub(1));
                let is_running = ordinal + 1 == starts.len() && self.running;
                let duration_ms = self
                    .active
                    .as_ref()
                    .filter(|(id, _)| id == &user.id)
                    .map(|(_, started)| elapsed_ms(*started))
                    .or_else(|| self.durations.get(&user.id).copied());
                TranscriptTurn {
                    id: format!("turn:{}", user.id),
                    user_message_id: user.id.clone(),
                    work_message_ids: responses
                        .iter()
                        .take(last_result)
                        .map(|m| m.id.clone())
                        .collect(),
                    last_message_id: responses.get(last_result).map(|m| m.id.clone()),
                    duration_ms,
                    is_running,
                }
            })
            .collect()
    }
}
fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
