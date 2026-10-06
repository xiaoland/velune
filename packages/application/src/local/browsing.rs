//! Read-only browsing and deferred execution preparation share one active view.
use super::*;
use crate::conversation::{ConversationActions, ConversationSnapshot, Message, MessageBlock};
pub(super) fn runtime_history_context(runtime: &RuntimeInstance) -> String {
    format!(
        "运行时 {}（id={}，type={}）；binary={}；nodeBinary={}；agentDir={}；sessionDir={}",
        runtime.name,
        runtime.id,
        runtime.type_id,
        runtime
            .settings
            .get("binary")
            .map(String::as_str)
            .unwrap_or("未配置"),
        runtime
            .settings
            .get("nodeBinary")
            .map(String::as_str)
            .unwrap_or("未配置"),
        runtime
            .settings
            .get("agentDir")
            .map(String::as_str)
            .unwrap_or("未配置"),
        runtime
            .settings
            .get("sessionDir")
            .map(String::as_str)
            .unwrap_or("默认"),
    )
}
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
        if !self.runtime_instance(id)?.enabled {
            return Err(RuntimeError::invalid("此运行时已停用，请先启用"));
        }
        if self.next_turn_runtime_id.as_deref() != Some(id) {
            self.next_turn_runtime_id = Some(id.into());
        }
        self.list()
    }
    pub(super) fn pi_history_config(
        &self,
        runtime: &RuntimeInstance,
    ) -> Result<PiConfig, RuntimeError> {
        Ok(PiConfig {
            runtime_type_id: runtime.type_id.clone(),
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
            return history::list(&self.history_config(runtime)?, &runtime.id).map_err(|error| {
                let variant = velune_agent_runtime::version::variant(&runtime.type_id);
                let family = variant.map(|value| value.family_id).unwrap_or("unknown");
                let runtime_type = variant.map(|value| value.id).unwrap_or("unknown");
                tracing::warn!(target: "velune_application", event = "runtime_history_read_failed", phase = error.phase(), failure_kind = error.code(), runtime_family = family, runtime_type, runtime_id=%runtime.id, runtime_name=%runtime.name, runtime_type_id=%runtime.type_id, binary=?runtime.settings.get("binary"), node_binary=?runtime.settings.get("nodeBinary"), agent_dir=?runtime.settings.get("agentDir"), detail=%error);
                RuntimeError::History(error)
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
                    can_rename: false,
                    can_delete: false,
                    id: format!("{}:{path}", runtime.id),
                    title: velune_conversation::conversation_title(
                        item["name"].as_str(),
                        item["firstMessage"].as_str(),
                    ),
                    updated_at_unix_ms: item["modifiedUnixMs"].as_i64(),
                    created_at_unix_ms: item["createdUnixMs"].as_i64(),
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
        let protocols = GatewayProtocol::runtime_provider_protocols(&runtime.type_id)
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
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用，请先启用"));
        }
        let native_id = id
            .strip_prefix(&format!("{runtime_id}:"))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        // Source-scoped lookup prevents arbitrary caller paths from being opened.
        let summary = self
            .summaries_for(runtime)
            .map_err(|error| {
                tracing::warn!(target: "velune_application", event="runtime_history_read_failed", phase="history_lookup", runtime_id=%runtime.id, runtime_name=%runtime.name, runtime_type_id=%runtime.type_id, binary=?runtime.settings.get("binary"), node_binary=?runtime.settings.get("nodeBinary"), agent_dir=?runtime.settings.get("agentDir"), detail=%error);
                if runtime.type_id == "pi-1.0.2" {
                    RuntimeError::context(&runtime_history_context(runtime), error)
                } else {
                    error
                }
            })?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| RuntimeError::invalid("此会话不属于所选运行时历史目录"))?;
        let mut snapshot = ConversationSnapshot {
            revision: 1,
            context_runtime_id: runtime_id.into(),
            conversation: summary,
            resource_id: None,
            model_record_key: None,
            run_state: RunState::Idle,
            messages: Vec::new(),
            transcript_turns: Vec::new(),
            message_identity_confirmations: Vec::new(),
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
            ).map_err(|error| {
                tracing::warn!(target: "velune_application", event="runtime_history_read_failed", phase="history_read", runtime_id=%runtime.id, runtime_name=%runtime.name, runtime_type_id=%runtime.type_id, binary=?runtime.settings.get("binary"), node_binary=?runtime.settings.get("nodeBinary"), agent_dir=?runtime.settings.get("agentDir"), detail=%error);
                RuntimeError::context(&runtime_history_context(runtime), error)
            })?;
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
            let read = history::read(&self.history_config(runtime)?, native_id).map_err(|error| {
                let variant = velune_agent_runtime::version::variant(&runtime.type_id);
                let family = variant.map(|value| value.family_id).unwrap_or("unknown");
                let runtime_type = variant.map(|value| value.id).unwrap_or("unknown");
                tracing::warn!(target: "velune_application", event = "runtime_history_read_failed", phase = error.phase(), failure_kind = error.code(), runtime_family = family, runtime_type, runtime_id=%runtime.id, runtime_name=%runtime.name, runtime_type_id=%runtime.type_id, binary=?runtime.settings.get("binary"), node_binary=?runtime.settings.get("nodeBinary"), agent_dir=?runtime.settings.get("agentDir"), detail=%error);
                RuntimeError::History(error)
            })?;
            snapshot.conversation.cwd = read.cwd;
            snapshot.messages = read.messages;
        }
        // A history projection is send-capable once the UI supplies the next
        // turn's model. The source record may intentionally have no model
        // marker, so this flag must not encode the old model-selection state.
        snapshot.actions.can_send = true;
        Ok(snapshot)
    }
    pub(super) fn invalidate_execution(&mut self) -> Result<(), RuntimeError> {
        let mut snapshot = self.native_snapshot();
        // Closing execution leaves its native history view usable. A read-only
        // view has no execution owner, so restoration must follow its source.
        let source = snapshot
            .as_ref()
            .map(|view| view.conversation.runtime_id.clone());
        let shutdown_result = self.shutdown_active();
        if let Some(id) = source.filter(|id| self.runtime_instance(id).is_ok_and(|r| r.enabled)) {
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
                    can_send: true,
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
        let previous = self.native_snapshot();
        let previous_runtime = self.next_turn_runtime_id.clone();
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
                    .map_err(|error| RuntimeError::context("conversation create", error))?;
            }
            if !is_new {
                self.pi
                    .projection
                    .as_mut()
                    .expect("started Pi")
                    .set_conversation(snapshot.conversation.clone());
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
            self.next_turn_runtime_id = previous_runtime;
            self.model_record_key = previous.as_ref().and_then(|s| s.model_record_key.clone());
            if let Some(mut view) = previous {
                view.messages.push(Message {
                    id: format!("prepare-failed:{}", view.revision),
                    role: velune_conversation::MessageRole::System,
                    timestamp_unix_ms: None,
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
