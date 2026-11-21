//! Local application conversations use cases.
use super::*;
use crate::conversation::ConversationSnapshot;
use crate::local::continuation::LogicalProjection;
use crate::{ComposedTurn, ConfigurationSnapshot, ModelSelection, SnapshotResult};

/// A turn the user sends in a conversation: where it comes from, the
/// runtime and model it runs on, and what was composed.
#[derive(Clone)]
pub(crate) struct TurnRequest {
    pub submission_id: Option<String>,
    pub source_runtime_id: String,
    pub conversation_id: String,
    pub runtime_id: String,
    pub selection: Option<ModelSelection>,
    pub turn: ComposedTurn,
}

enum SendTarget {
    Selected,
    Current,
}

/// What one send gives the active runtime. Without a message it resumes
/// the native turn.
pub(super) struct Prompt<'a> {
    pub runtime_id: &'a str,
    /// The text and the image paths it carries.
    pub message: Option<(&'a str, &'a [PathBuf])>,
    pub effort: Option<&'a str>,
    pub permission_mode: Option<&'a str>,
}

/// A change the user makes to a listed conversation.
#[derive(Clone, Copy)]
pub(super) enum ConversationChange<'a> {
    Rename(&'a str),
    Delete,
}

impl CoreRuntime {
    pub(super) fn check_submission_cancelled(
        &self,
        conversation_id: &str,
    ) -> Result<(), RuntimeError> {
        if self
            .submission_cancellation
            .as_ref()
            .is_some_and(|(id, cancelled)| {
                id == conversation_id && cancelled.load(std::sync::atomic::Ordering::Acquire)
            })
        {
            let mut failure = crate::Failure::invalid("发送已取消，消息尚未交给运行时");
            failure.code = "submission_cancelled".into();
            return Err(RuntimeError::Contract(failure));
        }
        Ok(())
    }

    /// A turn that needs another runtime process while sub-agents still run
    /// in the current one is queued; the queue sends it once they have ended.
    pub(crate) fn send_turn(
        &mut self,
        request: TurnRequest,
    ) -> Result<SnapshotResult, RuntimeError> {
        let result = self.send_and_restore(&request);
        self.update_send_control(&result);
        match result {
            Err(RuntimeError::OutstandingWork) => {
                self.check_submission_cancelled(&request.conversation_id)?;
                let queue_id = self.queue_after_work(request);
                self.accepted_submission_result(crate::SubmissionReceipt::Queued { queue_id })
            }
            result => result.map(|mut result| {
                result.submission_receipt = Some(crate::SubmissionReceipt::Sending);
                result
            }),
        }
    }

    /// Continue the granted conversation in its existing execution context.
    /// Resolve the target and submit in the same owner job.
    pub(crate) fn send_current_turn(
        &mut self,
        source_runtime_id: String,
        conversation_id: String,
        turn: ComposedTurn,
    ) -> Result<SnapshotResult, RuntimeError> {
        if self.session.side.is_some()
            || self.session.unsubmitted_draft.is_some()
            || (self.session.projected().is_some()
                && !self.addresses_source(&source_runtime_id, &conversation_id))
        {
            return Err(RuntimeError::invalid("会话来源与共享身份不匹配"));
        }
        let (runtime_id, selection) = if self.session.active_state.live().is_some() {
            let runtime_id = self
                .session
                .projected()
                .ok_or_else(|| RuntimeError::invalid("会话执行尚未准备"))?
                .conversation
                .runtime_id
                .clone();
            (runtime_id, self.session.model_selection.clone())
        } else {
            if self.observe_current_external_turn()
                || self
                    .session
                    .projected()
                    .is_some_and(|view| view.run_state == RunState::Elsewhere)
            {
                return Err(RuntimeError::invalid("另一个运行时进程正在执行此会话"));
            }
            let (history, projection) =
                self.read_conversation_source(&source_runtime_id, &conversation_id)?;
            let runtime_id = history.conversation.runtime_id.clone();
            self.present_history(history, projection);
            let selection = self.history_send_selection(&runtime_id)?;
            (runtime_id, selection)
        };
        let selection = selection.ok_or_else(|| {
            let mut failure = crate::Failure::new(
                crate::FailureKind::Conflict,
                "无法确认此会话的模型来源，请先在宿主选择模型并发送",
            );
            failure.code = "session_model_unconfirmed".into();
            RuntimeError::Contract(failure)
        })?;
        let result = self.send_to_target(
            &TurnRequest {
                submission_id: None,
                source_runtime_id,
                conversation_id,
                runtime_id,
                selection,
                turn,
            },
            SendTarget::Current,
            None,
        );
        self.update_send_control(&result);
        // A future send could see a different target; current-target sends
        // return OutstandingWork instead of queuing the sampled selection.
        result
    }

