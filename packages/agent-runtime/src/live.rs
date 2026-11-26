//! What every running conversation does the same way, whatever runtime runs
//! it. Kind-specific operations stay on the concrete session types.
use crate::conversation::{ConversationSnapshot, ConversationSummary, RuntimeInteractionReply};
use crate::creation::{DataRetention, Retained};
use crate::work::WorkStatus;
use crate::{TurnInput, acp, native, pi_session::PiSession};

/// Failure of a runtime operation, for display.
pub type LiveError = Box<dyn std::error::Error + Send + Sync>;
pub type LiveResult = Result<(), LiveError>;

pub trait LiveConversation {
    /// Request a new live observation without waiting or preparing a session.
    fn refresh_background_tasks(&mut self, _manual: bool) -> LiveResult {
        Ok(())
    }
    fn background_tasks(
        &self,
    ) -> Result<Retained<crate::conversation::BackgroundTaskList>, LiveError> {
        Ok(Retained::from_parts(
            Default::default(),
            DataRetention::default(),
        ))
    }
    fn read_background_task_output(
        &mut self,
        _task: &str,
        _request: Option<&str>,
    ) -> Result<Retained<crate::conversation::BackgroundTaskOutput>, LiveError> {
        Err("此运行时不能读取后台任务输出".into())
    }
    fn stop_background_task(&mut self, _task: &str) -> LiveResult {
        Err("此运行时不能独立停止后台任务".into())
    }
    /// The conversation as the runtime shows it, once it has one.
    fn snapshot(&self) -> Option<&ConversationSnapshot>;
    /// Reserve receive-data credit before making an owned display copy.
    fn owned_snapshot(&self) -> Result<Option<Retained<ConversationSnapshot>>, LiveError>;
    fn turn_running(&self) -> bool;
    /// A persisted conversation can be reopened after closing this connection.
    /// The application checks whether the current identity has been persisted.
    fn supports_reopen(&self) -> bool;
    /// Work that outlives the parent turn, as observed by this adapter.
    fn work(&self) -> Result<Retained<Vec<crate::work::BackgroundTask>>, LiveError> {
        Ok(Retained::from_parts(Vec::new(), DataRetention::default()))
    }
    /// Lifecycle evidence is independent of the task list used for display.
    fn work_status(&self) -> WorkStatus;
    /// A native request still needs this connection even if it doesn't block the turn.
    fn has_pending_requests(&self) -> bool {
        false
    }
    /// A best-effort metadata request still needs the current connection.
    fn has_pending_metadata(&self) -> bool {
        false
    }
    fn stop_background(&mut self) -> LiveResult {
        if self.work_status().is_pending() {
            Err("此运行时没有后台工作停止接口".into())
        } else {
            Ok(())
        }
    }
    fn can_stop_child(&self, _id: &str) -> bool {
        false
    }
    fn stop_child(&mut self, _id: &str) -> LiveResult {
        Err("此运行时不支持独立停止子会话".into())
    }
    fn set_conversation(&mut self, conversation: Retained<ConversationSummary>);
    /// Change native metadata without replacing the running conversation.
    fn rename(&mut self, _title: &str) -> LiveResult {
        Err("此运行时没有原生会话重命名接口".into())
    }
    /// Apply what the runtime reported since the last poll.
    fn poll(&mut self) -> LiveResult;
    /// Start a user turn.
    fn send(&mut self, input: &TurnInput) -> LiveResult;
    fn cancel(&mut self) -> LiveResult;
    fn shutdown(&mut self) -> LiveResult;
    /// The running turn takes further messages through the runtime's own channel.
    fn can_steer(&self) -> bool {
        false
    }
    fn steer(&mut self, _input: &TurnInput) -> LiveResult {
        Err("此运行时当前不能插入消息".into())
    }
    fn can_compact(&self) -> bool {
        false
    }
    fn compact(&mut self) -> LiveResult {
        Err("此运行时不支持主动压缩上下文".into())
    }
    /// Answer a request the runtime asked the user.
    fn reply(&mut self, _interaction_id: &str, _reply: RuntimeInteractionReply) -> LiveResult {
        Err("此运行时没有待回复的请求".into())
    }
}

