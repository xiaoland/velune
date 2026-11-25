//! What follows a conversation's turns: Velune's send queue, and whether the
//! user may continue after a turn that stopped or failed.
//!
//! The queue belongs to the conversation's session, so every window showing
//! the conversation shows the same one. It lives while the application runs.
use super::conversations::{Prompt, TurnRequest};
use super::*;
use crate::conversation::{
    FollowUp, QueuePause, QueueWait, QueuedMessage, QueuedMessageEditContext,
};
use crate::{ComposedTurn, ModelSelection, SnapshotResult};
use std::collections::VecDeque;

/// One message waiting for the running turn: the send request it becomes.
struct QueuedTurn {
    id: String,
    edit_revision: u64,
    request: TurnRequest,
}

#[derive(Clone)]
pub(super) struct ContextPreparationGate {
    pub(super) work_id: String,
    pub(super) queued_message_id: String,
    pub(super) target_runtime_id: String,
    pub(super) state: PreparationState,
    pub(super) dispatching: bool,
    pub(super) request_bound: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreparationState {
    Running,
    Cancelling,
    Ready,
    Failed,
    Cancelled,
}

pub(super) struct QueuedPreparedDispatch {
    turn: QueuedTurn,
    gate: ContextPreparationGate,
}

impl QueuedTurn {
    fn new(id: String, request: TurnRequest) -> Self {
        Self {
            id,
            edit_revision: 1,
            request,
        }
    }

    fn public(&self) -> QueuedMessage {
        QueuedMessage {
            id: self.id.clone(),
            submission_id: self.request.submission_id.clone(),
            text: self.request.turn.text.clone(),
            attachments: self.request.turn.attachments.clone(),
            edit_context: Some(QueuedMessageEditContext {
                revision: self.edit_revision,
                runtime_id: self.request.runtime_id.clone(),
                selection: self.request.selection.clone(),
                effort: self.request.turn.effort.clone(),
                permission_mode: self.request.turn.permission_mode.clone(),
            }),
        }
    }

    fn edit(&mut self) -> Result<&mut TurnRequest, RuntimeError> {
        self.edit_revision = self
            .edit_revision
            .checked_add(1)
            .ok_or_else(|| RuntimeError::invalid("此排队消息的编辑版本已达上限"))?;
        Ok(&mut self.request)
    }
}

#[derive(Default)]
pub(super) struct SendQueue {
    preparation: Option<ContextPreparationGate>,
    turns: VecDeque<QueuedTurn>,
    paused: Option<QueuePause>,
    /// The next message waits for work besides the turn to end.
    waiting: Option<QueueWait>,
    /// The session's last turn stopped or failed.
    interrupted: Option<QueuePause>,
}

impl SendQueue {
    pub(super) fn prepares_context(&self) -> bool {
        self.preparation.is_some()
    }
    pub(super) fn preparation_cancellable(&self) -> bool {
        self.preparation.as_ref().is_some_and(|gate| {
            matches!(
                gate.state,
                PreparationState::Running | PreparationState::Cancelling | PreparationState::Ready
            )
        })
    }
    pub(super) fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    /// Preserve the stop decision before completion clears session flags.
    /// This also covers a late enqueue after maintenance has already ended.
    pub(super) fn pause_after_stop(&mut self) {
        self.interrupted = Some(QueuePause::TurnStopped);
        self.paused.get_or_insert(QueuePause::TurnStopped);
    }

    fn position(&self, id: &str) -> Result<usize, RuntimeError> {
        self.turns
            .iter()
            .position(|turn| turn.id == id)
            .ok_or_else(|| RuntimeError::invalid("此排队消息已经发出或删除"))
    }

    fn editable_position(&self, id: &str) -> Result<usize, RuntimeError> {
        let index = self.position(id)?;
        if self.preparation.as_ref().is_some_and(|gate| {
            gate.queued_message_id == id && (gate.dispatching || gate.request_bound)
        }) {
            return Err(RuntimeError::invalid(
                "此排队消息已进入派发流程，修改未保存",
            ));
        }
        Ok(index)
    }

    fn position_at_revision(
        &self,
        id: &str,
        expected_revision: u64,
    ) -> Result<usize, RuntimeError> {
        let index = self.editable_position(id)?;
        if self.turns[index].edit_revision != expected_revision {
            let mut failure = crate::Failure::new(
                crate::FailureKind::Conflict,
                "此排队消息已被修改，请刷新后重试",
            );
            failure.code = "queued_message_changed".into();
            return Err(RuntimeError::Contract(failure));
        }
        Ok(index)
    }
}

fn queued_id() -> String {
    // A client's open editor can outlive a disconnected host's process.
    format!("queued-{}", crate::new_record_key())
}

fn same_turn(left: &TurnRequest, right: &TurnRequest) -> bool {
    serde_json::to_vec(&(
        &left.source_runtime_id,
        &left.conversation_id,
        &left.runtime_id,
        &left.selection,
        &left.turn,
    ))
    .ok()
        == serde_json::to_vec(&(
            &right.source_runtime_id,
            &right.conversation_id,
            &right.runtime_id,
            &right.selection,
            &right.turn,
        ))
        .ok()
}

impl CoreRuntime {
    pub(crate) fn queued_handoff_execution(
        &self,
        conversation_id: &str,
        target_runtime_id: &str,
    ) -> Option<String> {
        let session = std::iter::once(&self.session)
            .chain(&self.parked)
            .find(|session| session.conversation_id() == Some(conversation_id))?;
        let snapshot = session.projected()?;
        if snapshot.conversation.runtime_id != target_runtime_id {
            return None;
        }
        snapshot
            .executions
            .current
            .as_ref()
            .map(|current| current.execution_id.clone())
    }
    pub(crate) fn pause_queued_conversation(&mut self, conversation_id: &str) {
        if let Some(session) = std::iter::once(&mut self.session)
            .chain(&mut self.parked)
            .find(|session| session.conversation_id() == Some(conversation_id))
        {
            session.send_queue.pause_after_stop();
        }
    }
    pub(crate) fn cancel_accepted_queued_handoff(
        &mut self,
        conversation_id: &str,
        target_runtime_id: &str,
        execution_id: Option<&str>,
    ) -> Result<(), RuntimeError> {
        if let Some(expected) = execution_id
            && self
                .queued_handoff_execution(conversation_id, target_runtime_id)
                .as_deref()
                == Some(expected)
            && self.turn_running()
        {
            self.cancel(target_runtime_id)?;
        }
        Ok(())
    }
    pub(crate) fn handoff_queue_is_empty(&self, conversation_id: &str) -> bool {
        std::iter::once(&self.session)
            .chain(&self.parked)
            .find(|session| session.conversation_id() == Some(conversation_id))
            .is_none_or(|session| session.send_queue.is_empty())
    }
    pub(crate) fn queued_handoff_turn(
        &self,
        conversation_id: &str,
        queued_id: &str,
    ) -> Result<TurnRequest, RuntimeError> {
        let session = std::iter::once(&self.session)
            .chain(&self.parked)
            .find(|session| session.conversation_id() == Some(conversation_id))
            .ok_or_else(|| RuntimeError::invalid("排队会话已不可用"))?;
        let first = session
            .send_queue
            .turns
            .front()
            .filter(|turn| turn.id == queued_id)
            .ok_or_else(|| RuntimeError::invalid("此消息已不在队首，请刷新后重试"))?;
        Ok(first.request.clone())
    }

