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
    // A client's open editor can outlive a disconnected host's process.
    format!("queued-{}", crate::new_record_key())
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
        self.send_queue_submission(conversation_id, operation, None)
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
        let index = queue.position(id)?;
        let queued = &queue.turns[index];
        if queued.edit_revision != expected_revision {
            let mut failure = crate::Failure::new(
                crate::FailureKind::Conflict,
                "此排队消息已被修改，请重新打开编辑并核对内容",
            );
            failure.code = "queued_message_changed".into();
            return Err(RuntimeError::Contract(failure));
        }
        let previous = &queued.request;
        let target_changed = previous.runtime_id != target_runtime_id;
        let config_changed = target_changed
            || previous.selection != selection
            || previous.turn.effort != turn.effort
            || previous.turn.permission_mode != turn.permission_mode;
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
        if self.session.compacting {
            return Err(RuntimeError::invalid("上下文压缩期间不能修改发送队列"));
        }
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
                let index = queue.position(&id)?;
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
            self.snapshot_result()
        }
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
        self.native_snapshot()
            .is_some_and(|snapshot| Self::submission_admission(&self.session, &snapshot).can_steer)
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

    /// A turn starts in the addressed session.
    pub(super) fn follow_started_turn(&mut self) {
        self.session.send_queue.interrupted = None;
    }

    /// Send the first queued message as its own turn. One that is not
    /// accepted stays first and the queue waits; it is never resent unasked.
    fn send_next_queued(&mut self) {
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
            && session.work_settled()
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
            control_handoff: snapshot.follow_up.control_handoff,
            queue: queue.turns.iter().map(QueuedTurn::public).collect(),
            paused: queue.paused.clone(),
            waiting: queue.waiting.clone(),
            can_stop_sub_agents: self
                .session
                .work()
                .iter()
                .any(|task| task.state != velune_agent_runtime::work::BackgroundState::Stopping),
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
                && self.session.work_settled()
                && !snapshot.messages.is_empty(),
            editable: self.editable_messages(snapshot),
            can_fork: self.forkable(snapshot),
        }
    }
}

#[cfg(test)]
mod submission_tests;

#[cfg(test)]
mod edit_tests {
    use super::*;
    struct Fixture {
        root: PathBuf,
        application: crate::Application,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("velune-queued-edit-{}", crate::new_record_key()));
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
            let snapshot=serde_json::from_value(json!({"revision":1,"conversation":{"id":"source:one","runtimeId":"source","title":{"source":"untitled"}},"contextRuntimeId":"source","resourceId":null,"modelRecordKey":null,"runState":"idle","messages":[],"transcriptTurns":[],"transcriptItems":[],"transcriptOutline":[],"transcriptMessageIdentities":[],"actions":{"canSend":true,"canCancel":false,"canSwitch":true}})).unwrap();
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
            queue.waiting = Some(QueueWait::SubAgents);
            Self { root, application }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.application.close_stopping_work();
            let _ = std::fs::remove_dir_all(&self.root);
        }
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
        assert_eq!(snapshot.follow_up.waiting, Some(QueueWait::SubAgents));
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
}
