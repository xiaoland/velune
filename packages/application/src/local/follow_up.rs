//! What follows a conversation's turns: Velune's send queue, and whether the
//! user may continue after a turn that stopped or failed.
//!
//! The queue belongs to the conversation's session, so every window showing
//! the conversation shows the same one. It lives while the application runs.
use super::*;
use crate::conversation::{FollowUp, QueuePause, QueueWait, QueuedMessage};
use std::collections::VecDeque;

/// One message waiting for the running turn: the send request it becomes.
struct QueuedTurn {
    id: String,
    payload: Value,
}

impl QueuedTurn {
    fn public(&self) -> QueuedMessage {
        QueuedMessage {
            id: self.id.clone(),
            text: self.payload["text"].as_str().unwrap_or_default().into(),
            attachments: serde_json::from_value(self.payload["attachments"].clone())
                .unwrap_or_default(),
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
    interrupted: bool,
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
    pub(super) fn send_queue_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        let conversation_id = request["payload"]["conversationID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        let operation: SendQueueOperation =
            serde_json::from_value(request["payload"]["operation"].clone())?;
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
                queue.turns.push_back(QueuedTurn {
                    id: queued_id(),
                    payload: json!({
                        "sourceRuntimeID": source_runtime_id,
                        "conversationID": conversation_id,
                        "runtimeInstanceID": target_runtime_id,
                        "modelSelection": selection,
                        "text": turn.text,
                        "effort": turn.effort,
                        "permissionMode": turn.permission_mode,
                        "attachments": turn.attachments,
                    }),
                });
            }
            SendQueueOperation::Edit { id, text } => {
                let index = queue.position(&id)?;
                let turn = &mut queue.turns[index];
                if text.trim().is_empty() && turn.public().attachments.is_empty() {
                    return Err(RuntimeError::invalid("message text"));
                }
                turn.payload["text"] = Value::String(text);
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
                if let Err(error) = self.steer(&turn.payload) {
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
    pub(super) fn steer_turn_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.steer(&request["payload"])?;
        self.snapshot_result()
    }

    fn steer(&mut self, payload: &Value) -> Result<(), RuntimeError> {
        if !self.can_steer() {
            return Err(RuntimeError::invalid("此运行时当前不能插入消息"));
        }
        let runtime_id = self
            .native_snapshot()
            .map(|snapshot| snapshot.conversation.runtime_id)
            .ok_or_else(|| RuntimeError::invalid("conversation is not active"))?;
        let request = json!({ "payload": payload });
        let attachments = self.turn_attachments(self.runtime_instance(&runtime_id)?, &request)?;
        let text = super::attachments::compose_text(
            payload["text"].as_str().unwrap_or_default(),
            &attachments.files,
        );
        if text.trim().is_empty() && attachments.images.is_empty() {
            return Err(RuntimeError::invalid("message text"));
        }
        let input = pi::TurnInput {
            text,
            effort: None,
            permission_mode: None,
            images: super::attachments::turn_images(
                &json!({"payload": {"images": attachments.images}}),
            )?,
        };
        match &mut self.session.active_state {
            ActiveState::Native(session) => session
                .steer(&input)
                .map_err(|error| RuntimeError::context("message steer", error)),
            ActiveState::Pi => self
                .session
                .pi
                .client
                .as_mut()
                .ok_or_else(|| RuntimeError::invalid("会话执行尚未准备"))?
                .steer(&input)
                .map(drop)
                .map_err(|error| RuntimeError::context("message steer", error)),
            ActiveState::Acp(_) | ActiveState::History(_) | ActiveState::Empty => {
                Err(RuntimeError::invalid("此运行时当前不能插入消息"))
            }
        }
    }

    /// The running turn takes messages through the runtime's own channel.
    fn can_steer(&self) -> bool {
        match &self.session.active_state {
            ActiveState::Native(session) => session.can_steer(),
            ActiveState::Pi => self.session.pi.busy && !self.session.cancel_requested,
            ActiveState::Acp(_) | ActiveState::History(_) | ActiveState::Empty => false,
        }
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
        queue.interrupted = pause.is_some();
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
    pub(super) fn queue_after_work(&mut self, payload: Value) {
        let queue = &mut self.session.send_queue;
        queue.turns.push_back(QueuedTurn {
            id: queued_id(),
            payload,
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
        self.session.send_queue.interrupted = false;
    }

    /// Send the first queued message as its own turn. One that is not
    /// accepted stays first and the queue waits; it is never resent unasked.
    fn send_next_queued(&mut self) {
        let Some(turn) = self.session.send_queue.turns.pop_front() else {
            return;
        };
        let request =
            json!({"version": CONTRACT_VERSION, "action": "sendTurn", "payload": turn.payload});
        match self.send_and_restore(&request) {
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

    /// The follow-up the addressed session presents with `snapshot`.
    pub(super) fn follow_up(
        &self,
        snapshot: &crate::conversation::ConversationSnapshot,
    ) -> FollowUp {
        let queue = &self.session.send_queue;
        let idle = !self.turn_running();
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
            can_continue: idle
                && snapshot.pending_interactions.is_empty()
                && snapshot.actions.can_send
                && (queue.interrupted || snapshot.run_state == RunState::Failed),
            editable: self.editable_messages(snapshot),
        }
    }
}
