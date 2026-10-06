//! Read-only browsing and deferred execution preparation share one active view.
use super::*;
use crate::conversation::{ConversationActions, ConversationSnapshot, Message, MessageBlock};
impl CoreRuntime {
    pub(super) fn runtime_instance(&self, id: &str) -> Result<&RuntimeInstance, RuntimeError> {
        self.runtime_instances
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))
    }
    pub(super) fn select_runtime_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if self.busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let id = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        self.runtime_instance(id)?;
        if self.selected_runtime_id.as_deref() != Some(id) {
            self.invalidate_execution()?;
            self.active_state = ActiveState::Empty;
            self.model_record_key = None;
            self.selected_runtime_id = Some(id.into());
        }
        self.list()
    }
    fn pi_history_config(&self, runtime: &RuntimeInstance) -> Result<PiConfig, RuntimeError> {
        Ok(PiConfig {
            binary: setting_path(runtime, "binary")?,
            node_binary: Some(setting_path(runtime, "nodeBinary")?),
            sdk_helper: Some(self.options.resources_directory.join("pi_sessions.mjs")),
            agent_dir: Some(setting_path(runtime, "agentDir")?),
            session_dir: setting_path_optional(runtime, "sessionDir"),
            working_dir: None,
            provider: None,
            model: None,
            session: None,
            name: None,
            extension: None,
            rpc_entry: None,
            models_path: None,
            selection_file: None,
            gateway_token: None,
        })
    }
    pub(super) fn summaries_for(
        &self,
        runtime: &RuntimeInstance,
    ) -> Result<Vec<ConversationSummary>, RuntimeError> {
        if runtime.type_id != "pi-1.0.2" {
            return history::list(&self.history_config(runtime)?, &runtime.id).map_err(|_| {
                RuntimeError::invalid("运行时历史目录无法读取；请检查实例配置后重试")
            });
        }
        let value = pi_session_helper(&self.pi_history_config(runtime)?, None)?;
        value["sessions"]
            .as_array()
            .ok_or_else(|| RuntimeError::invalid("Pi 历史列表格式不匹配"))?
            .iter()
            .map(|item| {
                let path = item["path"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("Pi 历史条目标识无效"))?;
                Ok(ConversationSummary {
                    id: format!("{}:{path}", runtime.id),
                    title: item["name"]
                        .as_str()
                        .filter(|v| !v.is_empty())
                        .or_else(|| path.rsplit('/').next())
                        .unwrap_or("会话")
                        .into(),
                    updated_at: item["modified"].as_str().map(str::to_owned),
                    runtime_id: runtime.id.clone(),
                    cwd: item["cwd"].as_str().map(str::to_owned),
                })
            })
            .collect()
    }
    pub(super) fn validate_session_model(
        &self,
        runtime: &RuntimeInstance,
        key: &str,
    ) -> Result<(), RuntimeError> {
        let gateway = self
            .gateways
            .iter()
            .find(|g| g.id == runtime.gateway_id)
            .ok_or_else(|| RuntimeError::invalid("runtime gateway"))?;
        let provider = gateway
            .providers
            .iter()
            .find(|p| p.models.iter().any(|m| m.record_key == key))
            .ok_or_else(|| RuntimeError::invalid("所选会话模型已不存在，请重新选择"))?;
        let protocols = GatewayProtocol::runtime_protocols(&runtime.type_id)
            .ok_or_else(|| RuntimeError::invalid("runtime type"))?;
        if !protocols.contains(&provider.protocol) {
            return Err(RuntimeError::invalid("所选模型协议不适用于此运行时版本"));
        }
        Ok(())
    }
    pub(super) fn read_history(
        &self,
        runtime_id: &str,
        id: &str,
    ) -> Result<ConversationSnapshot, RuntimeError> {
        let runtime = self.runtime_instance(runtime_id)?;
        let native_id = id
            .strip_prefix(&format!("{runtime_id}:"))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        // Source-scoped lookup prevents arbitrary caller paths from being opened.
        let summary = self
            .summaries_for(runtime)?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| RuntimeError::invalid("此会话不属于所选运行时历史目录"))?;
        let mut snapshot = ConversationSnapshot {
            revision: 1,
            conversation: summary,
            resource_id: None,
            model_record_key: None,
            run_state: RunState::Idle,
            messages: Vec::new(),
            pending_interactions: Vec::new(),
            actions: ConversationActions {
                can_send: false,
                can_cancel: false,
                can_switch: true,
            },
        };
        if runtime.type_id == "pi-1.0.2" {
            let saved = pi_session_helper(
                &self.pi_history_config(runtime)?,
                Some(Path::new(native_id)),
            )?;
            let mut projection = PiProjection::new(snapshot.conversation.clone());
            projection.replace_history(&saved);
            snapshot.messages = projection.snapshot.expect("history projection").messages;
            snapshot.conversation.cwd = saved["cwd"].as_str().map(str::to_owned);
            let marker = &saved["virtualState"];
            if marker["provider"] == "velune" || marker["provider"] == "velune-gateway" {
                snapshot.model_record_key = marker["state"]["modelRecordKey"]
                    .as_str()
                    .filter(|key| self.validate_session_model(runtime, key).is_ok())
                    .map(str::to_owned);
            }
        } else {
            let read = history::read(&self.history_config(runtime)?, native_id)
                .map_err(|_| RuntimeError::invalid("会话历史详情无法读取，请检查实例配置后重试"))?;
            snapshot.conversation.cwd = read.cwd;
            snapshot.messages = read.messages;
        }
        snapshot.actions.can_send = snapshot.model_record_key.is_some();
        Ok(snapshot)
    }
    pub(super) fn invalidate_execution(&mut self) -> Result<(), RuntimeError> {
        let mut snapshot = self.current_snapshot();
        let selected = self.selected_runtime_id.clone();
        let shutdown_result = self.shutdown_active();
        if let Some(id) = selected.filter(|id| self.runtime_instance(id).is_ok()) {
            self.selected_runtime_id = Some(id.clone());
            if let Some(view) = snapshot.as_mut() {
                if view.model_record_key.as_deref().is_some_and(|key| {
                    self.validate_session_model(
                        self.runtime_instance(&id).expect("configured instance"),
                        key,
                    )
                    .is_err()
                }) {
                    view.model_record_key = None;
                }
                view.run_state = RunState::Idle;
                view.pending_interactions.clear();
                view.actions = ConversationActions {
                    can_send: view.model_record_key.is_some(),
                    can_cancel: false,
                    can_switch: true,
                };
                view.revision = view.revision.saturating_add(1);
                self.model_record_key = view.model_record_key.clone();
            }
            if let Some(snapshot) = snapshot {
                self.active_state = ActiveState::History(Box::new(snapshot));
            }
        }
        shutdown_result
    }
    pub(super) fn prepare_snapshot(
        &mut self,
        snapshot: ConversationSnapshot,
        key: &str,
        is_new: bool,
    ) -> Result<(), RuntimeError> {
        let previous = self.current_snapshot();
        let previous_runtime = self.selected_runtime_id.clone();
        let runtime_id = snapshot.conversation.runtime_id.clone();
        let runtime = self.runtime_instance(&runtime_id)?.clone();
        self.validate_session_model(&runtime, key)?;
        let cwd = snapshot
            .conversation
            .cwd
            .as_deref()
            .map(PathBuf::from)
            .ok_or_else(|| RuntimeError::invalid("会话工作目录不可用"))?;
        validate_session_cwd(&cwd)?;
        let gateway = self
            .gateways
            .iter()
            .find(|g| g.id == runtime.gateway_id)
            .expect("configured gateway");
        let provider = gateway
            .providers
            .iter()
            .find(|p| p.models.iter().any(|m| m.record_key == key))
            .expect("validated model");
        if !provider.authentication.description().configured {
            return Err(RuntimeError::invalid(
                "此模型的提供商尚未配置认证，请先完成提供商配置",
            ));
        }
        velune_agent_runtime::version::check_version(
            &runtime.type_id,
            &setting_path(&runtime, "binary")?,
            setting_path_optional(&runtime, "nodeBinary").as_deref(),
        )
        .map_err(|error| RuntimeError::Invalid(error.to_string()))?;
        if runtime.type_id == "pi-1.0.2" {
            runnable_pi_model(gateway, key)?;
        }
        gateway.to_gateway_config()?;
        let result = (|| {
            self.prepare_runtime(&runtime_id, key)?;
            if runtime.type_id != "pi-1.0.2" {
                if is_new {
                    self.native_create(&cwd, key)?;
                } else {
                    self.native_resume(&snapshot, key)?;
                }
                return Ok(());
            }
            let gateway = self
                .gateways
                .iter()
                .find(|g| g.id == runtime.gateway_id)
                .expect("prepared gateway");
            let physical = gateway
                .pi_binding_id(key, gateway.authentication_revision(key))
                .map_err(RuntimeError::invalid)?;
            let subscription = gateway.subscription(key);
            let session = if is_new {
                None
            } else {
                Some(
                    snapshot
                        .conversation
                        .id
                        .strip_prefix(&format!("{runtime_id}:"))
                        .ok_or_else(|| RuntimeError::invalid("conversation id"))?,
                )
            };
            self.start_pi_for_session(
                &cwd,
                session.map(Path::new),
                Some(key),
                Some(&physical),
                subscription,
            )?;
            if is_new {
                self.pi
                    .client
                    .as_mut()
                    .expect("started Pi")
                    .request(json!({"type":"new_session"}))
                    .map_err(|_| RuntimeError::invalid("conversation create"))?;
            }
            self.model_record_key = Some(key.into());
            self.bind_gateway_model()?;
            self.sync_projection()?;
            Ok(())
        })();
        if let Err(error) = result {
            // Preparation never submits a prompt. Failed setup is retryable;
            // an accepted send is never automatically dispatched a second time.
            let _ = self.shutdown_active();
            self.selected_runtime_id = previous_runtime;
            self.model_record_key = previous.as_ref().and_then(|s| s.model_record_key.clone());
            if let Some(mut view) = previous {
                view.messages.push(Message {
                    id: format!("prepare-failed:{}", view.revision),
                    role: "system".into(),
                    blocks: vec![MessageBlock::Notice {
                        text: "运行时准备失败；会话内容已保留，可修正配置后重试。".into(),
                    }],
                });
                view.revision = view.revision.saturating_add(1);
                self.active_state = ActiveState::History(Box::new(view));
            }
            return Err(error);
        }
        Ok(())
    }
}
