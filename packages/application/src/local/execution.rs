//! On-demand execution preparation and process/gateway lifecycle.
use super::*;
impl CoreRuntime {
    pub(super) fn prepare_runtime(
        &mut self,
        runtime_id: &str,
        model_record_key: &str,
    ) -> Result<(), RuntimeError> {
        let runtime = self
            .runtime_instances
            .iter()
            .find(|item| item.id == runtime_id)
            .cloned()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用，请先启用"));
        }
        let gateway = self
            .gateways
            .iter()
            .find(|item| item.id == runtime.gateway_id)
            .cloned()
            .ok_or_else(|| RuntimeError::invalid("gateway id"))?;
        if runtime.type_id != "pi-1.0.2" {
            self.shutdown_active()?;
            let runner = Runner::start_with_analytics(
                gateway.to_gateway_config()?,
                crate::authentication_resolver::ProviderResolver::capture(&gateway, &self.options),
                self.native_aliases(&runtime, model_record_key)?,
                Some(self.analytics.clone()),
            )
            .map_err(|error| RuntimeError::context("gateway startup", error))?;
            self.gateway_runner = Some(runner);
            let result = (|| {
                let injection = self.native_injection(&runtime, model_record_key)?;
                let mut config = self.native_config(&runtime, Some(injection))?;
                config.mcp_servers = self.collaboration_servers();
                NativeSession::connect(config)
                    .map_err(|error| RuntimeError::context("运行时连接失败", error))
            })();
            match result {
                Ok(session) => {
                    self.active_state = ActiveState::Native(Box::new(session));
                    self.execution_runtime_id = Some(runtime.id);
                    self.model_record_key = None;
                    return Ok(());
                }
                Err(error) => {
                    self.gateway_runner = None;
                    return Err(error);
                }
            }
        }
        runnable_pi_model(&gateway, model_record_key)?;
        let physical_model_id = gateway
            .pi_binding_id(
                model_record_key,
                gateway.authentication_revision(model_record_key),
            )
            .map_err(RuntimeError::invalid)?;
        let route_aliases = gateway
            .providers
            .iter()
            .flat_map(|p| &p.models)
            .map(|route| {
                gateway
                    .pi_binding_id(
                        &route.record_key,
                        gateway.authentication_revision(&route.record_key),
                    )
                    .map(|alias| (alias, route.record_key.clone()))
                    .map_err(RuntimeError::invalid)
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        self.shutdown_active()?;
        let runner = Runner::start_with_analytics(
            gateway.to_gateway_config()?,
            crate::authentication_resolver::ProviderResolver::capture(&gateway, &self.options),
            route_aliases,
            Some(self.analytics.clone()),
        )
        .map_err(|error| RuntimeError::context("gateway startup", error))?;
        let context = self.prepare_execution_context(&runtime)?;
        let tunnel = velune_agent_runtime::execution::LoopbackTunnel::open(
            &context.target,
            runner.endpoint(),
        )
        .map_err(|error| RuntimeError::context("网关 SSH 转发", error))?;
        let projection_dir = context.projection;
        let config = PiConfig {
            mcp_servers: self.collaboration_servers(),
            runtime_type_id: runtime.type_id.clone(),
            binary: setting_path(&runtime, "binary")?,
            node_binary: setting_path_optional(&runtime, "nodeBinary"),
            sdk_helper: setting_path_optional(&runtime, "sdkHelper")
                .or_else(|| Some(context.resources.join("pi_sessions.mjs"))),
            extension: Some(context.resources.join("pi_virtual_model.mjs")),
            agent_dir: Some(context.agent_dir),
            execution: context.target,
            os_home: context.os_home,
            working_dir: None,
            provider: Some("velune".into()),
            model: Some("auto".into()),
            rpc_entry: Some(context.resources.join("pi_rpc.mjs")),
            models_path: Some(projection_dir.join("models.json")),
            selection_file: Some(projection_dir.join("velune-selection.json")),
            gateway_token: Some(runner.token().into()),
            session_dir: setting_path_optional(&runtime, "sessionDir"),
            session: None,
            // A session's Pi name belongs to its persisted session_info entry.
            // Runtime names must not overwrite it when reopening a session.
            name: None,
        };
        config
            .validate_sdk()
            .map_err(|error| RuntimeError::Invalid(error.to_string()))?;
        let protocol = match gateway
            .resolve_dispatch(model_record_key)
            .map_err(RuntimeError::invalid)?
            .provider()
            .protocol
        {
            GatewayProtocol::ChatCompletionsV1 => "openai-completions",
            GatewayProtocol::ResponsesV1 => "openai-responses",
            GatewayProtocol::MessagesV1 => "anthropic-messages",
        };
        if let Err(error) = materialize_models(&config, &gateway, tunnel.endpoint(), protocol) {
            drop(runner);
            return Err(error);
        }
        let subscription_capability = gateway.subscription(model_record_key);
        write_selection_file(
            &config,
            model_record_key,
            &physical_model_id,
            subscription_capability,
        )?;
        self.gateway_runner = Some(runner);
        self.gateway_tunnel = Some(tunnel);
        self.pi.config = Some(config);
        self.active_state = ActiveState::Pi;
        self.execution_runtime_id = Some(runtime_id.into());
        self.pi.physical_model_id = None;
        self.model_record_key = None;
        self.pi.subscription_capability = false;
        Ok(())
    }

    pub(super) fn start_pi_for_session(
        &mut self,
        cwd: Option<&Path>,
        session: Option<&Path>,
        model_record_key: Option<&str>,
        physical_model_id: Option<&str>,
        subscription_capability: bool,
    ) -> Result<(), RuntimeError> {
        let runtime = self
            .execution_runtime_id
            .as_deref()
            .ok_or_else(|| RuntimeError::invalid("Pi 执行运行时不可用"))
            .and_then(|id| self.runtime_instance(id).cloned())?;
        let runtime_context = format!(
            "runtime startup；{}",
            super::browsing::runtime_context(&runtime)
        );
        let runtime_start_error = |phase: &'static str, error| {
            tracing::warn!(
                target: "velune_application",
                event = "runtime_start_failed",
                phase,
                runtime_id = %runtime.id,
                runtime_name = %runtime.name,
                runtime_type_id = %runtime.type_id,
                binary = ?runtime.settings.get("binary"),
                node_binary = ?runtime.settings.get("nodeBinary"),
                agent_dir = ?runtime.settings.get("agentDir"),
                detail = %error,
            );
            RuntimeError::context(&runtime_context, error)
        };
        if let Some(cwd) = cwd {
            self.validate_runtime_cwd(&runtime, cwd)?;
        }
        let cwd = cwd.map(Path::to_owned);
        let mut config = self
            .pi
            .config
            .clone()
            .ok_or_else(|| RuntimeError::invalid("会话执行尚未准备"))?;
        self.shutdown_pi()?;
        config.working_dir = cwd.clone();
        config.session = session.map(Path::to_owned);
        config.name = None;
        if let (Some(logical), Some(physical)) = (model_record_key, physical_model_id) {
            write_selection_file(&config, logical, physical, subscription_capability)?;
        }
        let mut client = pi::Client::spawn(config.clone())
            .map_err(|error| runtime_start_error("startup", error))?;
        // Establish readiness through RPC before exposing bootstrap events to
        // the projection. Pi can emit initialization records before its first
        // command response; those records must not settle a new turn.
        client
            .state()
            .map_err(|error| runtime_start_error("startup", error))?;
        let runtime_id = self.execution_runtime_id.as_deref().unwrap_or_default();
        let mut projection = PiProjection::new(ConversationSummary {
            can_rename: false,
            can_delete: false,
            id: session
                .map(|path| format!("{runtime_id}:{}", path.display()))
                .unwrap_or_else(|| "active".into()),
            title: velune_conversation::ConversationTitle::Untitled,
            updated_at_unix_ms: None,
            created_at_unix_ms: None,
            runtime_id: runtime_id.into(),
            cwd: cwd.as_ref().map(|path| path.to_string_lossy().into_owned()),
        });
        if let Some(snapshot) = projection.snapshot.as_mut() {
            snapshot.model_record_key = model_record_key.map(str::to_owned);
        }
        self.pi.client = Some(client);
        self.pi.config = Some(config);
        self.pi.projection = Some(projection);
        self.model_record_key = model_record_key.map(str::to_owned);
        self.pi.physical_model_id = physical_model_id.map(str::to_owned);
        self.pi.subscription_capability = subscription_capability;
        self.drain_runtime()?;
        Ok(())
    }

