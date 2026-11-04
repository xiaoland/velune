//! Local application conversations use cases.
use super::*;
use crate::conversation::ConversationSnapshot;
use crate::local::continuation::{LogicalProjection, display_segment};
use crate::{ComposedTurn, ConfigurationSnapshot, ModelSelection, SnapshotResult};

/// A turn the user sends in a conversation: where it comes from, the
/// runtime and model it runs on, and what was composed.
#[derive(Clone)]
pub(crate) struct TurnRequest {
    pub source_runtime_id: String,
    pub conversation_id: String,
    pub runtime_id: String,
    pub selection: Option<ModelSelection>,
    pub turn: ComposedTurn,
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
    /// A turn that needs another runtime process while sub-agents still run
    /// in the current one is queued; the queue sends it once they have ended.
    pub(crate) fn send_turn(
        &mut self,
        request: TurnRequest,
    ) -> Result<SnapshotResult, RuntimeError> {
        let result = self.send_and_restore(&request);
        if matches!(&result, Err(RuntimeError::Contract(failure)) if failure.code == "control_held_elsewhere")
        {
            self.control_held_elsewhere();
        } else if result.is_err() {
            self.session.control = None;
        }
        match result {
            Err(RuntimeError::OutstandingWork) => {
                self.queue_after_work(request);
                self.snapshot_result()
            }
            result => result,
        }
    }

    pub(super) fn send_and_restore(
        &mut self,
        request: &TurnRequest,
    ) -> Result<SnapshotResult, RuntimeError> {
        let result = self.send_requested_turn(request);
        if result.is_ok() {
            self.restore_archived(&request.conversation_id);
        }
        result
    }