    pub(crate) fn begin_queued_handoff(
        &mut self,
        request: TurnRequest,
        work_id: &str,
        retry: Option<&str>,
    ) -> Result<(String, SnapshotResult), RuntimeError> {
        self.require_input_access()?;
        let source = self
            .current_snapshot()
            .ok_or_else(|| RuntimeError::invalid("请选择会话"))?;
        if source.conversation.id != request.conversation_id
            || source.conversation.runtime_id != request.source_runtime_id
        {
            return Err(RuntimeError::invalid("排队来源会话已改变"));
        }
        let native = self
            .native_snapshot()
            .ok_or_else(|| RuntimeError::invalid("来源上下文不可用"))?;
        Self::submission_admission(&self.session, &native)
            .accept_queue
            .map_err(RuntimeError::invalid)?;
        if request.turn.text.trim().is_empty() && request.turn.attachments.is_empty() {
            return Err(RuntimeError::invalid("message text"));
        }
        let queue = &mut self.session.send_queue;
        let id = if let Some(id) = retry {
            if queue
                .turns
                .front()
                .is_none_or(|turn| turn.id != id || !same_turn(&turn.request, &request))
            {
                return Err(RuntimeError::invalid(
                    "排队消息或执行配置已改变，请重新打开确认",
                ));
            }
            id.to_owned()
        } else {
            if !queue.turns.is_empty() {
                return Err(RuntimeError::invalid(
                    "已有排队消息，请处理现有队列后再开始运行时切换",
                ));
            }
            let id = queued_id();
            queue
                .turns
                .push_back(QueuedTurn::new(id.clone(), request.clone()));
            id
        };
        queue.preparation = Some(ContextPreparationGate {
            work_id: work_id.into(),
            queued_message_id: id.clone(),
            target_runtime_id: request.runtime_id,
            state: PreparationState::Running,
            dispatching: false,
            request_bound: false,
        });
        queue.paused = None;
        queue.waiting = Some(QueueWait::ContextPreparation);
        let result = self.accepted_submission_result(crate::SubmissionReceipt::Queued {
            queue_id: id.clone(),
        })?;
        Ok((id, result))
    }

    pub(crate) fn queued_preparation_state(&self, work_id: &str) -> Option<PreparationState> {
        std::iter::once(&self.session)
            .chain(&self.parked)
            .find_map(|session| {
                session
                    .send_queue
                    .preparation
                    .as_ref()
                    .filter(|gate| gate.work_id == work_id)
                    .map(|gate| gate.state)
            })
    }

    pub(crate) fn sync_queued_preparation(&mut self, work_id: &str, state: PreparationState) {
        for session in std::iter::once(&mut self.session).chain(&mut self.parked) {
            let queue = &mut session.send_queue;
            let Some(gate) = queue
                .preparation
                .as_mut()
                .filter(|gate| gate.work_id == work_id)
            else {
                continue;
            };
            if gate.state == PreparationState::Cancelled && state != PreparationState::Cancelled {
                continue;
            }
            gate.state = state;
            if matches!(
                state,
                PreparationState::Failed | PreparationState::Cancelled
            ) {
                gate.dispatching = false;
                queue
                    .paused
                    .get_or_insert(if state == PreparationState::Cancelled {
                        QueuePause::TurnStopped
                    } else {
                        QueuePause::SendFailed
                    });
            }
        }
    }

    pub(crate) fn queued_prepared_candidate(&mut self, work_id: &str) -> Option<TurnRequest> {
        for session in std::iter::once(&mut self.session).chain(&mut self.parked) {
            let queue = &mut session.send_queue;
            let Some(gate) = queue
                .preparation
                .as_mut()
                .filter(|gate| gate.work_id == work_id)
            else {
                continue;
            };
            if gate.state != PreparationState::Ready || gate.dispatching || queue.paused.is_some() {
                return None;
            }
            let turn = queue
                .turns
                .front()
                .filter(|turn| turn.id == gate.queued_message_id)?;
            gate.dispatching = true;
            gate.request_bound = true;
            return Some(turn.request.clone());
        }
        None
    }

    pub(crate) fn pause_queued_preparation(&mut self, work_id: &str) {
        for session in std::iter::once(&mut self.session).chain(&mut self.parked) {
            let queue = &mut session.send_queue;
            if let Some(gate) = queue
                .preparation
                .as_mut()
                .filter(|gate| gate.work_id == work_id)
            {
                gate.dispatching = false;
                queue.paused = Some(QueuePause::SendFailed);
            }
        }
    }

    pub(crate) fn unbind_queued_preparation(&mut self, work_id: &str) {
        for session in std::iter::once(&mut self.session).chain(&mut self.parked) {
            if let Some(gate) = session
                .send_queue
                .preparation
                .as_mut()
                .filter(|gate| gate.work_id == work_id)
            {
                gate.request_bound = false;
            }
        }
    }
    pub(crate) fn require_queued_handoff_retry(
        &self,
        conversation_id: &str,
        queued_id: &str,
    ) -> Result<(), RuntimeError> {
        let session = std::iter::once(&self.session)
            .chain(&self.parked)
            .find(|session| session.conversation_id() == Some(conversation_id))
            .ok_or_else(|| RuntimeError::invalid("排队会话已不可用"))?;
        let gate = session
            .send_queue
            .preparation
            .as_ref()
            .filter(|gate| gate.queued_message_id == queued_id)
            .ok_or_else(|| RuntimeError::invalid("此消息不再需要摘要，请从现有队列继续"))?;
        if gate.dispatching
            || !matches!(
                gate.state,
                PreparationState::Failed | PreparationState::Cancelled
            )
        {
            return Err(RuntimeError::invalid(
                "接续准备仍在进行或可直接继续发送，请勿重复生成摘要",
            ));
        }
        Ok(())
    }
    pub(super) fn cancel_queued_preparation(&mut self) -> bool {
        let queue = &mut self.session.send_queue;
        let Some(gate) = &mut queue.preparation else {
            return false;
        };
        gate.state = PreparationState::Cancelled;
        queue.paused = Some(QueuePause::TurnStopped);
        true
    }

    pub(super) fn take_prepared_queue_turn(
        &mut self,
        work_id: &str,
        request: &TurnRequest,
        expected_queue_id: Option<&str>,
    ) -> Result<Option<QueuedPreparedDispatch>, RuntimeError> {
        let queue = &mut self.session.send_queue;
        let Some(gate) = &queue.preparation else {
            if expected_queue_id.is_some() {
                return Err(RuntimeError::invalid(
                    "拥有本次接续的排队消息已移除，未发送消息",
                ));
            }
            return Ok(None);
        };
        if gate.work_id != work_id
            || expected_queue_id.is_some_and(|id| gate.queued_message_id != id)
            || gate.state != PreparationState::Ready
            || queue.paused.is_some()
            || queue.turns.front().is_none_or(|turn| {
                turn.id != gate.queued_message_id || !same_turn(&turn.request, request)
            })
        {
            return Err(RuntimeError::invalid(
                "排队接续已取消、暂停或输入已改变；未发送消息",
            ));
        }
        let gate = queue.preparation.take().expect("validated preparation");
        let turn = queue.turns.pop_front().expect("validated queue head");
        queue.waiting = None;
        Ok(Some(QueuedPreparedDispatch { turn, gate }))
    }

    pub(super) fn finish_prepared_queue_turn(
        &mut self,
        dispatch: Option<QueuedPreparedDispatch>,
        accepted: bool,
    ) {
        if let Some(mut dispatch) = dispatch
            && !accepted
        {
            let committed = self
                .session
                .logical_projection
                .as_ref()
                .and_then(|projection| projection.link.segments.last())
                .is_some_and(|segment| {
                    segment.runtime_instance_id == dispatch.turn.request.runtime_id
                        && segment
                            .imported
                            .as_ref()
                            .and_then(|import| import.summary_work_id.as_deref())
                            == Some(dispatch.gate.work_id.as_str())
                });
            dispatch.gate.dispatching = false;
            let queue = &mut self.session.send_queue;
            queue.turns.push_front(dispatch.turn);
            queue.preparation = if committed { None } else { Some(dispatch.gate) };
            queue.paused = Some(QueuePause::SendFailed);
            queue.waiting = if committed {
                None
            } else {
                Some(QueueWait::ContextPreparation)
            };
        }
    }

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
        if self.can_submit_native(
            execution_workers::ExecutionCommandKind::Resume,
            runtime_id,
            &selection,
        ) {
            let (effort, permission_mode) =
                self.turn_options(effort.as_deref(), permission_mode.as_deref())?;
            let input = pi::TurnInput {
                text: String::new(),
                effort,
                permission_mode,
                images: Vec::new(),
            };
            self.enqueue_native_submission(execution_workers::ExecutionCommandKind::Resume, input)?;
            return self.snapshot_result();
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
        self.send_queue_submission(conversation_id, operation, None)
    }

