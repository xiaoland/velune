//! Local application conversations use cases.
use super::*;
use crate::conversation::ConversationSnapshot;
use crate::local::continuation::{LogicalProjection, display_segment};
impl CoreRuntime {
    pub(super) fn send_turn_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if self.busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let source_runtime_id = request["payload"]["sourceRuntimeID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("source runtime instance id"))?;
        let runtime_id = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        let conversation_id = request["payload"]["conversationID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        let model = request["payload"]["modelRecordKey"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("model id"))?;
        let text = request["payload"]["text"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| RuntimeError::invalid("message text"))?;
        let active_snapshot = self.native_snapshot();
        let active_conversation_id = self
            .logical_projection
            .as_ref()
            .map(|projection| projection.origin.id.as_str())
            .or_else(|| {
                active_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.conversation.id.as_str())
            });
        let active_source_runtime = self
            .logical_projection
            .as_ref()
            .map(|projection| projection.origin.runtime_id.as_str())
            .or_else(|| {
                active_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.conversation.runtime_id.as_str())
            });
        if active_conversation_id != Some(conversation_id)
            || active_source_runtime != Some(source_runtime_id)
        {
            let (source, projection) =
                self.read_conversation_source(source_runtime_id, conversation_id)?;
            self.invalidate_execution()?;
            self.logical_projection = projection;
            self.model_record_key = source.model_record_key.clone();
            self.active_state = ActiveState::History(Box::new(source));
        }
        let snapshot = self
            .native_snapshot()
            .ok_or_else(|| RuntimeError::invalid("conversation is not active"))?;
        if matches!(self.active_state, ActiveState::History(_)) && !snapshot.actions.can_send {
            return Err(RuntimeError::invalid(
                "当前原生上下文不可用，请修复来源后重新打开；未自动重发",
            ));
        }
        if !snapshot.pending_interactions.is_empty() {
            return Err(RuntimeError::invalid("请先处理当前运行时请求"));
        }
        self.validate_session_model(self.runtime_instance(runtime_id)?, model)?;
        if snapshot.conversation.runtime_id != runtime_id {
            return self.continue_in_runtime(runtime_id, model, text);
        }
        if matches!(self.active_state, ActiveState::History(_)) {
            self.prepare_snapshot(snapshot, model, false)?;
        } else if self.model_record_key.as_deref() != Some(model) {
            self.select_model(
                &json!({"payload":{"runtimeInstanceID":runtime_id,"modelRecordKey":model}}),
            )?;
        }
        self.send_action(&json!({"payload":{"runtimeInstanceID":runtime_id,"text":text}}))
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
        let key = request["payload"]["modelRecordKey"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("请选择会话模型"))?;
        if let Some(cwd) = &cwd {
            validate_session_cwd(cwd)?;
        }
        self.validate_session_model(&runtime, key)?;
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
        self.prepare_snapshot(snapshot, key, true)?;
        self.logical_projection = None;
        Ok(json!({"snapshot":self.current_snapshot()}))
    }
    pub(super) fn open_conversation(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if self.busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let runtime_id = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        let id = request["payload"]["conversationID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        let (snapshot, projection) =
            if let Some(link) = self.conversation_links.iter().find(|link| link.id == id) {
                if link
                    .segments
                    .first()
                    .is_none_or(|s| s.runtime_instance_id != runtime_id)
                {
                    return Err(RuntimeError::invalid("会话来源运行时不匹配"));
                }
                let (snapshot, projection) = self.read_link(link)?;
                (snapshot, Some(projection))
            } else {
                (self.read_history(runtime_id, id)?, None)
            };
        self.invalidate_execution()?;
        self.logical_projection = projection;
        self.model_record_key = snapshot.model_record_key.clone();
        self.active_state = ActiveState::History(Box::new(snapshot));
        Ok(json!({"snapshot":self.current_snapshot()}))
    }

    fn read_conversation_source(
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
            .logical_projection
            .as_ref()
            .map(|projection| projection.origin.id.as_str())
            .or_else(|| {
                active_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.conversation.id.as_str())
            });
        let active_runtime = self
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
            return Ok(json!({"snapshot":self.current_snapshot()}));
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
        Ok(json!({"snapshot":Some(snapshot)}))
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
            && matches!(self.active_state, ActiveState::Pi)
            && self.execution_runtime_id.as_deref() == Some(runtime_id)
            && self
                .native_snapshot()
                .is_some_and(|snapshot| snapshot.conversation.id == id)
        {
            // A newly created Pi SessionManager reserves its native file path but
            // persists only after the first user/assistant record. Verify fresh
            // native metadata before granting this exact active draft exception.
            self.pi
                .projection
                .as_mut()
                .expect("active Pi projection")
                .invalidate_history();
            self.sync_projection()?;
            if self
                .native_snapshot()
                .is_some_and(|snapshot| snapshot.conversation.id == id)
                && std::fs::symlink_metadata(native_id)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                if deleting {
                    self.invalidate_execution()?;
                    self.active_state = ActiveState::Empty;
                    self.model_record_key = None;
                } else {
                    self.pi
                        .client
                        .as_mut()
                        .expect("active Pi client")
                        .request(
                            json!({"type":"set_session_name","name":title.expect("rename title")}),
                        )
                        .map_err(|error| {
                            RuntimeError::context("Pi 原生草稿重命名失败，请重试", error)
                        })?;
                    self.pi
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
            velune_agent_runtime::native::manage_thread(
                &setting_path(&runtime, "binary")?,
                &setting_path(&runtime, "agentDir")?,
                native_id,
                title,
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
                self.active_state = ActiveState::Empty;
                self.model_record_key = None;
            }
        } else {
            let summary = summaries
                .into_iter()
                .find(|summary| {
                    summary.id == id && summary.title.display_text() == title.expect("rename title")
                })
                .ok_or_else(|| RuntimeError::invalid("运行时尚未确认会话重命名，请刷新后重试"))?;
            if current && let ActiveState::History(snapshot) = &mut self.active_state {
                snapshot.conversation = summary;
                snapshot.revision = snapshot.revision.saturating_add(1);
            }
        }
        self.list()
    }

    pub(super) fn send_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        if self.busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let text = request["payload"]["text"]
            .as_str()
            .filter(|item| !item.trim().is_empty())
            .ok_or_else(|| RuntimeError::invalid("message text"))?;
        if matches!(self.active_state, ActiveState::History(_)) {
            let snapshot = self.native_snapshot().expect("history snapshot");
            let key = snapshot
                .model_record_key
                .clone()
                .ok_or_else(|| RuntimeError::invalid("请选择此会话的模型"))?;
            self.prepare_snapshot(snapshot, &key, false)?;
        }
        if let ActiveState::Native(session) = &mut self.active_state {
            session
                .send(text)
                .map_err(|error| RuntimeError::context("message send", error))?;
            self.drain_runtime()?;
            return Ok(json!({"snapshot":self.current_snapshot()}));
        }
        if self.pi.physical_model_id.is_none() {
            return Err(RuntimeError::invalid("conversation model is unavailable"));
        }
        // Native session state or commands may change the selected provider.
        // Every dispatch must return to the application-owned gateway.
        self.bind_gateway_model()?;
        self.drain_runtime()?;
        let response = self
            .pi
            .client
            .as_mut()
            .ok_or_else(|| RuntimeError::invalid("会话执行尚未准备"))?
            .prompt(text)
            .map_err(|error| RuntimeError::context("message send", error))?;
        self.pi.busy = response["data"]["disposition"] != "handled";
        if !self.pi.busy {
            self.pi
                .projection
                .as_mut()
                .expect("accepted Pi prompt")
                .invalidate_history();
        }
        self.pi.turn_started = false;
        if self.busy()
            && let Some(snapshot) = self
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
        Ok(json!({"snapshot":self.current_snapshot()}))
    }

    pub(super) fn bind_gateway_model(&mut self) -> Result<(), RuntimeError> {
        let model_record_key = self
            .pi
            .config
            .as_ref()
            .and_then(|config| config.selection_file.as_ref())
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|value| value["modelRecordKey"].as_str().map(str::to_owned))
            .ok_or_else(|| RuntimeError::invalid("runtime model id"))?;
        self.pi
            .client
            .as_mut()
            .ok_or_else(|| RuntimeError::invalid("会话执行尚未准备"))?
            .request(json!({"type":"set_model","provider":"velune","modelId":"auto"}))
            .map_err(|error| RuntimeError::context("gateway model binding", error))?;
        let _ = model_record_key;
        Ok(())
    }

    pub(super) fn cancel_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        if let ActiveState::Native(session) = &mut self.active_state {
            session
                .cancel()
                .map_err(|error| RuntimeError::context("message cancel", error))?;
            self.drain_runtime()?;
            return Ok(json!({"snapshot":self.current_snapshot()}));
        }
        self.pi
            .client
            .as_mut()
            .ok_or_else(|| RuntimeError::invalid("会话执行尚未准备"))?
            .cancel()
            .map_err(|error| RuntimeError::context("message cancel", error))?;
        self.sync_projection()?;
        Ok(json!({"snapshot":self.current_snapshot()}))
    }

    pub(super) fn select_model(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        if self.busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        if matches!(self.active_state, ActiveState::History(_)) {
            let key = request["payload"]["modelRecordKey"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("model id"))?;
            self.validate_session_model(self.active_instance()?, key)?;
            self.model_record_key = Some(key.into());
            if let ActiveState::History(snapshot) = &mut self.active_state {
                snapshot.model_record_key = Some(key.into());
                snapshot.actions.can_send = true;
                snapshot.revision = snapshot.revision.saturating_add(1);
            }
            return Ok(json!({"snapshot":self.current_snapshot()}));
        }
        if matches!(self.active_state, ActiveState::Native(_)) {
            let key = request["payload"]["modelRecordKey"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("model id"))?;
            let runtime = self.active_instance()?;
            self.validate_session_model(runtime, key)?;
            let gateway = self
                .gateways
                .iter()
                .find(|g| g.id == runtime.gateway_id)
                .expect("configured gateway");
            let protocol = |key: &str| {
                gateway
                    .providers
                    .iter()
                    .find(|p| p.models.iter().any(|m| m.record_key == key))
                    .map(|p| p.protocol.clone())
            };
            if self.model_record_key.as_deref().and_then(protocol) != protocol(key) {
                self.invalidate_execution()?;
                return self.select_model(request);
            }
        }
        if let ActiveState::Native(session) = &self.active_state {
            if session.snapshot().is_none() {
                return Err(RuntimeError::invalid("conversation is not active"));
            }
            let key = request["payload"]["modelRecordKey"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("model id"))?;
            let runtime = self.active_instance()?.clone();
            self.select_native_model(&runtime, key)?;
            self.model_record_key = Some(key.into());
            self.drain_runtime()?;
            return Ok(json!({"snapshot":self.current_snapshot()}));
        }
        if self.pi.client.is_none() {
            return Err(RuntimeError::invalid("conversation is not active"));
        }
        let model_record_key = request["payload"]["modelRecordKey"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("model id"))?;
        let runtime = self
            .runtime_instances
            .iter()
            .find(|item| Some(&item.id) == self.execution_runtime_id.as_ref())
            .expect("active runtime instance is configured");
        let gateway = self
            .gateways
            .iter()
            .find(|item| item.id == runtime.gateway_id)
            .expect("runtime gateway is configured");
        runnable_pi_model(gateway, model_record_key)?;
        let selected_subscription_capability = gateway.subscription(model_record_key);
        let selected_physical_model_id = gateway
            .pi_binding_id(
                model_record_key,
                gateway.authentication_revision(model_record_key),
            )
            .map_err(RuntimeError::invalid)?;
        self.write_selection(
            model_record_key,
            &selected_physical_model_id,
            selected_subscription_capability,
        )?;
        self.pi
            .client
            .as_mut()
            .expect("prepared Pi client")
            .request(json!({"type":"set_model","provider":"velune","modelId":"auto"}))
            .map_err(|error| RuntimeError::context("model selection", error))?;
        self.pi
            .client
            .as_mut()
            .expect("prepared Pi client")
            .sync_virtual_selection()
            .map_err(|error| RuntimeError::context("model selection persistence", error))?;
        self.drain_runtime()?;
        self.model_record_key = Some(model_record_key.into());
        self.pi.subscription_capability = selected_subscription_capability;
        self.pi.physical_model_id = Some(selected_physical_model_id);
        self.pi.config.as_mut().expect("prepared Pi config").model = Some("velune/auto".into());
        self.sync_projection()?;
        Ok(json!({"snapshot":self.current_snapshot()}))
    }

    pub(super) fn write_selection(
        &self,
        model_record_key: &str,
        physical_model_id: &str,
        subscription_capability: bool,
    ) -> Result<(), RuntimeError> {
        let config = self
            .pi
            .config
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid("会话执行尚未准备"))?;
        write_selection_file(
            config,
            model_record_key,
            physical_model_id,
            subscription_capability,
        )
    }

    pub(super) fn ensure_active(&self, request: &Value) -> Result<(), RuntimeError> {
        let requested = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        let active_runtime = self
            .native_snapshot()
            .map(|snapshot| snapshot.conversation.runtime_id)
            .or_else(|| self.execution_runtime_id.clone());
        if active_runtime.as_deref() != Some(requested) {
            return Err(RuntimeError::invalid("runtime instance is not active"));
        }
        Ok(())
    }

    pub(super) fn sync_projection(&mut self) -> Result<(), RuntimeError> {
        self.drain_runtime()?;
        if matches!(self.active_state, ActiveState::Native(_))
            || self.pi.busy
            || self
                .pi
                .projection
                .as_ref()
                .is_some_and(PiProjection::history_synchronized)
        {
            return Ok(());
        }
        if let (Some(client), Some(projection)) =
            (self.pi.client.as_mut(), self.pi.projection.as_mut())
        {
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
                snapshot.model_record_key = self.model_record_key.clone();
                snapshot.actions.can_send = snapshot.model_record_key.is_some() && !self.pi.busy;
                snapshot.actions.can_cancel = self.pi.busy;
            }
        }
        self.drain_runtime()?;
        Ok(())
    }

    pub(super) fn drain_pi(&mut self) {
        let events = self
            .pi
            .client
            .as_mut()
            .map(|client| client.poll())
            .unwrap_or_default();
        for event in events {
            let is_settled = event["type"] == "agent_settled";
            let matched_settled = is_settled && self.pi.turn_started;
            // History and bootstrap message events do not begin an agent run.
            // Only the SDK's run boundary can authorize its settled event.
            if event["type"] == "agent_start" {
                self.pi.turn_started = true;
            }
            if matched_settled {
                self.pi.busy = false;
                self.pi.turn_started = false;
            }
            if (!is_settled || matched_settled)
                && let Some(projection) = self.pi.projection.as_mut()
            {
                projection.apply_event(&event);
            }
        }
    }
}
