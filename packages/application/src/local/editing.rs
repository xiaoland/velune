//! Editing a user message: the edited text goes out as a new turn that
//! replaces the message, either in its conversation, which goes back to just
//! before it, or in a new conversation holding what precedes it.
//!
//! Each runtime branches through its own native means; Velune keeps no copy
//! of the history. See `NativeSession::branch` and Pi's `velune-rewind`.
use super::*;
use crate::conversation::{
    ConversationSnapshot, EditPlacement, EditableMessage, Message, MessageBlock, MessageRole,
};

/// The runtime types whose native history can branch at a user message.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Branching {
    Codex,
    Claude,
    Pi,
}

impl Branching {
    fn of(kind: RuntimeKind) -> Option<Self> {
        match kind {
            RuntimeKind::Codex => Some(Self::Codex),
            RuntimeKind::ClaudeCode => Some(Self::Claude),
            RuntimeKind::Pi => Some(Self::Pi),
            RuntimeKind::DeepSeek | RuntimeKind::Acp => None,
        }
    }
}

/// The text a user message carries, for recognizing it in a fresh read.
fn message_text(message: &Message) -> String {
    message
        .blocks
        .iter()
        .filter_map(|block| match block {
            MessageBlock::Text { text } => Some(text.trim()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The entry a Pi message identity names: `pi:<session>:<entry>:<ordinal>`.
fn pi_entry(message_id: &str) -> Option<&str> {
    let mut parts = message_id.strip_prefix("pi:")?.rsplitn(3, ':');
    let _ordinal = parts.next()?;
    let entry = parts.next()?;
    parts.next()?;
    Some(entry)
}

impl CoreRuntime {
    /// The user messages of the addressed session that may be edited now.
    pub(super) fn editable_messages(
        &self,
        snapshot: &ConversationSnapshot,
    ) -> Vec<EditableMessage> {
        let editable = !self.session.executing()
            && snapshot.pending_interactions.is_empty()
            && snapshot.actions.can_send
            && snapshot.conversation.parent_conversation_id.is_none()
            && self.session.logical_projection.is_none()
            && self.session.unsubmitted_draft.is_none();
        let branching = self
            .runtime_instance(&snapshot.conversation.runtime_id)
            .ok()
            .and_then(|runtime| Branching::of(runtime.type_id));
        let (true, Some(branching)) = (editable, branching) else {
            return Vec::new();
        };
        let mut turns = std::collections::BTreeSet::new();
        snapshot
            .messages
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .enumerate()
            .filter_map(|(ordinal, message)| {
                let in_place = match branching {
                    // A Codex turn goes back as a whole: a message steered
                    // into it cannot be replaced on its own.
                    Branching::Codex => {
                        if !turns.insert(message.branch_point.clone()?) {
                            return None;
                        }
                        true
                    }
                    // Claude Code resumes after a record; the first message
                    // has none before it.
                    Branching::Claude => ordinal > 0,
                    Branching::Pi => true,
                };
                Some(EditableMessage {
                    message_id: message.id.clone(),
                    in_place,
                })
            })
            .collect()
    }

    /// Send `turn` in place of the user message `message_id`, continuing as
    /// `placement` says. A fork's result is the new conversation's snapshot.
    pub(crate) fn edit_message(
        &mut self,
        runtime_id: &str,
        conversation_id: &str,
        message_id: &str,
        placement: EditPlacement,
        selection: Option<crate::ModelSelection>,
        turn: crate::ComposedTurn,
    ) -> Result<crate::SnapshotResult, RuntimeError> {
        let current = self
            .current_snapshot()
            .filter(|snapshot| {
                snapshot.conversation.id == conversation_id
                    && snapshot.conversation.runtime_id == runtime_id
            })
            .ok_or_else(|| RuntimeError::invalid("会话未打开"))?;
        let editable = current
            .follow_up
            .editable
            .iter()
            .find(|message| message.message_id == message_id)
            .ok_or_else(|| RuntimeError::invalid("这条消息当前不能编辑"))?;
        if placement == EditPlacement::InPlace && !editable.in_place {
            return Err(RuntimeError::invalid(
                "此运行时不能在原会话中替换这条消息，请在新会话中继续",
            ));
        }
        let runtime = self.runtime_instance(runtime_id)?.clone();
        let branching = Branching::of(runtime.type_id).expect("editable runtime");
        // Nothing changes before the replacement is known to be deliverable.
        let attachments = self.turn_attachments(&runtime, &turn.attachments)?;
        let text = super::attachments::compose_text(&turn.text, &attachments.files);
        if text.trim().is_empty() && attachments.images.is_empty() {
            return Err(RuntimeError::invalid("message text"));
        }
        self.validate_model_selection(&runtime, selection.as_ref())?;
        if let Some(effort) = &turn.effort {
            self.validate_effort(&runtime, selection.as_ref(), effort)?;
        }
        if let Some(mode) = &turn.permission_mode {
            self.validate_permission_mode(&runtime, mode)?;
        }
        // The message as the native history has it now: what the view shows
        // may predate a change made outside Velune.
        let users = |snapshot: &ConversationSnapshot| {
            snapshot
                .messages
                .iter()
                .filter(|message| message.role == MessageRole::User)
                .cloned()
                .collect::<Vec<_>>()
        };
        let ordinal = users(&current)
            .iter()
            .position(|message| message.id == message_id)
            .expect("editable user message");
        let expected = users(&current)
            .get(ordinal)
            .map(message_text)
            .unwrap_or_default();
        let fresh = self.read_history(runtime_id, conversation_id)?;
        let original = users(&fresh)
            .into_iter()
            .nth(ordinal)
            .filter(|message| message_text(message) == expected)
            .ok_or_else(|| RuntimeError::invalid("会话已在别处改变，请重新打开后再编辑"))?;
        let first = ordinal == 0;

        let origin = self.session.key;
        if placement == EditPlacement::Fork {
            self.focus_new();
        }
        let branched = (|| {
            if placement == EditPlacement::Fork && first && branching != Branching::Pi {
                // Nothing precedes the first message: a new conversation in
                // the same directory.
                self.create_conversation(
                    runtime_id,
                    fresh.conversation.cwd.clone(),
                    selection.clone(),
                )?;
                return Ok(());
            }
            if placement == EditPlacement::InPlace {
                self.invalidate_execution()?;
            }
            self.prepare_with_selection(fresh.clone(), selection.as_ref(), false)?;
            match &mut self.session.active_state {
                ActiveState::Native(session) => {
                    let id = session
                        .snapshot()
                        .map(users)
                        .unwrap_or_default()
                        .into_iter()
                        .nth(ordinal)
                        .map(|message| message.id)
                        .ok_or_else(|| RuntimeError::invalid("运行时会话中没有这条消息"))?;
                    session
                        .branch(&id, placement)
                        .map_err(|error| RuntimeError::context("消息编辑", error))
                }
                ActiveState::Pi(session) => {
                    let entry = pi_entry(&original.id)
                        .ok_or_else(|| RuntimeError::invalid("这条消息没有 Pi 条目身份"))?;
                    session
                        .branch(entry, placement)
                        .map_err(|error| RuntimeError::context("消息编辑", error))?;
                    self.sync_projection()
                }
                _ => Err(RuntimeError::invalid("此运行时不提供消息编辑")),
            }
        })();
        if let Err(error) = branched {
            if placement == EditPlacement::Fork {
                // The new session holds nothing worth keeping; the original
                // conversation is addressed again as it was.
                let _ = self.shutdown_active();
                self.session.unsubmitted_draft = None;
                self.focus_key(origin);
            } else {
                let _ = self.invalidate_execution();
            }
            return Err(error);
        }
        let conversation = self
            .session
            .conversation_id()
            .ok_or_else(|| RuntimeError::invalid("会话未打开"))?
            .to_owned();
        self.send_turn(super::conversations::TurnRequest {
            source_runtime_id: runtime_id.into(),
            conversation_id: conversation,
            runtime_id: runtime_id.into(),
            selection,
            turn,
        })
    }
}