    pub(crate) fn withdraw_queued_turn(
        &mut self,
        conversation_id: &str,
        id: String,
        expected_revision: u64,
        composer: &mut crate::composer::ComposerInputs,
    ) -> Result<SnapshotResult, RuntimeError> {
        if self.session.conversation_id() != Some(conversation_id) {
            return Err(RuntimeError::invalid("排队会话已不可用"));
        }
        let queue = &self.session.send_queue;
        let index = queue.position_at_revision(&id, expected_revision)?;
        composer.restore_queued_turn(conversation_id, &queue.turns[index].request)?;
        // The same application owner still controls the queue, so its item
        // and dispatch eligibility cannot change between saving and removal.
        self.send_queue(conversation_id, SendQueueOperation::Remove { id })
    }

    fn replace_queued_turn(
        &mut self,
        id: &str,
        expected_revision: u64,
        target_runtime_id: String,
        selection: Option<ModelSelection>,
        retained_attachments: Vec<u32>,
        mut turn: ComposedTurn,
    ) -> Result<SnapshotResult, RuntimeError> {
        let queue = &self.session.send_queue;
        let index = queue.position_at_revision(id, expected_revision)?;
        let queued = &queue.turns[index];
        let previous = &queued.request;
        let target_changed = previous.runtime_id != target_runtime_id;
        let config_changed = target_changed
            || previous.selection != selection
            || previous.turn.effort != turn.effort
            || previous.turn.permission_mode != turn.permission_mode;
        if config_changed
            && queue
                .preparation
                .as_ref()
                .is_some_and(|gate| gate.queued_message_id == id)
        {
            return Err(RuntimeError::invalid(
                "已为此消息准备接续，当前仅可修改正文和附件",
            ));
        }
        let mut retained = std::collections::HashSet::new();
        let mut attachments =
            Vec::with_capacity(retained_attachments.len() + turn.attachments.len());
        for index in retained_attachments {
            let path = previous
                .turn
                .attachments
                .get(index as usize)
                .filter(|_| retained.insert(index))
                .ok_or_else(|| RuntimeError::invalid("要保留的排队附件无效，请重新打开编辑"))?;
            attachments.push(path.clone());
        }
        let retained_count = attachments.len();
        attachments.append(&mut turn.attachments);
        turn.attachments = attachments;
        if turn.text.trim().is_empty() && turn.attachments.is_empty() {
            return Err(RuntimeError::invalid("请输入消息或添加附件"));
        }
        // Existing inputs keep their captured meaning even after configuration
        // or files change. Dispatch still validates the entire request.
        let new_inputs = if target_changed {
            turn.attachments.as_slice()
        } else {
            &turn.attachments[retained_count..]
        };
        if config_changed || !new_inputs.is_empty() {
            let runtime = self
                .runtime_instance(&target_runtime_id)
                .map_err(|_| RuntimeError::invalid("所选运行时已不存在，请选择可用的运行时"))?;
            if config_changed {
                if !runtime.enabled {
                    return Err(RuntimeError::invalid(
                        "所选运行时已停用，请选择可用的运行时",
                    ));
                }
                self.validate_model_selection(runtime, selection.as_ref())?;
                if let Some(effort) = &turn.effort {
                    self.validate_effort(runtime, selection.as_ref(), effort)?;
                }
                if let Some(mode) = &turn.permission_mode {
                    self.validate_permission_mode(runtime, mode)?;
                }
            }
            self.turn_attachments(runtime, new_inputs)?;
        }

        let request = self.session.send_queue.turns[index].edit()?;
        request.runtime_id = target_runtime_id;
        request.selection = selection;
        request.turn = turn;
        self.snapshot_result()
    }

    pub(crate) fn send_queue_submission(
        &mut self,
        conversation_id: &str,
        operation: SendQueueOperation,
        submission_id: Option<String>,
    ) -> Result<SnapshotResult, RuntimeError> {
        if matches!(
            operation,
            SendQueueOperation::Add { .. } | SendQueueOperation::Resume
        ) {
            self.require_input_access()?;
        }
        if matches!(
            operation,
            SendQueueOperation::Add { .. } | SendQueueOperation::Resume
        ) {
            let snapshot = self
                .native_snapshot()
                .ok_or_else(|| RuntimeError::invalid("请选择会话"))?;
            Self::submission_admission(&self.session, &snapshot)
                .accept_queue
                .map_err(RuntimeError::invalid)?;
        }
        let interrupted_idle = if self.turn_running() {
            None
        } else {
            self.session.send_queue.interrupted.clone()
        };
        let added_id = matches!(&operation, SendQueueOperation::Add { .. }).then(queued_id);
        let queue = &mut self.session.send_queue;
        match &operation {
            SendQueueOperation::Move { id, index } => {
                queue.position(id)?;
                if queue.preparation.as_ref().is_some_and(|gate| {
                    gate.dispatching && (gate.queued_message_id == *id || *index == 0)
                }) {
                    return Err(RuntimeError::invalid("队首正在派发，暂不能改变顺序"));
                }
            }
            SendQueueOperation::Remove { id } => {
                queue.position(id)?;
            }
            SendQueueOperation::Steer { id } => {
                queue.position(id)?;
                if queue.preparation.is_some() {
                    return Err(RuntimeError::invalid("接续准备期间不能插入消息"));
                }
            }
            _ => {}
        }
        let affects_preparation = queue
            .preparation
            .as_ref()
            .is_some_and(|gate| match &operation {
                SendQueueOperation::Remove { id } => id == &gate.queued_message_id,
                SendQueueOperation::Move { id, index } => {
                    id == &gate.queued_message_id || *index == 0
                }
                _ => false,
            });
        if affects_preparation {
            if let Some(gate) = &mut queue.preparation {
                gate.state = PreparationState::Cancelled;
            }
            queue.paused = Some(QueuePause::TurnStopped);
        }
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
                queue.turns.push_back(QueuedTurn::new(
                    added_id.clone().expect("Add request identity"),
                    TurnRequest {
                        submission_id,
                        source_runtime_id,
                        conversation_id: conversation_id.to_owned(),
                        runtime_id: target_runtime_id,
                        selection,
                        turn,
                    },
                ));
            }
            SendQueueOperation::Edit { id, text } => {
                let index = queue.editable_position(&id)?;
                let turn = &mut queue.turns[index];
                if text.trim().is_empty() && turn.request.turn.attachments.is_empty() {
                    return Err(RuntimeError::invalid("message text"));
                }
                turn.edit()?.turn.text = text;
                return self.snapshot_result();
            }
            SendQueueOperation::Replace {
                id,
                expected_revision,
                target_runtime_id,
                selection,
                retained_attachments,
                turn,
            } => {
                return self.replace_queued_turn(
                    &id,
                    expected_revision,
                    target_runtime_id,
                    selection,
                    retained_attachments,
                    turn,
                );
            }
            SendQueueOperation::Remove { id } => {
                let index = queue.position(&id)?;
                queue.turns.remove(index);
                if queue
                    .preparation
                    .as_ref()
                    .is_some_and(|gate| gate.queued_message_id == id)
                {
                    // The service observes this exact gate's disappearance and
                    // revokes only its work/token; remaining items stay paused.
                    queue.preparation = None;
                    if queue.waiting == Some(QueueWait::ContextPreparation) {
                        queue.waiting = None;
                    }
                }
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
            SendQueueOperation::Resume => {
                if queue
                    .preparation
                    .as_ref()
                    .is_some_and(|gate| gate.state != PreparationState::Ready)
                {
                    return Err(RuntimeError::invalid(
                        "接续压缩尚未完成；失败或取消后请明确重新准备摘要",
                    ));
                }
                queue.paused = None;
            }
            SendQueueOperation::Withdraw { .. } => {
                return Err(RuntimeError::invalid("撤回排队消息需要共享输入存储"));
            }
            SendQueueOperation::Steer { id } => {
                let index = queue.position(&id)?;
                let turn = queue.turns.remove(index).expect("queued turn");
                if let Err(error) = self.steer(
                    &turn.request.turn,
                    Some(&turn.request.runtime_id),
                    turn.request.selection.as_ref(),
                ) {
                    let queue = &mut self.session.send_queue;
                    queue.turns.insert(index.min(queue.turns.len()), turn);
                    if matches!(&error, RuntimeError::Contract(failure) if failure.code == "runtime_operation_indeterminate")
                    {
                        queue.paused = Some(QueuePause::SendFailed);
                    }
                    return Err(error);
                }
                if self.session.send_queue.turns.is_empty() {
                    self.session.send_queue.paused = None;
                }
            }
        }
        // A queue added to or resumed while nothing runs starts at once.
        if !self.session.compacting
            && !self.turn_running()
            && self.session.send_queue.paused.is_none()
            && self.session.send_queue.waiting.is_none()
        {
            self.send_next_queued();
        }
        if let Some(queue_id) = added_id {
            let queued = self
                .session
                .send_queue
                .turns
                .iter()
                .any(|turn| turn.id == queue_id);
            let receipt = if queued {
                crate::SubmissionReceipt::Queued { queue_id }
            } else {
                crate::SubmissionReceipt::Sending
            };
            self.accepted_submission_result(receipt)
        } else {
            self.snapshot_result().map_err(RuntimeError::after_delivery)
        }
    }

