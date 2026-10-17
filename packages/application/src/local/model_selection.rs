//! Model provenance stays explicit across discovery, preparation, and continuation.
use super::*;
use crate::conversation::{ConversationSnapshot, TurnSettings};
use crate::{ModelSelection, RuntimeModel};
use velune_agent_runtime::native::CLAUDE_EFFORT_LEVELS;

pub(super) fn requested_selection(request: &Value) -> Result<ModelSelection, RuntimeError> {
    Ok(serde_json::from_value(
        request["payload"]["modelSelection"].clone(),
    )?)
}

impl CoreRuntime {
    /// Reasoning levels a turn may request for a managed model on this
    /// runtime: those the model declares, in the vocabulary the harness's own
    /// channel accepts. Empty when the harness has no such channel.
    pub(super) fn reasoning_efforts(
        &self,
        runtime: &RuntimeInstance,
        key: &str,
    ) -> Result<Vec<String>, RuntimeError> {
        let model = self
            .gateways
            .iter()
            .find(|gateway| gateway.id == runtime.gateway_id)
            .and_then(|gateway| gateway.resolve_dispatch(key).ok())
            .map(|resolved| resolved.model())
            .ok_or_else(|| RuntimeError::invalid("所选会话模型已不存在，请重新选择"))?;
        let declared = model.reasoning_levels.as_deref().unwrap_or_default();
        Ok(match runtime.type_id.as_str() {
            // Codex passes the level on as the Responses reasoning effort.
            "codex-0.159" => declared.to_vec(),
            "claude-code-2.1" => declared
                .iter()
                .filter(|level| CLAUDE_EFFORT_LEVELS.contains(&level.as_str()))
                .cloned()
                .collect(),
            "pi-1.0" => pi_declared_levels(model)
                .into_iter()
                .filter(|level| pi::PI_THINKING_LEVELS.contains(&level.as_str()))
                .collect(),
            _ => Vec::new(),
        })
    }

    /// The next turn's reasoning level, checked against the runtime and model
    /// the turn runs on: the one asked for, or else the conversation's
    /// current one while the model accepts it. Like a permission mode it is
    /// session state that harnesses keep differently (Claude Code takes one
    /// per launch, Codex resumes in its configured level, Pi keeps its own),
    /// so the current level is sent again where Velune knows it.
    pub(super) fn requested_effort(
        &mut self,
        request: &Value,
    ) -> Result<Option<String>, RuntimeError> {
        let runtime = self.active_instance()?.clone();
        let selection = self
            .session
            .model_selection
            .clone()
            .ok_or_else(|| RuntimeError::invalid("请选择此会话的模型"))?;
        if let Some(effort) = request["payload"]["effort"].as_str() {
            self.validate_effort(&runtime, &selection, effort)?;
            return Ok(Some(effort.into()));
        }
        let current = self
            .current_turn_settings(&runtime)
            .and_then(|settings| settings.effort);
        Ok(current.filter(|effort| self.validate_effort(&runtime, &selection, effort).is_ok()))
    }

    /// Settings of the conversation's last turn when it ran on `runtime`.
    fn current_turn_settings(&mut self, runtime: &RuntimeInstance) -> Option<TurnSettings> {
        self.current_snapshot()
            .and_then(|snapshot| snapshot.transcript_turns.into_iter().last())
            .and_then(|turn| turn.settings)
            .filter(|settings| settings.runtime_id.as_deref() == Some(runtime.id.as_str()))
    }

    /// The turn's permission mode: the one asked for, or else the
    /// conversation's current one. A mode is session state, but a harness
    /// does not always keep it (Codex resumes a thread in its configured
    /// policy), so the current mode is sent again where Velune knows it.
    pub(super) fn requested_permission_mode(
        &mut self,
        request: &Value,
    ) -> Result<Option<String>, RuntimeError> {
        let runtime = self.active_instance()?.clone();
        if let Some(mode) = request["payload"]["permissionMode"].as_str() {
            self.validate_permission_mode(&runtime, mode)?;
            return Ok(Some(mode.into()));
        }
        let current = self
            .current_turn_settings(&runtime)
            .and_then(|settings| settings.permission_mode);
        Ok(current.filter(|mode| self.validate_permission_mode(&runtime, mode).is_ok()))
    }

    /// A mode the runtime version declares; runtimes without modes take none.
    pub(super) fn validate_permission_mode(
        &self,
        runtime: &RuntimeInstance,
        mode: &str,
    ) -> Result<(), RuntimeError> {
        velune_agent_runtime::version::variant(&runtime.type_id)
            .is_some_and(|variant| {
                variant
                    .permission_modes
                    .iter()
                    .any(|known| known.id == mode)
            })
            .then_some(())
            .ok_or_else(|| RuntimeError::invalid("此运行时不提供该权限模式"))
    }