    pub(super) fn history_send_selection(
        &self,
        runtime_id: &str,
    ) -> Result<Option<Option<ModelSelection>>, RuntimeError> {
        // ACP loads the Agent's own session configuration. An absent override
        // lets the Agent keep that configuration, including its selected model.
        if self.runtime_instance(runtime_id)?.type_id == RuntimeKind::Acp {
            return Ok(Some(None));
        }
        // Pi history can identify its current managed selection directly.
        if let Some(model_record_key) = &self.session.model_record_key {
            return Ok(Some(Some(ModelSelection::Managed {
                model_record_key: model_record_key.clone(),
            })));
        }
        let Some(native) = self.native_snapshot() else {
            return Ok(None);
        };
        let Some(latest) = user_message_ids(&native).next() else {
            return Ok(None);
        };
        let latest = self.projected_conversation_message_id(&native.conversation.id, latest);
        let snapshot = self.decorate_snapshot(native);
        // Only the active segment's latest user message can establish the
        // current selection. A recorded None is an explicit native default.
        let mut transcript = velune_agent_runtime::transcript::TranscriptProjection::default();
        super::composer_choices::recall_turn_requests(
            &self.turn_requests,
            &mut transcript,
            &snapshot,
        );
        let selection = transcript
            .requested()
            .get(&latest)
            .filter(|request| request.runtime_id.as_deref() == Some(runtime_id))
            .map(|request| request.model_selection.clone());
        Ok(selection)
    }

    fn update_send_control(&mut self, result: &Result<SnapshotResult, RuntimeError>) {
        if matches!(result, Err(RuntimeError::Contract(failure)) if failure.code == "control_held_elsewhere")
        {
            self.control_held_elsewhere();
        } else if result.is_err()
            && let Ok(Some(target)) = self.control_target()
        {
            self.control_observations.invalidate(&target);
        }
    }

    pub(super) fn send_and_restore(
        &mut self,
        request: &TurnRequest,
    ) -> Result<SnapshotResult, RuntimeError> {
        self.send_to_target(request, SendTarget::Selected, None)
    }

    pub(crate) fn send_prepared_handoff(
        &mut self,
        request: TurnRequest,
        context: &velune_agent_runtime::import::Context,
        work_id: &str,
        queued_message_id: Option<&str>,
    ) -> Result<SnapshotResult, RuntimeError> {
        let queued = self.take_prepared_queue_turn(work_id, &request, queued_message_id)?;
        let previous = self.prepared_handoff_work_id.replace(work_id.to_owned());
        let result = self.send_to_target(&request, SendTarget::Selected, Some(context));
        self.prepared_handoff_work_id = previous;
        self.finish_prepared_queue_turn(
            queued,
            result.is_ok()
                || result
                    .as_ref()
                    .is_err_and(|error| error.delivery_accepted()),
        );
        let accepted = result.is_ok()
            || result
                .as_ref()
                .is_err_and(|error| error.delivery_accepted());
        let withdrawn = self
            .submission_cancellation
            .as_ref()
            .is_some_and(|(id, flag)| {
                id == &request.conversation_id && flag.load(std::sync::atomic::Ordering::Acquire)
            });
        if accepted && withdrawn {
            self.session.send_queue.pause_after_stop();
            if self.turn_running() {
                let _ = self.cancel(&request.runtime_id);
            }
        }
        self.update_send_control(&result);
        match result {
            Err(RuntimeError::OutstandingWork) => Err(RuntimeError::invalid(
                "来源仍有子 Agent 工作，未创建目标会话；请等待后重新准备摘要",
            )),
            result => result,
        }
    }

    fn send_to_target(
        &mut self,
        request: &TurnRequest,
        target: SendTarget,
        prepared_context: Option<&velune_agent_runtime::import::Context>,
    ) -> Result<SnapshotResult, RuntimeError> {
        let result = self.send_requested_turn(request, target, prepared_context);
        if result.is_ok() && !self.execution_submission_started() {
            self.restore_archived(&request.conversation_id);
        }
        result
    }

    fn addresses_source(&self, runtime_id: &str, conversation_id: &str) -> bool {
        if let Some(draft) = &self.session.unsubmitted_draft
            && draft.public_id == conversation_id
            && draft.original_runtime_id == runtime_id
        {
            return true;
        }
        self.session
            .logical_projection
            .as_ref()
            .map(|projection| &projection.origin)
            .or_else(|| {
                self.session
                    .projected()
                    .map(|snapshot| &snapshot.conversation)
            })
            .is_some_and(|source| source.id == conversation_id && source.runtime_id == runtime_id)
    }

