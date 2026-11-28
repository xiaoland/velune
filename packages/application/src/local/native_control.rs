//! Co-located Codex commands retain their protocol owner; only terminal cleanup
//! transfers the adapter to the bounded execution workers.
use super::*;
use execution_workers::{ExecutionCommandKind, ExecutionSuccess, ExecutionTicket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use velune_agent_runtime::creation::Retained;
use velune_agent_runtime::native::{
    CodexCommand, CodexDelivery, CodexInterruptDelivery, CodexOperationTicket,
};

#[derive(Default)]
pub(super) struct NativeControls {
    operations: Vec<Operation>,
    accepted_settings: Option<AcceptedSettings>,
    created_draft: Option<CreatedDraft>,
    pub(super) enabled: bool,
    pub(super) cleanup_required: bool,
}

struct Operation {
    application: ExecutionTicket,
    native: Arc<Retained<CodexOperationTicket>>,
    source: runtime_source::RuntimeSourceKey,
    deadline: Instant,
    effect: Effect,
}

enum Effect {
    CancelParent,
    CancelBackground,
    StopBackground,
    StopChild,
    Reply,
    Submission(Box<NativeSubmission>),
}

/// Per-process creation authority; never persisted or transferred to a new
/// draft just because the harness reused the same public/native identity.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct CreationToken(u64);

impl CreationToken {
    fn next() -> Option<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .ok()
        .map(Self)
    }
}

struct CreatedDraft {
    token: CreationToken,
    session: session::SessionKey,
    source: runtime_source::RuntimeSourceKey,
    configuration: RuntimeInstance,
    selection: Option<crate::ModelSelection>,
    native_id: String,
    cwd: String,
    target: velune_agent_runtime::control::Target,
}

struct NativeSubmission {
    kind: ExecutionCommandKind,
    created_draft: Option<CreationToken>,
    message_boundary: SubmissionMessageBoundary,
    settings: Option<crate::conversation::TurnSettings>,
    cancellation: Option<Arc<AtomicBool>>,
    delivery: Option<Arc<Retained<CodexDelivery>>>,
    stop_ticket: Option<Retained<CodexOperationTicket>>,
    stop_delivery: Option<Retained<CodexDelivery>>,
    stop_admission_error: Option<String>,
    effects_installed: bool,
    submitted_native_message_id: Option<String>,
    submitted_identity_unavailable: bool,
}

/// At most one unbound accepted input per session. Its echo must still belong
/// to this source, connection, turn and physical message boundary.
struct AcceptedSettings {
    session: session::SessionKey,
    source: runtime_source::RuntimeSourceKey,
    native: Arc<Retained<CodexOperationTicket>>,
    delivery: Arc<Retained<CodexDelivery>>,
    boundary: SubmissionMessageBoundary,
    settings: crate::conversation::TurnSettings,
}

/// A single physical message anchor, not a copy of the conversation history.
#[derive(Clone)]
struct SubmissionMessageBoundary {
    after_message_id: Option<String>,
    user_messages: usize,
}

enum SubmittedUser {
    Echo(String),
    AwaitingEcho,
    BoundaryUnavailable,
}

impl SubmissionMessageBoundary {
    fn capture(snapshot: &crate::conversation::ConversationSnapshot) -> Self {
        Self {
            after_message_id: snapshot.messages.last().map(|message| message.id.clone()),
            user_messages: snapshot
                .messages
                .iter()
                .filter(|message| message.role == crate::conversation::MessageRole::User)
                .count(),
        }
    }

    fn submitted_user(
        &self,
        snapshot: &crate::conversation::ConversationSnapshot,
        accepted_turn_id: Option<&str>,
    ) -> SubmittedUser {
        let Some(accepted_turn_id) = accepted_turn_id else {
            return SubmittedUser::BoundaryUnavailable;
        };
        let start = if let Some(mut anchor) = self.after_message_id.as_deref() {
            let mut steps = 0;
            while let Some(confirmation) = snapshot
                .message_identity_confirmations
                .iter()
                .find(|confirmation| confirmation.previous_id == anchor)
            {
                if confirmation.current_id == anchor {
                    break;
                }
                if steps == snapshot.message_identity_confirmations.len() {
                    return SubmittedUser::BoundaryUnavailable;
                }
                anchor = &confirmation.current_id;
                steps += 1;
            }
            let Some(index) = snapshot
                .messages
                .iter()
                .position(|message| message.id == anchor)
            else {
                return SubmittedUser::BoundaryUnavailable;
            };
            index + 1
        } else {
            0
        };
        if snapshot.messages[..start]
            .iter()
            .filter(|message| message.role == crate::conversation::MessageRole::User)
            .count()
            != self.user_messages
        {
            return SubmittedUser::BoundaryUnavailable;
        }
        match snapshot.messages[start..]
            .iter()
            .find(|message| message.role == crate::conversation::MessageRole::User)
        {
            Some(message) if message.branch_point.as_deref() == Some(accepted_turn_id) => {
                SubmittedUser::Echo(message.id.clone())
            }
            Some(_) => SubmittedUser::BoundaryUnavailable,
            None => SubmittedUser::AwaitingEcho,
        }
    }
}

