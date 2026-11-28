//! Pi edit dependencies stay on the session's protocol owner. Only captured
//! source reads, cold startup and terminal destruction use bounded IO workers.
use super::*;
use crate::local::execution_workers::{
    ExecutionCommandKind, ExecutionSuccess, ExecutionTicket, PendingExecution,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(in crate::local) struct PiControl {
    application: ExecutionTicket,
    native: pi::PiTicket,
    source: runtime_source::RuntimeSourceKey,
    cancel: bool,
}

/// An ordinary composed prompt whose exact Pi response is still pending.
/// Keep the application command ticket until the runtime accepts or rejects it.
pub(in crate::local) struct PiSubmission {
    pub(in crate::local) application: ExecutionTicket,
    pub(in crate::local) native: pi::PiTicket,
}

pub(in crate::local) struct PiEdit {
    application: ExecutionTicket,
    session: session::SessionKey,
    runtime: RuntimeInstance,
    gateway: Option<GatewayConfig>,
    sources: Vec<runtime_source::RuntimeSourceKey>,
    link: Option<crate::config::ConversationLink>,
    revision: String,
    conversation_id: String,
    message_id: String,
    native_id: String,
    entry: String,
    selection: Option<crate::ModelSelection>,
    input: Option<pi::TurnInput>,
    native_digest: Option<String>,
    authorized_source_digest: Option<String>,
    retained_digest: Option<(usize, String)>,
    retained_content: Option<String>,
    recovery_target: Option<PathBuf>,
    fallback_install: Option<(
        browsing::HistorySnapshot,
        super::super::continuation::LogicalProjection,
    )>,
    fallback_committed: bool,
    fallback_closed: bool,
    deletion: Option<bool>,
    delete_install_error: Option<RuntimeError>,
    source_to_prepare: Option<browsing::HistorySnapshot>,
    phase: Phase,
    branched: bool,
    cancelled: Arc<AtomicBool>,
    preparation_deadline: Instant,
}

enum Phase {
    Sources(Io),
    PrepareSource,
    Stop,
    Retire,
    Launch(Io),
    Source(pi::PiTicket),
    SourceVerified,
    DeleteRetire,
    DeleteRead(Io),
    DeleteComplete(Io),
    DeleteReady(Result<Box<browsing::LoadedHistory>, RuntimeError>),
    Verify(Io),
    Branch(pi::PiTicket),
    History(pi::PiTicket),
    Fork(Io),
    FallbackRead(Io),
    FallbackRetire,
    Model(pi::PiTicket),
    Prompt(pi::PiTicket),
}

struct Io {
    pending: Box<PendingExecution>,
}
pub(in crate::local) enum PiEditIoValue {
    Sources(Vec<browsing::LoadedHistory>, Option<pi::TurnInput>),
    Fork(PathBuf),
    DeleteSources(Vec<browsing::LoadedHistory>, String),
    VerifiedSources(Vec<browsing::LoadedHistory>, String),
    Deleted(Result<Box<browsing::LoadedHistory>, RuntimeError>),
    Launch(Box<PiEditLaunch>),
}

pub(in crate::local) struct PiEditLaunch {
    runner: Option<Runner>,
    tunnel: Option<velune_agent_runtime::execution::LoopbackTunnel>,
    prepared_resources: Option<String>,
}

type Launch = Box<dyn FnOnce(&mut ActiveState) -> Result<PiEditIoValue, RuntimeError> + Send>;

fn digest(messages: &[Message]) -> Result<String, RuntimeError> {
    use sha2::{Digest, Sha256};
    let native: Vec<_> = messages
        .iter()
        .filter(native_edit_message)
        .map(|message| {
            (
                &message.id,
                &message.role,
                &message.blocks,
                &message.branch_point,
            )
        })
        .collect();
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&native)?)
    ))
}

fn dependency_error(phase: &str, error: pi::Error) -> RuntimeError {
    match error {
        pi::Error::CommandFailed(_) => {
            RuntimeError::context(&format!("{phase}：原生明确拒绝"), error)
        }
        pi::Error::Sdk(ref detail) if detail.contains("尚未发送") => {
            RuntimeError::context(&format!("{phase}：未发送"), error)
        }
        _ => RuntimeError::operation_indeterminate(format!("{phase}：{error}")),
    }
}

impl Io {
    fn take(&mut self) -> Option<Result<(ActiveState, PiEditIoValue), RuntimeError>> {
        let (_, state, outcome) = self.pending.complete()?;
        Some(outcome.result.and_then(|()| {
            outcome
                .pi_edit
                .map(|value| (state, value))
                .ok_or_else(|| RuntimeError::operation_indeterminate("Pi 编辑 IO 没有返回结果"))
        }))
    }
}

impl CoreRuntime {
    pub(in crate::local) fn advance_pi_submission(&mut self) {
        let Some(submission) = self.session.pi_submission.take() else {
            return;
        };
        let ActiveState::Pi(pi) = &mut self.session.active_state else {
            self.execution_results.insert(
                submission.application,
                Err(RuntimeError::operation_indeterminate(
                    "Pi 确认输入前，原运行时连接已改变",
                )),
            );
            return;
        };
        let result = pi.take_confirmation(submission.native);
        match result {
            Ok(None) => self.session.pi_submission = Some(submission),
            Ok(Some(Ok(()))) => {
                if let Some(id) = self.session.conversation_id().map(str::to_owned) {
                    self.restore_archived(&id);
                }
                self.execution_results
                    .insert(submission.application, Ok(ExecutionSuccess::default()));
            }
            Ok(Some(Err(error))) | Err(error) => {
                self.execution_results.insert(
                    submission.application,
                    Err(dependency_error("Pi 输入回执", error)),
                );
            }
        }
    }

    pub(in crate::local) fn try_start_pi_control(
        &mut self,
        reply: Option<(&str, velune_conversation::RuntimeInteractionReply)>,
    ) -> Result<bool, RuntimeError> {
        if !matches!(self.session.active_state, ActiveState::Pi(_)) {
            return Ok(false);
        }
        let kind = if reply.is_some() {
            ExecutionCommandKind::Reply
        } else {
            ExecutionCommandKind::Cancel
        };
        let Some(application) = self.execution_control_ticket(kind)? else {
            return Ok(false);
        };
        if self.session.pi_control.is_some()
            || self.session.pi_cleanup_required
            || self.session.pi_io_cleanup.is_some()
        {
            return Err(RuntimeError::Busy);
        }
        let source = self.runtime_source_key(self.active_instance()?)?;
        let cancel = reply.is_none();
        let admitted = match reply {
            Some((id, reply)) => self.session.pi()?.begin_reply_confirmation(id, reply),
            None => self.session.pi()?.begin_cancel_confirmation(),
        };
        let native = match admitted {
            Ok(native) => native,
            Err(error) => {
                if self.session.pi()?.requires_failure_cleanup() {
                    self.session.pi_cleanup_required = true;
                    let _ = self.start_native_cleanup_worker();
                }
                // Reply validation is local, before DialogReply enqueue and
                // before any application ticket is started. Preserve the SDK's
                // typed local rejection; transport/timeouts remain unknown via
                // the existing runtime-operation classifier. Cancel can admit
                // several requests before failing, so retain its conservative
                // partial-delivery classification instead.
                return Err(if cancel {
                    dependency_error("Pi 控制未确认", error)
                } else {
                    RuntimeError::runtime_operation("Pi 回复未确认", Box::new(error))
                });
            }
        };
        self.session.pi_control = Some(PiControl {
            application,
            native,
            source,
            cancel,
        });
        self.mark_execution_started(application);
        Ok(true)
    }

