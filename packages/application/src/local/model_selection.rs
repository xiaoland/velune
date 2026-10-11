//! Model provenance stays explicit across discovery, preparation, and continuation.
use super::*;
use crate::conversation::ConversationSnapshot;
use crate::{ModelSelection, RuntimeModel};

pub(super) fn requested_selection(request: &Value) -> Result<ModelSelection, RuntimeError> {
    Ok(serde_json::from_value(
        request["payload"]["modelSelection"].clone(),
    )?)
}

impl CoreRuntime {
    pub(super) fn validate_model_selection(
        &self,
        runtime: &RuntimeInstance,
        selection: &ModelSelection,
    ) -> Result<(), RuntimeError> {
        if runtime.type_id == "acp-v1" {
            return match selection {
                ModelSelection::RuntimeDefault => Ok(()),
                ModelSelection::Native { provider_id: None, model_id } if !model_id.is_empty() && !model_id.chars().any(char::is_control) => Ok(()),
                _ => Err(RuntimeError::Unsupported("通用 ACP 只接受 Agent 默认模型或其会话中的原生模型选项，不支持 managed provider 注入".into())),
            };
        }
        match selection {
            ModelSelection::RuntimeDefault => Err(RuntimeError::Unsupported(
                "默认模型选择当前只适用于通用 ACP".into(),
            )),
            ModelSelection::Managed { model_record_key } => {
                self.validate_session_model(runtime, model_record_key)
            }
            ModelSelection::Native {
                provider_id,
                model_id,
            } => {
                if model_id.trim().is_empty()
                    || model_id.chars().any(char::is_control)
                    || provider_id
                        .as_ref()
                        .is_some_and(|id| id.trim().is_empty() || id.chars().any(char::is_control))
                {
                    return Err(RuntimeError::invalid("原生提供商或模型标识无效"));
                }
                if matches!(runtime.type_id.as_str(), "pi-1.0.2" | "dsh-acp-0.2.0-rc.2")
                    && provider_id.is_none()
                {
                    return Err(RuntimeError::invalid("此运行时的原生模型必须指定提供商"));
                }
                if runtime.type_id == "claude-code-2.1.286" && provider_id.is_some() {
                    return Err(RuntimeError::invalid(
                        "Claude Code 原生模型使用其既有单一提供商",
                    ));
                }
                Ok(())
            }
        }
    }

    pub(super) fn prepare_with_selection(
        &mut self,
        snapshot: ConversationSnapshot,
        selection: &ModelSelection,
        is_new: bool,
    ) -> Result<(), RuntimeError> {
        if self
            .runtime_instance(&snapshot.conversation.runtime_id)?
            .type_id
            == "acp-v1"
        {
            self.prepare_acp_snapshot(&snapshot, selection, is_new)?;
            self.session.model_selection = Some(selection.clone());
            self.session.model_record_key = None;
            return Ok(());
        }
        match selection {
            ModelSelection::RuntimeDefault => {
                return Err(RuntimeError::Unsupported(
                    "默认模型选择当前只适用于通用 ACP".into(),
                ));
            }
            ModelSelection::Managed { model_record_key } => {
                self.prepare_snapshot(snapshot, model_record_key, is_new)?
            }
            ModelSelection::Native {
                provider_id,
                model_id,
            } => {
                let previous = self.native_snapshot();
                let previous_selection = self.session.model_selection.clone();
                let result =
                    self.prepare_native_snapshot(&snapshot, provider_id.clone(), model_id, is_new);
                if let Err(error) = result {
                    let _ = self.shutdown_active();
                    if let Some(snapshot) = previous {
                        self.session.active_state = ActiveState::History(Box::new(snapshot));
                    }
                    self.session.model_selection = previous_selection;
                    return Err(error);
                }
            }
        }
        self.session.model_selection = Some(selection.clone());
        self.session.model_record_key = match selection {
            ModelSelection::Managed { model_record_key } => Some(model_record_key.clone()),
            ModelSelection::Native { .. } | ModelSelection::RuntimeDefault => None,
        };
        Ok(())
    }