    fn send_requested_turn(
        &mut self,
        request: &TurnRequest,
        target: SendTarget,
        prepared_context: Option<&velune_agent_runtime::import::Context>,
    ) -> Result<SnapshotResult, RuntimeError> {
        self.check_submission_cancelled(&request.conversation_id)?;
        if self.turn_running() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let source_runtime_id = request.source_runtime_id.as_str();
        let runtime_id = request.runtime_id.as_str();
        let conversation_id = request.conversation_id.as_str();
        let selection = request.selection.clone();
        let attachments = self.turn_attachments(
            self.runtime_instance(runtime_id)?,
            &request.turn.attachments,
        )?;
        let text = super::attachments::compose_text(&request.turn.text, &attachments.files);
        if text.trim().is_empty() && attachments.images.is_empty() {
            return Err(RuntimeError::invalid("message text"));
        }
        let text = text.as_str();
        let images = &attachments.images;
        let effort = request.turn.effort.as_deref();
        let permission_mode = request.turn.permission_mode.as_deref();
        if !self.addresses_source(source_runtime_id, conversation_id) {
            if matches!(target, SendTarget::Current) {
                return Err(RuntimeError::invalid("会话来源与共享身份不匹配"));
            }
            let (source, projection) =
                self.read_conversation_source(source_runtime_id, conversation_id)?;
            self.invalidate_execution()?;
            self.session.logical_projection = projection;
            self.session.unsubmitted_draft = None;
            self.session.model_record_key = source.model_record_key.clone();
            self.session.set_history(source);
        }
        {
            let context = self
                .submission_context()
                .ok_or_else(|| RuntimeError::invalid("conversation is not active"))?;
            self.check_handoff_input(&context)?;
            Self::check_input_access(context.actions.input_access)?;
            Self::submission_admission(&self.session, &context)
                .send
                .map_err(RuntimeError::invalid)?;
        }
        self.validate_model_selection(self.runtime_instance(runtime_id)?, selection.as_ref())?;
        if let Some(effort) = effort {
            self.validate_effort(
                self.runtime_instance(runtime_id)?,
                selection.as_ref(),
                effort,
            )?;
        }
        if let Some(mode) = permission_mode {
            self.validate_permission_mode(self.runtime_instance(runtime_id)?, mode)?;
        }
        if prepared_context.is_none()
            && request.turn.attachments.is_empty()
            && self.can_submit_native(
                execution_workers::ExecutionCommandKind::Send,
                runtime_id,
                &selection,
            )
        {
            let (effort, permission_mode) = self.turn_options(effort, permission_mode)?;
            let input = pi::TurnInput {
                text: text.into(),
                effort,
                permission_mode,
                images: Vec::new(),
            };
            self.enqueue_native_submission(execution_workers::ExecutionCommandKind::Send, input)?;
            return self.snapshot_result();
        }
        let source = self
            .session
            .projected()
            .expect("validated conversation")
            .conversation
            .clone();
        let same_runtime = source.runtime_id == runtime_id;
        let unsubmitted = self
            .session
            .unsubmitted_draft
            .as_ref()
            .is_some_and(|draft| {
                draft.native_id == source.id && draft.runtime_id == source.runtime_id
            });
        if unsubmitted {
            let supports_unpersisted_draft = velune_agent_runtime::version::variant(
                self.runtime_instance(&source.runtime_id)?.type_id,
            )
            .persists_on_first_turn;
            let needs_new_target = supports_unpersisted_draft
                && (!same_runtime
                    || matches!(self.session.active_state, ActiveState::History(_))
                    || self.session.model_selection.as_ref() != Some(&selection));
            if needs_new_target {
                self.replace_unsubmitted_target(runtime_id, selection.as_ref())?;
            }
            self.commit_unsubmitted_alias()?;
            if needs_new_target {
                return self.send_action(Prompt {
                    runtime_id,
                    message: Some((text, images)),
                    effort,
                    permission_mode,
                });
            }
        }
        let changes_provider = !unsubmitted
            && same_runtime
            && self.runtime_instance(runtime_id)?.type_id == RuntimeKind::Codex166
            && {
                let native_id = source
                    .id
                    .strip_prefix(&format!("{runtime_id}:"))
                    .ok_or_else(|| RuntimeError::invalid("Codex 会话身份无效"))?;
                !velune_agent_runtime::native::resume_provider_matches(
                    &self.history_config(self.runtime_instance(runtime_id)?)?,
                    native_id,
                    matches!(selection, Some(crate::ModelSelection::Managed { .. })),
                    source.cwd.as_deref(),
                )
                .map_err(|error| RuntimeError::context("检查 Codex 提供商切换", error))?
            };
        if !same_runtime || changes_provider {
            if matches!(target, SendTarget::Current) {
                return Err(RuntimeError::invalid(
                    "会话当前执行来源已变更，请在宿主确认",
                ));
            }
            let prompt = Prompt {
                runtime_id,
                message: Some((text, images)),
                effort,
                permission_mode,
            };
            return match prepared_context {
                Some(context) => {
                    self.continue_in_runtime_with_context(selection.as_ref(), prompt, Some(context))
                }
                None => self.continue_in_runtime(selection.as_ref(), prompt),
            };
        }
        if matches!(self.session.active_state, ActiveState::History(_))
            || self.session.model_selection.as_ref() != Some(&selection)
        {
            let snapshot = self.native_snapshot().expect("validated conversation");
            self.prepare_with_selection(snapshot, selection.as_ref(), false)?;
        }
        self.send_action(Prompt {
            runtime_id,
            message: Some((text, images)),
            effort,
            permission_mode,
        })
    }

