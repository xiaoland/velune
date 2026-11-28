//! Model provenance stays explicit across discovery, preparation, and continuation.
use super::*;
use crate::conversation::ConversationSnapshot;
use crate::{ModelSelection, RuntimeModel};
use velune_agent_runtime::creation::{DataRetention, Retained};
use velune_agent_runtime::native::CLAUDE_EFFORT_LEVELS;
use velune_agent_runtime::native::codex_projects::NativeProject;

pub(super) struct PiBindingAliases {
    config: GatewayConfig,
    model_keys: std::collections::HashMap<String, String>,
}

#[cfg(test)]
mod edit_model_tests {
    use super::*;
    use crate::conversation::TurnSettings;

    #[test]
    fn managed_edit_model_keeps_identity_after_history_is_named() {
        let root =
            std::env::temp_dir().join(format!("velune-edit-model-{}", crate::new_record_key()));
        std::fs::create_dir_all(&root).unwrap();
        let mut app = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        let core = &mut app.runtime;
        core.runtime_instances = vec![RuntimeInstance {
            enabled: true,
            id: "pi".into(),
            name: "Synthetic Pi".into(),
            type_id: RuntimeKind::Pi,
            gateway_id: "synthetic".into(),
            settings: Default::default(),
        }];
        core.gateways = vec![GatewayConfig {
            id: "synthetic".into(),
            name: "Synthetic".into(),
            routes: Vec::new(),
            providers: vec![crate::config::ProviderDefinition {
                id: "provider".into(),
                name: "ChatGPT1".into(),
                protocol: Protocol::Responses,
                endpoint: "http://127.0.0.1:1/v1".into(),
                execution_backend: crate::config::ExecutionBackend::Http,
                request_profile: crate::config::RequestProfile::Native,
                codex_client_version: None,
                quota_source: crate::config::ProviderQuotaSource::None,
                authentication: Default::default(),
                quota_authentication: Default::default(),
                billing: Default::default(),
                quota_limits: Vec::new(),
                models: vec![crate::config::ProviderModel {
                    record_key: "managed-model".into(),
                    provider_model_id: "gpt-6.1-sol".into(),
                    nickname: "GPT-6.1-Sol".into(),
                    icon: None,
                    context_window: None,
                    max_output_tokens: None,
                    reasoning_levels: None,
                    request_parameters: None,
                    pi_projection: None,
                }],
            }],
        }];
        let alias = core.gateways[0].pi_binding_id("managed-model", 0).unwrap();
        let mut turns = vec![crate::conversation::TranscriptTurn {
            id: "turn".into(),
            user_message_id: "user".into(),
            work_message_ids: vec![],
            last_message_id: Some("reply".into()),
            reply_start_block: 0,
            duration_ms: None,
            is_running: false,
            can_fork: false,
            outcome: Some(crate::conversation::TurnOutcome::Completed),
            usage: None,
            settings: Some(TurnSettings {
                runtime_id: Some("pi".into()),
                model: Some(alias),
                ..Default::default()
            }),
        }];
        core.name_turn_models(&mut turns);
        let settings = turns[0].settings.as_ref().unwrap();
        assert_eq!(settings.model.as_deref(), Some("GPT-6.1-Sol（ChatGPT1）"));
        assert_eq!(
            settings.model_selection,
            Some(ModelSelection::Managed {
                model_record_key: "managed-model".into()
            }),
            "a display label must not erase the managed identity needed by the editor"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn name_turn_models_caches_pi_aliases_skips_non_aliases_and_invalidates_on_gateway_change() {
        let root =
            std::env::temp_dir().join(format!("velune-name-models-{}", crate::new_record_key()));
        std::fs::create_dir_all(&root).unwrap();
        let mut app = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        let core = &mut app.runtime;
        core.runtime_instances = vec![RuntimeInstance {
            enabled: true,
            id: "pi".into(),
            name: "Synthetic Pi".into(),
            type_id: RuntimeKind::Pi,
            gateway_id: "synthetic".into(),
            settings: Default::default(),
        }];
        core.gateways = vec![GatewayConfig {
            id: "synthetic".into(),
            name: "Synthetic".into(),
            routes: Vec::new(),
            providers: vec![
                crate::config::ProviderDefinition {
                    id: "openai".into(),
                    name: "ChatGPT1".into(),
                    protocol: Protocol::Responses,
                    endpoint: "http://127.0.0.1:1/v1".into(),
                    execution_backend: crate::config::ExecutionBackend::Http,
                    request_profile: crate::config::RequestProfile::Native,
                    codex_client_version: None,
                    quota_source: crate::config::ProviderQuotaSource::None,
                    authentication: Default::default(),
                    quota_authentication: Default::default(),
                    billing: Default::default(),
                    quota_limits: Vec::new(),
                    models: vec![crate::config::ProviderModel {
                        record_key: "openai-model".into(),
                        provider_model_id: "gpt-6.2-sol".into(),
                        nickname: "GPT-6.2 Sol".into(),
                        icon: None,
                        context_window: None,
                        max_output_tokens: None,
                        reasoning_levels: None,
                        request_parameters: None,
                        pi_projection: None,
                    }],
                },
                crate::config::ProviderDefinition {
                    id: "gemini".into(),
                    name: "Google".into(),
                    protocol: Protocol::Gemini,
                    endpoint: "http://127.0.0.1:1/v1beta".into(),
                    execution_backend: crate::config::ExecutionBackend::Http,
                    request_profile: crate::config::RequestProfile::Native,
                    codex_client_version: None,
                    quota_source: crate::config::ProviderQuotaSource::None,
                    authentication: Default::default(),
                    quota_authentication: Default::default(),
                    billing: Default::default(),
                    quota_limits: Vec::new(),
                    models: vec![crate::config::ProviderModel {
                        record_key: "gemini-model".into(),
                        provider_model_id: "gemini-flash-latest".into(),
                        nickname: "Gemini Flash".into(),
                        icon: None,
                        context_window: None,
                        max_output_tokens: None,
                        reasoning_levels: None,
                        request_parameters: None,
                        pi_projection: None,
                    }],
                },
            ],
        }];
        let openai_alias = core.gateways[0].pi_binding_id("openai-model", 0).unwrap();
        let gemini_alias = core.gateways[0].pi_binding_id("gemini-model", 0).unwrap();
        assert!(openai_alias.starts_with("vln_"));
        assert!(gemini_alias.contains("--vln_"));

        let make_turn = |id: &str, model: &str| crate::conversation::TranscriptTurn {
            id: id.into(),
            user_message_id: format!("user-{id}"),
            work_message_ids: vec![],
            last_message_id: Some(format!("reply-{id}")),
            reply_start_block: 0,
            duration_ms: None,
            is_running: false,
            can_fork: false,
            outcome: Some(crate::conversation::TurnOutcome::Completed),
            usage: None,
            settings: Some(TurnSettings {
                runtime_id: Some("pi".into()),
                model: Some(model.into()),
                ..Default::default()
            }),
        };

        // Non-alias turns do not populate the gateway cache.
        let mut non_alias_turns = vec![make_turn("t0", "claude-sonnet-4")];
        core.name_turn_models(&mut non_alias_turns);
        assert!(core.pi_binding_aliases.borrow().is_empty());
        assert_eq!(
            non_alias_turns[0]
                .settings
                .as_ref()
                .unwrap()
                .model
                .as_deref(),
            Some("claude-sonnet-4")
        );

        // Both `vln_` and `--vln_` aliases resolve and populate the cache.
        let mut turns = vec![
            make_turn("t1", &openai_alias),
            make_turn("t2", &gemini_alias),
        ];
        core.name_turn_models(&mut turns);
        assert_eq!(core.pi_binding_aliases.borrow().len(), 1);
        assert_eq!(
            turns[0].settings.as_ref().unwrap().model.as_deref(),
            Some("GPT-6.2 Sol（ChatGPT1）")
        );
        assert_eq!(
            turns[1].settings.as_ref().unwrap().model.as_deref(),
            Some("Gemini Flash（Google）")
        );

        // Changing provider auth generation changes the alias and invalidates the cache.
        core.gateways[0].providers[0].authentication.generation = 2;
        let updated_alias = core.gateways[0].pi_binding_id("openai-model", 2).unwrap();
        assert_ne!(updated_alias, openai_alias);
        let mut stale_and_updated = vec![
            make_turn("t3", &openai_alias),
            make_turn("t4", &updated_alias),
        ];
        core.name_turn_models(&mut stale_and_updated);
        assert_eq!(
            stale_and_updated[0]
                .settings
                .as_ref()
                .unwrap()
                .model
                .as_deref(),
            Some(openai_alias.as_str())
        );
        assert_eq!(
            stale_and_updated[1]
                .settings
                .as_ref()
                .unwrap()
                .model
                .as_deref(),
            Some("GPT-6.2 Sol（ChatGPT1）")
        );

        let _ = std::fs::remove_dir_all(root);
    }
}

/// How this execution acquires history. A fork connects with the target's
/// configuration but must never resume or close the source writer.
#[derive(Clone, Copy)]
pub(super) enum ConversationStart<'a> {
    New,
    Resume,
    CodexFork(&'a str),
}

impl CoreRuntime {
    /// Reasoning levels a turn may request for a managed model on this
    /// runtime: those the model declares, in the vocabulary the harness's own
    /// channel accepts. Empty when the harness has no such channel.
    pub(crate) fn runtime_reasoning_efforts(
        &self,
        runtime_id: String,
        model_record_key: String,
    ) -> Result<Vec<String>, RuntimeError> {
        self.reasoning_efforts(self.runtime_instance(&runtime_id)?, &model_record_key)
    }

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
        Ok(match runtime.type_id {
            // Codex passes the level on as the Responses reasoning effort.
            RuntimeKind::Codex166 => declared.to_vec(),
            RuntimeKind::ClaudeCode => declared
                .iter()
                .filter(|level| CLAUDE_EFFORT_LEVELS.contains(&level.as_str()))
                .cloned()
                .collect(),
            RuntimeKind::Pi => pi_declared_levels(model)
                .into_iter()
                .filter(|level| pi::PI_THINKING_LEVELS.contains(&level.as_str()))
                .collect(),
            RuntimeKind::DeepSeek | RuntimeKind::Acp => Vec::new(),
        })
    }

    /// Explicit choices must validate. Reuse an omitted choice from the latest
    /// turn on this runtime only while the selected model and runtime accept it.
    pub(super) fn turn_options(
        &self,
        effort: Option<&str>,
        permission_mode: Option<&str>,
    ) -> Result<(Option<String>, Option<String>), RuntimeError> {
        let runtime = self.active_instance()?;
        let selection = self
            .session
            .model_selection
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid("请选择此会话的模型"))?;
        let turns = (effort.is_none() || permission_mode.is_none())
            .then(|| self.session.transcript_projection.turns());
        let previous = turns.as_deref().and_then(|turns| {
            velune_application_api::latest_runtime_turn_settings(turns, &runtime.id)
        });
        let effort = match effort {
            Some(value) => {
                self.validate_effort(runtime, selection.as_ref(), value)?;
                Some(value.into())
            }
            None => previous
                .and_then(|s| s.effort.as_ref())
                .filter(|value| {
                    self.validate_effort(runtime, selection.as_ref(), value)
                        .is_ok()
                })
                .cloned(),
        };
        let permission_mode = match permission_mode {
            Some(value) => {
                self.validate_permission_mode(runtime, value)?;
                Some(value.into())
            }
            None => previous
                .and_then(|s| s.permission_mode.as_ref())
                .filter(|value| self.validate_permission_mode(runtime, value).is_ok())
                .cloned(),
        };
        Ok((effort, permission_mode))
    }