    fn prepare_native_snapshot(
        &mut self,
        snapshot: &ConversationSnapshot,
        provider_id: Option<String>,
        model_id: &str,
        is_new: bool,
    ) -> Result<(), RuntimeError> {
        let runtime = self
            .runtime_instance(&snapshot.conversation.runtime_id)?
            .clone();
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用"));
        }
        let context = self.prepare_execution_context(&runtime)?;
        let cwd = snapshot.conversation.cwd.as_deref().map(Path::new);
        if let Some(cwd) = cwd {
            self.validate_runtime_cwd(&runtime, cwd)?;
        }
        velune_agent_runtime::version::check_version_at(
            &runtime.type_id,
            &setting_path(&runtime, "binary")?,
            setting_path_optional(&runtime, "nodeBinary").as_deref(),
            &context.target,
            context.os_home.as_deref(),
        )
        .map_err(|error| RuntimeError::context("运行时版本检查", error))?;
        self.shutdown_active()?;
        self.session.execution_runtime_id = Some(runtime.id.clone());
        if runtime.type_id == "pi-1.0.2" {
            let mut config = self.pi_history_config(&runtime)?;
            config.mcp_servers = self.collaboration_servers();
            config.provider = provider_id;
            config.model = Some(model_id.into());
            config.rpc_entry = Some(context.resources.join("pi_rpc.mjs"));
            config.extension = Some(context.resources.join("pi_virtual_model.mjs"));
            self.session.pi.config = Some(config);
            self.session.active_state = ActiveState::Pi;
            let native_id = (!is_new)
                .then(|| {
                    snapshot
                        .conversation
                        .id
                        .strip_prefix(&format!("{}:", runtime.id))
                        .ok_or_else(|| RuntimeError::invalid("conversation id"))
                })
                .transpose()?;
            self.start_pi_for_session(cwd, native_id.map(Path::new), None, None, false)?;
            if is_new {
                self.session
                    .pi
                    .client
                    .as_mut()
                    .expect("started Pi")
                    .request(json!({"type":"new_session"}))
                    .map_err(|error| RuntimeError::context("会话创建", error))?;
            } else {
                self.session
                    .pi
                    .projection
                    .as_mut()
                    .expect("started Pi")
                    .set_conversation(snapshot.conversation.clone());
            }
            self.session.pi.client.as_mut().expect("started Pi").request(json!({"type":"set_model","provider":self.session.pi.config.as_ref().and_then(|config| config.provider.as_ref()),"modelId":model_id}))
                .map_err(|error| RuntimeError::context("原生模型选择", error))?;
            self.sync_projection()?;
        } else {
            let mut config = self.native_config(&runtime, None)?;
            config.mcp_servers = self.collaboration_servers();
            let mut session = NativeSession::connect(config)
                .map_err(|error| RuntimeError::context("原生运行时连接", error))?;
            session
                .select_native_model(provider_id, model_id.into())
                .map_err(|error| RuntimeError::context("原生模型选择", error))?;
            if is_new {
                session
                    .create(cwd, &runtime.id)
                    .map_err(|error| RuntimeError::context("原生会话创建", error))?;
            } else {
                let native_id = snapshot
                    .conversation
                    .id
                    .strip_prefix(&format!("{}:", runtime.id))
                    .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
                session
                    .open(native_id, cwd, &runtime.id, snapshot.messages.clone())
                    .map_err(|error| RuntimeError::context("原生会话恢复", error))?;
                session.set_conversation(snapshot.conversation.clone());
            }
            self.session.active_state = ActiveState::Native(Box::new(session));
        }
        Ok(())
    }

    pub(super) fn native_config(
        &self,
        runtime: &RuntimeInstance,
        gateway: Option<GatewayInjection>,
    ) -> Result<NativeConfig, RuntimeError> {
        let context = self.prepare_execution_context(runtime)?;
        let kind = match runtime.type_id.as_str() {
            "codex-0.159.3" => NativeKind::Codex,
            "dsh-acp-0.2.0-rc.2" => NativeKind::DeepSeek,
            "claude-code-2.1.286" => NativeKind::ClaudeCode,
            _ => return Err(RuntimeError::invalid("native runtime type")),
        };
        Ok(NativeConfig {
            kind,
            binary: setting_path(runtime, "binary")?,
            node_binary: setting_path_optional(runtime, "nodeBinary"),
            agent_dir: context.agent_dir,
            resources_directory: context.resources,
            projection_directory: context.projection,
            execution: context.target,
            os_home: context.os_home,
            gateway,
            mcp_servers: Vec::new(),
        })
    }

    pub(super) fn runtime_models_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        let id = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        let runtime = self.runtime_instance(id)?.clone();
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用"));
        }
        if runtime.type_id == "acp-v1" {
            return if let ActiveState::Acp(session) = &self.session.active_state
                && self.session.execution_runtime_id.as_deref() == Some(&runtime.id)
                && session.snapshot().is_some()
            {
                let models = session
                    .models()
                    .map_err(|error| RuntimeError::context("ACP 模型目录", error))?;
                Ok(json!(
                    models
                        .into_iter()
                        .map(|model| RuntimeModel {
                            selection: ModelSelection::Native {
                                provider_id: None,
                                model_id: model.id
                            },
                            name: model.name,
                        })
                        .collect::<Vec<_>>()
                ))
            } else {
                Err(RuntimeError::Unsupported(
                    "ACP 模型目录属于会话，请先新建或打开该实例的会话".into(),
                ))
            };
        }
        let mut models = Vec::new();
        if runtime.type_id == "pi-1.0.2" {
            let context = self.prepare_execution_context(&runtime)?;
            let mut config = self.pi_history_config(&runtime)?;
            config.rpc_entry = Some(context.resources.join("pi_rpc.mjs"));
            config.extension = Some(context.resources.join("pi_virtual_model.mjs"));
            let mut client = pi::Client::spawn(config)
                .map_err(|error| RuntimeError::context("Pi 模型目录", error))?;
            let result = client.models();
            let shutdown = client.shutdown();
            let native = result.map_err(|error| RuntimeError::context("Pi 模型目录", error))?;
            shutdown.map_err(|error| RuntimeError::context("Pi 模型目录连接关闭", error))?;
            models.extend(native.into_iter().map(|model| RuntimeModel {
                selection: ModelSelection::Native {
                    provider_id: model.provider_id,
                    model_id: model.id,
                },
                name: model.name,
            }));
        } else {
            let mut session = NativeSession::connect(self.native_config(&runtime, None)?)
                .map_err(|error| RuntimeError::context("原生模型目录连接", error))?;
            let result = session.models();
            let shutdown = session.shutdown();
            let native = result.map_err(|error| RuntimeError::context("原生模型目录", error))?;
            shutdown.map_err(|error| RuntimeError::context("原生模型目录连接关闭", error))?;
            models.extend(native.into_iter().map(|model| RuntimeModel {
                selection: ModelSelection::Native {
                    provider_id: model.provider_id,
                    model_id: model.id,
                },
                name: model.name,
            }));
        }
        if let Some(gateway) = self
            .gateways
            .iter()
            .find(|gateway| gateway.id == runtime.gateway_id)
        {
            models.extend(
                gateway
                    .providers
                    .iter()
                    .flat_map(|provider| &provider.models)
                    .map(|model| RuntimeModel {
                        selection: ModelSelection::Managed {
                            model_record_key: model.record_key.clone(),
                        },
                        name: if model.nickname.is_empty() {
                            model.provider_model_id.clone()
                        } else {
                            model.nickname.clone()
                        },
                    }),
            );
        }
        Ok(serde_json::to_value(models)?)
    }
}