    fn send_requested_turn(
        &mut self,
        request: &TurnRequest,
    ) -> Result<SnapshotResult, RuntimeError> {
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
        let active_snapshot = self.native_snapshot();
        let active_conversation_id = self
            .session
            .logical_projection
            .as_ref()
            .map(|projection| projection.origin.id.as_str())
            .or_else(|| {
                active_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.conversation.id.as_str())
            });
        let active_source_runtime = self
            .session
            .logical_projection
            .as_ref()
            .map(|projection| projection.origin.runtime_id.as_str())
            .or_else(|| {
                active_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.conversation.runtime_id.as_str())
            });
        let known_draft_source = self
            .session
            .unsubmitted_draft
            .as_ref()
            .is_some_and(|draft| {
                draft.public_id == conversation_id && draft.original_runtime_id == source_runtime_id
            });
        if !known_draft_source
            && (active_conversation_id != Some(conversation_id)
                || active_source_runtime != Some(source_runtime_id))
        {
            let (source, projection) =
                self.read_conversation_source(source_runtime_id, conversation_id)?;
            self.invalidate_execution()?;
            self.session.logical_projection = projection;
            self.session.unsubmitted_draft = None;
            self.session.model_record_key = source.model_record_key.clone();
            self.session.active_state = ActiveState::History(Box::new(source));
        }
        self.require_input_access()?;
        let snapshot = self
            .native_snapshot()
            .ok_or_else(|| RuntimeError::invalid("conversation is not active"))?;
        if matches!(
            snapshot.run_state,
            RunState::Running | RunState::Stopping | RunState::Elsewhere
        ) {
            return Err(RuntimeError::invalid("请等待当前会话的执行结束"));
        }
        if snapshot.conversation.parent_conversation_id.is_some() {
            return Err(RuntimeError::invalid(
                "原生 sub-agent 会话由启动它的会话继续，不能直接发送",
            ));
        }
        if matches!(self.session.active_state, ActiveState::History(_))
            && !snapshot.actions.can_send
        {
            return Err(RuntimeError::invalid(
                "当前原生上下文不可用，请修复来源后重新打开；未自动重发",
            ));
        }
        if !snapshot.pending_interactions.is_empty() {
            return Err(RuntimeError::invalid("请先处理当前运行时请求"));
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
        let unsubmitted = self
            .session
            .unsubmitted_draft
            .as_ref()
            .is_some_and(|draft| {
                draft.native_id == snapshot.conversation.id
                    && draft.runtime_id == snapshot.conversation.runtime_id
            });
        if unsubmitted {
            let supports_unpersisted_draft = velune_agent_runtime::version::variant(
                self.runtime_instance(&snapshot.conversation.runtime_id)?
                    .type_id,
            )
            .persists_on_first_turn;
            let needs_new_target = supports_unpersisted_draft
                && (snapshot.conversation.runtime_id != runtime_id
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
            && snapshot.conversation.runtime_id == runtime_id
            && self.runtime_instance(runtime_id)?.type_id == RuntimeKind::Codex166
            && {
                let raw = self
                    .native_snapshot()
                    .ok_or_else(|| RuntimeError::invalid("会话上下文不可用"))?;
                let native_id = raw
                    .conversation
                    .id
                    .strip_prefix(&format!("{runtime_id}:"))
                    .ok_or_else(|| RuntimeError::invalid("Codex 会话身份无效"))?;
                !velune_agent_runtime::native::resume_provider_matches(
                    &self.history_config(self.runtime_instance(runtime_id)?)?,
                    native_id,
                    matches!(selection, Some(crate::ModelSelection::Managed { .. })),
                    raw.conversation.cwd.as_deref(),
                )
                .map_err(|error| RuntimeError::context("检查 Codex 提供商切换", error))?
            };
        if snapshot.conversation.runtime_id != runtime_id || changes_provider {
            return self.continue_in_runtime(
                selection.as_ref(),
                Prompt {
                    runtime_id,
                    message: Some((text, images)),
                    effort,
                    permission_mode,
                },
            );
        }
        if matches!(self.session.active_state, ActiveState::History(_))
            || self.session.model_selection.as_ref() != Some(&selection)
        {
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
        if self.turn_running() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let runtime = self.runtime_instance(runtime_id)?.clone();
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用，请先启用"));
        }
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
        self.prepare_with_selection(snapshot, selection.as_ref(), true)?;
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
        if self.session.executing()
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
        snapshot: ConversationSnapshot,
        projection: Option<LogicalProjection>,
    ) {
        self.session.logical_projection = projection;
        self.session.unsubmitted_draft = None;
        self.session.pending_native_fork = None;
        self.session.model_record_key = snapshot.model_record_key.clone();
        self.session.model_selection = snapshot
            .model_record_key
            .clone()
            .map(|model_record_key| Some(crate::ModelSelection::Managed { model_record_key }));
        self.session.active_state = ActiveState::History(Box::new(snapshot));
        // Follow the native file from this read on.
        self.session.source = None;
    }

    pub(super) fn read_conversation_source(
        &self,
        runtime_id: &str,
        id: &str,
    ) -> Result<(ConversationSnapshot, Option<LogicalProjection>), RuntimeError> {
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

    /// Reopen a queried view by its current native context. A continued
    /// conversation keeps its source identity, but queries address its last
    /// segment's runtime, including when no session is loaded yet.
    pub(super) fn read_conversation_context(
        &self,
        runtime_id: &str,
        id: &str,
    ) -> Result<(ConversationSnapshot, Option<LogicalProjection>), RuntimeError> {
        if let Some(link) = self.conversation_links.iter().find(|link| link.id == id) {
            if link
                .segments
                .last()
                .is_none_or(|segment| segment.runtime_instance_id != runtime_id)
            {
                return Err(RuntimeError::invalid("会话当前上下文运行时不匹配"));
            }
            let (snapshot, projection) = self.read_link(link)?;
            Ok((snapshot, Some(projection)))
        } else {
            Ok((self.read_history(runtime_id, id)?, None))
        }
    }

    /// Read one native conversation without changing the active execution or
    /// transcript projection.
    pub(crate) fn read_conversation(
        &mut self,
        runtime_id: &str,
        id: &str,
    ) -> Result<SnapshotResult, RuntimeError> {
        tracing::info!(target:"velune_application", event="conversation_read_requested", conversation_id=%id, runtime_id=%runtime_id);
        let active_snapshot = self.native_snapshot();
        let active_id = self
            .session
            .logical_projection
            .as_ref()
            .map(|projection| projection.origin.id.as_str())
            .or_else(|| {
                active_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.conversation.id.as_str())
            });
        let active_runtime = self
            .session
            .logical_projection
            .as_ref()
            .map(|projection| projection.origin.runtime_id.as_str())
            .or_else(|| {
                active_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.conversation.runtime_id.as_str())
            });
        if active_id == Some(id) && active_runtime == Some(runtime_id) {
            self.sync_projection()?;
            return self.snapshot_result();
        }
        let (mut snapshot, projection) = self.read_conversation_source(runtime_id, id)?;
        let control_target = self.read_control_target(&snapshot, projection.as_ref());
        if let Some(projection) = projection {
            let segment = projection
                .link
                .segments
                .last()
                .ok_or_else(|| RuntimeError::invalid("关联会话缺少当前段"))?;
            let index = projection.link.segments.len() - 1;
            let mut messages = projection.prefix;
            if index > 0 {
                messages.push(self.boundary_message(segment, index));
            }
            super::continuation::namespace_associations(&mut snapshot, index);
            snapshot
                .executions
                .records
                .extend(projection.executions.records);
            messages.extend(display_segment(segment, &snapshot.messages, index)?);
            snapshot.conversation = projection.origin;
            snapshot.context_runtime_id = segment.runtime_instance_id.clone();
            snapshot.messages = messages;
            self.set_management_capabilities(&mut snapshot.conversation);
        } else {
            snapshot.context_runtime_id = runtime_id.into();
            self.set_management_capabilities(&mut snapshot.conversation);
        }
        super::continuation::stamp_turn_runtime(
            &mut snapshot.messages,
            &snapshot.context_runtime_id,
        );
        // Read-only history must reproduce stable transcript turn IDs without
        // touching the shared live projection or its timing state.
        use velune_agent_runtime::transcript::{TranscriptEvent, TranscriptProjection};
        let mut transcript = TranscriptProjection::default();
        transcript.apply(TranscriptEvent::ConfirmIdentities(
            &snapshot.message_identity_confirmations,
        ));
        transcript.apply(TranscriptEvent::Replace {
            messages: &snapshot.messages,
            executions: &snapshot.executions,
        });
        super::composer_choices::recall_turn_requests(
            &self.turn_requests,
            &mut transcript,
            &snapshot,
        );
        transcript.apply(TranscriptEvent::ExecutionState(snapshot.run_state.clone()));
        snapshot.transcript_turns = transcript.turns();
        self.name_turn_models(&mut snapshot.transcript_turns);
        snapshot.transcript_items = transcript.items();
        snapshot.transcript_outline = transcript.outline();
        snapshot.transcript_message_identities = transcript.message_identities();
        snapshot.sub_agents = self.sub_agents(&snapshot);
        snapshot.collaborators = self.collaborators(&snapshot);
        self.apply_read_input_access(&mut snapshot, control_target);
        Ok(SnapshotResult {
            snapshot: Some(self.delivery.snapshot(snapshot)),
        })
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
            if current && let ActiveState::History(snapshot) = &mut self.session.active_state {
                snapshot.conversation = summary;
                snapshot.revision = snapshot.revision.saturating_add(1);
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
        if result.is_err() {
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
        self.begin_turn_observation();
        if let Some((gateway_id, session)) = self.backend_session_identity() {
            self.execution_backends
                .begin_user_turn(&gateway_id, &session)
                .map_err(|error| RuntimeError::context("CLI 新轮准备", error))?;
        }
        let input = pi::TurnInput {
            text: text.into(),
            effort: self.requested_effort(prompt.effort)?,
            permission_mode: self.requested_permission_mode(prompt.permission_mode)?,
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
        self.session.unsubmitted_draft = None;
        let sent = match &mut self.session.active_state {
            ActiveState::Native(session) if resuming => session
                .resume(input.effort.clone(), input.permission_mode.clone())
                .map_err(Into::into),
            state => match state.live_mut() {
                Some(live) => live.send(&input),
                None => Err("会话执行尚未准备".into()),
            },
        }
        .map_err(|error| RuntimeError::context("message send", error));
        if let ActiveState::Native(session) = &self.session.active_state {
            self.session.pending_native_fork = session.pending_fork();
        }
        if sent.is_err() && self.session.gateway_runner.is_some() && !self.session.executing() {
            self.finish_managed_turn()?;
        }
        sent?;
        self.session.cancel_requested = false;
        self.follow_started_turn();
        // Pi handles some messages at once, without a turn.
        if self.turn_running() {
            self.started_turn_observation();
        }
        self.sync_projection()?;
        if self.session.gateway_runner.is_some() && !self.session.executing() {
            self.finish_managed_turn()?;
        }
        self.snapshot_result()
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
        let runtime = self.runtime_instances.iter().find(|runtime| {
            Some(runtime.id.as_str()) == self.session.execution_runtime_id.as_deref()
        })?;
        Some((
            runtime.gateway_id.clone(),
            velune_gateway::SessionIdentity {
                namespace: "velune.conversation".into(),
                id: self.session.conversation_id()?.into(),
            },
        ))
    }

    pub(crate) fn cancel(&mut self, runtime_id: &str) -> Result<SnapshotResult, RuntimeError> {
        self.ensure_active(runtime_id)?;
        if let Some((gateway_id, session)) = self.backend_session_identity() {
            self.execution_backends
                .cancel_session(&gateway_id, &session);
        }
        self.session
            .active_state
            .live_mut()
            .ok_or_else(|| RuntimeError::invalid("会话执行尚未准备"))?
            .cancel()
            .map_err(|error| RuntimeError::context("message cancel", error))?;
        self.session.cancel_requested = true;
        // Stopping the turn stops the sub-agents it started too.
        if let ActiveState::Native(session) = &mut self.session.active_state {
            session
                .stop_background()
                .map_err(|error| RuntimeError::context("message cancel", error))?;
        }
        self.sync_projection()?;
        self.snapshot_result()
    }

    /// Stop the native sub-agents the conversation started, which may run
    /// after its turn has ended.
    pub(crate) fn stop_sub_agents(
        &mut self,
        runtime_id: &str,
    ) -> Result<SnapshotResult, RuntimeError> {
        self.ensure_active(runtime_id)?;
        if let ActiveState::Native(session) = &mut self.session.active_state {
            session
                .stop_background()
                .map_err(|error| RuntimeError::context("子 Agent 停止", error))?;
            self.drain_runtime()?;
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

impl CoreRuntime {
    /// Tells the transcript what this turn asks for, against the projection
    /// as it stands before the runtime shows the turn's user message.
    fn request_turn_settings(&mut self, runtime_id: &str, input: &pi::TurnInput) {
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
        let _ = self.current_snapshot();
        self.session.transcript_projection.request(settings);
    }
}
