//! What the user chose for coming turns, and Velune's record of what it sent.
use super::*;
use crate::config::{
    ComposerChoices, NewConversationChoice, NextTurnIntent, RuntimeComposerChoices,
};
use crate::conversation::{ConversationSnapshot, MessageBlock, MessageRole, TurnSettings};
use sha2::{Digest, Sha256};
use velune_agent_runtime::transcript::TranscriptProjection;

/// A turn Velune sent, by its user message's place among the conversation's
/// user messages and a digest of its text: native histories name messages
/// differently from a live run, so neither identity outlasts a reread.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TurnKey {
    pub(crate) ordinal: usize,
    pub(crate) digest: String,
}

pub(crate) type TurnRequests = BTreeMap<String, BTreeMap<TurnKey, TurnSettings>>;

fn text_digest(blocks: &[MessageBlock]) -> String {
    let mut hash = Sha256::new();
    for block in blocks {
        if let MessageBlock::Text { text } = block {
            hash.update(text.as_bytes());
            hash.update([0]);
        }
    }
    format!("{:x}", hash.finalize())[..16].to_owned()
}

fn user_turns(snapshot: &ConversationSnapshot) -> impl Iterator<Item = (TurnKey, &str)> {
    snapshot
        .messages
        .iter()
        .filter(|message| message.role == MessageRole::User)
        .enumerate()
        .map(|(ordinal, message)| {
            (
                TurnKey {
                    ordinal,
                    digest: text_digest(&message.blocks),
                },
                message.id.as_str(),
            )
        })
}

/// Gives `transcript` what Velune recorded for turns it sent, before it
/// folds `snapshot`'s messages.
pub(super) fn recall_turn_requests(
    requests: &TurnRequests,
    transcript: &mut TranscriptProjection,
    snapshot: &ConversationSnapshot,
) {
    let Some(recorded) = requests.get(&snapshot.conversation.id) else {
        return;
    };
    for (key, id) in user_turns(snapshot) {
        if let Some(settings) = recorded.get(&key) {
            transcript.recall_request(id, settings.clone());
        }
    }
}

impl CoreRuntime {
    /// Records the turns the transcript knows Velune sent. A history read
    /// shows the whole conversation, so turns it no longer has are dropped.
    pub(super) fn keep_turn_requests(&mut self, snapshot: &ConversationSnapshot) {
        let requested = self.session.transcript_projection.requested();
        let mut next = if matches!(self.session.active_state, ActiveState::History(_)) {
            BTreeMap::new()
        } else {
            self.turn_requests
                .get(&snapshot.conversation.id)
                .cloned()
                .unwrap_or_default()
        };
        for (key, id) in user_turns(snapshot) {
            if let Some(settings) = requested.get(id) {
                next.insert(key, settings.clone());
            }
        }
        if self
            .turn_requests
            .get(&snapshot.conversation.id)
            .map_or(next.is_empty(), |current| current == &next)
        {
            return;
        }
        let mut all = self.turn_requests.clone();
        if next.is_empty() {
            all.remove(&snapshot.conversation.id);
        } else {
            all.insert(snapshot.conversation.id.clone(), next);
        }
        self.store_turn_requests(all);
    }

    /// The record serves later reads; a failed write keeps the record for
    /// this run and leaves the file as it was.
    fn store_turn_requests(&mut self, next: TurnRequests) {
        if self.client_baseline.is_none()
            && let Err(error) = self.repository.store_turn_requests(&next)
        {
            tracing::warn!(target:"velune_application",event="turn_requests_store_failed",detail=%error);
        }
        self.turn_requests = next;
    }

    pub(super) fn composer_choices_action(
        &mut self,
        request: &Value,
    ) -> Result<Value, RuntimeError> {
        let payload = &request["payload"];
        let mut next = self.composer_choices.clone();
        match payload["operation"].as_str() {
            Some("list") => return Ok(serde_json::to_value(&self.composer_choices)?),
            Some("setIntent") => {
                let mut intent: NextTurnIntent = serde_json::from_value(payload["intent"].clone())?;
                if intent.conversation_id.is_empty() {
                    return Err(RuntimeError::invalid("conversation id"));
                }
                // A choice of nothing is kept: on a runtime the conversation
                // has no turn on, it asks for the runtime's own defaults.
                intent
                    .choices
                    .sort_by(|a, b| a.runtime_id.cmp(&b.runtime_id));
                if intent
                    .choices
                    .windows(2)
                    .any(|pair| pair[0].runtime_id == pair[1].runtime_id)
                {
                    return Err(RuntimeError::invalid("next-turn runtime choices"));
                }
                next.conversations
                    .retain(|kept| kept.conversation_id != intent.conversation_id);
                if intent.runtime_id.is_some() || !intent.choices.is_empty() {
                    next.conversations.push(intent);
                    next.conversations
                        .sort_by(|a, b| a.conversation_id.cmp(&b.conversation_id));
                }
            }
            Some("setRuntime") => {
                let choices: RuntimeComposerChoices =
                    serde_json::from_value(payload["choices"].clone())?;
                self.runtime_instance(&choices.runtime_id)?;
                next.runtimes
                    .retain(|kept| kept.runtime_id != choices.runtime_id);
                if !choices.last_chosen.is_empty()
                    || choices.new_conversation != NewConversationChoice::LastChosen
                {
                    next.runtimes.push(choices);
                    next.runtimes
                        .sort_by(|a, b| a.runtime_id.cmp(&b.runtime_id));
                }
            }
            _ => return Err(RuntimeError::invalid("composer choices operation")),
        }
        if next != self.composer_choices {
            if self.client_baseline.is_none() {
                self.repository.store_composer_choices(&next)?;
            }
            self.composer_choices = next;
        }
        Ok(Value::Null)
    }

    /// The conversation is already deleted; a failed write only leaves stale
    /// entries on disk, which the next successful write drops.
    pub(super) fn discard_conversation_choices(&mut self, conversation_id: &str) {
        if self
            .composer_choices
            .conversations
            .iter()
            .any(|intent| intent.conversation_id == conversation_id)
        {
            let mut next = self.composer_choices.clone();
            next.conversations
                .retain(|intent| intent.conversation_id != conversation_id);
            if self.client_baseline.is_none() {
                let _ = self.repository.store_composer_choices(&next);
            }
            self.composer_choices = next;
        }
        if self.turn_requests.contains_key(conversation_id) {
            let mut next = self.turn_requests.clone();
            next.remove(conversation_id);
            self.store_turn_requests(next);
        }
    }

    /// A deleted runtime instance takes its choices with it.
    pub(super) fn discard_runtime_choices(&mut self, runtime_id: &str) {
        let mut next: ComposerChoices = self.composer_choices.clone();
        next.runtimes.retain(|kept| kept.runtime_id != runtime_id);
        for intent in &mut next.conversations {
            intent
                .choices
                .retain(|choice| choice.runtime_id != runtime_id);
            if intent.runtime_id.as_deref() == Some(runtime_id) {
                intent.runtime_id = None;
            }
        }
        next.conversations
            .retain(|intent| intent.runtime_id.is_some() || !intent.choices.is_empty());
        if next != self.composer_choices {
            if self.client_baseline.is_none()
                && let Err(error) = self.repository.store_composer_choices(&next)
            {
                tracing::warn!(target:"velune_application",event="composer_choices_store_failed",detail=%error);
            }
            self.composer_choices = next;
        }
    }
}
