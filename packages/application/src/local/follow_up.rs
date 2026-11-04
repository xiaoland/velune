//! What follows a conversation's turns: Velune's send queue, and whether the
//! user may continue after a turn that stopped or failed.
//!
//! The queue belongs to the conversation's session, so every window showing
//! the conversation shows the same one. It lives while the application runs.
use super::conversations::{Prompt, TurnRequest};
use super::*;
use crate::conversation::{FollowUp, QueuePause, QueueWait, QueuedMessage};
use crate::{ComposedTurn, ModelSelection, SnapshotResult};
use std::collections::VecDeque;

/// One message waiting for the running turn: the send request it becomes.
struct QueuedTurn {
    id: String,
    request: TurnRequest,
}

impl QueuedTurn {
    fn public(&self) -> QueuedMessage {
        QueuedMessage {
            id: self.id.clone(),
            text: self.request.turn.text.clone(),
            attachments: self.request.turn.attachments.clone(),
        }
    }
}

#[derive(Default)]
pub(super) struct SendQueue {
    turns: VecDeque<QueuedTurn>,
    paused: Option<QueuePause>,
    /// The next message waits for work besides the turn to end.
    waiting: Option<QueueWait>,
    /// The session's last turn stopped or failed.
    interrupted: Option<QueuePause>,
}

impl SendQueue {
    pub(super) fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    fn position(&self, id: &str) -> Result<usize, RuntimeError> {
        self.turns
            .iter()
            .position(|turn| turn.id == id)
            .ok_or_else(|| RuntimeError::invalid("此排队消息已经发出或删除"))
    }
}