    /// A mode the runtime version declares; runtimes without modes take none.
    pub(super) fn validate_permission_mode(
        &self,
        runtime: &RuntimeInstance,
        mode: &str,
    ) -> Result<(), RuntimeError> {
        velune_agent_runtime::version::variant(runtime.type_id)
            .permission_modes
            .iter()
            .any(|known| known.id == mode)
            .then_some(())
            .ok_or_else(|| RuntimeError::invalid("此运行时不提供该权限模式"))
    }

    /// A managed model takes the levels it declares; the runtime's own
    /// models, and its default, any level the harness's channel accepts.
    pub(super) fn validate_effort(
        &self,
        runtime: &RuntimeInstance,
        selection: Option<&ModelSelection>,
        effort: &str,
    ) -> Result<(), RuntimeError> {
        let accepted = match selection {
            Some(ModelSelection::Managed { model_record_key }) => self
                .reasoning_efforts(runtime, model_record_key)?
                .iter()
                .any(|level| level == effort),
            Some(ModelSelection::Native { .. }) | None => match runtime.type_id {
                RuntimeKind::Codex166 => {
                    !effort.trim().is_empty() && !effort.chars().any(char::is_control)
                }
                RuntimeKind::ClaudeCode => CLAUDE_EFFORT_LEVELS.contains(&effort),
                RuntimeKind::Pi => pi::PI_THINKING_LEVELS.contains(&effort),
                RuntimeKind::DeepSeek | RuntimeKind::Acp => false,
            },
        };
        if !accepted {
            return Err(RuntimeError::invalid("所选模型或运行时不支持此推理强度"));
        }
        Ok(())
    }

