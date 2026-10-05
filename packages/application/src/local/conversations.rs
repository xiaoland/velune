//! Local application conversations use cases.
use super::*;
impl CoreRuntime {
    pub(super) fn create_conversation(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        if self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let cwd = request["payload"]["cwd"]
            .as_str()
            .map(PathBuf::from)
            .ok_or_else(|| RuntimeError::invalid("conversation working directory"))?;
        validate_session_cwd(&cwd)?;
        let default_model = self
            .runtime_instances
            .iter()
            .find(|item| Some(&item.id) == self.active_runtime_id.as_ref())
            .expect("active runtime instance is configured")
            .model_record_key
            .as_deref()
            .ok_or_else(|| RuntimeError::invalid("runtime model id"))?
            .to_owned();
        self.model_record_key = Some(default_model.clone());
        let gateway = self
            .runtime_instances
            .iter()
            .find(|item| Some(&item.id) == self.active_runtime_id.as_ref())
            .and_then(|runtime| {
                self.gateways
                    .iter()
                    .find(|gateway| gateway.id == runtime.gateway_id)
            })
            .ok_or_else(|| RuntimeError::invalid("runtime gateway"))?;
        self.subscription_capability = gateway.subscription(&default_model);
        let default_physical_model_id = gateway
            .pi_binding_id(
                &default_model,
                gateway.authentication_revision(&default_model),
            )
            .map_err(RuntimeError::invalid)?;
        self.start_pi_for_session(
            &cwd,
            None,
            Some(&default_model),
            Some(&default_physical_model_id),
            self.subscription_capability,
        )?;
        self.pi
            .as_mut()
            .expect("started Pi client")
            .request(json!({"type":"new_session"}))
            .map_err(|_| RuntimeError::invalid("conversation create"))?;
        self.bind_gateway_model()?;
        self.drain_pi();
        self.physical_model_id = Some(default_physical_model_id);
        self.sync_projection()?;
        Ok(json!({"snapshot":self.projection.as_ref().and_then(|item| item.snapshot.as_ref())}))
    }

    pub(super) fn open_conversation(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        if self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let id = request["payload"]["conversationID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        let runtime_id = self
            .active_runtime_id
            .as_ref()
            .expect("active runtime instance")
            .clone();
        let prefix = format!("{runtime_id}:");
        let path = id
            .strip_prefix(&prefix)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        // Read session metadata before starting Pi. The saved cwd is the
        // session's project context and must not fall back to the app cwd.
        let saved = pi_session_helper(
            self.pi_config.as_ref().expect("connected Pi config"),
            Some(&path),
        )?;
        let cwd = saved["cwd"]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| RuntimeError::invalid("session working directory is unavailable"))?;
        validate_session_cwd(&cwd)?;
        let restored_model = saved["virtualState"]["state"]["modelRecordKey"].as_str();
        let runtime = self
            .runtime_instances
            .iter()
            .find(|item| item.id == runtime_id)
            .expect("active runtime instance is configured");
        let gateway = self
            .gateways
            .iter()
            .find(|item| item.id == runtime.gateway_id)
            .expect("runtime gateway is configured");
        let model_record_key = restored_model
            .filter(|id| {
                ((saved["virtualState"]["provider"] == "velune"
                    && saved["virtualState"]["state"]["provider"] == "velune-gateway")
                    || saved["virtualState"]["provider"] == "velune-gateway"
                    || saved["model"]["provider"] == "velune-gateway")
                    && runnable_pi_model(gateway, id).is_ok()
            })
            .map(str::to_owned);
        self.physical_model_id = None;
        self.model_record_key = model_record_key.clone();
        self.subscription_capability = model_record_key
            .as_deref()
            .is_some_and(|id| gateway.subscription(id));
        if let Some(model_record_key) = model_record_key {
            let physical_model_id = gateway
                .pi_binding_id(
                    &model_record_key,
                    gateway.authentication_revision(&model_record_key),
                )
                .map_err(RuntimeError::invalid)?;
            self.physical_model_id = Some(physical_model_id.clone());
            self.start_pi_for_session(
                &cwd,
                Some(&path),
                Some(&model_record_key),
                Some(&physical_model_id),
                self.subscription_capability,
            )?;
        } else {
            self.start_pi_for_session(&cwd, Some(&path), None, None, false)?;
        }
        if let Some(projection) = self.projection.as_mut() {
            projection.set_conversation(ConversationSummary {
                id: id.into(),
                title: path
                    .file_stem()
                    .and_then(|item| item.to_str())
                    .unwrap_or("会话")
                    .into(),
                updated_at: None,
                runtime_id,
                cwd: Some(cwd.to_string_lossy().into_owned()),
            });
        }
        self.sync_projection()?;
        Ok(json!({"snapshot":self.projection.as_ref().and_then(|item| item.snapshot.as_ref())}))
    }