    pub(in crate::local) fn advance_pi_control(&mut self) {
        let Some(control) = self.session.pi_control.take() else {
            return;
        };
        let clear_queue_confirmed = self
            .session
            .pi()
            .is_ok_and(|pi| pi.cancel_prefix_confirmed(control.native));
        let result = match self.session.pi() {
            Ok(pi) => pi.take_confirmation(control.native),
            Err(error) => {
                self.execution_results
                    .insert(control.application, Err(error));
                return;
            }
        };
        let result = match result {
            Ok(Some(result)) => result.map_err(|error| {
                dependency_error(
                    if clear_queue_confirmed {
                        "Pi 清队列已确认；Abort 停止控制未确认"
                    } else {
                        "Pi 控制未确认"
                    },
                    error,
                )
            }),
            Ok(None) if control.source.is_current(self) => {
                self.session.pi_control = Some(control);
                return;
            }
            Ok(None) => {
                self.session
                    .pi()
                    .expect("Pi owner retained")
                    .cancel_confirmation(control.native);
                Err(RuntimeError::operation_indeterminate(
                    "Pi 控制期间来源已改变",
                ))
            }
            Err(error) => Err(dependency_error("Pi 控制票据失效", error)),
        };
        if control.cancel && result.is_ok() {
            self.session.cancel_requested = true;
            self.session.send_queue.pause_after_stop();
            if let Some((gateway_id, session)) = self.backend_session_identity() {
                self.session_backends()
                    .cancel_session(&gateway_id, &session);
            }
        }
        if self
            .session
            .pi()
            .is_ok_and(|pi| pi.requires_failure_cleanup())
        {
            self.session.pi_cleanup_required = true;
            let _ = self.start_native_cleanup_worker();
        }
        self.execution_results.insert(
            control.application,
            result.map(|()| ExecutionSuccess::default()),
        );
    }

    #[allow(clippy::too_many_arguments)] // Mirrors the edit/delete command inputs.
    pub(super) fn try_start_pi_mutation(
        &mut self,
        current: &ConversationSnapshot,
        message_id: &str,
        runtime: &RuntimeInstance,
        revision: Option<&str>,
        selection: Option<crate::ModelSelection>,
        turn: Option<crate::ComposedTurn>,
        deletion: Option<bool>,
    ) -> Result<Option<crate::SnapshotResult>, RuntimeError> {
        let kind = if deletion.is_some() {
            ExecutionCommandKind::Delete
        } else {
            ExecutionCommandKind::Edit
        };
        let Some(application) = self.execution_control_ticket(kind)? else {
            return Ok(None);
        };
        if runtime.type_id != RuntimeKind::Pi || self.session.side.is_some() {
            return Ok(None);
        }
        if self.session.pi_io_cleanup.is_some()
            || self.session.pi_edit.is_some()
            || self.session.pi_control.is_some()
            || self.session.pi_cleanup_required
        {
            return Err(RuntimeError::Busy);
        }
        let link = self.link_of(&current.conversation.id).cloned();
        let native_id = if let Some(link) = &link {
            let tail = link
                .segments
                .last()
                .ok_or_else(|| RuntimeError::invalid("会话缺少来源"))?;
            if tail.runtime_instance_id != runtime.id
                || tail.handoff.is_some()
                || tail.cutoff.is_some()
            {
                return Ok(None);
            }
            let Some(id) =
                message_id.strip_prefix(&format!("segment-{}:", link.segments.len() - 1))
            else {
                return Ok(None);
            };
            id
        } else {
            message_id
        };
        let Some(entry) = pi_entry(native_id).map(str::to_owned) else {
            return Ok(None);
        };
        let native_message_id = native_id.to_owned();
        self.validate_model_selection(runtime, selection.as_ref())?;
        if let Some(effort) = turn.as_ref().and_then(|turn| turn.effort.as_ref()) {
            self.validate_effort(runtime, selection.as_ref(), effort)?;
        }
        if let Some(mode) = turn.as_ref().and_then(|turn| turn.permission_mode.as_ref()) {
            self.validate_permission_mode(runtime, mode)?;
        }
        let (reads, sources) = self.capture_pi_edit_sources(current)?;
        let runtime_copy = runtime.clone();
        let composed = turn;
        let io = self.pi_edit_io(application, move |_| {
            let input = if let Some(turn) = composed {
                let attachments = turn.attachments;
                let text = turn.text;
                let effort = turn.effort;
                let permission_mode = turn.permission_mode;
                let mut images = Vec::new();
                let mut files = Vec::new();
                if !attachments.is_empty()
                    && !matches!(
                        runtime_copy.execution()?.transport,
                        crate::RuntimeTransport::Local
                    )
                {
                    return Err(RuntimeError::invalid("远端运行时暂不支持附件"));
                }
                for path in attachments.iter().map(PathBuf::from) {
                    if !path.is_absolute() || !path.is_file() {
                        return Err(RuntimeError::invalid("附件必须是已有文件的绝对路径"));
                    }
                    if let Some(kind) = pi::ImageType::from_path(&path) {
                        images
                            .push(pi::TurnImage::read(&path, kind).map_err(RuntimeError::Invalid)?);
                    } else {
                        fs::File::open(&path)
                            .map_err(|error| RuntimeError::context("读取编辑附件", error))?;
                        files.push(path);
                    }
                }
                let text = super::super::attachments::compose_text(&text, &files);
                if text.trim().is_empty() && images.is_empty() {
                    return Err(RuntimeError::invalid("message text"));
                }
                Some(pi::TurnInput {
                    text,
                    images,
                    effort,
                    permission_mode,
                })
            } else {
                None
            };
            let loaded = reads
                .into_iter()
                .map(browsing::HistoryRead::load)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(PiEditIoValue::Sources(loaded, input))
        })?;
        let cancelled = self
            .submission_cancellation
            .as_ref()
            .filter(|(id, _)| id == &current.conversation.id)
            .map(|(_, flag)| flag.clone())
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        self.session.pi_edit = Some(PiEdit {
            application,
            session: self.session.key,
            runtime: runtime.clone(),
            gateway: if matches!(selection, Some(crate::ModelSelection::Managed { .. })) {
                self.gateways
                    .iter()
                    .find(|gateway| gateway.id == runtime.gateway_id)
                    .cloned()
            } else {
                None
            },
            sources,
            link,
            revision: revision.unwrap_or_default().into(),
            conversation_id: current.conversation.id.clone(),
            message_id: native_message_id,
            native_id: self
                .session
                .projected()
                .expect("source projection retained")
                .conversation
                .id
                .clone(),
            entry,
            selection,
            input: None,
            native_digest: None,
            authorized_source_digest: None,
            retained_digest: None,
            retained_content: None,
            recovery_target: None,
            fallback_install: None,
            fallback_committed: false,
            fallback_closed: false,
            deletion,
            delete_install_error: None,
            source_to_prepare: None,
            phase: Phase::Sources(io),
            branched: false,
            cancelled,
            preparation_deadline: Instant::now() + Duration::from_secs(30),
        });
        self.mark_execution_started(application);
        self.snapshot_result().map(Some)
    }

    fn capture_pi_edit_sources(
        &self,
        current: &ConversationSnapshot,
    ) -> Result<
        (
            Vec<browsing::HistoryRead>,
            Vec<runtime_source::RuntimeSourceKey>,
        ),
        RuntimeError,
    > {
        let addresses = if let Some(link) = self.link_of(&current.conversation.id) {
            link.segments
                .iter()
                .map(|segment| {
                    (
                        segment.runtime_instance_id.clone(),
                        segment.native_conversation_id.clone(),
                    )
                })
                .collect()
        } else {
            vec![(
                current.context_runtime_id.clone(),
                current.conversation.id.clone(),
            )]
        };
        let mut reads = Vec::new();
        let mut sources = Vec::new();
        for (runtime, id) in addresses {
            let read = self.prepare_history_read(&runtime, &id)?;
            sources.push(read.source().clone());
            reads.push(read);
        }
        Ok((reads, sources))
    }