impl NativeControls {
    // Ordinary controls on this connection keep the pending echo. A ticket
    // from a replaced epoch/generation must never revive its metadata.
    fn observe_connection(&mut self, ticket: &CodexOperationTicket) {
        if self.accepted_settings.as_ref().is_some_and(|pending| {
            pending.native.session != ticket.session
                || pending.native.generation != ticket.generation
        }) {
            self.accepted_settings = None;
        }
    }

    pub(super) fn pending(&self) -> bool {
        !self.operations.is_empty() || self.cleanup_required
    }

    pub(super) fn submission_pending(&self) -> bool {
        self.operations
            .iter()
            .any(|op| matches!(op.effect, Effect::Submission(_)))
    }

    pub(super) fn awaiting_turn_acceptance(&self) -> bool {
        self.operations.iter().any(|op| {
            matches!(&op.effect, Effect::Submission(submission)
            if submission.kind != ExecutionCommandKind::Steer && submission.delivery.is_none())
        })
    }
}

impl session::Session {
    pub(super) fn owns_native_control(&self) -> bool {
        self.side.is_none()
            && self.execution().is_some_and(|configuration| {
                configuration.runtime.type_id == RuntimeKind::Codex166
                    && configuration.gateway().is_none()
            })
            && matches!(self.active_state, ActiveState::Native(_))
    }
}

impl CoreRuntime {
    /// False retains the synchronous facade/other adapters and unsupported
    /// cancellation states on their existing path. Admission never runs RPC IO.
    pub(super) fn start_native_control(
        &mut self,
        command: CodexCommand,
    ) -> Result<bool, RuntimeError> {
        let (kind, effect) = match &command {
            CodexCommand::Cancel => (ExecutionCommandKind::Cancel, Effect::CancelParent),
            CodexCommand::StopBackground => {
                (ExecutionCommandKind::StopBackground, Effect::StopBackground)
            }
            CodexCommand::StopChild { .. } => (ExecutionCommandKind::Cancel, Effect::StopChild),
            CodexCommand::Reply { .. } => (ExecutionCommandKind::Reply, Effect::Reply),
            _ => unreachable!("only the control seam is composed here"),
        };
        let Some(application) = self.execution_control_ticket(kind)? else {
            return Ok(false);
        };
        if !self.session.owns_native_control() {
            return Ok(false);
        }
        if self.session.native_controls.cleanup_required {
            return Err(RuntimeError::Busy);
        }
        let pending = self.session.native_controls.pending();
        let source = self.runtime_source_key(self.active_instance()?)?;
        // Match the native synchronous control budget, shared by both phases
        // of parent cancellation rather than renewing it after the parent ACK.
        let deadline = Instant::now() + Duration::from_secs(15);
        let ActiveState::Native(native) = &mut self.session.active_state else {
            unreachable!()
        };
        let legacy_cancel = matches!(effect, Effect::CancelParent) && !native.can_steer();
        let ticket = match native.enqueue_codex_command(command, deadline) {
            Ok(ticket) => ticket,
            // Retry backoff and unidentified maintenance/starting turns retain
            // their legacy cancellation path, only after proven non-dispatch.
            // An identified Stopping turn still uses the co-located port.
            Err(velune_agent_runtime::native::CodexAdmissionError::Rejected(_))
                if legacy_cancel && !pending =>
            {
                return Ok(false);
            }
            Err(error) => return Err(RuntimeError::Invalid(format!("Codex 控制未派发：{error}"))),
        };
        self.session.native_controls.observe_connection(&ticket);
        self.session.native_controls.enabled = true;
        self.session.native_controls.operations.push(Operation {
            application,
            native: Arc::new(ticket),
            source,
            deadline,
            effect,
        });
        self.mark_execution_started(application);
        // This is application queue policy, not a native cancellation fact.
        // In particular a turn terminal arriving before interrupt ACK must not
        // dispatch the next queued turn.
        if kind == ExecutionCommandKind::Cancel
            && self
                .session
                .native_controls
                .operations
                .last()
                .is_some_and(|op| matches!(op.effect, Effect::CancelParent))
        {
            self.session.send_queue.pause_after_stop();
        }
        Ok(true)
    }