    /// Deliver the instruction and its explicit choices to the active turn.
    pub(crate) fn steer_turn(
        &mut self,
        turn: &ComposedTurn,
        target_runtime_id: Option<&str>,
        selection: Option<&ModelSelection>,
    ) -> Result<SnapshotResult, RuntimeError> {
        self.steer(turn, target_runtime_id, selection)?;
        self.snapshot_result().map_err(RuntimeError::after_delivery)
    }

    fn steer(
        &mut self,
        turn: &ComposedTurn,
        target_runtime_id: Option<&str>,
        requested_model: Option<&ModelSelection>,
    ) -> Result<(), RuntimeError> {
        self.require_input_access()?;
        if !self.can_steer() {
            return Err(RuntimeError::invalid("此运行时当前不能插入消息"));
        }
        let runtime = self.active_instance()?;
        let runtime_id = runtime.id.clone();
        let claude_control =
            runtime.type_id == RuntimeKind::ClaudeCode && turn.permission_mode.is_some();
        let selection = self.session.model_selection.clone().flatten();
        if target_runtime_id.is_some_and(|target| target != runtime_id) {
            return Err(RuntimeError::invalid("当前轮次不能切换运行时，请排队发送此消息"));
        }
        if requested_model.is_some_and(|requested| Some(requested) != selection.as_ref()) {
            return Err(RuntimeError::invalid("当前轮次不能切换模型，请排队发送此消息"));
        }
        if let Some(effort) = turn.effort.as_deref() {
            self.validate_effort(runtime, selection.as_ref(), effort)?;
        }
        if let Some(mode) = turn.permission_mode.as_deref() {
            self.validate_permission_mode(runtime, mode)?;
        }
        if runtime.type_id == RuntimeKind::Pi && turn.effort.is_some() {
            return Err(RuntimeError::invalid(
                "Pi 无法确认插入消息的推理强度，请排队发送此消息",
            ));
        }
        if turn.attachments.is_empty()
            && self.can_submit_native(
                execution_workers::ExecutionCommandKind::Steer,
                &runtime_id,
                &selection,
            )
        {
            if turn.text.trim().is_empty() {
                return Err(RuntimeError::invalid("message text"));
            }
            return self.enqueue_native_submission(
                execution_workers::ExecutionCommandKind::Steer,
                pi::TurnInput {
                    text: turn.text.clone(),
                    effort: turn.effort.clone(),
                    permission_mode: turn.permission_mode.clone(),
                    images: Vec::new(),
                },
            );
        }
        let attachments = self.turn_attachments(self.active_instance()?, &turn.attachments)?;
        let text = super::attachments::compose_text(&turn.text, &attachments.files);
        if text.trim().is_empty() && attachments.images.is_empty() {
            return Err(RuntimeError::invalid("message text"));
        }
        let input = pi::TurnInput {
            text,
            effort: turn.effort.clone(),
            permission_mode: turn.permission_mode.clone(),
            images: super::attachments::turn_images(&attachments.images)?,
        };
        self.retire_native_settings_for_submission();
        if claude_control {
            // Wait for the permission ACK on the existing execution owner.
            // Service submissions can still answer reads while this waits.
            return self.start_live_operation("steer", move |state| {
                state
                    .live_mut()
                    .ok_or_else(|| RuntimeError::invalid("此运行时当前不能插入消息"))?
                    .steer(&input)
                    .map_err(|error| RuntimeError::runtime_operation("插入消息", error))
            });
        }
        self.session
            .active_state
            .live_mut()
            .ok_or_else(|| RuntimeError::invalid("此运行时当前不能插入消息"))?
            .steer(&input)
            .map_err(|error| RuntimeError::runtime_operation("插入消息", error))
    }

    /// The running turn takes messages through the runtime's own channel.
    fn can_steer(&self) -> bool {
        self.session.projected()
            .is_some_and(|snapshot| Self::submission_admission(&self.session, snapshot).can_steer)
    }