    fn pi_edit_io(
        &self,
        ticket: ExecutionTicket,
        work: impl FnOnce(&mut ActiveState) -> Result<PiEditIoValue, RuntimeError> + Send + 'static,
    ) -> Result<Io, RuntimeError> {
        let (target, _) = self.runtime_location(
            self.runtime_instance(
                self.session
                    .projected()
                    .ok_or_else(|| RuntimeError::invalid("会话未打开"))?
                    .context_runtime_id
                    .as_str(),
            )?,
        )?;
        let pending = self
            .execution_workers
            .submit(
                ActiveState::Empty,
                target,
                ticket,
                None,
                "pi_edit_io",
                move |state| match work(state) {
                    Ok(value) => {
                        let mut outcome = execution_workers::OperationOutcome::from(Ok(()));
                        outcome.pi_edit = Some(value);
                        outcome
                    }
                    Err(error) => Err(error).into(),
                },
            )
            .map_err(|(_, error)| error)?;
        Ok(Io {
            pending: Box::new(pending),
        })
    }

    pub(in crate::local) fn cancel_pi_edit(
        &mut self,
        runtime_id: &str,
    ) -> Result<bool, RuntimeError> {
        let Some(edit) = &self.session.pi_edit else {
            return Ok(false);
        };
        if edit.runtime.id != runtime_id {
            return Err(RuntimeError::invalid("编辑运行时身份不匹配"));
        }
        let cancelled = edit.cancelled.clone();
        if self.session.pi_control.is_some() {
            return Err(RuntimeError::Busy);
        }
        cancelled.store(true, Ordering::Release);
        self.session.cancel_requested = true;
        self.session.send_queue.pause_after_stop();
        if matches!(self.session.active_state, ActiveState::Pi(_))
            && !self.try_start_pi_control(None)?
        {
            self.session
                .pi()?
                .cancel()
                .map_err(|error| RuntimeError::context("停止消息编辑", error))?;
        }
        self.advance_pi_edit();
        Ok(true)
    }

    /// Returns on every pending dependency. No sleep, native RPC wait, source
    /// helper or cold startup is allowed on this path.
    pub(in crate::local) fn advance_pi_edit(&mut self) {
        let Some(mut edit) = self.session.pi_edit.take() else {
            return;
        };
        self.session.pi_edit_advancing = true;
        let result = self.advance_pi_edit_phase(&mut edit);
        self.session.pi_edit_advancing = false;
        match result {
            Ok(false) => self.session.pi_edit = Some(edit),
            Ok(true) => {
                self.execution_results
                    .insert(edit.application, Ok(ExecutionSuccess::default()));
            }
            Err(error) => {
                if let Phase::Source(ticket)
                | Phase::Branch(ticket)
                | Phase::History(ticket)
                | Phase::Model(ticket)
                | Phase::Prompt(ticket) = &edit.phase
                    && let ActiveState::Pi(pi) = &mut self.session.active_state
                {
                    pi.cancel_confirmation(*ticket);
                }
                self.session.send_queue.pause_after_stop();
                if !error.delivery_accepted() {
                    self.session.transcript_projection.withdraw_request();
                }
                let error = if let Some(target) = &edit.recovery_target {
                    if !edit.fallback_committed {
                        RuntimeError::operation_completed(format!(
                            "原生副本 {} 已创建、替换未发送；原关联保留，请核对恢复副本：{error}",
                            target.display()
                        ))
                    } else {
                        error
                    }
                } else if edit.branched && edit.deletion.is_some() {
                    RuntimeError::operation_completed(format!(
                        "消息删除已执行，但新历史未确认；请重新读取核对：{error}"
                    ))
                } else if edit.branched && !matches!(edit.phase, Phase::Prompt(_)) {
                    RuntimeError::operation_completed(format!(
                        "消息编辑已执行回退，但尚未发送替换内容；请重新读取会话并核对：{error}"
                    ))
                } else {
                    error
                };
                if edit.deletion.is_some()
                    && (edit.branched || matches!(edit.phase, Phase::DeleteComplete(_)))
                    && self.session.key == edit.session
                    && matches!(&self.session.active_state, ActiveState::History(snapshot) if snapshot.conversation.id == edit.native_id)
                {
                    // Known/unknown mutation invalidates only this old cached
                    // tail. Never overwrite a newly addressed source with it.
                    self.session.active_state = ActiveState::Empty;
                    self.session.logical_projection = None;
                    self.session.source = None;
                }
                // Keep the physical owner/HOME until bounded cleanup completes;
                // draining a receipt or timing out is not resource release.
                if matches!(self.session.active_state, ActiveState::Pi(_)) {
                    self.session.pi_cleanup_required = true;
                    if !error.delivery_accepted()
                        && (edit.branched || matches!(edit.phase, Phase::Branch(_)))
                    {
                        self.session
                            .pi()
                            .expect("Pi owner retained")
                            .clear_edit_projection();
                        self.session.logical_projection = None;
                        self.session.source = None;
                    }
                    let _ = self.start_native_cleanup_worker();
                }
                let phase = std::mem::replace(&mut edit.phase, Phase::Stop);
                if let Phase::Sources(io)
                | Phase::Launch(io)
                | Phase::DeleteRead(io)
                | Phase::DeleteComplete(io)
                | Phase::Verify(io)
                | Phase::Fork(io)
                | Phase::FallbackRead(io) = phase
                {
                    io.pending.abandon();
                    self.session.pi_io_cleanup = Some(io.pending);
                }
                self.execution_results.insert(edit.application, Err(error));
            }
        }
    }

