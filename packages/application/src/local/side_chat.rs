//! Side chats: questions asked on a conversation's context without adding
//! to it, in a temporary copy its runtime makes and never saves. Each is a
//! session of its own, open until it is closed.
use super::*;
use crate::conversation::ConversationSnapshot;
use session::SideChat;

/// A side chat's view: its own identity, without what the copy inherited.
pub(super) fn shown(side: &SideChat, mut snapshot: ConversationSnapshot) -> ConversationSnapshot {
    snapshot.context_runtime_id = snapshot.conversation.runtime_id.clone();
    snapshot.conversation.id = side.id.clone();
    snapshot.conversation.can_rename = false;
    snapshot.conversation.can_delete = false;
    snapshot.conversation.parent_conversation_id = None;
    snapshot
        .messages
        .retain(|message| !side.inherited.contains(&message.id));
    snapshot
}

impl CoreRuntime {
    /// Open a side chat on the addressed conversation, whose questions go
    /// with `selection`, `effort` and `permission_mode`: the conversation's
    /// next-turn choices. It must have been sent; a sub-agent's has none.
    pub(crate) fn open_side_chat(
        &mut self,
        runtime_id: &str,
        conversation_id: &str,
        selection: Option<crate::ModelSelection>,
        effort: Option<String>,
        permission_mode: Option<String>,
    ) -> Result<crate::SnapshotResult, RuntimeError> {
        let parent = self
            .queried_snapshot(runtime_id, conversation_id)?
            .ok_or_else(|| RuntimeError::invalid("此会话不可用"))?;
        let runtime = self.runtime_instance(runtime_id)?.clone();
        if !matches!(
            runtime.type_id,
            RuntimeKind::Codex | RuntimeKind::ClaudeCode | RuntimeKind::Pi
        ) {
            return Err(RuntimeError::invalid("此运行时不提供侧聊"));
        }
        if self.session.side.is_some() || parent.conversation.parent_conversation_id.is_some() {
            return Err(RuntimeError::invalid("此会话不提供侧聊"));
        }
        if self.session.unsubmitted_draft.is_some()
            || !parent
                .messages
                .iter()
                .any(|message| message.role == velune_conversation::MessageRole::User)
        {
            return Err(RuntimeError::invalid("请先发送一条消息，再打开侧聊"));
        }
        self.validate_model_selection(&runtime, selection.as_ref())?;
        if let Some(effort) = &effort {
            self.validate_effort(&runtime, selection.as_ref(), effort)?;
        }
        if let Some(mode) = &permission_mode {
            self.validate_permission_mode(&runtime, mode)?;
        }
        let native = self
            .native_snapshot()
            .ok_or_else(|| RuntimeError::invalid("此会话不可用"))?;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = format!(
            "side-chat:{}",
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        self.focus_new();
        self.session.side = Some(SideChat {
            id,
            effort,
            permission_mode,
            inherited: Default::default(),
        });
        let opened = self
            .prepare_with_selection(native, selection.as_ref(), false)
            .and_then(|()| self.sync_projection());
        if let Err(error) = opened {
            let _ = self.shutdown_active();
            self.session = Default::default();
            return Err(error);
        }
        let inherited = self
            .session
            .projected()
            .map(|view| {
                view.messages
                    .iter()
                    .map(|message| message.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(side) = &mut self.session.side {
            side.inherited = inherited;
        }
        self.snapshot_result()
    }

    /// Ask the addressed side chat `text`.
    pub(crate) fn send_side_chat(
        &mut self,
        side_chat_id: &str,
        text: String,
    ) -> Result<crate::SnapshotResult, RuntimeError> {
        let side = self
            .session
            .side
            .as_ref()
            .filter(|side| side.id == side_chat_id)
            .ok_or_else(|| RuntimeError::invalid("侧聊已关闭"))?;
        if text.trim().is_empty() {
            return Err(RuntimeError::invalid("message text"));
        }
        let input = pi::TurnInput {
            text,
            effort: side.effort.clone(),
            permission_mode: side.permission_mode.clone(),
            images: Vec::new(),
        };
        self.drain_runtime()?;
        if self.turn_running() {
            return Err(RuntimeError::invalid("侧聊仍在回答"));
        }
        if let Some((gateway_id, session)) = self.backend_session_identity() {
            self.execution_backends
                .begin_user_turn(&gateway_id, &session)
                .map_err(|error| RuntimeError::context("CLI 新轮准备", error))?;
        }
        if matches!(self.session.active_state, ActiveState::Pi(_))
            && self.session.gateway_runner.is_some()
        {
            self.bind_gateway_model()?;
        }
        self.session
            .active_state
            .live_mut()
            .ok_or_else(|| RuntimeError::invalid("侧聊已关闭"))?
            .send(&input)
            .map_err(|error| RuntimeError::context("message send", error))?;
        self.session.cancel_requested = false;
        self.sync_projection()?;
        self.snapshot_result()
    }

    /// Close the addressed side chat; what runs in it stops, and its
    /// runtime copy goes with it.
    pub(crate) fn close_side_chat(&mut self, side_chat_id: &str) -> Result<(), RuntimeError> {
        if self
            .session
            .side
            .as_ref()
            .is_none_or(|side| side.id != side_chat_id)
        {
            return Ok(());
        }
        if self.turn_running()
            && let Some((gateway_id, session)) = self.backend_session_identity()
        {
            self.execution_backends
                .cancel_session(&gateway_id, &session);
        }
        let result = self.shutdown_active();
        self.session = Default::default();
        result
    }
}