fn queued_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    format!(
        "queued-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

impl CoreRuntime {
    /// Continue the addressed context using its declared native or message strategy.
    pub(crate) fn resume_turn(
        &mut self,
        runtime_id: &str,
        selection: Option<ModelSelection>,
        effort: Option<String>,
        permission_mode: Option<String>,
    ) -> Result<SnapshotResult, RuntimeError> {
        self.ensure_active(runtime_id)?;
        self.require_input_access()?;
        let snapshot = self
            .current_snapshot()
            .ok_or_else(|| RuntimeError::invalid("conversation is not active"))?;
        if !snapshot.follow_up.can_resume {
            return Err(RuntimeError::invalid("当前会话不能继续"));
        }
        let runtime = self.runtime_instance(&snapshot.context_runtime_id)?.clone();
        self.validate_model_selection(&runtime, selection.as_ref())?;
        if let Some(effort) = &effort {
            self.validate_effort(&runtime, selection.as_ref(), effort)?;
        }
        if let Some(mode) = &permission_mode {
            self.validate_permission_mode(&runtime, mode)?;
        }
        if matches!(self.session.active_state, ActiveState::History(_))
            || self.session.model_selection.as_ref() != Some(&selection)
        {
            let native = self.native_snapshot().expect("resumable context");
            self.prepare_with_selection(native, selection.as_ref(), false)?;
        }
        let message = match velune_agent_runtime::version::variant(runtime.type_id).resume {
            velune_agent_runtime::version::ResumeStrategy::Native => None,
            velune_agent_runtime::version::ResumeStrategy::Message(text) => Some((text, &[][..])),
            velune_agent_runtime::version::ResumeStrategy::Unsupported => {
                return Err(RuntimeError::invalid("此运行时不支持继续"));
            }
        };
        let result = self.send_action(Prompt {
            runtime_id,
            message,
            effort: effort.as_deref(),
            permission_mode: permission_mode.as_deref(),
        })?;
        self.restore_archived(&snapshot.conversation.id);
        Ok(result)
    }

    pub(crate) fn send_queue(
        &mut self,
        conversation_id: &str,
        operation: SendQueueOperation,
    ) -> Result<SnapshotResult, RuntimeError> {
        if self.session.compacting {
            return Err(RuntimeError::invalid("上下文压缩期间不能修改发送队列"));
        }
        if matches!(
            operation,
            SendQueueOperation::Add { .. } | SendQueueOperation::Resume
        ) {
            self.require_input_access()?;
        }
        let snapshot = self.native_snapshot();
        if matches!(
            operation,
            SendQueueOperation::Add { .. } | SendQueueOperation::Resume
        ) && snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.conversation.parent_conversation_id.is_some()
                || matches!(snapshot.run_state, RunState::Stopping | RunState::Elsewhere)
        }) {
            return Err(RuntimeError::invalid("当前会话不能接纳新的排队消息"));
        }
        let interrupted_idle = if self.turn_running() {
            None
        } else {
            self.session.send_queue.interrupted.clone()
        };
        let queue = &mut self.session.send_queue;
        match operation {
            SendQueueOperation::Add {
                source_runtime_id,
                target_runtime_id,
                selection,
                turn,
            } => {
                if turn.text.trim().is_empty() && turn.attachments.is_empty() {
                    return Err(RuntimeError::invalid("message text"));
                }
                // A late queue request after stop must not start a new turn.
                // An existing queue retains the user's pause/resume decision.
                if queue.turns.is_empty() && queue.paused.is_none() {
                    queue.paused = interrupted_idle;
                }
                queue.turns.push_back(QueuedTurn {
                    id: queued_id(),
                    request: TurnRequest {
                        source_runtime_id,
                        conversation_id: conversation_id.to_owned(),
                        runtime_id: target_runtime_id,
                        selection,
                        turn,
                    },
                });
            }
            SendQueueOperation::Edit { id, text } => {
                let index = queue.position(&id)?;
                let turn = &mut queue.turns[index];
                if text.trim().is_empty() && turn.request.turn.attachments.is_empty() {
                    return Err(RuntimeError::invalid("message text"));
                }
                turn.request.turn.text = text;
            }
            SendQueueOperation::Remove { id } => {
                let index = queue.position(&id)?;
                queue.turns.remove(index);
                if queue.turns.is_empty() {
                    queue.paused = None;
                    queue.waiting = None;
                }
            }
            SendQueueOperation::Move { id, index } => {
                let from = queue.position(&id)?;
                let turn = queue.turns.remove(from).expect("queued turn");
                let to = (index as usize).min(queue.turns.len());
                queue.turns.insert(to, turn);
            }
            SendQueueOperation::Resume => queue.paused = None,
            SendQueueOperation::Steer { id } => {
                let index = queue.position(&id)?;
                let turn = queue.turns.remove(index).expect("queued turn");
                if let Err(error) = self.steer(&turn.request.turn) {
                    let queue = &mut self.session.send_queue;
                    queue.turns.insert(index.min(queue.turns.len()), turn);
                    return Err(error);
                }
                if self.session.send_queue.turns.is_empty() {
                    self.session.send_queue.paused = None;
                }
            }
        }
        // A queue added to or resumed while nothing runs starts at once.
        if !self.turn_running()
            && self.session.send_queue.paused.is_none()
            && self.session.send_queue.waiting.is_none()
        {
            self.send_next_queued();
        }
        self.snapshot_result()
    }

    /// Deliver a composed message into the running turn through the
    /// runtime's own channel. Its next-turn choices do not apply: the turn
    /// keeps its own.
    pub(crate) fn steer_turn(
        &mut self,
        turn: &ComposedTurn,
    ) -> Result<SnapshotResult, RuntimeError> {
        self.steer(turn)?;
        self.snapshot_result()
    }

    fn steer(&mut self, turn: &ComposedTurn) -> Result<(), RuntimeError> {
        self.require_input_access()?;
        if !self.can_steer() {
            return Err(RuntimeError::invalid("此运行时当前不能插入消息"));
        }
        let attachments = self.turn_attachments(self.active_instance()?, &turn.attachments)?;
        let text = super::attachments::compose_text(&turn.text, &attachments.files);
        if text.trim().is_empty() && attachments.images.is_empty() {
            return Err(RuntimeError::invalid("message text"));
        }
        let input = pi::TurnInput {
            text,
            effort: None,
            permission_mode: None,
            images: super::attachments::turn_images(&attachments.images)?,
        };
        self.session
            .active_state
            .live_mut()
            .ok_or_else(|| RuntimeError::invalid("此运行时当前不能插入消息"))?
            .steer(&input)
            .map_err(|error| RuntimeError::context("message steer", error))
    }

    /// The running turn takes messages through the runtime's own channel.
    fn can_steer(&self) -> bool {
        if self.session.compacting {
            return false;
        }
        // A turn being stopped takes nothing more.
        !self.session.cancel_requested
            && self
                .session
                .active_state
                .live()
                .is_some_and(LiveConversation::can_steer)
    }

    /// The addressed session's turn ended with `outcome`: the queue goes on
    /// after a normal end and waits for the user otherwise.
    pub(super) fn follow_finished_turn(&mut self, outcome: crate::TurnOutcome) {
        let queue = &mut self.session.send_queue;
        let pause = match outcome {
            crate::TurnOutcome::Completed => None,
            crate::TurnOutcome::Cancelled => Some(QueuePause::TurnStopped),
            crate::TurnOutcome::Failed => Some(QueuePause::TurnFailed),
        };
        queue.interrupted = pause.clone();
        if queue.turns.is_empty() {
            return;
        }
        if pause.is_some() {
            queue.paused = pause;
        } else if queue.paused.is_none() {
            self.send_next_queued();
        }
    }

    /// A send that needs another runtime process while sub-agents still run
    /// in the current one goes last in the queue, which waits for them.
    pub(super) fn queue_after_work(&mut self, request: TurnRequest) {
        let queue = &mut self.session.send_queue;
        queue.turns.push_back(QueuedTurn {
            id: queued_id(),
            request,
        });
        queue.waiting = Some(QueueWait::SubAgents);
    }

    /// The addressed session's runtime process does nothing more: a queue
    /// waiting for its work goes on.
    pub(super) fn follow_ended_work(&mut self) {
        let queue = &mut self.session.send_queue;
        if queue.waiting.take().is_some() && queue.paused.is_none() {
            self.send_next_queued();
        }
    }

    /// A turn starts in the addressed session.
    pub(super) fn follow_started_turn(&mut self) {
        self.session.send_queue.interrupted = None;
    }

    /// Send the first queued message as its own turn. One that is not
    /// accepted stays first and the queue waits; it is never resent unasked.
    fn send_next_queued(&mut self) {
        if self.current_snapshot().is_some_and(|snapshot| {
            matches!(
                snapshot.actions.input_access,
                crate::conversation::InputAccess::CheckingControl
                    | crate::conversation::InputAccess::ControlUnavailable
                    | crate::conversation::InputAccess::ExternalController
                    | crate::conversation::InputAccess::ExternalExecution
            )
        }) {
            // Checking a released native handle is asynchronous. No message
            // was submitted: keep the queue waiting instead of latching a
            // send failure that would require the user's intervention.
            self.session.send_queue.waiting = Some(QueueWait::Control);
            return;
        }
        let Some(turn) = self.session.send_queue.turns.pop_front() else {
            return;
        };
        match self.send_and_restore(&turn.request) {
            Ok(_) => {}
            Err(RuntimeError::OutstandingWork) => {
                let queue = &mut self.session.send_queue;
                queue.turns.push_front(turn);
                queue.waiting = Some(QueueWait::SubAgents);
            }
            Err(error) => {
                tracing::warn!(target: "velune_application", event = "queued_turn_not_sent", detail = %error);
                let queue = &mut self.session.send_queue;
                queue.turns.push_front(turn);
                queue.paused = Some(QueuePause::SendFailed);
            }
        }
    }

    /// Whether `session`, presenting `snapshot`, can start a native turn
    /// without a user message: its own runtime resumes turns, it is idle
    /// with nothing to answer or still working, and its last turn stopped,
    /// failed or the native history marks it interrupted.
    pub(super) fn resumable(
        &self,
        session: &session::Session,
        snapshot: &crate::conversation::ConversationSnapshot,
    ) -> bool {
        let idle = !session.turn_running()
            && matches!(snapshot.run_state, RunState::Idle | RunState::Failed);
        idle && self
            .runtime_instance(&snapshot.context_runtime_id)
            .is_ok_and(|runtime| {
                runtime.enabled
                    && !matches!(
                        velune_agent_runtime::version::variant(runtime.type_id).resume,
                        velune_agent_runtime::version::ResumeStrategy::Unsupported
                    )
            })
            && snapshot.pending_interactions.is_empty()
            && snapshot.actions.can_send
            && session.work().is_empty()
            && snapshot.conversation.parent_conversation_id.is_none()
            && snapshot
                .messages
                .iter()
                .any(|message| message.role == crate::conversation::MessageRole::User)
            && (session.send_queue.interrupted.is_some()
                || snapshot.run_state == RunState::Failed
                || snapshot.follow_up.can_resume
                || snapshot
                    .executions
                    .records
                    .iter()
                    .rev()
                    .find(|record| record.kind == crate::conversation::ExecutionKind::UserTurn)
                    .is_some_and(|record| {
                        matches!(
                            record.outcome,
                            Some(
                                crate::conversation::TurnOutcome::Cancelled
                                    | crate::conversation::TurnOutcome::Failed
                            )
                        )
                    }))
    }

    /// The follow-up the addressed session presents with `snapshot`.
    pub(super) fn follow_up(
        &self,
        snapshot: &crate::conversation::ConversationSnapshot,
    ) -> FollowUp {
        let queue = &self.session.send_queue;
        let idle =
            !self.turn_running() && matches!(snapshot.run_state, RunState::Idle | RunState::Failed);
        FollowUp {
            queue: queue.turns.iter().map(QueuedTurn::public).collect(),
            paused: queue.paused.clone(),
            waiting: queue.waiting.clone(),
            can_stop_sub_agents: self
                .session
                .work()
                .iter()
                .any(|task| task.state != velune_agent_runtime::work::BackgroundState::Stopping),
            can_steer: self.can_steer(),
            can_resume: self.resumable(&self.session, snapshot),
            is_compacting: self.session.compacting,
            can_compact: idle
                && self
                    .runtime_instance(&snapshot.context_runtime_id)
                    .is_ok_and(|runtime| {
                        runtime.enabled
                            && velune_agent_runtime::version::variant(runtime.type_id).can_compact
                    })
                && snapshot.actions.can_send
                && snapshot.pending_interactions.is_empty()
                && self.session.work().is_empty()
                && snapshot.conversation.parent_conversation_id.is_none()
                && !snapshot.messages.is_empty(),
            editable: self.editable_messages(snapshot),
            can_fork: self.forkable(snapshot),
        }
    }
}