    fn advance_pi_edit_phase(&mut self, edit: &mut PiEdit) -> Result<bool, RuntimeError> {
        #[cfg(all(test, unix))]
        if matches!(edit.phase, Phase::Prompt(_))
            && let Some(expired) = execution_workers::pi_edit_deadline_test_gate::take(
                edit.runtime.settings.get("binary"),
            )
        {
            edit.preparation_deadline = Instant::now() - Duration::from_secs(1);
            let _ = expired.send(());
        }
        // A received native acceptance remains a business fact even if a
        // later cancel/source change wins the refresh. No successor is released.
        if let Phase::Prompt(ticket) = &edit.phase
            && let Some(result) = self
                .session
                .pi()?
                .take_confirmation(*ticket)
                .map_err(|error| RuntimeError::context("替换输入票据", error))?
        {
            result.map_err(|error| dependency_error("替换输入未确认", error))?;
            if self.session.pi()?.requires_failure_cleanup() {
                return Err(RuntimeError::after_delivery(
                    "替换输入已被原生接受，但连接已失败；请核对结果，不要重复发送",
                ));
            }
            self.restore_archived(&edit.conversation_id);
            return Ok(true);
        }
        if matches!(edit.phase, Phase::DeleteComplete(_) | Phase::DeleteReady(_)) {
            // Harvest an available mutation fact before evaluating install
            // authority. Pending IO keeps the same fact channel and lease.
            let completion = if let Phase::DeleteComplete(io) = &mut edit.phase {
                io.take()
            } else {
                None
            };
            let harvested = completion.is_some();
            if let Some(result) = completion {
                let (_, PiEditIoValue::Deleted(loaded)) = result.map_err(|error| {
                    RuntimeError::operation_indeterminate(format!(
                        "消息删除操作已结束，但修改结果未确认；请核对原生备份，不要重试：{error}"
                    ))
                })?
                else {
                    unreachable!()
                };
                edit.branched = true;
                edit.phase = Phase::DeleteReady(loaded);
            }
            if edit.delete_install_error.is_none() {
                edit.delete_install_error = self.check_pi_edit_install_fence(edit).err();
            }
            if harvested || matches!(edit.phase, Phase::DeleteComplete(_)) {
                return Ok(false);
            }
            #[cfg(all(test, unix))]
            if execution_workers::pi_delete_test_gate::install_held(
                edit.runtime.settings.get("binary"),
            ) {
                return Ok(false);
            }
            if let Some(error) = edit.delete_install_error.take() {
                return Err(error);
            }
            let Phase::DeleteReady(loaded) = std::mem::replace(&mut edit.phase, Phase::Stop) else {
                unreachable!()
            };
            let fresh = self.finish_history_read(*loaded?)?;
            self.session.set_history(fresh);
            return Ok(true);
        }
        self.check_pi_edit_install_fence(edit)?;
        match &mut edit.phase {
            Phase::Sources(io) => {
                let Some(result) = io.take() else {
                    return Ok(false);
                };
                let (_, PiEditIoValue::Sources(loaded, input)) = result? else {
                    unreachable!()
                };
                let current = self
                    .current_snapshot()
                    .ok_or_else(|| RuntimeError::invalid("会话未打开"))?;
                self.check_edit_revision(&current, Some(&edit.revision))?;
                let sources = loaded
                    .into_iter()
                    .map(|loaded| self.finish_history_read(loaded))
                    .collect::<Result<Vec<_>, _>>()?;
                self.check_pi_edit_sources(&current, edit.link.as_ref(), &sources)?;
                let source = sources
                    .into_iter()
                    .last()
                    .ok_or_else(|| RuntimeError::invalid("会话缺少来源"))?;
                if !source.messages.iter().any(|message| {
                    message.id == edit.message_id && message.role == MessageRole::User
                }) {
                    return Err(RuntimeError::invalid("会话中没有这条用户消息"));
                }
                edit.native_digest = Some(digest(&source.messages)?);
                let boundary = source
                    .messages
                    .iter()
                    .position(|message| message.id == edit.message_id)
                    .expect("validated Pi boundary");
                edit.retained_content = Some(super::super::continuation::prefix_digest(
                    &source.messages[..boundary],
                )?);
                edit.retained_digest = Some((boundary, digest(&source.messages[..boundary])?));
                if input.is_some() {
                    edit.input = input;
                }
                edit.source_to_prepare = Some(source);
                edit.phase = Phase::PrepareSource;
            }
            Phase::PrepareSource => {
                if self.session.work_pending() && !self.turn_running() {
                    return Ok(false);
                }
                let source = edit
                    .source_to_prepare
                    .take()
                    .expect("captured native source");
                if self.turn_running() {
                    // Pi cancellation is admission-only and stays co-located.
                    self.session
                        .pi()?
                        .cancel()
                        .map_err(|error| RuntimeError::context("停止编辑来源", error))?;
                    self.session.cancel_requested = false;
                    edit.phase = Phase::Stop;
                    return Ok(false);
                }
                self.ensure_work_ended()?;
                let reusable = matches!(self.session.active_state, ActiveState::Pi(_))
                    && !self.session.configuration_changed
                    && self
                        .session
                        .execution_configuration
                        .as_ref()
                        .is_some_and(|configuration| configuration.runtime == edit.runtime)
                    && self.session.model_selection.as_ref() == Some(&edit.selection);
                if reusable {
                    let key = self.session.model_record_key.clone();
                    edit.phase = Phase::Source(
                        self.session
                            .pi()?
                            .begin_edit_history(&edit.runtime.id, key)
                            .map_err(|error| RuntimeError::context("同步编辑来源", error))?,
                    );
                } else {
                    if self.session.active_state.live().is_some() {
                        // This is terminal destruction, not a live adapter loan.
                        self.start_native_cleanup_worker()?;
                        edit.phase = Phase::Retire;
                        return Ok(false);
                    }
                    let launch = self.capture_pi_edit_launch(edit, source)?;
                    edit.phase = Phase::Launch(self.pi_edit_io(edit.application, launch)?);
                }
            }
            Phase::Stop | Phase::Retire => {
                if matches!(self.session.active_state, ActiveState::Pending(_))
                    || self.turn_running()
                    || self.session.work_pending()
                {
                    return Ok(false);
                }
                let current = self
                    .current_snapshot()
                    .ok_or_else(|| RuntimeError::invalid("会话未打开"))?;
                self.check_edit_revision(&current, Some(&edit.revision))?;
                let (reads, _) = self.capture_pi_edit_sources(&current)?;
                edit.phase = Phase::Sources(self.pi_edit_io(edit.application, move |_| {
                    Ok(PiEditIoValue::Sources(
                        reads
                            .into_iter()
                            .map(browsing::HistoryRead::load)
                            .collect::<Result<Vec<_>, _>>()?,
                        None,
                    ))
                })?);
            }
            Phase::Launch(io) => {
                let Some(result) = io.take() else {
                    return Ok(false);
                };
                let (state, PiEditIoValue::Launch(launch)) = result? else {
                    unreachable!()
                };
                let PiEditLaunch {
                    runner,
                    tunnel,
                    prepared_resources,
                } = *launch;
                self.session.active_state = state;
                self.session.gateway_runner = runner;
                self.session.gateway_tunnel = tunnel;
                if let Some(key) = prepared_resources {
                    self.prepared_remote_resources.borrow_mut().insert(key);
                }
                let gateway =
                    if matches!(edit.selection, Some(crate::ModelSelection::Managed { .. })) {
                        Some(
                            self.gateways
                                .iter()
                                .find(|gateway| gateway.id == edit.runtime.gateway_id)
                                .cloned()
                                .ok_or_else(|| RuntimeError::invalid("gateway id"))?,
                        )
                    } else {
                        None
                    };
                self.capture_execution_configuration(
                    &edit.runtime,
                    gateway.as_ref().map(|gateway| {
                        (
                            gateway,
                            velune_gateway::SessionIdentity {
                                namespace: "velune.conversation".into(),
                                id: edit.conversation_id.clone(),
                            },
                        )
                    }),
                );
                self.session.execution_runtime_id = Some(edit.runtime.id.clone());
                self.session.model_selection = Some(edit.selection.clone());
                self.session.model_record_key = match &edit.selection {
                    Some(crate::ModelSelection::Managed { model_record_key }) => {
                        Some(model_record_key.clone())
                    }
                    _ => None,
                };
                let key = self.session.model_record_key.clone();
                let history = self
                    .session
                    .pi()?
                    .begin_edit_history(&edit.runtime.id, key)
                    .map_err(|error| RuntimeError::context("同步编辑来源", error))?;
                edit.phase = if edit.fallback_committed {
                    Phase::History(history)
                } else {
                    Phase::Source(history)
                };
            }
            Phase::Source(ticket) => {
                let Some(result) = self
                    .session
                    .pi()?
                    .take_confirmation(*ticket)
                    .map_err(|error| RuntimeError::context("编辑来源票据", error))?
                else {
                    return Ok(false);
                };
                result.map_err(|error| RuntimeError::context("同步编辑来源", error))?;
                if self.session.projected().is_none_or(|snapshot| {
                    digest(&snapshot.messages).ok().as_ref() != edit.native_digest.as_ref()
                }) {
                    return Err(edit_history_changed("prepared_source"));
                }
                edit.phase = Phase::SourceVerified;
            }
            Phase::SourceVerified => {
                if self.session.work_pending() {
                    return Ok(false);
                }
                let current = self
                    .current_snapshot()
                    .ok_or_else(|| RuntimeError::invalid("会话未打开"))?;
                let (reads, _) = self.capture_pi_edit_sources(&current)?;
                let source = PathBuf::from(
                    edit.native_id
                        .strip_prefix(&format!("{}:", edit.runtime.id))
                        .ok_or_else(|| RuntimeError::invalid("Pi 来源身份无效"))?,
                );
                let execution = self.pi_history_configuration(&edit.runtime)?.execution;
                edit.phase = Phase::Verify(self.pi_edit_io(edit.application, move |_| {
                    use sha2::{Digest, Sha256};
                    let before = execution
                        .read_file(&source)
                        .map_err(|error| RuntimeError::context("捕获编辑来源摘要", error))?;
                    let loaded = reads
                        .into_iter()
                        .map(browsing::HistoryRead::load)
                        .collect::<Result<Vec<_>, _>>()?;
                    if execution
                        .read_file(&source)
                        .map_err(|error| RuntimeError::context("核对编辑来源摘要", error))?
                        != before
                    {
                        return Err(RuntimeError::invalid("编辑来源读取期间已改变"));
                    }
                    Ok(PiEditIoValue::VerifiedSources(
                        loaded,
                        format!("{:x}", Sha256::digest(before)),
                    ))
                })?);
            }
            Phase::Verify(io) => {
                let Some(result) = io.take() else {
                    return Ok(false);
                };
                let (_, PiEditIoValue::VerifiedSources(loaded, source_digest)) = result? else {
                    unreachable!()
                };
                let sources = loaded
                    .into_iter()
                    .map(|loaded| self.finish_history_read(loaded))
                    .collect::<Result<Vec<_>, _>>()?;
                let current = self
                    .current_snapshot()
                    .ok_or_else(|| RuntimeError::invalid("会话未打开"))?;
                self.check_pi_edit_sources(&current, edit.link.as_ref(), &sources)?;
                if sources.last().is_none_or(|source| {
                    digest(&source.messages).ok().as_ref() != edit.native_digest.as_ref()
                }) {
                    return Err(edit_history_changed("source_read"));
                }
                edit.authorized_source_digest = Some(source_digest);
                self.require_input_access()?;
                if edit.deletion.is_some() {
                    edit.phase = Phase::DeleteRetire;
                    return Ok(false);
                }
                edit.phase = Phase::Branch(
                    self.session
                        .pi()?
                        .begin_edit_branch(&edit.entry)
                        .map_err(|error| RuntimeError::context("消息编辑", error))?,
                );
            }
            Phase::Branch(ticket) => {
                let Some(result) = self
                    .session
                    .pi()?
                    .take_confirmation(*ticket)
                    .map_err(|error| RuntimeError::context("编辑回退票据", error))?
                else {
                    return Ok(false);
                };
                if let Err(error) = result {
                    if self.session.pi()?.take_edit_fallback(*ticket) {
                        self.start_pi_edit_fallback(edit)?;
                        return Ok(false);
                    }
                    return Err(dependency_error("消息编辑回退未完成；替换未发送", error));
                }
                edit.branched = true;
                let key = self.session.model_record_key.clone();
                edit.phase = Phase::History(
                    self.session
                        .pi()?
                        .begin_edit_history(&edit.runtime.id, key)
                        .map_err(|error| RuntimeError::context("同步编辑回退", error))?,
                );
            }
            Phase::DeleteRetire => {
                if matches!(self.session.active_state, ActiveState::Pending(_)) {
                    return Ok(false);
                }
                if self.session.active_state.live().is_some() {
                    let _ = self.start_native_cleanup_worker();
                    return Ok(false);
                }
                let current = self
                    .current_snapshot()
                    .ok_or_else(|| RuntimeError::invalid("会话未打开"))?;
                let (reads, _) = self.capture_pi_edit_sources(&current)?;
                let path = PathBuf::from(
                    edit.native_id
                        .strip_prefix(&format!("{}:", edit.runtime.id))
                        .ok_or_else(|| RuntimeError::invalid("Pi 删除来源身份"))?,
                );
                edit.phase = Phase::DeleteRead(self.pi_edit_io(edit.application, move |_| {
                    use sha2::{Digest, Sha256};
                    let before = fs::read(&path)?;
                    let loaded = reads
                        .into_iter()
                        .map(browsing::HistoryRead::load)
                        .collect::<Result<Vec<_>, _>>()?;
                    if fs::read(&path)? != before {
                        return Err(RuntimeError::invalid("Pi 删除来源读取期间已改变"));
                    }
                    Ok(PiEditIoValue::DeleteSources(
                        loaded,
                        format!("{:x}", Sha256::digest(before)),
                    ))
                })?);
            }
            Phase::DeleteRead(io) => {
                let Some(result) = io.take() else {
                    return Ok(false);
                };
                let (_, PiEditIoValue::DeleteSources(loaded, expected_digest)) = result? else {
                    unreachable!()
                };
                let sources = loaded
                    .into_iter()
                    .map(|loaded| self.finish_history_read(loaded))
                    .collect::<Result<Vec<_>, _>>()?;
                let current = self
                    .current_snapshot()
                    .ok_or_else(|| RuntimeError::invalid("会话未打开"))?;
                self.check_edit_revision(&current, Some(&edit.revision))?;
                self.check_pi_edit_sources(&current, edit.link.as_ref(), &sources)?;
                if sources.last().is_none_or(|source| {
                    digest(&source.messages).ok().as_ref() != edit.native_digest.as_ref()
                }) {
                    return Err(RuntimeError::invalid("Pi 删除来源已改变"));
                }
                self.require_input_access()?;
                self.backup_edit_links()?;
                let config = self.pi_history_configuration(&edit.runtime)?;
                if !matches!(
                    config.execution,
                    velune_agent_runtime::execution::ExecutionTarget::Local
                ) {
                    return Err(RuntimeError::invalid("远程 Pi 暂不支持安全删除消息"));
                }
                let preparation = self.execution_preparation(&edit.runtime)?.1;
                let path = PathBuf::from(
                    edit.native_id
                        .strip_prefix(&format!("{}:", edit.runtime.id))
                        .expect("validated native id"),
                );
                let id = edit.message_id.clone();
                let truncate = edit.deletion.expect("delete action");
                let backup = self.options.home_directory.join("conversation-edits");
                let read = self.prepare_history_read(&edit.runtime.id, &edit.native_id)?;
                #[cfg(all(test, unix))]
                let binary = edit.runtime.settings.get("binary").cloned();
                edit.phase =
                    Phase::DeleteComplete(self.pi_edit_io(edit.application, move |_| {
                        preparation.prepare()?;
                        velune_agent_runtime::history::delete_pi_message(
                            &config,
                            &path,
                            &id,
                            truncate,
                            &expected_digest,
                            &backup,
                        )
                        .map_err(|error| RuntimeError::context("删除 Pi 消息", error))?;
                        #[cfg(all(test, unix))]
                        execution_workers::pi_delete_test_gate::after_mutation(binary);
                        Ok(PiEditIoValue::Deleted(read.load().map(Box::new)))
                    })?);
            }
            Phase::DeleteComplete(_) | Phase::DeleteReady(_) => {
                unreachable!("mutation facts harvested before fences")
            }
            Phase::Fork(io) => {
                let Some(result) = io.take() else {
                    return Ok(false);
                };
                let (_, PiEditIoValue::Fork(target)) = result? else {
                    unreachable!()
                };
                edit.recovery_target = Some(target.clone());
                let current = self
                    .current_snapshot()
                    .ok_or_else(|| RuntimeError::invalid("会话未打开"))?;
                self.check_edit_revision(&current, Some(&edit.revision))?;
                let (mut reads, _) = self.capture_pi_edit_sources(&current)?;
                reads.push(self.prepare_history_read(
                    &edit.runtime.id,
                    &format!("{}:{}", edit.runtime.id, target.display()),
                )?);
                edit.phase = Phase::FallbackRead(self.pi_edit_io(edit.application, move |_| {
                    Ok(PiEditIoValue::Sources(
                        reads
                            .into_iter()
                            .map(browsing::HistoryRead::load)
                            .collect::<Result<Vec<_>, _>>()?,
                        None,
                    ))
                })?);
            }
            Phase::FallbackRead(io) => {
                let Some(result) = io.take() else {
                    return Ok(false);
                };
                let (_, PiEditIoValue::Sources(loaded, _)) = result? else {
                    unreachable!()
                };
                let mut sources = loaded
                    .into_iter()
                    .map(|loaded| self.finish_history_read(loaded))
                    .collect::<Result<Vec<_>, _>>()?;
                let target = sources
                    .pop()
                    .ok_or_else(|| RuntimeError::invalid("原生副本没有历史"))?;
                let current = self
                    .current_snapshot()
                    .ok_or_else(|| RuntimeError::invalid("会话未打开"))?;
                self.check_edit_revision(&current, Some(&edit.revision))?;
                self.check_pi_edit_sources(&current, edit.link.as_ref(), &sources)?;
                if sources.last().is_none_or(|source| {
                    digest(&source.messages).ok().as_ref() != edit.native_digest.as_ref()
                }) || super::super::continuation::prefix_digest(&target.messages)
                    .ok()
                    .as_ref()
                    != edit.retained_content.as_ref()
                {
                    return Err(RuntimeError::invalid("原生副本或编辑来源未匹配保留前缀"));
                }
                let projection =
                    self.pi_edit_fallback_projection(edit, &current, &sources, &target)?;
                edit.fallback_install = Some((target, projection));
                edit.phase = Phase::FallbackRetire;
            }
            Phase::FallbackRetire => {
                if matches!(self.session.active_state, ActiveState::Pending(_)) {
                    return Ok(false);
                }
                if self.session.active_state.live().is_some() {
                    // Only actual terminal cleanup may transfer this adapter.
                    let _ = self.start_native_cleanup_worker();
                    return Ok(false);
                }
                let current = self
                    .current_snapshot()
                    .ok_or_else(|| RuntimeError::invalid("会话未打开"))?;
                self.check_edit_revision(&current, Some(&edit.revision))?;
                if !edit.fallback_closed {
                    edit.fallback_closed = true;
                    let (mut reads, _) = self.capture_pi_edit_sources(&current)?;
                    let target = edit
                        .recovery_target
                        .as_ref()
                        .expect("verified fork identity");
                    reads.push(self.prepare_history_read(
                        &edit.runtime.id,
                        &format!("{}:{}", edit.runtime.id, target.display()),
                    )?);
                    edit.phase =
                        Phase::FallbackRead(self.pi_edit_io(edit.application, move |_| {
                            Ok(PiEditIoValue::Sources(
                                reads
                                    .into_iter()
                                    .map(browsing::HistoryRead::load)
                                    .collect::<Result<Vec<_>, _>>()?,
                                None,
                            ))
                        })?);
                    return Ok(false);
                }

                let (target, projection) = edit
                    .fallback_install
                    .take()
                    .expect("verified fork retained");
                let mut links = self.conversation_links.clone();
                links.retain(|link| link.id != projection.link.id);
                links.push(projection.link.clone());
                self.backup_edit_links()?;
                self.store_links(links)?;
                edit.link = Some(projection.link.clone());
                edit.native_id = target.conversation.id.clone();
                edit.fallback_committed = true;
                edit.branched = true;
                self.session.logical_projection = Some(projection);
                // Association commits first. Any subsequent failure leaves this
                // exact target available; it is never re-created or replayed.
                let launch = self.capture_pi_edit_launch(edit, target.try_clone()?)?;
                self.session.active_state = ActiveState::History(Box::new(target));
                edit.phase = Phase::Launch(self.pi_edit_io(edit.application, launch)?);
            }
            Phase::History(ticket) => {
                let Some(result) = self
                    .session
                    .pi()?
                    .take_confirmation(*ticket)
                    .map_err(|error| RuntimeError::context("编辑历史票据", error))?
                else {
                    return Ok(false);
                };
                result.map_err(|error| RuntimeError::context("同步编辑回退", error))?;
                let (count, expected) = edit
                    .retained_digest
                    .as_ref()
                    .expect("validated retained prefix");
                let native = self
                    .session
                    .projected()
                    .ok_or_else(|| RuntimeError::invalid("回退缺少原生历史"))?;
                let messages: Vec<_> = native
                    .messages
                    .iter()
                    .filter(native_edit_message)
                    .cloned()
                    .collect();
                let matches = if edit.fallback_committed {
                    super::super::continuation::prefix_digest(&messages)
                        .ok()
                        .as_ref()
                        == edit.retained_content.as_ref()
                } else {
                    digest(&messages)? == *expected
                };
                if messages.len() != *count || !matches {
                    return Err(RuntimeError::invalid(
                        "回退历史未匹配原生编辑前缀；替换未发送",
                    ));
                }
                self.sync_projection()?;
                match &edit.selection {
                    Some(crate::ModelSelection::Managed { .. }) => {
                        edit.phase = Phase::Model(
                            self.session
                                .pi()?
                                .begin_edit_model(Some("velune"), "auto")
                                .map_err(|error| RuntimeError::context("编辑模型选择", error))?,
                        );
                    }
                    Some(crate::ModelSelection::Native {
                        provider_id,
                        model_id,
                    }) => {
                        edit.phase = Phase::Model(
                            self.session
                                .pi()?
                                .begin_edit_model(provider_id.as_deref(), model_id)
                                .map_err(|error| RuntimeError::context("编辑模型选择", error))?,
                        );
                    }
                    None => {
                        self.admit_pi_edit_prompt(edit)?;
                    }
                }
            }
            Phase::Model(ticket) => {
                let Some(result) = self
                    .session
                    .pi()?
                    .take_confirmation(*ticket)
                    .map_err(|error| RuntimeError::context("编辑模型票据", error))?
                else {
                    return Ok(false);
                };
                result.map_err(|error| RuntimeError::context("编辑模型选择", error))?;
                self.admit_pi_edit_prompt(edit)?;
            }
            Phase::Prompt(_) => {}
        }
        Ok(false)
    }