impl LiveConversation for PiSession {
    fn refresh_background_tasks(&mut self, manual: bool) -> LiveResult {
        if manual {
            PiSession::refresh_background_tasks(self);
        }
        Ok(())
    }
    fn background_tasks(
        &self,
    ) -> Result<Retained<crate::conversation::BackgroundTaskList>, LiveError> {
        Ok(Retained::from_parts(
            PiSession::background_tasks(self),
            DataRetention::default(),
        ))
    }
    fn read_background_task_output(
        &mut self,
        task: &str,
        request: Option<&str>,
    ) -> Result<Retained<crate::conversation::BackgroundTaskOutput>, LiveError> {
        Ok(Retained::from_parts(
            PiSession::read_background_task_output(self, task, request)?,
            DataRetention::default(),
        ))
    }
    fn stop_background_task(&mut self, task: &str) -> LiveResult {
        Ok(PiSession::stop_background_task(self, task)?)
    }
    fn can_stop_child(&self, id: &str) -> bool {
        PiSession::can_stop_child(self, id)
    }
    fn stop_child(&mut self, id: &str) -> LiveResult {
        Ok(PiSession::stop_child(self, id)?)
    }
    fn has_pending_requests(&self) -> bool {
        self.snapshot().is_some_and(|snapshot| {
            snapshot
                .pending_interactions
                .iter()
                .any(crate::conversation::RuntimeInteraction::requires_live_reply)
        })
    }

    fn snapshot(&self) -> Option<&ConversationSnapshot> {
        PiSession::snapshot(self)
    }
    fn owned_snapshot(&self) -> Result<Option<Retained<ConversationSnapshot>>, LiveError> {
        if PiSession::snapshot(self).is_none() {
            return Ok(None);
        }
        Ok(Some(PiSession::owned_snapshot(self)?))
    }
    fn turn_running(&self) -> bool {
        PiSession::turn_running(self)
    }
    fn supports_reopen(&self) -> bool {
        true
    }
    fn work(&self) -> Result<Retained<Vec<crate::work::BackgroundTask>>, LiveError> {
        Ok(Retained::from_parts(
            PiSession::work(self),
            DataRetention::default(),
        ))
    }
    fn work_status(&self) -> WorkStatus {
        PiSession::work_status(self)
    }
    fn has_pending_metadata(&self) -> bool {
        PiSession::has_pending_metadata(self)
    }
    fn stop_background(&mut self) -> LiveResult {
        Ok(PiSession::stop_background(self)?)
    }
    fn set_conversation(&mut self, conversation: Retained<ConversationSummary>) {
        PiSession::set_conversation(self, conversation)
    }
    fn rename(&mut self, title: &str) -> LiveResult {
        Ok(PiSession::set_name(self, title)?)
    }
    fn poll(&mut self) -> LiveResult {
        PiSession::poll(self);
        Ok(())
    }
    fn send(&mut self, input: &TurnInput) -> LiveResult {
        Ok(PiSession::send(self, input)?)
    }
    fn cancel(&mut self) -> LiveResult {
        Ok(PiSession::cancel(self)?)
    }
    fn shutdown(&mut self) -> LiveResult {
        Ok(PiSession::shutdown(self)?)
    }
    fn can_steer(&self) -> bool {
        PiSession::turn_running(self)
    }
    fn steer(&mut self, input: &TurnInput) -> LiveResult {
        Ok(PiSession::steer(self, input)?)
    }
    fn can_compact(&self) -> bool {
        true
    }
    fn compact(&mut self) -> LiveResult {
        Ok(PiSession::compact(self)?)
    }
    fn reply(&mut self, interaction_id: &str, reply: RuntimeInteractionReply) -> LiveResult {
        Ok(PiSession::reply(self, interaction_id, reply)?)
    }
}