    /// No selection leaves the model to the runtime, which every runtime type
    /// has a default for.
    pub(super) fn validate_model_selection(
        &self,
        runtime: &RuntimeInstance,
        selection: Option<&ModelSelection>,
    ) -> Result<(), RuntimeError> {
        let Some(selection) = selection else {
            return Ok(());
        };
        if runtime.type_id == RuntimeKind::Acp {
            return match selection {
                ModelSelection::Native { provider_id: None, model_id } if !model_id.is_empty() && !model_id.chars().any(char::is_control) => Ok(()),
                _ => Err(RuntimeError::Unsupported("通用 ACP 只接受 Agent 默认模型或其会话中的原生模型选项，不支持 managed provider 注入".into())),
            };
        }
        match selection {
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
                if matches!(runtime.type_id, RuntimeKind::Pi | RuntimeKind::DeepSeek)
                    && provider_id.is_none()
                {
                    return Err(RuntimeError::invalid("此运行时的原生模型必须指定提供商"));
                }
                if runtime.type_id == RuntimeKind::ClaudeCode && provider_id.is_some() {
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
        snapshot: impl Into<browsing::HistorySnapshot>,
        selection: Option<&ModelSelection>,
        is_new: bool,
    ) -> Result<(), RuntimeError> {
        self.prepare_with_project(snapshot, selection, is_new, None)
    }
    pub(super) fn prepare_with_project(
        &mut self,
        snapshot: impl Into<browsing::HistorySnapshot>,
        selection: Option<&ModelSelection>,
        is_new: bool,
        project: Option<&Retained<NativeProject>>,
    ) -> Result<(), RuntimeError> {
        let start = if is_new {
            ConversationStart::New
        } else {
            ConversationStart::Resume
        };
        self.prepare_with_start(snapshot, selection, start, project)
    }

    pub(super) fn prepare_codex_fork(
        &mut self,
        source: impl Into<browsing::HistorySnapshot>,
        selection: Option<&ModelSelection>,
        through_turn: &str,
    ) -> Result<(), RuntimeError> {
        self.prepare_with_start(
            source,
            selection,
            ConversationStart::CodexFork(through_turn),
            None,
        )
    }

    fn prepare_with_start(
        &mut self,
        snapshot: impl Into<browsing::HistorySnapshot>,
        selection: Option<&ModelSelection>,
        start: ConversationStart<'_>,
        project: Option<&Retained<NativeProject>>,
    ) -> Result<(), RuntimeError> {
        let snapshot = snapshot.into().into_retained();
        let is_new = matches!(start, ConversationStart::New);
        self.ensure_work_ended()?;
        let conversation_id = snapshot.conversation.id.clone();
        if self
            .runtime_instance(&snapshot.conversation.runtime_id)?
            .type_id
            == RuntimeKind::Acp
        {
            let owner = self.telemetry.clone();
            telemetry::observe(owner.as_ref(), "harness.prepare", || {
                self.prepare_acp_snapshot(&snapshot, selection, is_new)
            })?;
            self.session.model_selection = Some(selection.cloned());
            self.session.model_record_key = None;
            self.session.pending_native_fork = None;
            return Ok(());
        }
        if let Some(ModelSelection::Managed { model_record_key }) = selection {
            self.prepare_snapshot(snapshot, model_record_key, start, project)?;
        } else {
            let native = match selection {
                Some(ModelSelection::Native {
                    provider_id,
                    model_id,
                }) => Some((provider_id.clone(), model_id.as_str())),
                _ => None,
            };
            let previous = self.owned_history_for_release()?;
            let previous_selection = self.session.model_selection.clone();
            let result = self.prepare_native_snapshot(&snapshot, native, start, project);
            if let Err(error) = result {
                let _ = self.shutdown_active();
                if let Some(snapshot) = previous {
                    self.session.set_history(snapshot);
                }
                self.session.model_selection = previous_selection;
                if matches!(&error, RuntimeError::Contract(failure) if failure.code == "control_held_elsewhere")
                {
                    self.control_held_elsewhere();
                }
                return Err(error);
            }
        }
        self.session.mark_native_project(project.is_some());
        self.session.model_selection = Some(selection.cloned());
        self.session.model_record_key = match selection {
            Some(ModelSelection::Managed { model_record_key }) => Some(model_record_key.clone()),
            _ => None,
        };
        if is_new
            || self
                .session
                .pending_native_fork
                .as_ref()
                .is_some_and(|pending| pending.conversation_id() != conversation_id)
        {
            self.session.pending_native_fork = None;
        }
        if let Some(pending) = &self.session.pending_native_fork
            && pending.conversation_id() == conversation_id
            && let ActiveState::Native(session) = &mut self.session.active_state
        {
            session
                .restore_pending_fork(pending)
                .map_err(|error| RuntimeError::context("恢复待创建的会话分支", error))?;
        }
        Ok(())
    }

    /// Prepares the runtime on its own provider: with `native`'s provider
    /// and model, or else with its own default.
    fn prepare_native_snapshot(
        &mut self,
        snapshot: &Retained<ConversationSnapshot>,
        native: Option<(Option<String>, &str)>,
        start: ConversationStart<'_>,
        project: Option<&Retained<NativeProject>>,
    ) -> Result<(), RuntimeError> {
        let is_new = matches!(start, ConversationStart::New);
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
        let binary = setting_path(&runtime, "binary")?;
        let node = setting_path_optional(&runtime, "nodeBinary");
        telemetry::observe(self.telemetry.as_ref(), "harness.version", || {
            velune_agent_runtime::version::check_version_at(
                runtime.type_id,
                &binary,
                node.as_deref(),
                &context.target,
                context.os_home.as_deref(),
            )
        })
        .map_err(|error| RuntimeError::context("运行时版本检查", error))?;
        self.shutdown_active()?;
        if runtime.type_id == RuntimeKind::Pi {
            let servers = self.collaboration_servers();
            let mut config = self.pi_history_config(&runtime)?;
            config.mcp_servers = servers;
            config.model_compaction_reserves =
                pi_composition::pi_compaction_reserves(&runtime, None)?;
            config.provider = native.as_ref().and_then(|(provider, _)| provider.clone());
            config.model = native.as_ref().map(|(_, model)| (*model).to_owned());
            config.rpc_entry = Some(context.resources.join("pi_rpc.mjs"));
            config.extension = Some(context.resources.join("pi_virtual_model.mjs"));
            self.session.install_execution(
                ActiveState::Pi(Box::new(pi::PiSession::new(config))),
                execution::SessionExecution::native(runtime),
            );
            self.start_pi_for_session(cwd, (!is_new).then_some(snapshot), None)?;
            let side = self.session.side.is_some();
            let session = self.session.pi()?;
            if side {
                session.restore_context(snapshot);
            }
            if let Some((_, model_id)) = native {
                let provider = session.config().provider.clone();
                session
                    .set_model(provider.as_deref(), model_id)
                    .map_err(|error| RuntimeError::context("原生模型选择", error))?;
            }
            self.sync_projection()?;
        } else {
            let servers = self.collaboration_servers();
            let mut config = self.native_config(&runtime, None)?;
            let uses_default_model = native.is_none();
            let auto_compaction = native
                .as_ref()
                .map(|(provider_id, model_id)| {
                    runtime.model_auto_compaction(&ModelSelection::Native {
                        provider_id: provider_id.clone(),
                        model_id: (*model_id).to_owned(),
                    })
                })
                .transpose()?
                .flatten();
            if runtime.type_id != RuntimeKind::DeepSeek {
                config.auto_compaction = auto_compaction.clone();
            }
            config.mcp_servers = servers;
            let mut session = telemetry::observe(self.telemetry.as_ref(), "harness.start", || {
                NativeSession::connect(config)
            })
            .map_err(|error| RuntimeError::context("原生运行时连接", error))?;
            if runtime.type_id == RuntimeKind::Codex166 && uses_default_model {
                session.set_default_model_auto_compactions(runtime.model_auto_compactions()?);
            }
            if let Some((provider_id, model_id)) = native {
                session
                    .select_native_model(Retained::from_parts(
                        (provider_id, model_id.to_owned()),
                        DataRetention::default(),
                    ))
                    .map_err(|error| RuntimeError::context("原生模型选择", error))?;
            }
            if let ConversationStart::CodexFork(through_turn) = start {
                let source_id = snapshot
                    .conversation
                    .id
                    .strip_prefix(&format!("{}:", runtime.id))
                    .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
                session
                    .fork_codex_from(source_id, cwd, &runtime.id, through_turn)
                    .map_err(super::native_composition::fork_failure)?;
            } else if is_new {
                if let Some(side) = &self.session.side {
                    session
                        .create_side_context(cwd, &runtime.id, &side.context)
                        .map_err(|error| RuntimeError::context("侧聊上下文创建", error))?;
                } else {
                    if let Some(context) = self.session.reference_context.take() {
                        session
                            .reference_context(context)
                            .map_err(|error| RuntimeError::context("编辑引用上下文", error))?;
                    }
                    match project {
                        Some(project) => session.create_in_project(cwd, &runtime.id, project),
                        None => session.create(cwd, &runtime.id),
                    }
                    .map_err(|error| RuntimeError::context("原生会话创建", error))?;
                }
            } else {
                let native_id = snapshot
                    .conversation
                    .id
                    .strip_prefix(&format!("{}:", runtime.id))
                    .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
                if self.session.side.is_some() {
                    session
                        .open_side(native_id, cwd, &runtime.id)
                        .map_err(|error| RuntimeError::context("侧聊打开", error))?;
                } else {
                    let history = snapshot
                        .try_clone()
                        .map_err(|error| RuntimeError::context("会话历史保留额度", error))?
                        .map(|snapshot| snapshot.messages);
                    telemetry::observe(self.telemetry.as_ref(), "harness.resume", || {
                        session.open(native_id, cwd, &runtime.id, history)
                    })
                    .map_err(|error| RuntimeError::context("原生会话恢复", error))?;
                    session.restore_context(snapshot).map_err(|error| {
                        RuntimeError::context("native context retained credit", error)
                    })?;
                }
            }
            if runtime.type_id == RuntimeKind::DeepSeek {
                // The ACP catalog resolves the provider/model on open or create.
                // Launch its overlay on the selected model's first turn.
                let auto_compaction = match session.dsh_current_model() {
                    Some((provider_id, model_id)) if uses_default_model => runtime
                        .model_auto_compaction(&ModelSelection::Native {
                            provider_id: Some(provider_id),
                            model_id,
                        })?,
                    _ => auto_compaction,
                };
                session.set_auto_compaction(auto_compaction);
            }
            self.session.install_execution(
                ActiveState::Native(Box::new(session)),
                execution::SessionExecution::native(runtime),
            );
        }
        Ok(())
    }

    pub(super) fn native_config(
        &self,
        runtime: &RuntimeInstance,
        gateway: Option<GatewayInjection>,
    ) -> Result<NativeConfig, RuntimeError> {
        let context = self.prepare_execution_context(runtime)?;
        let kind = NativeKind::of(runtime.type_id)
            .ok_or_else(|| RuntimeError::invalid("native runtime type"))?;
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
            auto_compaction: None,
            mcp_servers: Vec::new(),
            session_agents: Vec::new(),
        })
    }