    fn check_pi_edit_install_fence(&self, edit: &PiEdit) -> Result<(), RuntimeError> {
        if edit.session != self.session.key
            || !edit.sources.iter().all(|source| source.is_current(self))
            || self.runtime_instance(&edit.runtime.id)? != &edit.runtime
            || edit.gateway.as_ref().is_some_and(|captured| {
                self.gateways
                    .iter()
                    .find(|gateway| gateway.id == captured.id)
                    != Some(captured)
            })
            || self.link_of(&edit.conversation_id) != edit.link.as_ref()
            || (self.session.configuration_changed
                && matches!(self.session.active_state, ActiveState::Pi(_)))
        {
            return Err(RuntimeError::operation_indeterminate(
                "编辑期间会话来源或执行配置已改变；替换未重放",
            ));
        }
        if self
            .session
            .projected()
            .is_some_and(|snapshot| snapshot.conversation.id != edit.native_id)
        {
            return Err(RuntimeError::operation_indeterminate(
                "Pi 原生会话身份已改变；替换未重放",
            ));
        }
        if !matches!(edit.phase, Phase::Prompt(_))
            && (edit.cancelled.load(Ordering::Acquire) || self.session.cancel_requested)
        {
            return Err(RuntimeError::invalid("消息编辑已取消；替换未重放"));
        }
        // An admitted prompt retains its exact fact channel after user Cancel.
        // The independent reserved control stops native work; its ACK cannot
        // fabricate acceptance/rejection or erase a later matching prompt fact.
        // This budget authorizes preparation only, not model or human time.
        // Once prompt admission occurs, the SDK retains its finite physical
        // write budget and exact outcome channel. Edit exposes no run budget;
        // do not turn the old preparation deadline into an implicit one.
        if !matches!(edit.phase, Phase::Prompt(_)) && Instant::now() >= edit.preparation_deadline {
            return Err(RuntimeError::operation_indeterminate(
                "Pi 消息编辑准备等待超时；请核对原生结果，不要重复发送",
            ));
        }
        Ok(())
    }

