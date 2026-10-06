//! Local application conversations use cases.
use super::*;
impl CoreRuntime {
    pub(super) fn create_conversation(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if self.busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let runtime_id = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        let runtime = self.runtime_instance(runtime_id)?.clone();
        let cwd = request["payload"]["cwd"]
            .as_str()
            .map(PathBuf::from)
            .ok_or_else(|| RuntimeError::invalid("conversation working directory"))?;
        let key = request["payload"]["modelRecordKey"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("请选择会话模型"))?;
        validate_session_cwd(&cwd)?;
        self.validate_session_model(&runtime, key)?;
        let snapshot = PiProjection::new(ConversationSummary {
            id: "new".into(),
            title: "新会话".into(),
            updated_at: None,
            runtime_id: runtime_id.into(),
            cwd: Some(cwd.to_string_lossy().into_owned()),
        })
        .snapshot
        .expect("new projection");
        self.prepare_snapshot(snapshot, key, true)?;
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
        // Resolve and project the destination before releasing the old view.
        let snapshot = self.read_history(runtime_id, id).inspect_err(|_| {
            if let Some(slot)=self.runtime_instances.iter().position(|r|r.id==runtime_id) {
                tracing::warn!(target:"velune_application",event="runtime_history_read_failed",phase="history_open",failure_kind="history_access_failed",runtime_slot=slot+1);
            }
        })?;
        self.invalidate_execution()?;
        self.selected_runtime_id = Some(runtime_id.into());
        self.model_record_key = snapshot.model_record_key.clone();
        self.active_state = ActiveState::History(Box::new(snapshot));
        Ok(json!({"snapshot":self.current_snapshot()}))
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
            let snapshot = self.current_snapshot().expect("history snapshot");
            let key = snapshot
                .model_record_key
                .clone()
                .ok_or_else(|| RuntimeError::invalid("请选择此会话的模型"))?;
            self.prepare_snapshot(snapshot, &key, false)?;
        }
        if let ActiveState::Native(session) = &mut self.active_state {
            session
                .send(text)
                .map_err(|_| RuntimeError::invalid("message send"))?;
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
            .map_err(|_| RuntimeError::invalid("message send"))?;
        if let Some(projection) = self.pi.projection.as_mut() {
            projection.append_user(text);
        }
        self.pi.busy = response["data"]["disposition"] != "handled";
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
        self.drain_runtime()?;
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
            .map_err(|_| RuntimeError::invalid("gateway model binding"))?;
        let _ = model_record_key;
        Ok(())
    }

    pub(super) fn cancel_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        if let ActiveState::Native(session) = &mut self.active_state {
            session
                .cancel()
                .map_err(|_| RuntimeError::invalid("message cancel"))?;
            self.drain_runtime()?;
            return Ok(json!({"snapshot":self.current_snapshot()}));
        }
        self.pi
            .client
            .as_mut()
            .ok_or_else(|| RuntimeError::invalid("会话执行尚未准备"))?
            .cancel()
            .map_err(|_| RuntimeError::invalid("message cancel"))?;
        self.drain_runtime()?;
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
            .find(|item| Some(&item.id) == self.selected_runtime_id.as_ref())
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
            .map_err(|_| RuntimeError::invalid("model selection"))?;
        self.pi
            .client
            .as_mut()
            .expect("prepared Pi client")
            .sync_virtual_selection()
            .map_err(|_| RuntimeError::invalid("model selection persistence"))?;
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
        if self.selected_runtime_id.as_deref() != Some(requested) {
            return Err(RuntimeError::invalid("runtime instance is not active"));
        }
        Ok(())
    }

    pub(super) fn sync_projection(&mut self) -> Result<(), RuntimeError> {
        self.drain_runtime()?;
        if matches!(self.active_state, ActiveState::Native(_)) {
            return Ok(());
        }
        if let (Some(client), Some(projection)) =
            (self.pi.client.as_mut(), self.pi.projection.as_mut())
        {
            let (state, messages) = client
                .state()
                .map_err(|_| RuntimeError::invalid("runtime state"))?;
            if let Some(path) = state["data"]["sessionFile"].as_str() {
                let runtime_id = self
                    .selected_runtime_id
                    .as_deref()
                    .expect("a Pi client has an active runtime instance");
                projection.set_conversation(ConversationSummary {
                    id: format!("{runtime_id}:{path}"),
                    title: Path::new(path)
                        .file_stem()
                        .and_then(|item| item.to_str())
                        .unwrap_or("会话")
                        .into(),
                    updated_at: None,
                    runtime_id: runtime_id.into(),
                    cwd: self
                        .pi
                        .config
                        .as_ref()
                        .and_then(|config| config.working_dir.as_ref())
                        .map(|path| path.to_string_lossy().into_owned()),
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