    /// Called immediately after native poll, including its failure barrier.
    /// Full native ticket equality includes epoch, connection and actual wire id.
    pub(super) fn harvest_native_controls(&mut self) {
        loop {
            let receipt = match &mut self.session.active_state {
                ActiveState::Native(native) => native.take_codex_receipt(),
                _ => None,
            };
            let Some(receipt) = receipt else { break };
            self.session
                .native_controls
                .observe_connection(&receipt.ticket);
            let Some(index) =
                self.session
                    .native_controls
                    .operations
                    .iter()
                    .position(|operation| {
                        operation.native.as_ref().as_ref() == &receipt.ticket
                            || matches!(&operation.effect, Effect::Submission(submission)
                        if submission.stop_ticket.as_deref() == Some(&receipt.ticket))
                    })
            else {
                continue;
            };
            let mut operation = self.session.native_controls.operations.remove(index);
            if let Effect::Submission(submission) = &mut operation.effect {
                let is_input = operation.native.as_ref().as_ref() == &receipt.ticket;
                let delivery = receipt.map(|receipt| receipt.delivery);
                if is_input {
                    submission.delivery = Some(Arc::new(delivery));
                } else {
                    submission.stop_delivery = Some(delivery);
                }
                self.session.native_controls.operations.push(operation);
                continue;
            }
            if !operation.source.is_current(self) {
                let parent_accepted = matches!(operation.effect, Effect::CancelBackground);
                let accepted = parent_accepted
                    || matches!(
                        &receipt.delivery,
                        CodexDelivery::Accepted { .. } | CodexDelivery::Written
                    )
                    || matches!(&receipt.delivery, CodexDelivery::Interrupts { children }
                        if children.iter().any(|child| matches!(child.delivery, CodexInterruptDelivery::Accepted)));
                let evidence = control_delivery(&receipt.delivery)
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "原生控制已受理或回复已写入".into());
                let evidence = if parent_accepted {
                    format!("父 turn 停止已受理；{evidence}")
                } else {
                    evidence
                };
                let detail = format!("运行时来源已改变；未刷新或继续派发；{evidence}");
                let error = if accepted {
                    RuntimeError::operation_completed(detail)
                } else {
                    RuntimeError::operation_indeterminate(detail)
                };
                self.execution_results
                    .insert(operation.application, Err(error));
                continue;
            }
            if matches!(operation.effect, Effect::CancelParent)
                && matches!(receipt.delivery, CodexDelivery::Accepted { .. })
            {
                // Acceptance belongs to the parent only, even if its later
                // background phase rejects, times out or cannot be admitted.
                self.session.cancel_requested = self.session.turn_running();
                let ActiveState::Native(native) = &mut self.session.active_state else {
                    unreachable!()
                };
                match native.enqueue_codex_command(CodexCommand::StopBackground, operation.deadline)
                {
                    Ok(ticket) => {
                        operation.native = Arc::new(ticket);
                        operation.effect = Effect::CancelBackground;
                        self.session.native_controls.operations.push(operation);
                        continue;
                    }
                    Err(error) => {
                        self.execution_results.insert(
                            operation.application,
                            Err(RuntimeError::operation_completed(format!(
                                "父 turn 停止已受理；后台停止未派发：{error}"
                            ))),
                        );
                        continue;
                    }
                }
            }
            let result = control_delivery(&receipt.delivery);
            let result = if matches!(operation.effect, Effect::CancelBackground) {
                result.map_err(|error| {
                    RuntimeError::operation_completed(format!("父 turn 停止已受理；{error}",))
                })
            } else {
                result
            };
            self.execution_results.insert(
                operation.application,
                result.map(|()| ExecutionSuccess::default()),
            );
        }
    }

    /// Called only after the worker-created adapter has passed source-checked
    /// installation. The loaded native thread already owns its captured provider
    /// and cwd; deriving its full target here is pure configuration resolution.
    pub(super) fn capture_created_draft(&mut self) -> Option<CreationToken> {
        self.session.native_controls.created_draft = None;
        if !self.session.owns_native_control()
            || self.session.logical_projection.is_some()
            || self.session.pending_native_fork.is_some()
            || self.session.configuration_changed
        {
            return None;
        }
        let configuration = self.session.execution()?;
        let selection = self.session.model_selection.as_ref()?;
        if configuration.native_project
            || matches!(selection, Some(crate::ModelSelection::Managed { .. }))
            || !self
                .runtime_instance(&configuration.runtime.id)
                .is_ok_and(|runtime| runtime.enabled && runtime == &configuration.runtime)
        {
            return None;
        }
        let source = self.runtime_source_key(&configuration.runtime).ok()?;
        if !source.is_current(self) {
            return None;
        }
        let snapshot = self.session.projected()?;
        let draft = self.session.unsubmitted_draft.as_ref()?;
        if !snapshot.messages.is_empty()
            || snapshot.context_runtime_id != configuration.runtime.id
            || snapshot.conversation.runtime_id != configuration.runtime.id
            || draft.native_id != snapshot.conversation.id
            || draft.public_id != draft.native_id
            || draft.runtime_id != configuration.runtime.id
            || draft.original_runtime_id != configuration.runtime.id
        {
            return None;
        }
        let cwd = snapshot.conversation.cwd.as_ref()?;
        if !Path::new(cwd).is_absolute() {
            return None;
        }
        let target = self.read_control_target(snapshot, None).ok().flatten()?;
        if !matches!(&self.session.active_state, ActiveState::Native(native)
            if !native.restart_needed() && native.owns_control(&target))
        {
            return None;
        }
        let token = CreationToken::next()?;
        let grant = CreatedDraft {
            token,
            session: self.session.key,
            source,
            configuration: configuration.runtime.clone(),
            selection: selection.clone(),
            native_id: snapshot.conversation.id.clone(),
            cwd: cwd.clone(),
            target,
        };
        self.session.native_controls.created_draft = Some(grant);
        Some(token)
    }

    fn created_draft_grant(&self) -> Option<&CreatedDraft> {
        let grant = self.session.native_controls.created_draft.as_ref()?;
        let draft = self.session.unsubmitted_draft.as_ref()?;
        let configuration = self.session.execution()?;
        let snapshot = self.session.projected()?;
        if grant.session != self.session.key
            || draft.creation_token != Some(grant.token)
            || !grant.source.is_current(self)
            || configuration.runtime != grant.configuration
            || self.session.model_selection.as_ref() != Some(&grant.selection)
            || self.session.logical_projection.is_some()
            || self.session.side.is_some()
            || self.session.pending_native_fork.is_some()
            || configuration.native_project
            || configuration.gateway().is_some()
            || self.session.configuration_changed
            || draft.native_id != grant.native_id
            || draft.public_id != grant.native_id
            || draft.runtime_id != grant.configuration.id
            || draft.original_runtime_id != grant.configuration.id
            || snapshot.conversation.id != grant.native_id
            || snapshot.context_runtime_id != grant.configuration.id
            || snapshot.conversation.runtime_id != grant.configuration.id
            || snapshot.conversation.cwd.as_deref() != Some(grant.cwd.as_str())
            || snapshot
                .messages
                .iter()
                .any(|message| message.role == crate::conversation::MessageRole::User)
        {
            return None;
        }
        let target = self.read_control_target(snapshot, None).ok().flatten()?;
        if target != grant.target
            || !matches!(&self.session.active_state, ActiveState::Native(native)
            if !native.restart_needed() && native.owns_control(&target))
        {
            return None;
        }
        Some(grant)
    }

    fn settle_created_draft(
        &mut self,
        token: CreationToken,
        source: &runtime_source::RuntimeSourceKey,
    ) {
        let Some(grant) = self.session.native_controls.created_draft.as_ref() else {
            return;
        };
        let matches_original = grant.token == token
            && grant.session == self.session.key
            && &grant.source == source
            && self
                .session
                .unsubmitted_draft
                .as_ref()
                .is_some_and(|draft| {
                    draft.creation_token == Some(token)
                        && draft.native_id == grant.native_id
                        && draft.public_id == grant.native_id
                        && draft.runtime_id == grant.configuration.id
                        && draft.original_runtime_id == grant.configuration.id
                })
            && self.session.projected().is_some_and(|snapshot| {
                snapshot.conversation.id == grant.native_id
                    && snapshot.context_runtime_id == grant.configuration.id
            });
        if matches_original {
            // Neither source freshness nor a view is needed to settle the
            // original authority. They remain mandatory for installing effects.
            // The created identity/history is not replaced or reconstructed.
            self.session.unsubmitted_draft = None;
            self.session.native_controls.created_draft = None;
        }
    }

    /// Reuse only this connection's confirmed physical context and captured
    /// provider/model. Cold reads, defaults sampled from another helper, model
    /// changes and restart/preparation remain on their existing paths.
    pub(super) fn can_submit_native(
        &self,
        kind: ExecutionCommandKind,
        runtime_id: &str,
        selection: &Option<crate::ModelSelection>,
    ) -> bool {
        if !self.submission_scope(kind)
            || !self.session.owns_native_control()
            || self.session.native_controls.submission_pending()
            || self.session.native_controls.cleanup_required
            || self.session.configuration_changed
            || self.session.compacting
            || self.session.pending_native_fork.is_some()
            || self.session.model_selection.as_ref() != Some(selection)
            || matches!(selection, Some(crate::ModelSelection::Managed { .. }))
        {
            return false;
        }
        let Some(configuration) = self.session.execution() else {
            return false;
        };
        if configuration.native_project
            || configuration.runtime.id != runtime_id
            || !self
                .runtime_instance(runtime_id)
                .is_ok_and(|runtime| runtime.enabled && runtime == &configuration.runtime)
        {
            return false;
        }
        if self.session.unsubmitted_draft.is_some() {
            return kind == ExecutionCommandKind::Send && self.created_draft_grant().is_some();
        }
        let Ok(Some(target)) = self.control_target() else {
            return false;
        };
        matches!(&self.session.active_state, ActiveState::Native(native)
            if !native.restart_needed() && native.owns_control(&target))
    }

    /// Bind only the accepted turn's exact echo. A terminal without that echo,
    /// a lost boundary/source/owner, or a later submission retires this one
    /// bounded record; none of those events retracts native acceptance.
    pub(super) fn refresh_native_accepted_settings(&mut self) {
        let Some(pending) = self.session.native_controls.accepted_settings.take() else {
            return;
        };
        if pending.session != self.session.key
            || !pending.source.is_current(self)
            || self.session.native_controls.cleanup_required
        {
            return;
        }
        if matches!(self.session.active_state, ActiveState::Pending(_)) {
            // A compatibility control may temporarily own this same adapter.
            // Do not bind its frozen view or lose an otherwise valid late echo.
            self.session.native_controls.accepted_settings = Some(pending);
            return;
        }
        let Ok(Some(target)) = self.control_target() else {
            return;
        };
        if !matches!(&self.session.active_state, ActiveState::Native(native)
            if !native.restart_needed() && !native.requires_failure_cleanup() && native.owns_control(&target))
        {
            return;
        }
        let Some(snapshot) = self.session.projected() else {
            return;
        };
        match pending
            .boundary
            .submitted_user(snapshot, accepted_turn_id(&pending.delivery))
        {
            SubmittedUser::Echo(id) => {
                let id = self.projected_conversation_message_id(&snapshot.conversation.id, &id);
                self.session
                    .transcript_projection
                    .recall_request(&id, pending.settings);
            }
            SubmittedUser::AwaitingEcho if self.session.turn_running() => {
                self.session.native_controls.accepted_settings = Some(pending);
            }
            SubmittedUser::AwaitingEcho | SubmittedUser::BoundaryUnavailable => {}
        }
    }

    /// Resolve an already projected echo before establishing a new input
    /// boundary. Unmatched old settings cannot claim this next submission.
    pub(super) fn retire_native_settings_for_submission(&mut self) {
        self.refresh_native_accepted_settings();
        self.session.native_controls.accepted_settings = None;
    }

    pub(super) fn enqueue_native_submission(
        &mut self,
        kind: ExecutionCommandKind,
        input: pi::TurnInput,
    ) -> Result<(), RuntimeError> {
        let application = self
            .execution_control_ticket(kind)?
            .ok_or(RuntimeError::Busy)?;
        let source = self.runtime_source_key(self.active_instance()?)?;
        let created_draft = if self.session.unsubmitted_draft.is_some() {
            if kind != ExecutionCommandKind::Send {
                return Err(RuntimeError::Busy);
            }
            Some(self.created_draft_grant().ok_or(RuntimeError::Busy)?.token)
        } else {
            None
        };
        let conversation_id = self
            .session
            .conversation_id()
            .unwrap_or_default()
            .to_owned();
        self.check_submission_cancelled(&conversation_id)?;
        let cancellation = self
            .submission_cancellation
            .as_ref()
            .filter(|(id, _)| id == &conversation_id)
            .map(|(_, flag)| flag.clone());
        let message_boundary = SubmissionMessageBoundary::capture(
            self.session
                .projected()
                .ok_or_else(|| RuntimeError::invalid("会话执行尚未准备"))?,
        );
        let selection = self.session.model_selection.clone().flatten();
        let settings =
            (kind == ExecutionCommandKind::Send).then(|| crate::conversation::TurnSettings {
                runtime_id: Some(
                    self.active_instance()
                        .expect("confirmed runtime")
                        .id
                        .clone(),
                ),
                model: selection
                    .as_ref()
                    .and_then(|model| self.turn_model_name(source.runtime_id(), model)),
                effort: input.effort.clone(),
                permission_mode: input.permission_mode.clone(),
                model_selection: selection,
            });
        let command = match kind {
            ExecutionCommandKind::Send => CodexCommand::Send(input),
            ExecutionCommandKind::Resume => CodexCommand::Resume {
                effort: input.effort,
                permission_mode: input.permission_mode,
            },
            ExecutionCommandKind::Steer => CodexCommand::Steer(input),
            _ => unreachable!("submission scope"),
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        let ActiveState::Native(native) = &mut self.session.active_state else {
            return Err(RuntimeError::Busy);
        };
        let native = telemetry::observe(self.telemetry.as_ref(), "harness.dispatch", || {
            native.enqueue_codex_command(command, deadline)
        })
        .map_err(|error| RuntimeError::Invalid(format!("Codex 输入未派发：{error}")))?;
        // Admission cannot poll or install an echo. Resolve any already seen
        // old echo, then retire the old turn's unbound metadata before this
        // new submission can produce input on the same connection.
        self.retire_native_settings_for_submission();
        if kind != ExecutionCommandKind::Steer {
            self.begin_turn_observation();
        }
        self.session.native_controls.enabled = true;
        self.session.native_controls.operations.push(Operation {
            application,
            native: Arc::new(native),
            source,
            deadline,
            effect: Effect::Submission(Box::new(NativeSubmission {
                kind,
                created_draft,
                message_boundary,
                settings,
                cancellation,
                delivery: None,
                stop_ticket: None,
                stop_delivery: None,
                stop_admission_error: None,
                effects_installed: false,
                submitted_native_message_id: None,
                submitted_identity_unavailable: false,
            })),
        });
        self.mark_execution_started(application);
        Ok(())
    }

    pub(super) fn advance_native_submissions(&mut self) {
        let mut index = 0;
        while index < self.session.native_controls.operations.len() {
            if !matches!(
                self.session.native_controls.operations[index].effect,
                Effect::Submission(_)
            ) {
                index += 1;
                continue;
            }
            let mut operation = self.session.native_controls.operations.remove(index);
            if let Some(result) = self.advance_native_submission(&mut operation) {
                self.execution_results.insert(operation.application, result);
            } else {
                self.session
                    .native_controls
                    .operations
                    .insert(index, operation);
                index += 1;
            }
        }
    }

    fn advance_native_submission(
        &mut self,
        operation: &mut Operation,
    ) -> Option<Result<ExecutionSuccess, RuntimeError>> {
        let Effect::Submission(submission) = &mut operation.effect else {
            unreachable!()
        };
        let current_source = operation.source.is_current(self);
        let cancelled = submission
            .cancellation
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire));
        if cancelled {
            self.session.send_queue.pause_after_stop();
            if current_source && submission.stop_ticket.is_none() && self.session.turn_running() {
                let ActiveState::Native(native) = &mut self.session.active_state else {
                    unreachable!()
                };
                match native.enqueue_codex_command(CodexCommand::Cancel, operation.deadline) {
                    Ok(ticket) => submission.stop_ticket = Some(ticket),
                    // A missing early turn id is a temporary admission failure,
                    // not evidence that the stop intent was fulfilled. Retain the
                    // flag and retry against the same owner/deadline, never input.
                    Err(error) => submission.stop_admission_error = Some(error.to_string()),
                }
            }
        }
        let receipt = submission.delivery.as_ref()?;
        let delivery = receipt.as_ref().as_ref();
        let accepted = matches!(delivery, CodexDelivery::Accepted { .. });
        if (accepted
            || matches!(
                delivery,
                CodexDelivery::Indeterminate { .. }
                    | CodexDelivery::Written
                    | CodexDelivery::Interrupts { .. }
            ))
            && let Some(token) = submission.created_draft
        {
            // Unknown delivery is not proof of an unsubmitted draft. Retire
            // only the captured creation authority, preserving its actual
            // identity/history and the original Accepted/unknown outcome.
            self.settle_created_draft(token, &operation.source);
        }
        if accepted && current_source && !submission.effects_installed {
            submission.effects_installed = true;
            if submission.kind != ExecutionCommandKind::Steer {
                if submission.created_draft.is_none() {
                    self.session.unsubmitted_draft = None;
                }
                self.session.cancel_requested = false;
                self.follow_started_turn();
                if self.session.turn_running() {
                    self.started_turn_observation();
                }
                if let Some(id) = self.session.conversation_id().map(str::to_owned) {
                    self.restore_archived(&id);
                }
            }
            if submission.kind == ExecutionCommandKind::Send {
                let accepted_turn = accepted_turn_id(delivery);
                match self.session.projected().map(|snapshot| {
                    submission
                        .message_boundary
                        .submitted_user(snapshot, accepted_turn)
                }) {
                    Some(SubmittedUser::Echo(id)) => {
                        submission.submitted_native_message_id = Some(id)
                    }
                    Some(SubmittedUser::AwaitingEcho) => {}
                    _ => submission.submitted_identity_unavailable = true,
                }
                if let Some(settings) = submission
                    .settings
                    .take()
                    .filter(|_| !submission.submitted_identity_unavailable)
                {
                    if let (Some(id), Some(snapshot)) = (
                        &submission.submitted_native_message_id,
                        self.session.projected(),
                    ) {
                        let id =
                            self.projected_conversation_message_id(&snapshot.conversation.id, id);
                        self.session
                            .transcript_projection
                            .recall_request(&id, settings);
                    } else if accepted_turn.is_some() {
                        self.session.native_controls.accepted_settings = Some(AcceptedSettings {
                            session: self.session.key,
                            source: operation.source.clone(),
                            native: operation.native.clone(),
                            delivery: receipt.clone(),
                            boundary: submission.message_boundary.clone(),
                            settings,
                        });
                    }
                }
            }
        }
        if submission.stop_ticket.is_some() && submission.stop_delivery.is_none() {
            return None;
        }
        if accepted
            && cancelled
            && current_source
            && submission.stop_ticket.is_none()
            && self.session.turn_running()
            && Instant::now() < operation.deadline
        {
            return None;
        }
        let receipt = submission.delivery.take().expect("delivery checked");
        let result = match receipt.as_ref().as_ref() {
            CodexDelivery::Accepted { .. } => {
                if !current_source {
                    let stop = submission
                        .stop_delivery
                        .take()
                        .map(|delivery| match control_delivery(&delivery) {
                            Ok(()) => "；停止请求已受理（不代表 turn 已结束）".to_owned(),
                            Err(error) => format!("；停止回执：{error}"),
                        })
                        .unwrap_or_default();
                    Err(RuntimeError::after_delivery(format!(
                        "运行时来源已改变；未刷新或新增派发{stop}"
                    )))
                } else if let Some(stop) = submission.stop_delivery.take() {
                    let result = control_delivery(&stop).map_err(RuntimeError::after_delivery);
                    if result.is_ok() {
                        self.session.cancel_requested = self.session.turn_running();
                    }
                    result.map(|()| ExecutionSuccess {
                        submitted_native_message_id: submission.submitted_native_message_id.take(),
                    })
                } else if cancelled && self.session.turn_running() {
                    Err(RuntimeError::after_delivery(format!(
                        "输入已受理，但停止未派发：{}",
                        submission
                            .stop_admission_error
                            .as_deref()
                            .unwrap_or("未取得可停止的 turn 身份")
                    )))
                } else {
                    Ok(ExecutionSuccess {
                        submitted_native_message_id: submission.submitted_native_message_id.take(),
                    })
                }
            }
            CodexDelivery::Rejected { error } => Err(RuntimeError::Invalid(format!(
                "Codex 明确拒绝输入；输入未受理：{error}"
            ))),
            CodexDelivery::Indeterminate { detail } => {
                Err(RuntimeError::operation_indeterminate(detail))
            }
            CodexDelivery::Written | CodexDelivery::Interrupts { .. } => Err(
                RuntimeError::operation_indeterminate("输入未取得原生 Accepted 回执；未自动重发"),
            ),
        };
        let result = if accepted && current_source && submission.submitted_identity_unavailable {
            let detail = result
                .err()
                .map(|error| format!("；{error}"))
                .unwrap_or_default();
            Err(RuntimeError::after_delivery(format!(
                "原消息边界或 turn 身份不可确认；未猜测 submitted ID 或绑定请求设置{detail}"
            )))
        } else {
            result
        };
        if result.is_err() {
            self.session.send_queue.pause_after_stop();
            if submission.kind != ExecutionCommandKind::Steer && !accepted {
                self.complete_turn_observation(
                    if matches!(&result, Err(RuntimeError::Invalid(_))) {
                        velune_telemetry::Outcome::Failure
                    } else {
                        velune_telemetry::Outcome::Unknown
                    },
                );
            }
        }
        Some(result)
    }

    pub(super) fn schedule_native_cleanup(&mut self) -> Result<(), RuntimeError> {
        if !self.session.native_controls.cleanup_required
            || !matches!(self.session.active_state, ActiveState::Native(_))
        {
            return Ok(());
        }
        self.start_native_cleanup_worker()
    }
}