    fn admit_pi_edit_prompt(&mut self, edit: &mut PiEdit) -> Result<(), RuntimeError> {
        self.refresh_transcript();
        let input = edit.input.as_mut().expect("validated input retained");
        let (effort, permission_mode) =
            self.turn_options(input.effort.as_deref(), input.permission_mode.as_deref())?;
        input.effort = effort;
        input.permission_mode = permission_mode;
        self.request_turn_settings(&edit.runtime.id, input);
        self.begin_turn_observation();
        if let Some((gateway_id, session)) = self.backend_session_identity() {
            self.session_backends()
                .begin_user_turn(&gateway_id, &session)
                .map_err(|error| RuntimeError::context("CLI 新轮准备", error))?;
        }
        let ticket = self
            .session
            .pi()?
            .begin_edit_prompt(input)
            .map_err(|error| RuntimeError::context("替换输入准入", error))?;
        edit.phase = Phase::Prompt(ticket);
        self.session.cancel_requested = false;
        self.follow_started_turn();
        self.started_turn_observation();
        Ok(())
    }

    fn check_pi_edit_sources(
        &self,
        current: &ConversationSnapshot,
        link: Option<&crate::config::ConversationLink>,
        sources: &[browsing::HistorySnapshot],
    ) -> Result<(), RuntimeError> {
        let mut displayed = Vec::new();
        for (index, source) in sources.iter().enumerate() {
            if let Some(link) = link {
                let segment = &link.segments[index];
                let count = segment
                    .cutoff
                    .as_ref()
                    .map(|cutoff| {
                        usize::try_from(cutoff.message_count)
                            .map_err(|_| RuntimeError::invalid("会话截止位置无效"))
                    })
                    .transpose()?
                    .unwrap_or(source.messages.len());
                let Some(prefix) = source.messages.get(..count) else {
                    if let Some(cutoff) = &segment.cutoff {
                        super::super::continuation::trace_prefix_mismatch(
                            "conversation_edit_source_conflict",
                            &link.id,
                            index,
                            segment,
                            "sealed",
                            cutoff,
                            &source.messages,
                        );
                    }
                    return Err(RuntimeError::invalid("会话来源已改变"));
                };
                if let Some(cutoff) = &segment.cutoff
                    && !super::super::continuation::prefix_digest_matches(
                        prefix,
                        &cutoff.prefix_digest,
                    )?
                {
                    super::super::continuation::trace_prefix_mismatch(
                        "conversation_edit_source_conflict",
                        &link.id,
                        index,
                        segment,
                        "sealed",
                        cutoff,
                        &source.messages,
                    );
                    return Err(RuntimeError::invalid("会话来源已改变"));
                }
                if let Some(imported) = &segment.imported
                    && super::super::continuation::verified_import_prefix(segment, &source.messages)
                        .is_none()
                {
                    super::super::continuation::trace_prefix_mismatch(
                        "conversation_edit_source_conflict",
                        &link.id,
                        index,
                        segment,
                        "imported",
                        &imported.prefix,
                        &source.messages,
                    );
                    return Err(RuntimeError::invalid("会话导入前缀已改变"));
                }
                if super::super::continuation::shows_boundary(link, index) {
                    displayed.push(self.boundary_message(segment, index));
                }
                displayed.extend(super::super::continuation::display_segment(
                    segment, prefix, index,
                )?);
            } else {
                displayed.extend(source.messages.iter().cloned());
            }
        }
        if !same_edit_history(current, &displayed) {
            return Err(edit_history_changed("source_read"));
        }
        Ok(())
    }