    pub(super) fn shutdown_pi(&mut self) -> Result<(), RuntimeError> {
        if let Some(mut client) = self.pi.client.take() {
            client
                .shutdown()
                .map_err(|error| RuntimeError::context("runtime shutdown", error))?;
        }
        self.pi.busy = false;
        self.pi.turn_started = false;
        self.pi.projection = None;
        self.pi.physical_model_id = None;
        self.model_record_key = None;
        self.pi.subscription_capability = false;
        if let Some(config) = self.pi.config.as_mut() {
            config.working_dir = None;
            config.session = None;
        }
        Ok(())
    }

    pub(super) fn shutdown_active(&mut self) -> Result<(), RuntimeError> {
        let state = std::mem::replace(&mut self.active_state, ActiveState::Empty);
        let result = match state {
            ActiveState::Acp(mut session) => session
                .shutdown()
                .map_err(|error| RuntimeError::context("ACP shutdown", error)),
            ActiveState::Native(mut session) => session
                .shutdown()
                .map_err(|error| RuntimeError::context("runtime shutdown", error)),
            ActiveState::Pi => self
                .pi
                .client
                .take()
                .map(|mut client| {
                    client
                        .shutdown()
                        .map_err(|error| RuntimeError::context("runtime shutdown", error))
                })
                .unwrap_or(Ok(())),
            ActiveState::Empty | ActiveState::History(_) => Ok(()),
        };
        self.pi = PiState::default();
        self.gateway_runner = None;
        self.gateway_tunnel = None;
        self.execution_runtime_id = None;
        self.model_record_key = None;
        result
    }
}