    pub(crate) fn runtime_models(
        &mut self,
        runtime_id: String,
    ) -> Result<Retained<Vec<RuntimeModel>>, RuntimeError> {
        let runtime = self.runtime_instance(&runtime_id)?.clone();
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用"));
        }
        if runtime.type_id == RuntimeKind::Acp {
            // The native catalog belongs to a live ACP session and is listed
            // only while one runs; the Agent's own default is no selection.
            let mut models = Vec::new();
            if let ActiveState::Acp(session) = &self.session.active_state
                && self.session.execution_runtime_id() == Some(&runtime.id)
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
            return Ok(Retained::from_parts(models, DataRetention::default()));
        }
        let native = if runtime.type_id == RuntimeKind::Pi {
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
            Retained::from_parts(native, DataRetention::default())
        } else {
            let mut session = NativeSession::connect(self.native_config(&runtime, None)?)
                .map_err(|error| RuntimeError::context("原生模型目录连接", error))?;
            let result = session.models();
            let shutdown = session.shutdown();
            let native = result.map_err(|error| RuntimeError::context("原生模型目录", error))?;
            shutdown.map_err(|error| RuntimeError::context("原生模型目录连接关闭", error))?;
            native
        };
        let (native, retention) = native.into_parts();
        let mut models: Vec<_> = native
            .into_iter()
            .map(|option| RuntimeModel {
                selection: ModelSelection::Native {
                    provider_id: option.model.provider_id,
                    model_id: option.model.id,
                },
                name: option.model.name,
                reasoning_efforts: option.reasoning_efforts,
            })
            .collect();
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
                    name: format!("{model_name}（{}）", provider.name),
                    reasoning_efforts: self.reasoning_efforts(&runtime, &model.record_key)?,
                });
            }
            for route in &gateway.routes {
                models.push(RuntimeModel {
                    selection: ModelSelection::Managed {
                        model_record_key: route.record_key.clone(),
                    },
                    name: format!("模型路由：{}", route.name),
                    reasoning_efforts: self.reasoning_efforts(&runtime, &route.record_key)?,
                });
            }
        }
        Ok(Retained::from_parts(models, retention))
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
                    return Some(format!("模型路由：{}", route.name));
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
                    Some(format!("{name}（{}）", provider.name))
                })
            }
            ModelSelection::Native { model_id, .. } => {
                let kind = self.runtime_instance(runtime_id).ok()?.type_id;
                Some(velune_conversation::model_variant::display_model_name(
                    kind, model_id,
                ))
            }
        }
    }

    /// A runtime that records the gateway's route alias as its model (Pi
    /// does) names a managed model; turns show the model's own name instead.
    /// A native variant id (a Codex speed variant) reads as its model name.
    pub(super) fn name_turn_models(&self, turns: &mut [crate::conversation::TranscriptTurn]) {
        for settings in turns.iter_mut().filter_map(|turn| turn.settings.as_mut()) {
            let (Some(runtime_id), Some(model)) = (&settings.runtime_id, &settings.model) else {
                continue;
            };
            if matches!(
                &settings.model_selection,
                Some(ModelSelection::Native { .. })
            ) || settings.model_selection.is_none()
            {
                // An unknown runtime's model id is not ours to reinterpret.
                let Some(kind) = self
                    .runtime_instance(runtime_id)
                    .ok()
                    .map(|runtime| runtime.type_id)
                else {
                    continue;
                };
                let name = velune_conversation::model_variant::display_model_name(kind, model);
                if name != *model {
                    settings.model = Some(name);
                    continue;
                }
            }
            if !model.starts_with("vln_") && !model.contains("--vln_") {
                continue;
            }
            let Some(gateway) = self.runtime_instance(runtime_id).ok().and_then(|runtime| {
                self.gateways
                    .iter()
                    .find(|gateway| gateway.id == runtime.gateway_id)
            }) else {
                continue;
            };
            if let Some(model_record_key) = self.pi_binding_model_key(gateway, model) {
                let selection = ModelSelection::Managed { model_record_key };
                if let Some(name) = self.turn_model_name(runtime_id, &selection) {
                    settings.model = Some(name);
                    // The readable title is presentation; retain the matched
                    // route identity so editing never reconstructs it from text.
                    if settings.model_selection.is_none() {
                        settings.model_selection = Some(selection);
                    }
                }
            }
        }
        for turn in turns.iter_mut() {
            // A harness reports its own guess at a model it does not know;
            // the managed model's configuration describes the actual upstream.
            if let (Some(settings), Some(usage)) = (&turn.settings, turn.usage.as_mut())
                && usage.context_tokens.is_some()
                && let Some(window) =
                    self.managed_context_window(settings, usage.context_window.is_some())
            {
                usage.context_window = Some(window);
            }
        }
    }

    fn pi_binding_model_key(&self, gateway: &GatewayConfig, alias: &str) -> Option<String> {
        let mut cache = self.pi_binding_aliases.borrow_mut();
        let needs_rebuild = cache
            .get(&gateway.id)
            .is_none_or(|cached| &cached.config != gateway);
        if needs_rebuild {
            let mut model_keys = std::collections::HashMap::new();
            for key in gateway.selectable_keys() {
                if let Ok(binding_alias) =
                    gateway.pi_binding_id(key, gateway.authentication_revision(key))
                {
                    model_keys
                        .entry(binding_alias)
                        .or_insert_with(|| key.to_owned());
                }
            }
            cache.insert(
                gateway.id.clone(),
                PiBindingAliases {
                    config: gateway.clone(),
                    model_keys,
                },
            );
        }
        cache
            .get(&gateway.id)
            .and_then(|cached| cached.model_keys.get(alias).cloned())
    }

    /// The window Velune gave the runtime for a turn's managed model: the
    /// model Velune named, or the only one with the provider model ID the
    /// runtime recorded. The runtime's own provider may serve a model under
    /// the same ID, so that match only fills in a window the runtime did not
    /// report; a turn Velune sent with a native model is never managed.
    fn managed_context_window(
        &self,
        settings: &crate::conversation::TurnSettings,
        reported: bool,
    ) -> Option<u64> {
        if matches!(
            settings.model_selection,
            Some(ModelSelection::Native { .. })
        ) {
            return None;
        }
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
            if reported {
                return None;
            }
            let mut by_id = models
                .iter()
                .filter(|candidate| &candidate.provider_model_id == model);
            by_id.next().filter(|_| by_id.next().is_none()).copied()
        })?;
        found.context_window.map(u64::from)
    }
}