    fn start_pi_edit_fallback(&mut self, edit: &mut PiEdit) -> Result<(), RuntimeError> {
        let current = self
            .current_snapshot()
            .ok_or_else(|| RuntimeError::invalid("会话未打开"))?;
        self.check_edit_revision(&current, Some(&edit.revision))?;
        let config = self.pi_history_configuration(&edit.runtime)?;
        let preparation = self.execution_preparation(&edit.runtime)?.1;
        let source = PathBuf::from(
            edit.native_id
                .strip_prefix(&format!("{}:", edit.runtime.id))
                .ok_or_else(|| RuntimeError::invalid("Pi 来源身份无效"))?,
        );
        let entry = edit.entry.clone();
        let expected_digest = edit
            .authorized_source_digest
            .clone()
            .ok_or_else(|| RuntimeError::invalid("编辑缺少已授权来源摘要"))?;
        edit.phase = Phase::Fork(self.pi_edit_io(edit.application, move |_| {
            preparation.prepare()?;
            let target = velune_agent_runtime::history::fork_pi_session(
                &config,
                &source,
                velune_agent_runtime::history::PiForkBoundary::BeforeEditedEntry {
                    entry: &entry,
                    expected_digest: &expected_digest,
                },
            )
            .map_err(super::super::native_composition::fork_failure)?;
            Ok(PiEditIoValue::Fork(target))
        })?);
        Ok(())
    }