fn accepted_turn_id(delivery: &CodexDelivery) -> Option<&str> {
    match delivery {
        CodexDelivery::Accepted { turn_id, .. } => turn_id.as_deref(),
        _ => None,
    }
}

fn control_delivery(delivery: &CodexDelivery) -> Result<(), RuntimeError> {
    match delivery {
        CodexDelivery::Accepted { .. } | CodexDelivery::Written => Ok(()),
        CodexDelivery::Rejected { error } => Err(RuntimeError::Invalid(format!(
            "Codex 明确拒绝控制：{error}"
        ))),
        CodexDelivery::Indeterminate { detail } => {
            Err(RuntimeError::operation_indeterminate(detail))
        }
        CodexDelivery::Interrupts { children } => {
            let accepted = children
                .iter()
                .filter(|child| matches!(child.delivery, CodexInterruptDelivery::Accepted))
                .count();
            let uncertain = children.iter().any(|child| {
                matches!(child.delivery, CodexInterruptDelivery::Indeterminate { .. })
            });
            if children
                .iter()
                .all(|child| matches!(child.delivery, CodexInterruptDelivery::Accepted))
            {
                return Ok(());
            }
            let deliveries: Vec<_> = children
                .iter()
                .map(|child| {
                    let detail = match &child.delivery {
                        CodexInterruptDelivery::Accepted => "Accepted".into(),
                        CodexInterruptDelivery::Rejected { error } => format!("Rejected({error})"),
                        CodexInterruptDelivery::Indeterminate { detail } => {
                            format!("Indeterminate({detail})")
                        }
                        CodexInterruptDelivery::NotDispatched => "NotDispatched".into(),
                    };
                    format!(
                        "{} / {} / {:?}: {detail}",
                        child.thread_id, child.turn_id, child.request_id
                    )
                })
                .collect();
            let detail = format!(
                "后台停止：{accepted}/{} 已受理；{}",
                children.len(),
                deliveries.join("；")
            );
            if uncertain {
                Err(RuntimeError::operation_indeterminate(detail))
            } else if accepted > 0 {
                Err(RuntimeError::operation_completed(detail))
            } else {
                Err(RuntimeError::Invalid(detail))
            }
        }
    }
}