    /// The addressed session's turn ended with `outcome`: the queue goes on
    /// after a normal end and waits for the user otherwise.
    pub(super) fn follow_finished_turn(&mut self, outcome: crate::TurnOutcome) {
        let withdrawn = self
            .submission_cancellation
            .as_ref()
            .is_some_and(|(id, flag)| {
                self.session.conversation_id() == Some(id.as_str())
                    && flag.load(std::sync::atomic::Ordering::Acquire)
            });
        let queue = &mut self.session.send_queue;
        if withdrawn {
            queue.pause_after_stop();
            return;
        }
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
    pub(super) fn queue_after_work(&mut self, request: TurnRequest) -> String {
        let id = queued_id();
        let queue = &mut self.session.send_queue;
        queue.turns.push_back(QueuedTurn::new(id.clone(), request));
        queue.waiting = Some(QueueWait::SubAgents);
        id
    }

    /// The addressed session's runtime process does nothing more: a queue
    /// waiting for its work goes on.
    pub(super) fn follow_ended_work(&mut self) {
        if let Err(error) = self.advance_conversation_handoff() {
            tracing::warn!(target: "velune_application", event = "conversation_handoff_progress_failed", detail = %error);
        }
        if self.current_handoff_blocks_dispatch() {
            return;
        }
        let queue = &mut self.session.send_queue;
        if queue.waiting.take().is_some() && queue.paused.is_none() {
            self.send_next_queued();
        }
    }

    /// Maintenance holds delivery, but accepts edits and new queued input.
    pub(super) fn follow_finished_compaction(&mut self, failed: bool) {
        let outcome = self.session.projected().and_then(|snapshot| {
            let activity = snapshot.executions.current.as_ref()?;
            snapshot
                .executions
                .records
                .iter()
                .find(|record| {
                    record.id == activity.execution_id
                        && record.kind == velune_conversation::ExecutionKind::ContextMaintenance
                })
                .and_then(|record| record.outcome)
        });
        let queue = &mut self.session.send_queue;
        if self.session.cancel_requested || outcome == Some(crate::TurnOutcome::Cancelled) {
            queue.pause_after_stop();
        } else if failed || outcome == Some(crate::TurnOutcome::Failed) {
            queue.interrupted = Some(QueuePause::TurnFailed);
            queue.paused.get_or_insert(QueuePause::TurnFailed);
        } else if queue.paused.is_none() && !queue.turns.is_empty() {
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
        if self.session.native_controls.submission_pending() {
            self.session
                .send_queue
                .waiting
                .get_or_insert(QueueWait::Control);
            return;
        }
        if self.session.send_queue.preparation.is_some() {
            self.session.send_queue.waiting = Some(QueueWait::ContextPreparation);
            return;
        }
        if self.session.compacting {
            return;
        }
        if self.current_handoff_blocks_dispatch() {
            let queue = &mut self.session.send_queue;
            if !queue.turns.is_empty() && queue.paused.is_none() {
                queue.waiting.get_or_insert(QueueWait::Control);
            }
            return;
        }
        if self.submission_context().is_some_and(|snapshot| {
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
            Err(RuntimeError::Contract(failure)) if failure.code == "control_held_elsewhere" => {
                self.control_held_elsewhere();
                let queue = &mut self.session.send_queue;
                queue.turns.push_front(turn);
                queue.waiting = Some(QueueWait::Control);
            }
            Err(error) if error.delivery_accepted() => {
                tracing::warn!(target: "velune_application", event = "queued_turn_delivery_accepted_refresh_failed", detail = %error);
                self.session.send_queue.paused = Some(QueuePause::SendFailed);
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
        session.side.is_none()
            && Self::submission_admission(session, snapshot).send.is_ok()
            && self
                .runtime_instance(&snapshot.context_runtime_id)
                .is_ok_and(|runtime| {
                    runtime.enabled
                        && !matches!(
                            velune_agent_runtime::version::variant(runtime.type_id).resume,
                            velune_agent_runtime::version::ResumeStrategy::Unsupported
                        )
                })
            && !session.work_pending()
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
        let admission = Self::submission_admission(&self.session, snapshot);
        FollowUp {
            context_preparation: queue.preparation.as_ref().map(|gate| {
                crate::conversation::ContextPreparation {
                    queued_message_id: gate.queued_message_id.clone(),
                    work_id: gate.work_id.clone(),
                    target_runtime_id: gate.target_runtime_id.clone(),
                    is_preparing: matches!(
                        gate.state,
                        PreparationState::Running | PreparationState::Cancelling
                    ),
                    is_dispatching: gate.dispatching,
                    can_edit_message: !gate.dispatching && !gate.request_bound,
                    can_retry_summary: matches!(
                        gate.state,
                        PreparationState::Failed | PreparationState::Cancelled
                    ),
                }
            }),
            control_handoff: snapshot.follow_up.control_handoff,
            queue: queue.turns.iter().map(QueuedTurn::public).collect(),
            paused: queue.paused.clone(),
            waiting: queue.waiting.clone(),
            can_stop_sub_agents: self.session.work().is_ok_and(|work| {
                work.iter()
                    .any(|task| task.state != velune_agent_runtime::work::BackgroundState::Stopping)
            }),
            can_queue: admission.offer_queue,
            can_steer: admission.can_steer,
            can_resume: self.resumable(&self.session, snapshot),
            is_compacting: self.session.compacting,
            can_compact: admission.send.is_ok()
                && self
                    .runtime_instance(&snapshot.context_runtime_id)
                    .is_ok_and(|runtime| {
                        runtime.enabled
                            && velune_agent_runtime::version::variant(runtime.type_id).can_compact
                    })
                && !self.session.work_pending()
                && !snapshot.messages.is_empty(),
            editable: self.editable_messages(snapshot),
            can_fork: self.forkable(snapshot),
        }
    }
}

#[cfg(all(test, unix))]
mod maintenance_queue_tests {
    use super::*;

    fn drain_until(core: &mut CoreRuntime, ready: impl Fn(&CoreRuntime) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        loop {
            core.drain_runtime().unwrap();
            if ready(core) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Pi maintenance did not reach the expected confirmed state"
            );
            std::thread::yield_now();
        }
    }

    /// Exercise the actual Pi RPC reader, projection and core drain boundary:
    /// queue admission while compacting is not enough to prove delivery safety.
    #[test]
    fn compaction_queue_lifecycle_dispatches_only_after_uninterrupted_success() {
        for mode in ["success", "failed", "cancelled", "cancel-race", "paused"] {
            let mut nonce = [0; 8];
            getrandom::fill(&mut nonce).unwrap();
            let root = std::env::temp_dir().join(format!(
                "velune-maintenance-queue-{:x}",
                u64::from_le_bytes(nonce)
            ));
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("mode"), mode).unwrap();
            let helper = root.join("rpc.py");
            std::fs::write(&helper, r#"
import json, pathlib, sys, threading, time
root = pathlib.Path(__file__).parent
mode = (root / 'mode').read_text()
history = {'messages':[{'id':'source-user','role':'user','content':'source history'}]}
lock = threading.Lock()
def emit(value):
    with lock:
        print(json.dumps(value), flush=True)
def complete(request):
    while not (root / 'release').exists(): time.sleep(.005)
    aborted = mode == 'cancelled'
    emit({'type':'compaction_end','aborted':aborted,'result':{} if mode != 'failed' else None})
    emit({'type':'response','id':request['id'],'command':'compact','success':mode != 'failed','data':{},'error':'synthetic failure'})
for line in sys.stdin:
    request = json.loads(line)
    command = request['type']
    data = {}
    if command == 'compact':
        emit({'type':'compaction_start'})
        threading.Thread(target=complete, args=(request,), daemon=True).start()
        continue
    if command == 'get_commands': data = {'commands':[]}
    elif command == 'prompt':
        if request.get('message','').startswith('/velune-projection-sync '):
            token = request['message'].split(' ',1)[1]
            assert token == request['id']
            messages = history['messages']
            emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.session-metadata',
                  'statusText':json.dumps({'requestId':token,'snapshotVersion':1,'messages':messages,
                      'messageIds':[m['id'] for m in messages],'identityConfirmations':[],
                      'sessionFile':str(root/'session.jsonl'),'name':'Synthetic','cwd':str(root)})})
        else:
            with (root / 'sent').open('a') as output:
                output.write(request['message'] + '\n')
            (root / 'sent-recorded').touch()
        data = {'disposition':'handled'}
    elif command == 'get_messages': data = history
    elif command == 'abort': (root / 'release').write_text('release')
    emit({'type':'response','id':request['id'],'command':command,'success':True,'data':data})
"#).unwrap();
            let mut application = crate::Application::open(crate::Options {
                home_directory: root.clone(),
                resources_directory: root.clone(),
            })
            .unwrap();
            let core = &mut application.runtime;
            core.runtime_instances = vec![RuntimeInstance {
                enabled: true,
                id: "pi".into(),
                name: "Synthetic Pi".into(),
                type_id: RuntimeKind::Pi,
                gateway_id: String::new(),
                settings: Default::default(),
            }];
            let config: PiConfig = serde_json::from_value(json!({
                "execution":{"kind":"local"}, "os_home":root,
                "binary":"/usr/bin/python3", "node_binary":"/usr/bin/python3", "rpc_entry":helper,
            }))
            .unwrap();
            let client = pi::Client::spawn(config.clone()).unwrap();
            let conversation_id = format!("pi:{}", root.join("session.jsonl").display());
            let mut projection = PiProjection::new(ConversationSummary {
                native_read_state: None,
                id: conversation_id.clone(),
                runtime_id: "pi".into(),
                title: crate::conversation::ConversationTitle::Untitled,
                cwd: Some(root.to_string_lossy().into_owned()),
                created_at_unix_ms: None,
                updated_at_unix_ms: None,
                can_rename: false,
                can_delete: false,
                parent_conversation_id: None,
            });
            projection.snapshot.as_mut().unwrap().messages = serde_json::from_value(json!([
                {"id":"source-user","role":"user","completed":true,"blocks":[{"kind":"text","text":"source history"}]}
            ])).unwrap();
            let mut session = pi::PiSession::new(config);
            session.start(client, projection);
            core.session.active_state = ActiveState::Pi(Box::new(session));
            core.session.execution_runtime_id = Some("pi".into());
            core.session.model_selection = Some(None);
            assert!(
                core.session.work_pending(),
                "startup discovery is not settled"
            );
            drain_until(core, |core| !core.session.work_pending());
            assert_eq!(
                core.native_snapshot().unwrap().messages[0].id,
                "source-user"
            );
            if mode == "paused" {
                core.session.send_queue.paused = Some(QueuePause::TurnFailed);
            }
            core.compact_conversation("pi", None).unwrap();
            assert!(core.session.compacting, "{mode}");
            core.send_queue(
                &conversation_id,
                SendQueueOperation::Add {
                    source_runtime_id: "pi".into(),
                    target_runtime_id: "pi".into(),
                    selection: None,
                    turn: ComposedTurn {
                        text: "queued after maintenance".into(),
                        attachments: vec![],
                        effort: None,
                        permission_mode: None,
                    },
                },
            )
            .unwrap();
            assert_eq!(core.session.send_queue.turns.len(), 1);
            assert!(
                !root.join("sent").exists(),
                "{mode}: sent during maintenance"
            );
            if matches!(mode, "cancelled" | "cancel-race") {
                core.cancel("pi").unwrap();
            } else {
                std::fs::write(root.join("release"), "release").unwrap();
            }
            drain_until(core, |core| !core.session.compacting);
            if mode == "success" {
                drain_until(core, |core| {
                    core.session.send_queue.turns.is_empty() && root.join("sent-recorded").exists()
                });
                assert!(core.session.send_queue.turns.is_empty());
                assert!(core.session.send_queue.paused.is_none());
                assert_eq!(
                    std::fs::read_to_string(root.join("sent")).unwrap(),
                    "queued after maintenance\n"
                );
            } else {
                assert_eq!(
                    core.session.send_queue.turns.len(),
                    1,
                    "{mode}: queue was consumed"
                );
                assert!(!root.join("sent").exists(), "{mode}: queue was dispatched");
                assert_eq!(
                    core.session.send_queue.paused,
                    Some(if matches!(mode, "cancelled" | "cancel-race") {
                        QueuePause::TurnStopped
                    } else {
                        QueuePause::TurnFailed
                    }),
                    "{mode}"
                );
            }
            core.shutdown_active().unwrap();
            application.close_stopping_work().unwrap();
            drop(application);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

#[cfg(test)]
mod preparation_tests {
    use super::*;
    struct Fixture {
        root: PathBuf,
        application: crate::Application,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "velune-queued-preparation-{}",
                crate::new_record_key()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let mut application = crate::Application::open(crate::Options {
                home_directory: root.clone(),
                resources_directory: root.clone(),
            })
            .unwrap();
            application.runtime.runtime_instances = vec![RuntimeInstance {
                id: "source".into(),
                enabled: true,
                name: "Synthetic".into(),
                type_id: RuntimeKind::Pi,
                gateway_id: String::new(),
                settings: Default::default(),
            }];
            let snapshot: crate::conversation::ConversationSnapshot=serde_json::from_value(json!({"revision":1,"conversation":{"id":"source:one","runtimeId":"source","title":{"source":"untitled"}},"contextRuntimeId":"source","resourceId":null,"modelRecordKey":null,"runState":"idle","messages":[],"transcriptTurns":[],"transcriptItems":[],"transcriptOutline":[],"transcriptMessageIdentities":[],"actions":{"canSend":true,"canCancel":false,"canSwitch":true}})).unwrap();
            application.runtime.session.set_history(snapshot);
            let request = TurnRequest {
                submission_id: None,
                source_runtime_id: "source".into(),
                conversation_id: "source:one".into(),
                runtime_id: "target".into(),
                selection: None,
                turn: ComposedTurn {
                    text: "first".into(),
                    attachments: vec![],
                    effort: None,
                    permission_mode: None,
                },
            };
            let queue = &mut application.runtime.session.send_queue;
            queue
                .turns
                .push_back(QueuedTurn::new("head".into(), request.clone()));
            let mut tail = request;
            tail.turn.text = "tail".into();
            queue.turns.push_back(QueuedTurn::new("tail".into(), tail));
            queue.preparation = Some(ContextPreparationGate {
                work_id: "work-1".into(),
                queued_message_id: "head".into(),
                target_runtime_id: "target".into(),
                state: PreparationState::Running,
                dispatching: false,
                request_bound: false,
            });
            queue.waiting = Some(QueueWait::ContextPreparation);
            Self { root, application }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.application.close_stopping_work();
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn withdraw_command(expected_revision: u64) -> crate::Command {
        crate::Command::EditSendQueue {
            submission_id: None,
            runtime_id: "source".into(),
            conversation_id: "source:one".into(),
            operation: SendQueueOperation::Withdraw {
                id: "head".into(),
                expected_revision,
            },
        }
    }

    #[test]
    fn withdrawing_queued_turn_restores_complete_composer_on_execution_owner() {
        use velune_application_api::{ComposerContextKey, ComposerInputOperation};

        let mut fixture = Fixture::new();
        let new_file = fixture.root.join("new.txt");
        fs::write(&new_file, b"new attachment").unwrap();
        let view = crate::ClientViewId("synthetic-withdraw".into());
        let application = &mut fixture.application;
        application
            .composer
            .legacy_edit("source:one".into(), Some("original draft".into()), None)
            .unwrap();
        let existing = application
            .composer
            .upload("existing.txt".into(), b"existing attachment".to_vec())
            .unwrap();
        let owner_id = application.composer.owner_id();
        let context = ComposerContextKey::Conversation {
            owner_id: owner_id.clone(),
            conversation_id: "source:one".into(),
        };
        let old = application
            .composer
            .operate(ComposerInputOperation::Read {
                since_revision: None,
            })
            .unwrap();
        let old_id = &old
            .bindings
            .iter()
            .find(|item| item.context == context)
            .unwrap()
            .input_id;
        let old_path = application.composer.attachment_path(&existing.id).unwrap();
        let turn = &mut application.runtime.session.send_queue.turns[0].request;
        let selection = ModelSelection::Native {
            provider_id: Some("synthetic".into()),
            model_id: "synthetic-model".into(),
        };
        turn.selection = Some(selection.clone());
        turn.turn.effort = Some("high".into());
        turn.turn.permission_mode = Some("read-only".into());
        turn.turn.attachments = vec![
            old_path.to_str().unwrap().into(),
            new_file.to_str().unwrap().into(),
        ];
        application.runtime.session.send_queue.waiting = Some(QueueWait::ContextPreparation);

        let crate::Response::EditSendQueue(result) = application
            .execute_command(withdraw_command(1), true, &view)
            .unwrap()
        else {
            panic!("withdrawal returned the wrong response");
        };
        let follow_up = result.snapshot.unwrap().follow_up;
        assert_eq!(follow_up.paused, Some(QueuePause::TurnStopped));
        assert_eq!(follow_up.queue.len(), 1);
        assert_eq!(follow_up.queue[0].id, "tail");
        assert!(follow_up.context_preparation.is_none());
        assert_ne!(follow_up.waiting, Some(QueueWait::ContextPreparation));

        let state = application
            .composer
            .operate(ComposerInputOperation::Read {
                since_revision: None,
            })
            .unwrap();
        assert_eq!(state.revision, old.revision + 1);
        let binding = state
            .bindings
            .iter()
            .find(|item| item.context == context)
            .unwrap();
        assert_ne!(&binding.input_id, old_id);
        assert!(
            state
                .inputs
                .iter()
                .any(|item| &item.id == old_id && item.content.text == "original draft")
        );
        let restored = state
            .inputs
            .iter()
            .find(|item| item.id == binding.input_id)
            .unwrap();
        assert_eq!(restored.content.text, "first");
        assert_eq!(restored.content.attachments.len(), 2);
        assert_eq!(restored.content.attachments[0], existing);
        let imported = &restored.content.attachments[1];
        assert_eq!(imported.name, "new.txt");
        let crate::Response::ReadComposerAttachment(content) = application
            .execute_command(
                crate::Command::ReadComposerAttachment {
                    attachment_id: imported.id.clone(),
                },
                true,
                &view,
            )
            .unwrap()
        else {
            panic!("composer attachment returned the wrong response");
        };
        assert_eq!(content.bytes, b"new attachment");
        let target = application.composer.target(&context).unwrap();
        assert_eq!(
            target.choices.runtime_owner_id.as_deref(),
            Some(owner_id.as_str())
        );
        assert_eq!(target.choices.runtime_id.as_deref(), Some("target"));
        let choice = &target.choices.choices[0].choice;
        assert_eq!(choice.model, Some(selection));
        assert_eq!(choice.effort.as_deref(), Some("high"));
        assert_eq!(choice.permission_mode.as_deref(), Some("read-only"));
    }

    #[test]
    fn withdrawing_stale_or_dispatching_queue_item_leaves_composer_and_queue_untouched() {
        use velune_application_api::ComposerInputOperation;

        let mut fixture = Fixture::new();
        let application = &mut fixture.application;
        let view = crate::ClientViewId("synthetic-withdraw".into());
        let read = ComposerInputOperation::Read {
            since_revision: None,
        };
        let before = application.composer.operate(read.clone()).unwrap();
        application
            .runtime
            .send_queue(
                "source:one",
                SendQueueOperation::Edit {
                    id: "head".into(),
                    text: "new revision".into(),
                },
            )
            .unwrap();
        let error = application
            .execute_command(withdraw_command(1), false, &view)
            .err()
            .expect("stale withdrawal was accepted");
        assert_eq!(error.code, "queued_message_changed");
        assert_eq!(
            application.runtime.queued_preparation_state("work-1"),
            Some(PreparationState::Running)
        );
        application
            .runtime
            .sync_queued_preparation("work-1", PreparationState::Ready);
        assert!(
            application
                .runtime
                .queued_prepared_candidate("work-1")
                .is_some()
        );
        assert!(
            application
                .execute_command(withdraw_command(2), false, &view)
                .is_err()
        );
        application.runtime.pause_queued_preparation("work-1");
        assert!(
            application
                .execute_command(withdraw_command(2), false, &view)
                .is_err()
        );
        assert_eq!(
            application.runtime.session.send_queue.turns[0]
                .request
                .turn
                .text,
            "new revision"
        );
        assert_eq!(application.composer.operate(read).unwrap(), before);
    }

    #[test]
    fn withdrawing_queue_item_keeps_it_and_cleans_imports_if_saving_fails() {
        use velune_application_api::ComposerInputOperation;

        let mut fixture = Fixture::new();
        let first = fixture.root.join("first.txt");
        let second = fixture.root.join("second.txt");
        fs::write(&first, b"first").unwrap();
        let application = &mut fixture.application;
        let view = crate::ClientViewId("synthetic-withdraw".into());
        let read = ComposerInputOperation::Read {
            since_revision: None,
        };
        let before = application.composer.operate(read.clone()).unwrap();
        application.runtime.session.send_queue.turns[0]
            .request
            .turn
            .attachments = vec![
            first.to_str().unwrap().into(),
            second.to_str().unwrap().into(),
        ];
        let attached = fixture.root.join("composer-attachments");
        assert!(
            application
                .execute_command(withdraw_command(1), false, &view)
                .is_err()
        );
        assert_eq!(fs::read_dir(&attached).unwrap().count(), 0);

        fs::write(&second, b"second").unwrap();
        // Prevent creating the temporary catalog without changing the stored catalog.
        fs::create_dir(fixture.root.join("composer-inputs.json.tmp")).unwrap();
        assert!(
            application
                .execute_command(withdraw_command(1), false, &view)
                .is_err()
        );
        assert_eq!(fs::read_dir(attached).unwrap().count(), 0);
        assert_eq!(application.composer.operate(read).unwrap(), before);
        assert_eq!(application.runtime.session.send_queue.turns[0].id, "head");
        assert_eq!(
            application.runtime.queued_preparation_state("work-1"),
            Some(PreparationState::Running)
        );
    }

    #[test]
    fn replacing_queue_input_keeps_slot_and_pause_and_rejects_stale_or_invalid_edits() {
        let mut fixture = Fixture::new();
        let retained = fixture.root.join("retained.txt");
        let added = fixture.root.join("new.txt");
        std::fs::write(&retained, "retained attachment").unwrap();
        std::fs::write(&added, "new attachment").unwrap();
        let retained = retained.to_string_lossy().into_owned();
        let added = added.to_string_lossy().into_owned();
        let core = &mut fixture.application.runtime;
        core.runtime_instances.push(RuntimeInstance {
            id: "codex".into(),
            enabled: true,
            name: "Synthetic Codex".into(),
            type_id: RuntimeKind::Codex166,
            gateway_id: String::new(),
            settings: Default::default(),
        });
        let queue = &mut core.session.send_queue;
        queue.paused = Some(QueuePause::TurnStopped);
        let head = queue.turns[0].public();
        let tail = &mut queue.turns[1].request;
        tail.submission_id = Some("original submission".into());
        tail.turn.attachments = vec!["/synthetic/removed.txt".into(), retained.clone()];
        let model = ModelSelection::Native {
            provider_id: None,
            model_id: "synthetic-model".into(),
        };
        let replace =
            |expected_revision, retained_attachments, path: String| SendQueueOperation::Replace {
                id: "tail".into(),
                expected_revision,
                target_runtime_id: "codex".into(),
                selection: Some(model.clone()),
                retained_attachments,
                turn: ComposedTurn {
                    text: "edited tail".into(),
                    attachments: vec![path],
                    effort: Some("high".into()),
                    permission_mode: Some("read-only".into()),
                },
            };
        let tail_before = core.session.send_queue.turns[1].public();
        for invalid in [
            replace(1, vec![1, 1], added.clone()),
            replace(1, vec![2], added.clone()),
            replace(1, vec![1], "relative.txt".into()),
        ] {
            assert!(core.send_queue("source:one", invalid).is_err());
            assert_eq!(core.session.send_queue.turns[1].public(), tail_before);
        }
        let snapshot = core
            .send_queue_submission(
                "source:one",
                replace(1, vec![1], added.clone()),
                Some("edit transport only".into()),
            )
            .unwrap()
            .snapshot
            .unwrap();
        let queue = &snapshot.follow_up.queue;
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0], head);
        assert_eq!(queue[1].id, "tail");
        assert_eq!(
            queue[1].submission_id.as_deref(),
            Some("original submission")
        );
        assert_eq!(queue[1].text, "edited tail");
        assert_eq!(queue[1].attachments, [retained, added.clone()]);
        assert_eq!(
            queue[1].edit_context,
            Some(QueuedMessageEditContext {
                revision: 2,
                runtime_id: "codex".into(),
                selection: Some(model.clone()),
                effort: Some("high".into()),
                permission_mode: Some("read-only".into()),
            })
        );
        assert_eq!(snapshot.follow_up.paused, Some(QueuePause::TurnStopped));
        assert_eq!(
            snapshot.follow_up.waiting,
            Some(QueueWait::ContextPreparation)
        );
        assert_eq!(
            core.session.send_queue.turns[1].request.source_runtime_id,
            "source"
        );

        core.send_queue(
            "source:one",
            SendQueueOperation::Edit {
                id: "tail".into(),
                text: "legacy change".into(),
            },
        )
        .unwrap();
        let stale = core.send_queue("source:one", replace(2, vec![0], added.clone()));
        assert!(
            matches!(stale, Err(RuntimeError::Contract(failure)) if failure.code == "queued_message_changed")
        );
        let tail = &core.session.send_queue.turns[1];
        assert_eq!(tail.edit_revision, 3);
        assert_eq!(tail.request.turn.text, "legacy change");
        let request = tail.request.clone();
        core.send_queue(
            "source:one",
            SendQueueOperation::Remove { id: "tail".into() },
        )
        .unwrap();
        let new_id = core.queue_after_work(request);
        assert_ne!(new_id, "tail");
        assert!(
            core.send_queue("source:one", replace(3, vec![0], added))
                .is_err()
        );
        assert_eq!(core.session.send_queue.turns[1].id, new_id);
    }

    #[test]
    fn replacing_prepared_queue_input_keeps_target_and_never_reuses_bound_input() {
        let mut fixture = Fixture::new();
        let core = &mut fixture.application.runtime;
        let queue = &mut core.session.send_queue;
        // This configured destination is intentionally gone. An edit may
        // still preserve the accepted choices and attachments verbatim.
        queue.turns[0].request.turn.attachments = vec!["/synthetic/captured.txt".into()];
        let replace = |revision, runtime: &str, effort: Option<&str>| SendQueueOperation::Replace {
            id: "head".into(),
            expected_revision: revision,
            target_runtime_id: runtime.into(),
            selection: None,
            retained_attachments: vec![0],
            turn: ComposedTurn {
                text: "edited before binding".into(),
                attachments: vec![],
                effort: effort.map(str::to_owned),
                permission_mode: None,
            },
        };
        for invalid in [
            replace(1, "source", None),
            replace(1, "target", Some("high")),
        ] {
            assert!(core.send_queue("source:one", invalid).is_err());
        }
        core.send_queue("source:one", replace(1, "target", None))
            .unwrap();
        assert_eq!(
            core.queued_preparation_state("work-1"),
            Some(PreparationState::Running)
        );
        core.sync_queued_preparation("work-1", PreparationState::Ready);
        let prepared = core.queued_prepared_candidate("work-1").unwrap();
        assert_eq!(prepared.turn.text, "edited before binding");
        assert_eq!(prepared.turn.attachments, ["/synthetic/captured.txt"]);
        assert!(
            core.send_queue("source:one", replace(2, "target", None))
                .is_err()
        );
        core.pause_queued_preparation("work-1");
        assert!(
            core.send_queue("source:one", replace(2, "target", None))
                .is_err()
        );
        assert_eq!(core.session.send_queue.turns[0].edit_revision, 2);
        core.unbind_queued_preparation("work-1");
        core.send_queue("source:one", replace(2, "target", None))
            .unwrap();
        assert_eq!(core.session.send_queue.turns[0].edit_revision, 3);
        assert_eq!(core.session.send_queue.paused, Some(QueuePause::SendFailed));
    }

    #[test]
    fn submission_receipt_survives_missing_presentation_after_queue_acceptance() {
        let mut fixture = Fixture::new();
        let core = &mut fixture.application.runtime;
        let mut request = core.queued_handoff_turn("source:one", "head").unwrap();
        request.submission_id = Some("accepted-without-presentation".into());
        let queue_id = core.queue_after_work(request);
        // Queue acceptance belongs to the session even if its presentation
        // becomes unavailable. The response must not invite a fresh submit.
        core.session.active_state = ActiveState::default();
        let result = core
            .accepted_submission_result(crate::SubmissionReceipt::Queued {
                queue_id: queue_id.clone(),
            })
            .unwrap();
        assert!(result.snapshot.is_none());
        assert_eq!(
            result.submission_receipt,
            Some(crate::SubmissionReceipt::Queued { queue_id })
        );
        assert_eq!(core.session.send_queue.turns.len(), 3);
    }

    #[test]
    fn submission_tracking_does_not_change_prepared_turn_identity_or_input() {
        let mut fixture = Fixture::new();
        let core = &mut fixture.application.runtime;
        let original = core.queued_handoff_turn("source:one", "head").unwrap();
        let mut tracked = original.clone();
        tracked.submission_id = Some("new-transport-attempt".into());
        assert!(same_turn(&original, &tracked));
        assert_eq!(
            serde_json::to_value(&original.turn).unwrap(),
            serde_json::to_value(&tracked.turn).unwrap()
        );
        assert_eq!(original.selection, tracked.selection);
        // Prepared summary reuse binds semantic turn/configuration, not its
        // client-side transport attempt identity.
        core.sync_queued_preparation("work-1", PreparationState::Ready);
        core.take_prepared_queue_turn("work-1", &tracked, Some("head"))
            .unwrap();
    }

    #[test]
    fn preparation_gate_keeps_native_idle_and_admits_queue_without_dispatching_or_paid_text_retry()
    {
        let mut fixture = Fixture::new();
        let core = &mut fixture.application.runtime;
        let native = core.native_snapshot().unwrap();
        let admission = CoreRuntime::submission_admission(&core.session, &native);
        assert!(
            admission.send.is_err()
                && admission.accept_queue.is_ok()
                && admission.offer_queue
                && !admission.can_steer
        );
        core.send_next_queued();
        assert_eq!(core.session.send_queue.turns.len(), 2);
        core.send_queue(
            "source:one",
            SendQueueOperation::Edit {
                id: "head".into(),
                text: "edited before binding".into(),
            },
        )
        .unwrap();
        assert_eq!(
            core.session
                .send_queue
                .preparation
                .as_ref()
                .unwrap()
                .work_id,
            "work-1"
        );
        assert_eq!(
            core.session.send_queue.turns[0].request.turn.text,
            "edited before binding"
        );
        assert!(
            core.require_queued_handoff_retry("source:one", "head")
                .is_err(),
            "active work cannot replace its cancellation handle"
        );
        core.sync_queued_preparation("work-1", PreparationState::Ready);
        assert!(core.queued_prepared_candidate("work-1").is_some());
        assert!(
            core.queued_prepared_candidate("work-1").is_none(),
            "only one prepared dispatch may be outstanding"
        );
        assert!(
            core.send_queue(
                "source:one",
                SendQueueOperation::Edit {
                    id: "head".into(),
                    text: "must not send this late edit".into()
                }
            )
            .is_err()
        );
        core.pause_queued_preparation("work-1");
        assert!(
            core.send_queue(
                "source:one",
                SendQueueOperation::Edit {
                    id: "head".into(),
                    text: "bound input stays locked".into()
                }
            )
            .is_err()
        );
        core.unbind_queued_preparation("work-1");
        core.send_queue(
            "source:one",
            SendQueueOperation::Edit {
                id: "head".into(),
                text: "safe pre-target retry edit".into(),
            },
        )
        .unwrap();
        assert_eq!(core.native_snapshot().unwrap().run_state, RunState::Idle);
    }
    #[test]
    fn removing_or_reordering_the_owned_head_revokes_only_that_gate_and_late_worker_cannot_send() {
        let mut fixture = Fixture::new();
        let core = &mut fixture.application.runtime;
        assert!(
            core.send_queue(
                "source:one",
                SendQueueOperation::Move {
                    id: "missing".into(),
                    index: 0
                }
            )
            .is_err()
        );
        assert_eq!(
            core.queued_preparation_state("work-1"),
            Some(PreparationState::Running)
        );
        core.send_queue(
            "source:one",
            SendQueueOperation::Move {
                id: "tail".into(),
                index: 1,
            },
        )
        .unwrap();
        assert_eq!(
            core.queued_preparation_state("work-1"),
            Some(PreparationState::Running)
        );
        let request = core.queued_handoff_turn("source:one", "head").unwrap();
        core.send_queue(
            "source:one",
            SendQueueOperation::Remove { id: "head".into() },
        )
        .unwrap();
        assert_eq!(core.session.send_queue.turns.front().unwrap().id, "tail");
        assert!(core.session.send_queue.paused.is_some());
        assert!(
            core.take_prepared_queue_turn("work-1", &request, Some("head"))
                .is_err(),
            "removed queued work cannot use the legacy direct-send path"
        );
    }
    #[test]
    fn failed_or_cancelled_work_keeps_head_and_tail_and_session_owner_survives_view_changes() {
        for state in [PreparationState::Failed, PreparationState::Cancelled] {
            let mut fixture = Fixture::new();
            let core = &mut fixture.application.runtime;
            core.sync_queued_preparation("work-1", state);
            assert!(core.session.send_queue.paused.is_some());
            assert_eq!(core.session.send_queue.turns.len(), 2);
            core.require_queued_handoff_retry("source:one", "head")
                .unwrap();
            assert!(
                core.session.pinned(),
                "pending input is owned by core, independently of its windows"
            );
            let owned = std::mem::take(&mut core.session);
            core.parked.push(owned);
            assert_eq!(
                core.queued_handoff_turn("source:one", "head")
                    .unwrap()
                    .turn
                    .text,
                "first"
            );
            assert_eq!(core.queued_preparation_state("work-1"), Some(state));
        }
    }
}