    pub(super) fn validate_effort(
        &self,
        runtime: &RuntimeInstance,
        selection: &ModelSelection,
        effort: &str,
    ) -> Result<(), RuntimeError> {
        let accepted = match selection {
            ModelSelection::Managed { model_record_key } => self
                .reasoning_efforts(runtime, model_record_key)?
                .iter()
                .any(|level| level == effort),
            ModelSelection::Native { .. } => match runtime.type_id.as_str() {
                "codex-0.159" => !effort.trim().is_empty() && !effort.chars().any(char::is_control),
                "claude-code-2.1" => CLAUDE_EFFORT_LEVELS.contains(&effort),
                "pi-1.0" => pi::PI_THINKING_LEVELS.contains(&effort),
                _ => false,
            },
            ModelSelection::RuntimeDefault => false,
        };
        if !accepted {
            return Err(RuntimeError::invalid("所选模型或运行时不支持此推理强度"));
        }
        Ok(())
    }

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
                if matches!(runtime.type_id.as_str(), "pi-1.0" | "dsh-acp-0.2")
                    && provider_id.is_none()
                {
                    return Err(RuntimeError::invalid("此运行时的原生模型必须指定提供商"));
                }
                if runtime.type_id == "claude-code-2.1" && provider_id.is_some() {
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
        self.ensure_work_ended()?;
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
        if runtime.type_id == "pi-1.0" {
            let servers = self.collaboration_servers();
            let mut config = self.pi_history_config(&runtime)?;
            config.mcp_servers = servers;
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
            self.start_pi_for_session(cwd, native_id.map(Path::new), None, None)?;
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
            let servers = self.collaboration_servers();
            let mut config = self.native_config(&runtime, None)?;
            config.mcp_servers = servers;
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
            "codex-0.159" => NativeKind::Codex,
            "dsh-acp-0.2" => NativeKind::DeepSeek,
            "claude-code-2.1" => NativeKind::ClaudeCode,
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
            session_agents: Vec::new(),
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
            // The Agent's own default is always offered; its native catalog
            // belongs to a live ACP session and is listed only while one runs.
            let mut models = vec![RuntimeModel {
                selection: ModelSelection::RuntimeDefault,
                name: "Agent 默认模型".into(),
                reasoning_efforts: Vec::new(),
            }];
            if let ActiveState::Acp(session) = &self.session.active_state
                && self.session.execution_runtime_id.as_deref() == Some(&runtime.id)
                && session.snapshot().is_some()
            {
                let native = session
                    .models()
                    .map_err(|error| RuntimeError::context("ACP 模型目录", error))?;
                models.extend(native.into_iter().map(|option| RuntimeModel {
                    selection: ModelSelection::Native {
                        provider_id: None,
                        model_id: option.model.id,
                    },
                    name: option.model.name,
                    reasoning_efforts: option.reasoning_efforts,
                }));
            }
            return Ok(json!(models));
        }
        let mut models = Vec::new();
        if runtime.type_id == "pi-1.0" {
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
            models.extend(native.into_iter().map(|option| RuntimeModel {
                selection: ModelSelection::Native {
                    provider_id: option.model.provider_id,
                    model_id: option.model.id,
                },
                name: option.model.name,
                reasoning_efforts: option.reasoning_efforts,
            }));
        } else {
            let mut session = NativeSession::connect(self.native_config(&runtime, None)?)
                .map_err(|error| RuntimeError::context("原生模型目录连接", error))?;
            let result = session.models();
            let shutdown = session.shutdown();
            let native = result.map_err(|error| RuntimeError::context("原生模型目录", error))?;
            shutdown.map_err(|error| RuntimeError::context("原生模型目录连接关闭", error))?;
            models.extend(native.into_iter().map(|option| RuntimeModel {
                selection: ModelSelection::Native {
                    provider_id: option.model.provider_id,
                    model_id: option.model.id,
                },
                name: option.model.name,
                reasoning_efforts: option.reasoning_efforts,
            }));
        }
        if let Some(gateway) = self
            .gateways
            .iter()
            .find(|gateway| gateway.id == runtime.gateway_id)
        {
            for (provider, model) in gateway
                .providers
                .iter()
                .flat_map(|provider| provider.models.iter().map(move |model| (provider, model)))
            {
                let model_name = if model.nickname.is_empty() {
                    &model.provider_model_id
                } else {
                    &model.nickname
                };
                models.push(RuntimeModel {
                    selection: ModelSelection::Managed {
                        model_record_key: model.record_key.clone(),
                    },
                    // Managed records may share a model name across providers.
                    name: format!("{model_name} · {}", provider.name),
                    reasoning_efforts: self.reasoning_efforts(&runtime, &model.record_key)?,
                });
            }
            for route in &gateway.routes {
                models.push(RuntimeModel {
                    selection: ModelSelection::Managed {
                        model_record_key: route.record_key.clone(),
                    },
                    name: format!("{} · 模型路由", route.name),
                    reasoning_efforts: self.reasoning_efforts(&runtime, &route.record_key)?,
                });
            }
        }
        Ok(serde_json::to_value(models)?)
    }