#[cfg(test)]
mod submission_boundary_tests {
    use super::*;
    use crate::conversation::{ConversationSnapshot, Message, MessageRole};

    fn history_snapshot() -> ConversationSnapshot {
        let mut snapshot = PiProjection::new(ConversationSummary {
            native_read_state: None,
            id: "native:thread".into(),
            runtime_id: "native".into(),
            title: velune_conversation::ConversationTitle::Untitled,
            created_at_unix_ms: None,
            updated_at_unix_ms: None,
            cwd: None,
            can_rename: false,
            can_delete: false,
            parent_conversation_id: None,
        })
        .snapshot
        .unwrap();
        snapshot.messages = vec![
            user("u1", "turn-1"),
            user("u2", "turn-2"),
            Message {
                id: "provisional-anchor".into(),
                role: MessageRole::Assistant,
                blocks: Vec::new(),
                completed: true,
                timestamp_unix_ms: None,
                turn_settings: None,
                turn_usage: None,
                branch_point: None,
            },
        ];
        snapshot
    }

    fn user(id: &str, turn: &str) -> Message {
        Message {
            id: id.into(),
            role: MessageRole::User,
            blocks: Vec::new(),
            completed: true,
            timestamp_unix_ms: None,
            turn_settings: None,
            turn_usage: None,
            branch_point: Some(turn.into()),
        }
    }