impl LiveConversation for native::NativeSession {
    fn refresh_background_tasks(&mut self, manual: bool) -> LiveResult {
        native::NativeSession::refresh_background_tasks(self, manual);
        Ok(())
    }
    fn background_tasks(
        &self,
    ) -> Result<Retained<crate::conversation::BackgroundTaskList>, LiveError> {
        Ok(native::NativeSession::background_tasks(self)?)
    }
    fn read_background_task_output(
        &mut self,
        task: &str,
        request: Option<&str>,
    ) -> Result<Retained<crate::conversation::BackgroundTaskOutput>, LiveError> {
        Ok(native::NativeSession::read_background_task_output(
            self, task, request,
        )?)
    }
    fn stop_background_task(&mut self, task: &str) -> LiveResult {
        Ok(native::NativeSession::stop_background_task(self, task)?)
    }
    fn snapshot(&self) -> Option<&ConversationSnapshot> {
        native::NativeSession::snapshot(self)
    }
    fn owned_snapshot(&self) -> Result<Option<Retained<ConversationSnapshot>>, LiveError> {
        if native::NativeSession::snapshot(self).is_none() {
            return Ok(None);
        }
        Ok(Some(native::NativeSession::owned_snapshot(self)?))
    }
    fn turn_running(&self) -> bool {
        native::NativeSession::turn_running(self)
    }
    fn supports_reopen(&self) -> bool {
        true
    }
    fn work(&self) -> Result<Retained<Vec<crate::work::BackgroundTask>>, LiveError> {
        Ok(native::NativeSession::work(self)?)
    }
    fn work_status(&self) -> WorkStatus {
        native::NativeSession::work_status(self)
    }
    fn has_pending_requests(&self) -> bool {
        native::NativeSession::has_pending_requests(self)
    }
    fn has_pending_metadata(&self) -> bool {
        native::NativeSession::has_pending_metadata(self)
    }
    fn stop_background(&mut self) -> LiveResult {
        Ok(native::NativeSession::stop_background(self)?)
    }
    fn set_conversation(&mut self, conversation: Retained<ConversationSummary>) {
        native::NativeSession::set_conversation(self, conversation)
    }
    fn rename(&mut self, title: &str) -> LiveResult {
        Ok(native::NativeSession::rename(self, title)?)
    }
    fn poll(&mut self) -> LiveResult {
        Ok(native::NativeSession::poll(self)?)
    }
    fn send(&mut self, input: &TurnInput) -> LiveResult {
        Ok(native::NativeSession::send(self, input)?)
    }
    fn cancel(&mut self) -> LiveResult {
        Ok(native::NativeSession::cancel(self)?)
    }
    fn shutdown(&mut self) -> LiveResult {
        Ok(native::NativeSession::shutdown(self)?)
    }
    fn can_steer(&self) -> bool {
        native::NativeSession::can_steer(self)
    }
    fn steer(&mut self, input: &TurnInput) -> LiveResult {
        Ok(native::NativeSession::steer(self, input)?)
    }
    fn can_compact(&self) -> bool {
        native::NativeSession::can_compact(self)
    }
    fn compact(&mut self) -> LiveResult {
        Ok(native::NativeSession::compact(self)?)
    }
    fn reply(&mut self, interaction_id: &str, reply: RuntimeInteractionReply) -> LiveResult {
        Ok(native::NativeSession::reply(self, interaction_id, reply)?)
    }
}

impl LiveConversation for acp::Client {
    fn has_pending_requests(&self) -> bool {
        self.snapshot().is_some_and(|snapshot| {
            snapshot
                .pending_interactions
                .iter()
                .any(crate::conversation::RuntimeInteraction::requires_live_reply)
        })
    }

    fn snapshot(&self) -> Option<&ConversationSnapshot> {
        acp::Client::snapshot(self)
    }
    fn owned_snapshot(&self) -> Result<Option<Retained<ConversationSnapshot>>, LiveError> {
        if acp::Client::snapshot(self).is_none() {
            return Ok(None);
        }
        Ok(Some(acp::Client::owned_snapshot(self)?))
    }
    fn turn_running(&self) -> bool {
        acp::Client::turn_running(self)
    }
    fn work_status(&self) -> WorkStatus {
        WorkStatus::Untracked
    }
    fn supports_reopen(&self) -> bool {
        // Velune reopens generic ACP through session/load, which also replays history.
        self.capabilities().load_session
    }
    fn set_conversation(&mut self, conversation: Retained<ConversationSummary>) {
        acp::Client::set_conversation(self, conversation)
    }
    fn poll(&mut self) -> LiveResult {
        Ok(acp::Client::poll(self)?)
    }
    fn send(&mut self, input: &TurnInput) -> LiveResult {
        Ok(acp::Client::send(self, &input.text, &input.images)?)
    }
    fn cancel(&mut self) -> LiveResult {
        Ok(acp::Client::cancel(self)?)
    }
    fn shutdown(&mut self) -> LiveResult {
        Ok(acp::Client::shutdown(self)?)
    }
    fn reply(&mut self, interaction_id: &str, reply: RuntimeInteractionReply) -> LiveResult {
        Ok(acp::Client::reply(self, interaction_id, reply)?)
    }
}