    /// How a turn names the model it was sent with: the managed model as the
    /// model list names it, a runtime model by its ID.
    pub(super) fn turn_model_name(
        &self,
        runtime_id: &str,
        selection: &ModelSelection,
    ) -> Option<String> {
        match selection {
            ModelSelection::Managed { model_record_key } => {
                let runtime = self.runtime_instance(runtime_id).ok()?;
                let gateway = self
                    .gateways
                    .iter()
                    .find(|gateway| gateway.id == runtime.gateway_id)?;
                if let Some(route) = gateway.route(model_record_key) {
                    return Some(format!("{} · 模型路由", route.name));
                }
                gateway.providers.iter().find_map(|provider| {
                    let model = provider
                        .models
                        .iter()
                        .find(|model| &model.record_key == model_record_key)?;
                    let name = if model.nickname.is_empty() {
                        &model.provider_model_id
                    } else {
                        &model.nickname
                    };
                    Some(format!("{name} · {}", provider.name))
                })
            }
            ModelSelection::Native { model_id, .. } => Some(model_id.clone()),
            ModelSelection::RuntimeDefault => None,
        }
    }

    /// A runtime that records the gateway's route alias as its model (Pi
    /// does) names a managed model; turns show the model's own name instead.
    pub(super) fn name_turn_models(&self, turns: &mut [crate::conversation::TranscriptTurn]) {
        for settings in turns.iter_mut().filter_map(|turn| turn.settings.as_mut()) {
            let (Some(runtime_id), Some(model)) = (&settings.runtime_id, &settings.model) else {
                continue;
            };
            let Some(gateway) = self.runtime_instance(runtime_id).ok().and_then(|runtime| {
                self.gateways
                    .iter()
                    .find(|gateway| gateway.id == runtime.gateway_id)
            }) else {
                continue;
            };
            let key = gateway
                .selectable_keys()
                .find(|key| {
                    gateway
                        .pi_binding_id(key, gateway.authentication_revision(key))
                        .is_ok_and(|alias| &alias == model)
                })
                .map(str::to_owned);
            if let Some(name) = key.and_then(|model_record_key| {
                self.turn_model_name(runtime_id, &ModelSelection::Managed { model_record_key })
            }) {
                settings.model = Some(name);
            }
        }
        for turn in turns.iter_mut() {
            // A harness reports its own guess at a model it does not know;
            // the managed model's configuration describes the actual upstream.
            if let (Some(settings), Some(usage)) = (&turn.settings, turn.usage.as_mut())
                && usage.context_tokens.is_some()
                && let Some(window) = self.managed_context_window(settings)
            {
                usage.context_window = Some(window);
            }
        }
    }

    /// The window Velune gave the runtime for a turn's managed model: the
    /// model Velune named, or the only one with the provider model ID the
    /// runtime recorded.
    fn managed_context_window(&self, settings: &crate::conversation::TurnSettings) -> Option<u64> {
        let (runtime_id, model) = (settings.runtime_id.as_ref()?, settings.model.as_ref()?);
        let runtime = self.runtime_instance(runtime_id).ok()?;
        let gateway = self
            .gateways
            .iter()
            .find(|gateway| gateway.id == runtime.gateway_id)?;
        let models: Vec<_> = gateway
            .providers
            .iter()
            .flat_map(|provider| &provider.models)
            .collect();
        let named = models.iter().find(|candidate| {
            self.turn_model_name(
                runtime_id,
                &ModelSelection::Managed {
                    model_record_key: candidate.record_key.clone(),
                },
            )
            .as_ref()
                == Some(model)
        });
        let found = named.copied().or_else(|| {
            let mut by_id = models
                .iter()
                .filter(|candidate| &candidate.provider_model_id == model);
            by_id.next().filter(|_| by_id.next().is_none()).copied()
        })?;
        found.context_window.map(u64::from)
    }
}