    fn pi_edit_fallback_projection(
        &self,
        edit: &PiEdit,
        current: &ConversationSnapshot,
        sources: &[browsing::HistorySnapshot],
        target: &ConversationSnapshot,
    ) -> Result<super::super::continuation::LogicalProjection, RuntimeError> {
        use super::super::continuation::{LogicalProjection, prefix_digest};
        use crate::config::{ConversationCutoff, ConversationImport, ConversationLink};
        let source = sources
            .last()
            .ok_or_else(|| RuntimeError::invalid("编辑来源缺失"))?;
        let (count, _) = edit
            .retained_digest
            .as_ref()
            .expect("validated native prefix");
        let mut link = edit.link.clone().unwrap_or(ConversationLink {
            id: edit.conversation_id.clone(),
            segments: vec![self.segment_reference(source)?],
        });
        let display_id = if edit.link.is_some() {
            format!("segment-{}:{}", link.segments.len() - 1, edit.message_id)
        } else {
            edit.message_id.clone()
        };
        let boundary = current
            .messages
            .iter()
            .position(|message| message.id == display_id)
            .ok_or_else(|| RuntimeError::invalid("逻辑编辑边界已改变"))?;
        link.segments.last_mut().expect("native source tail").cutoff = Some(ConversationCutoff {
            message_count: *count as u64,
            prefix_digest: prefix_digest(&source.messages[..*count])?,
        });
        let mut segment = self.segment_reference(target)?;
        segment.imported = Some(ConversationImport {
            summary_work_id: None,
            prefix: ConversationCutoff {
                message_count: target.messages.len() as u64,
                prefix_digest: prefix_digest(&target.messages)?,
            },
        });
        link.segments.push(segment);
        let prefix = current.messages[..boundary].to_vec();
        let mut executions = current.executions.clone();
        executions.retain(&prefix);
        Ok(LogicalProjection {
            origin: self
                .session
                .logical_projection
                .as_ref()
                .map(|projection| projection.origin.clone())
                .unwrap_or_else(|| current.conversation.clone()),
            work_notification_ids: current
                .work_notification_ids
                .iter()
                .filter(|id| prefix.iter().any(|message| &message.id == *id))
                .cloned()
                .collect(),
            retained_reply_ids: current
                .retained_reply_ids
                .iter()
                .filter(|id| prefix.iter().any(|message| &message.id == *id))
                .cloned()
                .collect(),
            prefix,
            executions,
            link,
            retention: sources.iter().try_fold(
                velune_agent_runtime::creation::DataRetention::default(),
                |retention, source| {
                    source
                        .retention
                        .try_clone()
                        .map(|guard| retention.merge(guard))
                        .map_err(|error| RuntimeError::context("history edit prefix credit", error))
                },
            )?,
            sealed_sources: self
                .session
                .logical_projection
                .as_ref()
                .map(|projection| projection.sealed_sources.clone())
                .unwrap_or_default(),
        })
    }

    fn capture_pi_edit_launch(
        &mut self,
        edit: &PiEdit,
        source: impl Into<browsing::HistorySnapshot>,
    ) -> Result<Launch, RuntimeError> {
        let source = source.into();
        let runtime = &edit.runtime;
        let (context, preparation) = self.execution_preparation(runtime)?;
        let mut config = self.pi_history_configuration(runtime)?;
        let managed_key = match &edit.selection {
            Some(crate::ModelSelection::Managed { model_record_key }) => {
                Some(model_record_key.as_str())
            }
            _ => None,
        };
        config.model_compaction_reserves =
            crate::local::pi_composition::pi_compaction_reserves(runtime, managed_key)?;
        config.mcp_servers = self.collaboration_servers();
        config.rpc_entry = Some(context.resources.join("pi_rpc.mjs"));
        config.extension = Some(context.resources.join("pi_virtual_model.mjs"));
        config.sdk_helper = setting_path_optional(runtime, "sdkHelper").or(config.sdk_helper);
        config.working_dir = source.conversation.cwd.as_ref().map(PathBuf::from);
        config.session = Some(PathBuf::from(
            source
                .conversation
                .id
                .strip_prefix(&format!("{}:", runtime.id))
                .ok_or_else(|| RuntimeError::invalid("Pi 会话身份无效"))?,
        ));
        // Resume the source's native model during read/branch preparation.
        // The requested override is sent only after authoritative history is installed.
        let managed = if let Some(crate::ModelSelection::Managed { model_record_key }) =
            &edit.selection
        {
            let gateway = self
                .gateways
                .iter()
                .find(|gateway| gateway.id == runtime.gateway_id)
                .cloned()
                .ok_or_else(|| RuntimeError::invalid("gateway id"))?;
            runnable_pi_model(&gateway, model_record_key)?;
            let physical = gateway
                .pi_binding_id(
                    model_record_key,
                    gateway.authentication_revision(model_record_key),
                )
                .map_err(RuntimeError::invalid)?;
            let aliases = gateway
                .selectable_keys()
                .map(|key| {
                    gateway
                        .pi_binding_id(key, gateway.authentication_revision(key))
                        .map(|alias| (alias, key.to_owned()))
                        .map_err(RuntimeError::invalid)
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            let resolver = crate::authentication_resolver::ProviderResolver::capture_for_dispatch(
                &gateway,
                &self.options,
                self.subscription_quota.clone(),
            );
            let scheduling = self.scheduling_state.clone();
            let quota = self.subscription_quota.source(&gateway.id);
            let backends = self.execution_backends.clone();
            let analytics = self.analytics.clone();
            let telemetry = self.telemetry.clone();
            let identity = velune_gateway::SessionIdentity {
                namespace: "velune.conversation".into(),
                id: edit.conversation_id.clone(),
            };
            let key = model_record_key.clone();
            let protocol = crate::local::pi_composition::pi_gateway_api(
                gateway
                    .resolve_dispatch(&key)
                    .map_err(RuntimeError::invalid)?
                    .provider()
                    .protocol,
            );
            let lane = self.lane_projection(&context.projection);
            config.provider = Some("velune".into());
            config.model = Some("auto".into());
            config.models_path = Some(lane.join("models.json"));
            config.selection_file = Some(lane.join("velune-selection.json"));
            Some((
                gateway, physical, aliases, resolver, scheduling, quota, backends, analytics,
                telemetry, identity, key, protocol,
            ))
        } else {
            None
        };
        Ok(Box::new(move |state| {
            let prepared_resources = preparation.prepare()?;
            if let Some(cwd) = &config.working_dir
                && !config
                    .execution
                    .is_directory(cwd)
                    .map_err(|error| RuntimeError::context("会话工作目录检查", error))?
            {
                return Err(RuntimeError::invalid(
                    "会话工作目录必须是执行机器上的已有目录",
                ));
            }
            velune_agent_runtime::version::check_version_at(
                RuntimeKind::Pi,
                &config.binary,
                config.node_binary.as_deref(),
                &config.execution,
                config.os_home.as_deref(),
            )
            .map_err(|error| RuntimeError::context("运行时版本检查", error))?;
            let (runner, tunnel, model_key) = if let Some((
                gateway,
                physical,
                aliases,
                resolver,
                scheduling,
                quota,
                backends,
                analytics,
                telemetry,
                identity,
                key,
                protocol,
            )) = managed
            {
                let runner = Runner::start_with_scheduling(
                    gateway.to_gateway_config()?,
                    resolver,
                    aliases,
                    Some(analytics),
                    scheduling,
                    quota,
                    Some(identity),
                    backends,
                )
                .map_err(|error| RuntimeError::context("gateway startup", error))?;
                runner.set_telemetry(telemetry, None, None);
                let tunnel = velune_agent_runtime::execution::LoopbackTunnel::open(
                    &config.execution,
                    runner.endpoint(),
                )
                .map_err(|error| RuntimeError::context("网关 SSH 转发", error))?;
                config.gateway_token = Some(runner.token().into());
                materialize_models(&config, &gateway, tunnel.endpoint(), protocol)?;
                write_selection_file(&config, &key, &physical)?;
                (Some(runner), Some(tunnel), Some(key))
            } else {
                (None, None, None)
            };
            config
                .validate_sdk()
                .map_err(|error| RuntimeError::context("Pi SDK 检查", error))?;
            let mut client = pi::Client::spawn(config.clone())
                .map_err(|error| RuntimeError::context("Pi 启动", error))?;
            client
                .state()
                .map_err(|error| RuntimeError::context("Pi readiness", error))?;
            let mut projection = PiProjection::new(source.conversation.clone());
            if let Some(snapshot) = &mut projection.snapshot {
                snapshot.model_record_key = model_key;
            }
            let mut pi = pi::PiSession::new(config);
            pi.start(client, projection);
            pi.restore_context(&source);
            *state = ActiveState::Pi(Box::new(pi));
            Ok(PiEditIoValue::Launch(Box::new(PiEditLaunch {
                runner,
                tunnel,
                prepared_resources,
            })))
        }))
    }
}
