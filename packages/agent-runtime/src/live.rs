//! What every running conversation does the same way, whatever runtime runs
//! it. Kind-specific operations stay on the concrete session types.
use crate::conversation::{ConversationSnapshot, ConversationSummary, RuntimeInteractionReply};
use crate::{TurnInput, acp, native, pi_session::PiSession};

/// Failure of a runtime operation, for display.
pub type LiveError = Box<dyn std::error::Error + Send + Sync>;
pub type LiveResult = Result<(), LiveError>;

pub trait LiveConversation {
    /// The conversation as the runtime shows it, once it has one.
    fn snapshot(&self) -> Option<&ConversationSnapshot>;
    fn turn_running(&self) -> bool;
    fn set_conversation(&mut self, conversation: ConversationSummary);
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
    fn snapshot(&self) -> Option<&ConversationSnapshot> {
        PiSession::snapshot(self)
    }
    fn turn_running(&self) -> bool {
        PiSession::turn_running(self)
    }
    fn set_conversation(&mut self, conversation: ConversationSummary) {
        PiSession::set_conversation(self, conversation)
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
}

impl LiveConversation for native::NativeSession {
    fn snapshot(&self) -> Option<&ConversationSnapshot> {
        native::NativeSession::snapshot(self)
    }
    fn turn_running(&self) -> bool {
        native::NativeSession::turn_running(self)
    }
    fn set_conversation(&mut self, conversation: ConversationSummary) {
        native::NativeSession::set_conversation(self, conversation)
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
    fn snapshot(&self) -> Option<&ConversationSnapshot> {
        acp::Client::snapshot(self)
    }
    fn turn_running(&self) -> bool {
        acp::Client::turn_running(self)
    }
    fn set_conversation(&mut self, conversation: ConversationSummary) {
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