    #[test]
    fn submission_boundary_resolves_provisional_identity_and_selects_only_the_accepted_turn() {
        let mut snapshot = history_snapshot();
        let boundary = SubmissionMessageBoundary::capture(&snapshot);
        assert!(matches!(
            boundary.submitted_user(&snapshot, Some("turn-3")),
            SubmittedUser::AwaitingEcho
        ));
        snapshot.messages.last_mut().unwrap().id = "confirmed-anchor".into();
        snapshot.message_identity_confirmations = vec![
            crate::conversation::MessageIdentityConfirmation {
                previous_id: "provisional-anchor".into(),
                current_id: "intermediate-anchor".into(),
            },
            crate::conversation::MessageIdentityConfirmation {
                previous_id: "intermediate-anchor".into(),
                current_id: "confirmed-anchor".into(),
            },
        ];
        snapshot.messages.push(user("u3", "turn-3"));
        assert!(
            matches!(boundary.submitted_user(&snapshot, Some("turn-3")), SubmittedUser::Echo(id) if id == "u3")
        );
        assert!(matches!(
            boundary.submitted_user(&snapshot, Some("different-turn")),
            SubmittedUser::BoundaryUnavailable
        ));
        let mut user_anchor = history_snapshot();
        user_anchor.messages.pop();
        let boundary = SubmissionMessageBoundary::capture(&user_anchor);
        user_anchor.messages.last_mut().unwrap().id = "confirmed-u2".into();
        user_anchor.message_identity_confirmations.push(
            crate::conversation::MessageIdentityConfirmation {
                previous_id: "u2".into(),
                current_id: "confirmed-u2".into(),
            },
        );
        assert!(matches!(
            boundary.submitted_user(&user_anchor, Some("turn-3")),
            SubmittedUser::AwaitingEcho
        ));
        user_anchor.messages.push(user("u3", "turn-3"));
        assert!(
            matches!(boundary.submitted_user(&user_anchor, Some("turn-3")), SubmittedUser::Echo(id) if id == "u3")
        );
    }

