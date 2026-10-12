//! Local application conversations use cases.
use super::*;
use crate::conversation::ConversationSnapshot;
use crate::local::continuation::{LogicalProjection, display_segment};
impl CoreRuntime {
    pub(super) fn send_turn_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if self.busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        self.session.cancel_requested = false;
        let source_runtime_id = request["payload"]["sourceRuntimeID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("source runtime instance id"))?;
        let runtime_id = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        let conversation_id = request["payload"]["conversationID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        let selection = super::model_selection::requested_selection(request)?;
        let attachments = self.turn_attachments(self.runtime_instance(runtime_id)?, request)?;
        let text = super::attachments::compose_text(
            request["payload"]["text"].as_str().unwrap_or_default(),
            &attachments.files,
        );
        if text.trim().is_empty() && attachments.images.is_empty() {
            return Err(RuntimeError::invalid("message text"));
        }
        let text = text.as_str();
        let images = &attachments.images;
        let effort = request["payload"]["effort"].as_str();
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
        let snapshot = self
            .native_snapshot()
            .ok_or_else(|| RuntimeError::invalid("conversation is not active"))?;
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
        self.validate_model_selection(self.runtime_instance(runtime_id)?, &selection)?;
        if let Some(effort) = effort {
            self.validate_effort(self.runtime_instance(runtime_id)?, &selection, effort)?;
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
            let supports_unpersisted_draft = matches!(
                self.runtime_instance(&snapshot.conversation.runtime_id)?
                    .type_id
                    .as_str(),
                "codex-0.159.3" | "claude-code-2.1.286" | "pi-1.0.2"
            );
            let needs_new_target = supports_unpersisted_draft
                && (snapshot.conversation.runtime_id != runtime_id
                    || matches!(self.session.active_state, ActiveState::History(_))
                    || self.session.model_selection.as_ref() != Some(&selection));
            if needs_new_target {
                self.replace_unsubmitted_target(runtime_id, &selection)?;
            }
            self.commit_unsubmitted_alias()?;
            if needs_new_target {
                return self.send_action(
                    &json!({"payload":{"runtimeInstanceID":runtime_id,"text":text,"effort":effort,"images":images}}),
                );
            }
        }
        if snapshot.conversation.runtime_id != runtime_id {
            return self.continue_in_runtime(runtime_id, &selection, text, effort, images);
        }
        if matches!(self.session.active_state, ActiveState::History(_))
            || self.session.model_selection.as_ref() != Some(&selection)
        {
            self.prepare_with_selection(snapshot, &selection, false)?;
        }
        self.send_action(
            &json!({"payload":{"runtimeInstanceID":runtime_id,"text":text,"effort":effort,"images":images}}),
        )
    }

    pub(super) fn create_conversation(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if self.busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let runtime_id = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        let runtime = self.runtime_instance(runtime_id)?.clone();
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用，请先启用"));
        }
        let cwd = request["payload"]["cwd"].as_str().map(PathBuf::from);
        let selection = super::model_selection::requested_selection(request)?;
        if let Some(cwd) = &cwd {
            self.validate_runtime_cwd(&runtime, cwd)?;
        }
        self.validate_model_selection(&runtime, &selection)?;
        let snapshot = PiProjection::new(ConversationSummary {
            can_rename: false,
            can_delete: false,
            id: "new".into(),
            title: velune_conversation::ConversationTitle::Untitled,
            updated_at_unix_ms: None,
            created_at_unix_ms: None,
            runtime_id: runtime_id.into(),
            cwd: cwd.map(|path| path.to_string_lossy().into_owned()),
        })
        .snapshot
        .expect("new projection");
        self.prepare_with_selection(snapshot, &selection, true)?;
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
    pub(super) fn open_conversation(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        // Reopening a conversation whose turn is running shows it as it runs;
        // its native history is read again once the turn has ended.
        if self.busy() {
            return self.snapshot_result();
        }
        let runtime_id = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        let id = request["payload"]["conversationID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
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
        self.session.model_record_key = snapshot.model_record_key.clone();
        self.session.model_selection = snapshot
            .model_record_key
            .clone()
            .map(|model_record_key| crate::ModelSelection::Managed { model_record_key });
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

    pub(super) fn read_conversation(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        let runtime_id = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        let id = request["payload"]["conversationID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
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
            for confirmation in &mut snapshot.message_identity_confirmations {
                confirmation.previous_id = format!("segment-{index}:{}", confirmation.previous_id);
                confirmation.current_id = format!("segment-{index}:{}", confirmation.current_id);
            }
            messages.extend(display_segment(segment, &snapshot.messages, index)?);
            snapshot.conversation = projection.origin;
            snapshot.context_runtime_id = segment.runtime_instance_id.clone();
            snapshot.messages = messages;
            self.set_management_capabilities(&mut snapshot.conversation);
        } else {
            snapshot.context_runtime_id = runtime_id.into();
            self.set_management_capabilities(&mut snapshot.conversation);
        }
        // Read-only history must reproduce stable transcript turn IDs without
        // touching the shared live projection or its timing state.
        use velune_agent_runtime::transcript::{TranscriptEvent, TranscriptProjection};
        let mut transcript = TranscriptProjection::default();
        transcript.apply(TranscriptEvent::ConfirmIdentities(
            &snapshot.message_identity_confirmations,
        ));
        transcript.apply(TranscriptEvent::ReplaceMessages(&snapshot.messages));
        transcript.apply(TranscriptEvent::ExecutionState(snapshot.run_state.clone()));
        snapshot.transcript_turns = transcript.turns();
        snapshot.transcript_items = transcript.items();
        snapshot.transcript_outline = transcript.outline();
        snapshot.transcript_message_identities = transcript.message_identities();
        Ok(json!({"snapshot":self.delivery.snapshot(snapshot)}))
    }

    pub(super) fn manage_conversation(
        &mut self,
        request: &Value,
        deleting: bool,
    ) -> Result<Value, RuntimeError> {
        if self.busy() {
            return Err(RuntimeError::invalid("请先停止正在执行的任务，再修改会话"));
        }
        if let Some(link) = self
            .conversation_links
            .iter()
            .find(|link| Some(link.id.as_str()) == request["payload"]["conversationID"].as_str())
            .cloned()
        {
            if link.segments.first().is_none_or(|s| {
                Some(s.runtime_instance_id.as_str())
                    != request["payload"]["runtimeInstanceID"].as_str()
            }) {
                return Err(RuntimeError::invalid("会话来源运行时不匹配"));
            }
            return self.manage_link(link, request, deleting);
        }
        self.manage_native_conversation(request, deleting)
    }

    pub(super) fn manage_native_conversation(
        &mut self,
        request: &Value,
        deleting: bool,
    ) -> Result<Value, RuntimeError> {
        if self.busy() {
            return Err(RuntimeError::invalid("请先停止正在执行的任务，再修改会话"));
        }
        let runtime_id = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        let id = request["payload"]["conversationID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        let runtime = self.runtime_instance(runtime_id)?.clone();
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用，请先启用"));
        }
        if runtime.type_id == "acp-v1" {
            return if deleting {
                self.delete_acp_conversation(&runtime, id)
            } else {
                Err(RuntimeError::Unsupported(
                    "通用 ACP 没有会话重命名接口".into(),
                ))
            };
        }
        let variant = velune_agent_runtime::version::variant(&runtime.type_id)
            .ok_or_else(|| RuntimeError::invalid("runtime type"))?;
        if (deleting && !variant.can_delete_conversations)
            || (!deleting && !variant.can_rename_conversations)
        {
            return Err(RuntimeError::invalid(
                "此运行时适配器尚未接入原生会话重命名与删除接口",
            ));
        }
        let title = if deleting {
            None
        } else {
            Some(
                request["payload"]["title"]
                    .as_str()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| RuntimeError::invalid("会话名称不能为空"))?,
            )
        };
        let native_id = id
            .strip_prefix(&format!("{runtime_id}:"))
            .filter(|id| !id.is_empty())
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        // Re-resolve within the configured instance before invoking a native mutation.
        let listed = self
            .summaries_for(&runtime)?
            .iter()
            .any(|summary| summary.id == id);
        if !listed
            && runtime.type_id == "pi-1.0.2"
            && matches!(self.session.active_state, ActiveState::Pi)
            && self.session.execution_runtime_id.as_deref() == Some(runtime_id)
            && self
                .native_snapshot()
                .is_some_and(|snapshot| snapshot.conversation.id == id)
        {
            // A newly created Pi SessionManager reserves its native file path but
            // persists only after the first user/assistant record. Verify fresh
            // native metadata before granting this exact active draft exception.
            self.session
                .pi
                .projection
                .as_mut()
                .expect("active Pi projection")
                .invalidate_history();
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
                if deleting {
                    self.invalidate_execution()?;
                    self.session.active_state = ActiveState::Empty;
                    self.session.model_record_key = None;
                } else {
                    self.session
                        .pi
                        .client
                        .as_mut()
                        .expect("active Pi client")
                        .request(
                            json!({"type":"set_session_name","name":title.expect("rename title")}),
                        )
                        .map_err(|error| {
                            RuntimeError::context("Pi 原生草稿重命名失败，请重试", error)
                        })?;
                    self.session
                        .pi
                        .projection
                        .as_mut()
                        .expect("active Pi projection")
                        .invalidate_history();
                    self.sync_projection()?;
                    if !self.native_snapshot().is_some_and(|snapshot| {
                        snapshot.conversation.id == id
                            && snapshot.conversation.title.display_text()
                                == title.expect("rename title")
                    }) {
                        return Err(RuntimeError::invalid("Pi 未确认原生草稿名称，请重试"));
                    }
                }
                if deleting {
                    self.session.unsubmitted_draft = None;
                }
                return self.list();
            }
        }
        if !listed {
            return Err(RuntimeError::invalid("此会话不属于所选运行时历史目录"));
        }
        let current = self
            .native_snapshot()
            .is_some_and(|snapshot| snapshot.conversation.id == id);
        if current {
            self.invalidate_execution()?;
        }
        if runtime.type_id == "pi-1.0.2" {
            history::manage_pi(
                &self.pi_history_config(&runtime)?,
                Path::new(native_id),
                title,
            )
            .map_err(|error| {
                RuntimeError::context("Pi 原生会话修改失败；请检查来源目录与文件权限后重试", error)
            })?;
        } else {
            let context = self.execution_context(&runtime)?;
            velune_agent_runtime::native::manage_thread_at(
                &setting_path(&runtime, "binary")?,
                &context.agent_dir,
                native_id,
                title,
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
        let summaries = self.summaries_for(&runtime)?;
        if deleting {
            if summaries.iter().any(|summary| summary.id == id) {
                return Err(RuntimeError::invalid(
                    "运行时尚未确认会话删除，请刷新后重试",
                ));
            }
            if current {
                self.session.active_state = ActiveState::Empty;
                self.session.model_record_key = None;
            }
        } else {
            let summary = summaries
                .into_iter()
                .find(|summary| {
                    summary.id == id && summary.title.display_text() == title.expect("rename title")
                })
                .ok_or_else(|| RuntimeError::invalid("运行时尚未确认会话重命名，请刷新后重试"))?;
            if current && let ActiveState::History(snapshot) = &mut self.session.active_state {
                snapshot.conversation = summary;
                snapshot.revision = snapshot.revision.saturating_add(1);
            }
        }
        self.session.unsubmitted_draft = None;
        self.list()
    }

    pub(super) fn send_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        if self.busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let text = request["payload"]["text"].as_str().unwrap_or_default();
        let images = super::attachments::turn_images(request)?;
        if text.trim().is_empty() && images.is_empty() {
            return Err(RuntimeError::invalid("message text"));
        }
        if matches!(self.session.active_state, ActiveState::History(_)) {
            let snapshot = self.native_snapshot().expect("history snapshot");
            let selection = self
                .session
                .model_selection
                .clone()
                .ok_or_else(|| RuntimeError::invalid("请选择此会话的模型"))?;
            self.prepare_with_selection(snapshot, &selection, false)?;
        }
        let input = pi::TurnInput {
            text: text.into(),
            effort: self.requested_effort(request)?,
            images,
        };
        if let ActiveState::Acp(session) = &mut self.session.active_state {
            self.session.unsubmitted_draft = None;
            session
                .send(text, &input.images)
                .map_err(|error| RuntimeError::context("ACP message send", error))?;
            self.drain_runtime()?;
            return self.snapshot_result();
        }
        if let ActiveState::Native(session) = &mut self.session.active_state {
            self.session.unsubmitted_draft = None;
            let result = session
                .send(&input)
                .map_err(|error| RuntimeError::context("message send", error));
            if result.is_err() && self.session.gateway_runner.is_some() {
                self.finish_managed_turn()?;
            }
            result?;
            self.drain_runtime()?;
            if self.session.gateway_runner.is_some() && !self.busy() {
                self.finish_managed_turn()?;
            }
            return self.snapshot_result();
        }
        if matches!(
            self.session.model_selection,
            Some(crate::ModelSelection::Managed { .. })
        ) {
            self.bind_gateway_model()?;
        }
        self.drain_runtime()?;
        self.session.unsubmitted_draft = None;
        let response = self
            .session
            .pi
            .client
            .as_mut()
            .ok_or_else(|| RuntimeError::invalid("会话执行尚未准备"))?
            .prompt(&input)
            .map_err(|error| RuntimeError::context("message send", error));
        if response.is_err() && self.session.gateway_runner.is_some() {
            self.finish_managed_turn()?;
        }
        let response = response?;
        self.session.pi.busy = response["data"]["disposition"] != "handled";
        if !self.session.pi.busy {
            self.session
                .pi
                .projection
                .as_mut()
                .expect("accepted Pi prompt")
                .invalidate_history();
        }
        self.session.pi.turn_started = false;
        if self.busy()
            && let Some(snapshot) = self
                .session
                .pi
                .projection
                .as_mut()
                .and_then(|item| item.snapshot.as_mut())
        {
            snapshot.run_state = RunState::Running;
            snapshot.actions.can_send = false;
            snapshot.actions.can_cancel = true;
        }
        self.sync_projection()?;
        if self.session.gateway_runner.is_some() && !self.busy() {
            self.finish_managed_turn()?;
        }
        self.snapshot_result()
    }

    pub(super) fn bind_gateway_model(&mut self) -> Result<(), RuntimeError> {
        self.session
            .pi
            .client
            .as_mut()
            .ok_or_else(|| RuntimeError::invalid("会话执行尚未准备"))?
            .request(json!({"type":"set_model","provider":"velune","modelId":"auto"}))
            .map_err(|error| RuntimeError::context("gateway model binding", error))?;
        Ok(())
    }

    pub(super) fn cancel_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        self.session.cancel_requested = true;
        if let ActiveState::Acp(session) = &mut self.session.active_state {
            session
                .cancel()
                .map_err(|error| RuntimeError::context("ACP cancel", error))?;
            self.drain_runtime()?;
            return self.snapshot_result();
        }
        if let ActiveState::Native(session) = &mut self.session.active_state {
            session
                .cancel()
                .map_err(|error| RuntimeError::context("message cancel", error))?;
            self.drain_runtime()?;
            return self.snapshot_result();
        }
        self.session
            .pi
            .client
            .as_mut()
            .ok_or_else(|| RuntimeError::invalid("会话执行尚未准备"))?
            .cancel()
            .map_err(|error| RuntimeError::context("message cancel", error))?;
        self.sync_projection()?;
        self.snapshot_result()
    }

    pub(super) fn ensure_active(&self, request: &Value) -> Result<(), RuntimeError> {
        let requested = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
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
        if matches!(
            self.session.active_state,
            ActiveState::Native(_) | ActiveState::Acp(_)
        ) || self.session.pi.busy
            || self
                .session
                .pi
                .projection
                .as_ref()
                .is_some_and(PiProjection::history_synchronized)
        {
            return Ok(());
        }
        if let (Some(client), Some(projection)) = (
            self.session.pi.client.as_mut(),
            self.session.pi.projection.as_mut(),
        ) {
            let (state, messages) = client
                .state()
                .map_err(|error| RuntimeError::context("runtime state", error))?;
            // Events received before get_messages' response are covered by that
            // authoritative context. Do not replay them after replacing history.
            for event in client.take_buffered_events() {
                projection.apply_event(&event);
            }
            if let Some(path) = state["data"]["sessionFile"].as_str() {
                let runtime_id = self
                    .session
                    .execution_runtime_id
                    .as_deref()
                    .expect("a Pi client has an active runtime instance");
                projection.set_conversation(ConversationSummary {
                    can_rename: false,
                    can_delete: false,
                    id: format!("{runtime_id}:{path}"),
                    title: velune_conversation::conversation_title(
                        state["data"]["sessionName"].as_str(),
                        None,
                    ),
                    created_at_unix_ms: projection
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.conversation.created_at_unix_ms),
                    updated_at_unix_ms: projection
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.conversation.updated_at_unix_ms),
                    runtime_id: runtime_id.into(),
                    cwd: projection
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.conversation.cwd.clone()),
                });
            }
            projection.replace_history(&messages);
            if let Some(snapshot) = projection.snapshot.as_mut() {
                snapshot.model_record_key = self.session.model_record_key.clone();
                snapshot.actions.can_send = !self.session.pi.busy;
                snapshot.actions.can_cancel = self.session.pi.busy;
            }
        }
        self.drain_runtime()?;
        Ok(())
    }

    pub(super) fn drain_pi(&mut self) {
        let events = self
            .session
            .pi
            .client
            .as_mut()
            .map(|client| client.poll())
            .unwrap_or_default();
        for event in events {
            let is_settled = event["type"] == "agent_settled";
            let matched_settled = is_settled && self.session.pi.turn_started;
            // History and bootstrap message events do not begin an agent run.
            // Only the SDK's run boundary can authorize its settled event.
            if event["type"] == "agent_start" {
                self.session.pi.turn_started = true;
            }
            if matched_settled {
                self.session.pi.busy = false;
                self.session.pi.turn_started = false;
            }
            if (!is_settled || matched_settled)
                && let Some(projection) = self.session.pi.projection.as_mut()
            {
                projection.apply_event(&event);
            }
        }
    }
}