    pub(super) fn send_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        if self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let text = request["payload"]["text"]
            .as_str()
            .filter(|item| !item.trim().is_empty())
            .ok_or_else(|| RuntimeError::invalid("message text"))?;
        if self.physical_model_id.is_none() {
            return Err(RuntimeError::invalid("conversation model is unavailable"));
        }
        // Native session state or commands may change the selected provider.
        // Every dispatch must return to the application-owned gateway.
        self.bind_gateway_model()?;
        self.drain_pi();
        let response = self
            .pi
            .as_mut()
            .ok_or_else(|| RuntimeError::invalid("runtime is not connected"))?
            .prompt(text)
            .map_err(|_| RuntimeError::invalid("message send"))?;
        if let Some(projection) = self.projection.as_mut() {
            projection.append_user(text);
        }
        self.pi_busy = response["data"]["disposition"] != "handled";
        self.pi_turn_started = false;
        if self.pi_busy
            && let Some(snapshot) = self
                .projection
                .as_mut()
                .and_then(|item| item.snapshot.as_mut())
        {
            snapshot.run_state = RunState::Running;
            snapshot.actions.can_send = false;
            snapshot.actions.can_cancel = true;
        }
        self.drain_pi();
        Ok(json!({"snapshot":self.projection.as_ref().and_then(|item| item.snapshot.as_ref())}))
    }

    pub(super) fn bind_gateway_model(&mut self) -> Result<(), RuntimeError> {
        let model_record_key = self
            .pi_config
            .as_ref()
            .and_then(|config| config.selection_file.as_ref())
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|value| value["modelRecordKey"].as_str().map(str::to_owned))
            .ok_or_else(|| RuntimeError::invalid("runtime model id"))?;
        self.pi
            .as_mut()
            .ok_or_else(|| RuntimeError::invalid("runtime is not connected"))?
            .request(json!({"type":"set_model","provider":"velune","modelId":"auto"}))
            .map_err(|_| RuntimeError::invalid("gateway model binding"))?;
        let _ = model_record_key;
        Ok(())
    }

    pub(super) fn cancel_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        self.pi
            .as_mut()
            .ok_or_else(|| RuntimeError::invalid("runtime is not connected"))?
            .cancel()
            .map_err(|_| RuntimeError::invalid("message cancel"))?;
        self.drain_pi();
        Ok(json!({"snapshot":self.projection.as_ref().and_then(|item| item.snapshot.as_ref())}))
    }

    pub(super) fn select_model(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        if self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        if self.pi.is_none() {
            return Err(RuntimeError::invalid("conversation is not active"));
        }
        let model_record_key = request["payload"]["modelRecordKey"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("model id"))?;
        let runtime = self
            .runtime_instances
            .iter()
            .find(|item| Some(&item.id) == self.active_runtime_id.as_ref())
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
            .as_mut()
            .expect("connected Pi client")
            .request(json!({"type":"set_model","provider":"velune","modelId":"auto"}))
            .map_err(|_| RuntimeError::invalid("model selection"))?;
        self.pi
            .as_mut()
            .expect("connected Pi client")
            .sync_virtual_selection()
            .map_err(|_| RuntimeError::invalid("model selection persistence"))?;
        self.drain_pi();
        self.subscription_capability = selected_subscription_capability;
        self.physical_model_id = Some(selected_physical_model_id);
        self.pi_config.as_mut().expect("connected Pi config").model = Some("velune/auto".into());
        self.sync_projection()?;
        Ok(json!({"snapshot":self.projection.as_ref().and_then(|item| item.snapshot.as_ref())}))
    }

    pub(super) fn write_selection(
        &self,
        model_record_key: &str,
        physical_model_id: &str,
        subscription_capability: bool,
    ) -> Result<(), RuntimeError> {
        let config = self
            .pi_config
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid("runtime is not connected"))?;
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
        if self.active_runtime_id.as_deref() != Some(requested) {
            return Err(RuntimeError::invalid("runtime instance is not active"));
        }
        Ok(())
    }

    pub(super) fn sync_projection(&mut self) -> Result<(), RuntimeError> {
        self.drain_pi();
        if let (Some(client), Some(projection)) = (self.pi.as_mut(), self.projection.as_mut()) {
            let (state, messages) = client
                .state()
                .map_err(|_| RuntimeError::invalid("runtime state"))?;
            if let Some(path) = state["data"]["sessionFile"].as_str() {
                let runtime_id = self
                    .active_runtime_id
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
                        .pi_config
                        .as_ref()
                        .and_then(|config| config.working_dir.as_ref())
                        .map(|path| path.to_string_lossy().into_owned()),
                });
            }
            projection.replace_history(&messages);
            if let Some(snapshot) = projection.snapshot.as_mut() {
                snapshot.model_record_key = self.model_record_key.clone();
                snapshot.actions.can_send = snapshot.model_record_key.is_some() && !self.pi_busy;
                snapshot.actions.can_cancel = self.pi_busy;
            }
        }
        self.drain_pi();
        Ok(())
    }

    pub(super) fn drain_pi(&mut self) {
        let events = self
            .pi
            .as_mut()
            .map(|client| client.poll())
            .unwrap_or_default();
        for event in events {
            let is_settled = event["type"] == "agent_settled";
            let matched_settled = is_settled && self.pi_turn_started;
            // History and bootstrap message events do not begin an agent run.
            // Only the SDK's run boundary can authorize its settled event.
            if event["type"] == "agent_start" {
                self.pi_turn_started = true;
            }
            if matched_settled {
                self.pi_busy = false;
                self.pi_turn_started = false;
            }
            if (!is_settled || matched_settled)
                && let Some(projection) = self.projection.as_mut()
            {
                projection.apply_event(&event);
            }
        }
    }

    pub(super) fn session_summaries(&self) -> Result<Vec<ConversationSummary>, RuntimeError> {
        let Some(config) = self.pi_config.as_ref() else {
            return Ok(Vec::new());
        };
        let value = pi_session_helper(config, None)?;
        let runtime_id = self.active_runtime_id.as_deref().unwrap_or_default();
        let summaries = value["sessions"]
            .as_array()
            .ok_or_else(|| RuntimeError::invalid("Pi session helper response"))?
            .iter()
            .map(|item| {
                let path = item["path"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("Pi session entry"))?;
                let title = item["name"]
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .or_else(|| path.rsplit('/').next())
                    .unwrap_or("会话");
                Ok(ConversationSummary {
                    id: format!("{runtime_id}:{path}"),
                    title: title.into(),
                    updated_at: item["modified"].as_str().map(str::to_owned),
                    runtime_id: runtime_id.into(),
                    cwd: item["cwd"].as_str().map(str::to_owned),
                })
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        Ok(summaries)
    }
}