    pub(crate) fn create_conversation(
        &mut self,
        runtime_id: &str,
        cwd: Option<String>,
        selection: Option<ModelSelection>,
    ) -> Result<SnapshotResult, RuntimeError> {
        self.create_conversation_target(runtime_id, cwd, None, selection)
    }
    pub(crate) fn create_conversation_target(
        &mut self,
        runtime_id: &str,
        cwd: Option<String>,
        project_key: Option<String>,
        selection: Option<ModelSelection>,
    ) -> Result<SnapshotResult, RuntimeError> {
        if self.turn_running() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let runtime = self.runtime_instance(runtime_id)?.clone();
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用，请先启用"));
        }
        if self.dispatches_native_creation()
            && runtime.type_id == RuntimeKind::Codex166
            && project_key.is_none()
            && !matches!(&selection, Some(ModelSelection::Managed { .. }))
        {
            return self.initialize_codex_conversation(runtime, cwd, selection);
        }
        let project = project_key
            .map(|key| self.creation_project(&runtime, &key))
            .transpose()?;
        let cwd = match &project {
            Some(project) => {
                if cwd.is_some() {
                    return Err(RuntimeError::invalid("项目与单个工作目录不能同时选择"));
                }
                Some(project.roots[0].path.clone())
            }
            None => cwd,
        };
        let cwd = if cwd.is_none()
            && matches!(
                runtime.type_id,
                RuntimeKind::Pi | RuntimeKind::Acp | RuntimeKind::DeepSeek
            ) {
            // These runtimes require a real cwd (Pi uses process.cwd()). Keep projectless
            // work away from the app's launch directory on the target host.
            let directory = self
                .user_home(&runtime)
                .ok_or_else(|| RuntimeError::invalid("无法确定执行机器的用户目录"))?
                .join("Documents/Velune");
            self.prepare_runtime_location(&runtime)?
                .0
                .create_dir_all(&directory)
                .map_err(|error| RuntimeError::context("创建无工作区目录失败", error))?;
            Some(directory)
        } else {
            cwd.map(PathBuf::from)
        };
        if let Some(cwd) = &cwd {
            self.validate_runtime_cwd(&runtime, cwd)?;
        }
        self.validate_model_selection(&runtime, selection.as_ref())?;
        let snapshot = PiProjection::new(ConversationSummary {
            native_read_state: None,
            can_rename: false,
            can_delete: false,
            id: "new".into(),
            title: velune_conversation::ConversationTitle::Untitled,
            updated_at_unix_ms: None,
            created_at_unix_ms: None,
            runtime_id: runtime_id.into(),
            cwd: cwd.map(|path| path.to_string_lossy().into_owned()),
            parent_conversation_id: None,
        })
        .snapshot
        .expect("new projection");
        self.prepare_with_project(snapshot, selection.as_ref(), true, project.as_deref())?;
        self.session.logical_projection = None;
        let native = self
            .native_snapshot()
            .expect("created conversation")
            .conversation;
        self.session.unsubmitted_draft = Some(UnsubmittedDraft {
            public_id: native.id.clone(),
            original_runtime_id: native.runtime_id.clone(),
            runtime_id: native.runtime_id,
            native_id: native.id,
            creation_token: None,
        });
        self.snapshot_result()
    }
    pub(crate) fn open_conversation(
        &mut self,
        runtime_id: &str,
        id: &str,
    ) -> Result<SnapshotResult, RuntimeError> {
        // Reopening a conversation whose turn or sub-agents run shows it as it
        // runs; its native history is read again once they have ended.
        // Some idle conversations can only be reopened through the live handle.
        if (self.session.requires_live_connection() && self.addresses_source(runtime_id, id))
            || self.session.executing()
            || self.session.unsubmitted_draft.is_some()
            || self.session.side.is_some()
            || self
                .session
                .pending_native_fork
                .as_ref()
                .is_some_and(|pending| {
                    pending.conversation_id() == id
                        && self.session.projected().is_some_and(|snapshot| {
                            snapshot.conversation.runtime_id == runtime_id
                                && snapshot.conversation.id == id
                        })
                })
        {
            // An explicit refresh can resample input ownership without
            // replacing a live or running session.
            let control_target = self.control_target().ok().flatten();
            if let Some(target) = control_target {
                self.control_observations.invalidate(&target);
            }
            return self.snapshot_result();
        }
        let (snapshot, projection) = self.read_conversation_source(runtime_id, id)?;
        self.invalidate_execution()?;
        self.present_history(snapshot, projection);
        self.snapshot_result()
    }

    /// Show a conversation as read from its native history in the addressed
    /// session, which holds no execution.
    pub(super) fn present_history(
        &mut self,
        snapshot: impl Into<browsing::HistorySnapshot>,
        projection: Option<LogicalProjection>,
    ) {
        let snapshot = snapshot.into();
        self.session.logical_projection = projection;
        self.session.unsubmitted_draft = None;
        self.session.pending_native_fork = None;
        self.session.model_record_key = snapshot.model_record_key.clone();
        self.session.model_selection = snapshot
            .model_record_key
            .clone()
            .map(|model_record_key| Some(crate::ModelSelection::Managed { model_record_key }));
        self.session.set_history(snapshot);
        // Follow the native file from this read on.
        self.session.source = None;
    }

    pub(super) fn read_conversation_source(
        &self,
        runtime_id: &str,
        id: &str,
    ) -> Result<(browsing::HistorySnapshot, Option<LogicalProjection>), RuntimeError> {
        if let Some(link) = self.conversation_links.iter().find(|link| link.id == id) {
            if link
                .segments
                .first()
                .is_none_or(|segment| segment.runtime_instance_id != runtime_id)
            {
                return Err(RuntimeError::invalid("会话来源运行时不匹配"));
            }
            let (snapshot, projection) = self.read_link(link)?;
            Ok((snapshot, Some(projection)))
        } else {
            Ok((self.read_history(runtime_id, id)?, None))
        }
    }

    pub(crate) fn read_conversation(
        &mut self,
        runtime_id: &str,
        id: &str,
    ) -> Result<SnapshotResult, RuntimeError> {
        match self.prepare_conversation_read(runtime_id, id)? {
            super::conversation_history::ConversationRead::Ready(snapshot) => Ok(*snapshot),
            super::conversation_history::ConversationRead::Load(request) => {
                self.finish_conversation_read(request.load())
            }
        }
    }

    pub(crate) fn rename_conversation(
        &mut self,
        runtime_id: &str,
        id: &str,
        title: &str,
    ) -> Result<ConfigurationSnapshot, RuntimeError> {
        self.manage_conversation(runtime_id, id, ConversationChange::Rename(title))
    }

    pub(crate) fn delete_conversation(
        &mut self,
        runtime_id: &str,
        id: &str,
    ) -> Result<ConfigurationSnapshot, RuntimeError> {
        let result = self.manage_conversation(runtime_id, id, ConversationChange::Delete)?;
        self.discard_conversation_choices(id);
        Ok(result)
    }

    fn manage_conversation(
        &mut self,
        runtime_id: &str,
        id: &str,
        change: ConversationChange,
    ) -> Result<ConfigurationSnapshot, RuntimeError> {
        let deleting = matches!(change, ConversationChange::Delete);
        if deleting && self.session.executing() {
            return Err(RuntimeError::invalid(
                "请先停止正在执行的任务与子 Agent，再修改会话",
            ));
        }
        if let Some(link) = self
            .conversation_links
            .iter()
            .find(|link| link.id == id)
            .cloned()
        {
            if link
                .segments
                .first()
                .is_none_or(|s| s.runtime_instance_id != runtime_id)
            {
                return Err(RuntimeError::invalid("会话来源运行时不匹配"));
            }
            let result = self.manage_link(link, change);
            if deleting && result.is_ok() {
                self.forget_conversation(id);
            }
            return result;
        }
        let result = self.manage_native_conversation(runtime_id, id, change);
        if deleting && result.is_ok() {
            self.forget_conversation(id);
        }
        result
    }

    pub(super) fn manage_native_conversation(
        &mut self,
        runtime_id: &str,
        id: &str,
        change: ConversationChange,
    ) -> Result<ConfigurationSnapshot, RuntimeError> {
        let deleting = matches!(change, ConversationChange::Delete);
        if deleting && self.session.executing() {
            return Err(RuntimeError::invalid(
                "请先停止正在执行的任务与子 Agent，再修改会话",
            ));
        }
        let runtime = self.runtime_instance(runtime_id)?.clone();
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用，请先启用"));
        }
        if runtime.type_id == RuntimeKind::Acp {
            return if deleting {
                self.delete_acp_conversation(&runtime, id)
            } else {
                Err(RuntimeError::Unsupported(
                    "通用 ACP 没有会话重命名接口".into(),
                ))
            };
        }
        let variant = velune_agent_runtime::version::variant(runtime.type_id);
        if (deleting && !variant.can_delete_conversations)
            || (!deleting && !variant.can_rename_conversations)
        {
            return Err(RuntimeError::invalid(
                "此运行时适配器尚未接入原生会话重命名与删除接口",
            ));
        }
        let title = match change {
            ConversationChange::Delete => None,
            ConversationChange::Rename(title) => Some(
                Some(title.trim())
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| RuntimeError::invalid("会话名称不能为空"))?,
            ),
        };
        let native_id = id
            .strip_prefix(&format!("{runtime_id}:"))
            .filter(|id| !id.is_empty())
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        if runtime.type_id == RuntimeKind::ClaudeCode && native_id.contains('/') {
            return Err(RuntimeError::invalid(
                "Claude Code 子 Agent 会话没有自己的名称",
            ));
        }
        if let Some(title) = title {
            let mut renamed_live = false;
            // A linked conversation may name a source owned by another open
            // session. Always use its live owner instead of a second file writer.
            self.for_each_session(|core| {
                if core.session.execution_runtime_id.as_deref() == Some(runtime_id)
                    && core
                        .native_snapshot()
                        .is_some_and(|view| view.conversation.id == id)
                    && let Some(live) = core.session.active_state.live_mut()
                {
                    live.rename(title)
                        .map_err(|error| RuntimeError::context("原生会话重命名失败", error))?;
                    renamed_live = true;
                    core.sync_projection()?;
                }
                Ok(())
            })?;
            if renamed_live {
                return self.list();
            }
        }
        // Re-resolve within the configured instance before invoking a native mutation.
        let listed_summary = self.conversation_summary(&runtime, id)?;
        let listed = listed_summary.is_some();
        if deleting
            && !listed
            && runtime.type_id == RuntimeKind::Pi
            && matches!(self.session.active_state, ActiveState::Pi(_))
            && self.session.execution_runtime_id.as_deref() == Some(runtime_id)
            && self
                .native_snapshot()
                .is_some_and(|snapshot| snapshot.conversation.id == id)
        {
            // A newly created Pi SessionManager reserves its native file path but
            // persists only after the first user/assistant record. Verify fresh
            // native metadata before granting this exact active draft exception.
            self.session.pi()?.invalidate_history();
            self.sync_projection()?;
            if self
                .native_snapshot()
                .is_some_and(|snapshot| snapshot.conversation.id == id)
                && !self
                    .execution_context(&runtime)?
                    .target
                    .path_exists(Path::new(native_id))
                    .map_err(|error| RuntimeError::context("原生草稿路径检查", error))?
            {
                self.invalidate_execution()?;
                self.session.active_state = ActiveState::Empty;
                self.session.model_record_key = None;
                self.session.unsubmitted_draft = None;
                return self.list();
            }
        }
        if !listed {
            return Err(RuntimeError::invalid("此会话不属于所选运行时历史目录"));
        }
        let current = self
            .native_snapshot()
            .is_some_and(|snapshot| snapshot.conversation.id == id);
        if current && deleting {
            self.invalidate_execution()?;
        }
        if runtime.type_id == RuntimeKind::Pi {
            history::manage_pi(
                &self.pi_history_config(&runtime)?,
                Path::new(native_id),
                title,
            )
            .map_err(|error| {
                RuntimeError::context("Pi 原生会话修改失败；请检查来源目录与文件权限后重试", error)
            })?;
        } else if runtime.type_id == RuntimeKind::ClaudeCode {
            let summary = listed_summary.expect("listed conversation");
            let cwd = summary
                .cwd
                .as_deref()
                .ok_or_else(|| RuntimeError::invalid("Claude Code 会话没有工作目录，无法重命名"))?;
            let context = self.execution_context(&runtime)?;
            velune_agent_runtime::native::rename_claude_session_at(
                &setting_path(&runtime, "binary")?,
                &context.agent_dir,
                native_id,
                Path::new(&cwd),
                title.expect("Claude Code deletes no sessions"),
                &context.target,
                context.os_home.as_deref(),
            )
            .map_err(|error| {
                RuntimeError::context(
                    "Claude Code 会话重命名失败；请检查运行时版本与目录后重试",
                    error,
                )
            })?;
        } else {
            let context = self.execution_context(&runtime)?;
            velune_agent_runtime::native::manage_thread_at(
                runtime.type_id,
                &setting_path(&runtime, "binary")?,
                &context.agent_dir,
                native_id,
                title.map_or(
                    velune_agent_runtime::native::ThreadChange::Delete,
                    velune_agent_runtime::native::ThreadChange::Rename,
                ),
                &context.target,
                context.os_home.as_deref(),
            )
            .map_err(|error| {
                RuntimeError::context(
                    "Codex 原生会话修改失败；请检查运行时版本与目录后重试",
                    error,
                )
            })?;
        }
        if deleting && variant.can_archive_conversations {
            // Deletion must be confirmed against both native collections. The
            // active index refresh below cannot invalidate a cached archive.
            let source = self.runtime_source_key(&runtime)?;
            let config = self.history_config(&runtime)?;
            let summaries = if config.provider == "codex" {
                self.read_codex_projects(&runtime, true)?
            } else {
                browsing::shared_summaries(history::list_archived(&config, &runtime.id)?)
            };
            self.archived_listing.borrow_mut().insert(
                runtime.id.clone(),
                browsing::HistoryArchive { source, summaries },
            );
            self.reached.borrow_mut().remove(id);
        }
        let summary = self.conversation_summary(&runtime, id)?;
        if deleting {
            if summary.is_some() {
                return Err(RuntimeError::invalid(
                    "运行时尚未确认会话删除，请刷新后重试",
                ));
            }
            if current {
                self.session.active_state = ActiveState::Empty;
                self.session.model_record_key = None;
            }
        } else {
            let summary = summary
                .filter(|summary| summary.title.display_text() == title.expect("rename title"))
                .ok_or_else(|| RuntimeError::invalid("运行时尚未确认会话重命名，请刷新后重试"))?;
            if current {
                let (summary, guard) = summary.into_retained()?.into_parts();
                self.session.invalidate_history_cache_weight();
                if let ActiveState::History(snapshot) = &mut self.session.active_state {
                    snapshot.snapshot.conversation = summary;
                    snapshot.retention = std::mem::take(&mut snapshot.retention).merge(guard);
                    snapshot.revision = snapshot.revision.saturating_add(1);
                }
            }
        }
        self.session.unsubmitted_draft = None;
        self.list()
    }

    /// A turn that is not sent withdraws what it asked for from the transcript.
    pub(super) fn send_action(&mut self, prompt: Prompt) -> Result<SnapshotResult, RuntimeError> {
        let previous = self
            .session
            .turn_observation
            .as_ref()
            .map(|op| op.operation_id.clone());
        let result = self.send_prompt(prompt);
        if result
            .as_ref()
            .is_err_and(|error| !error.delivery_accepted())
        {
            self.session.transcript_projection.withdraw_request();
            if self
                .session
                .turn_observation
                .as_ref()
                .map(|op| &op.operation_id)
                != previous.as_ref()
            {
                self.complete_turn_observation(velune_telemetry::Outcome::Failure);
            }
        } else if !self.turn_running() {
            let failed = self
                .session
                .projected()
                .is_some_and(|view| view.run_state == RunState::Failed);
            self.complete_turn_observation(if failed {
                velune_telemetry::Outcome::Failure
            } else {
                velune_telemetry::Outcome::Success
            });
        }
        result
    }

    fn send_prompt(&mut self, prompt: Prompt) -> Result<SnapshotResult, RuntimeError> {
        if self.session.pi_edit.is_some() || self.session.pi_cleanup_required {
            return Err(RuntimeError::Busy);
        }
        // Queued and collaboration turns can arrive without a command's Login::Blocks guard.
        if self.login_running() {
            return Err(RuntimeError::invalid("请先完成登录，再发送消息"));
        }
        self.ensure_active(prompt.runtime_id)?;
        if self.turn_running() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        if let ActiveState::Native(session) = &self.session.active_state
            && session.restart_needed()
        {
            self.ensure_work_ended()?;
        }
        let (text, images) = prompt.message.unwrap_or_default();
        let images = super::attachments::turn_images(images)?;
        let resuming = prompt.message.is_none();
        if !resuming && text.trim().is_empty() && images.is_empty() {
            return Err(RuntimeError::invalid("message text"));
        }
        if matches!(self.session.active_state, ActiveState::History(_)) {
            let snapshot = self.native_snapshot().expect("history snapshot");
            let selection = self
                .session
                .model_selection
                .clone()
                .ok_or_else(|| RuntimeError::invalid("请选择此会话的模型"))?;
            self.prepare_with_selection(snapshot, selection.as_ref(), false)?;
        }
        self.retire_native_settings_for_submission();
        self.begin_turn_observation();
        if let Some((gateway_id, session)) = self.backend_session_identity() {
            self.session_backends()
                .begin_user_turn(&gateway_id, &session)
                .map_err(|error| RuntimeError::context("CLI 新轮准备", error))?;
        }
        // Preparation may have changed the native session. Capture the pre-send
        // transcript once for inherited options and the new user-message boundary.
        self.refresh_transcript();
        let (effort, permission_mode) = self.turn_options(prompt.effort, prompt.permission_mode)?;
        let input = pi::TurnInput {
            text: text.into(),
            effort,
            permission_mode,
            images,
        };
        // Requests bind to the next user message; a native resume has none.
        if !resuming {
            self.request_turn_settings(prompt.runtime_id, &input);
        }
        if matches!(self.session.active_state, ActiveState::Pi(_))
            && matches!(
                self.session.model_selection,
                Some(Some(crate::ModelSelection::Managed { .. }))
            )
        {
            self.bind_gateway_model()?;
        }
        self.drain_runtime()?;
        self.check_submission_cancelled(self.session.conversation_id().unwrap_or_default())?;
        self.session.unsubmitted_draft = None;
        let mut submitted = None;
        let sent =
            telemetry::observe(
                self.telemetry.as_ref(),
                "harness.dispatch",
                || match &mut self.session.active_state {
                    ActiveState::Native(session) if resuming => session
                        .resume(input.effort.clone(), input.permission_mode.clone())
                        .map_err(Into::into),
                    state => match state.live_mut() {
                        Some(live) => {
                            let before = live.snapshot().and_then(|snapshot| {
                                user_message_ids(snapshot).next().map(str::to_owned)
                            });
                            let result = live.send(&input);
                            if result.is_ok() && !resuming {
                                submitted = live.snapshot().and_then(|snapshot| {
                                    let mut ids = user_message_ids(snapshot);
                                    let latest = ids.next()?;
                                    (Some(latest) != before.as_deref()
                                        && ids.next() == before.as_deref())
                                    .then(|| latest.to_owned())
                                });
                            }
                            result
                        }
                        None => Err("会话执行尚未准备".into()),
                    },
                },
            )
            .map_err(|error| RuntimeError::context("message send", error));
        let pending_fork = self.session.refresh_pending_native_fork();
        if sent.is_err() && self.session.gateway_runner.is_some() && !self.session.executing() {
            self.finish_managed_turn()?;
        }
        sent?;
        pending_fork?;
        self.session.cancel_requested = false;
        self.follow_started_turn();
        // Pi handles some messages at once, without a turn.
        if self.turn_running() {
            self.started_turn_observation();
        }
        self.sync_projection()
            .map_err(RuntimeError::after_delivery)?;
        if self.session.gateway_runner.is_some() && !self.session.executing() {
            self.finish_managed_turn()
                .map_err(RuntimeError::after_delivery)?;
        }
        let mut result = self
            .snapshot_result()
            .map_err(RuntimeError::after_delivery)?;
        if let (Some(id), Some(native), Some(snapshot)) = (
            submitted,
            self.session.projected(),
            result.snapshot.as_ref(),
        ) {
            let id = self.projected_conversation_message_id(&native.conversation.id, &id);
            let id = snapshot
                .message_identity_confirmations
                .iter()
                .find(|confirmation| confirmation.previous_id == id)
                .map_or(id.as_str(), |confirmation| &confirmation.current_id);
            result.submitted_message_id = snapshot
                .messages
                .iter()
                .find(|message| message.id == id)
                .map(|message| message.id.clone());
        }
        Ok(result)
    }

    pub(super) fn bind_gateway_model(&mut self) -> Result<(), RuntimeError> {
        self.session
            .pi()?
            .set_model(Some("velune"), "auto")
            .map_err(|error| RuntimeError::context("gateway model binding", error))
    }

    pub(super) fn backend_session_identity(
        &self,
    ) -> Option<(String, velune_gateway::SessionIdentity)> {
        let configuration = self.session.execution_configuration.as_ref()?;
        Some((
            configuration.runtime.gateway_id.clone(),
            configuration.gateway_session.clone()?,
        ))
    }

    pub(crate) fn cancel(&mut self, runtime_id: &str) -> Result<SnapshotResult, RuntimeError> {
        if self.cancel_pi_edit(runtime_id)? {
            return self.snapshot_result();
        }
        if self.session.pi_cleanup_required {
            return Err(RuntimeError::operation_indeterminate(
                "Pi 正在等待真实进程清理；停止结果未确认",
            ));
        }
        self.ensure_active(runtime_id)?;
        if let ActiveState::Pi(pi) = &self.session.active_state
            && pi.requires_failure_cleanup()
        {
            self.session.pi_cleanup_required = true;
            let _ = self.start_native_cleanup_worker();
            return Err(RuntimeError::operation_indeterminate(
                "Pi 连接已失败；停止未派发，真实进程清理尚未完成",
            ));
        }
        if self.cancel_queued_preparation() {
            return self.snapshot_result();
        }
        if let Some(view) = self.session.projected()
            && let Some(parent_id) = view.conversation.parent_conversation_id.clone()
        {
            let child_id = view.conversation.id.clone();
            let previous = self.session.key;
            if !self.focus_existing(&parent_id) {
                return Err(RuntimeError::invalid(
                    "此子 Agent 没有本机可用的独立停止接口",
                ));
            }
            let native =
                self.start_native_control(velune_agent_runtime::native::CodexCommand::StopChild {
                    conversation_id: child_id.clone(),
                });
            if !matches!(native, Ok(false)) {
                self.focus_key(previous);
                native?;
                return self.snapshot_result();
            }
            let result = self.start_live_operation("stop_child", move |state| {
                state
                    .live_mut()
                    .filter(|parent| parent.can_stop_child(&child_id))
                    .ok_or_else(|| RuntimeError::invalid("此子 Agent 没有本机可用的独立停止接口"))?
                    .stop_child(&child_id)
                    .map_err(|error| RuntimeError::runtime_operation("子 Agent 停止", error))
            });
            self.focus_key(previous);
            result?;
            return self.snapshot_result();
        }
        if self.try_start_pi_control(None)? {
            return self.snapshot_result();
        }
        if self.start_native_control(velune_agent_runtime::native::CodexCommand::Cancel)? {
            return self.snapshot_result();
        }
        self.start_live_operation_with_outcome("cancel", |state| {
            let Some(live) = state.live_mut() else {
                return Err(RuntimeError::invalid("会话执行尚未准备")).into();
            };
            if let Err(error) = live.cancel() {
                return Err(RuntimeError::runtime_operation("message cancel", error)).into();
            }
            let result = live.stop_background().map_err(|error| {
                RuntimeError::operation_indeterminate(format!(
                    "当前 turn 的停止请求已受理，但后台工作的停止结果未确认：{error}"
                ))
            });
            execution_workers::OperationOutcome::after_cancellation(result)
        })?;
        if !matches!(self.session.active_state, ActiveState::Pending(_)) {
            self.sync_projection()?;
        }
        self.snapshot_result()
    }

    /// Stop the native sub-agents the conversation started, which may run
    /// after its turn has ended.
    pub(crate) fn stop_sub_agents(
        &mut self,
        runtime_id: &str,
    ) -> Result<SnapshotResult, RuntimeError> {
        self.ensure_active(runtime_id)?;
        if self.start_native_control(velune_agent_runtime::native::CodexCommand::StopBackground)? {
            return self.snapshot_result();
        }
        if self.session.active_state.live().is_some() {
            self.start_live_operation("stop_background", |state| {
                state
                    .live_mut()
                    .ok_or_else(|| RuntimeError::invalid("会话执行尚未准备"))?
                    .stop_background()
                    .map_err(|error| RuntimeError::runtime_operation("后台工作停止", error))
            })?;
            if !matches!(self.session.active_state, ActiveState::Pending(_)) {
                self.drain_runtime()?;
            }
        }
        self.snapshot_result()
    }

    pub(super) fn ensure_active(&self, requested: &str) -> Result<(), RuntimeError> {
        let active_runtime = self
            .native_snapshot()
            .map(|snapshot| snapshot.conversation.runtime_id)
            .or_else(|| self.session.execution_runtime_id.clone());
        if active_runtime.as_deref() != Some(requested) {
            return Err(RuntimeError::invalid("runtime instance is not active"));
        }
        Ok(())
    }

    pub(super) fn sync_projection(&mut self) -> Result<(), RuntimeError> {
        self.drain_runtime()?;
        let runtime_id = self.session.execution_runtime_id.clone();
        let model_record_key = self.session.model_record_key.clone();
        if let ActiveState::Pi(session) = &mut self.session.active_state
            && session
                .sync_history(
                    runtime_id
                        .as_deref()
                        .expect("a Pi session has an active runtime instance"),
                    model_record_key,
                )
                .map_err(|error| RuntimeError::context("runtime state", error))?
        {
            self.drain_runtime()?;
        }
        Ok(())
    }
}

fn user_message_ids(
    snapshot: &crate::conversation::ConversationSnapshot,
) -> impl Iterator<Item = &str> {
    snapshot
        .messages
        .iter()
        .rev()
        .filter(|message| message.role == crate::conversation::MessageRole::User)
        .map(|message| message.id.as_str())
}

impl CoreRuntime {
    /// Tells the transcript what this turn asks for, against the projection
    /// as it stands before the runtime shows the turn's user message.
    pub(super) fn request_turn_settings(&mut self, runtime_id: &str, input: &pi::TurnInput) {
        let selection = self.session.model_selection.clone().flatten();
        let model = selection
            .as_ref()
            .and_then(|selection| self.turn_model_name(runtime_id, selection));
        let settings = crate::conversation::TurnSettings {
            runtime_id: Some(runtime_id.to_owned()),
            model,
            effort: input.effort.clone(),
            permission_mode: input.permission_mode.clone(),
            model_selection: selection,
        };
        self.session.transcript_projection.request(settings);
    }
}