    #[test]
    fn submission_boundary_missing_anchor_or_confirmation_cycle_never_guesses_a_user() {
        let mut changed_prefix = history_snapshot();
        let boundary = SubmissionMessageBoundary::capture(&changed_prefix);
        changed_prefix.messages.remove(0);
        assert!(matches!(
            boundary.submitted_user(&changed_prefix, Some("turn-3")),
            SubmittedUser::BoundaryUnavailable
        ));
        let mut snapshot = history_snapshot();
        let boundary = SubmissionMessageBoundary::capture(&snapshot);
        snapshot.messages.pop();
        snapshot.messages.push(user("u3", "turn-3"));
        assert!(matches!(
            boundary.submitted_user(&snapshot, Some("turn-3")),
            SubmittedUser::BoundaryUnavailable
        ));
        snapshot.message_identity_confirmations = vec![
            crate::conversation::MessageIdentityConfirmation {
                previous_id: "provisional-anchor".into(),
                current_id: "intermediate-anchor".into(),
            },
            crate::conversation::MessageIdentityConfirmation {
                previous_id: "intermediate-anchor".into(),
                current_id: "provisional-anchor".into(),
            },
        ];
        assert!(matches!(
            boundary.submitted_user(&snapshot, Some("turn-3")),
            SubmittedUser::BoundaryUnavailable
        ));
    }
}
