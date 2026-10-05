//! Local application connection use cases.
use super::*;
use sha2::Digest;
impl CoreRuntime {
    pub(super) fn connect_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if request["action"] == "runtimeAction"
            && request["payload"]["actionID"].as_str() != Some("connect")
        {
            return Err(RuntimeError::Unsupported("runtime action".into()));
        }
        if self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let runtime_id = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        self.connect(runtime_id)?;
        Ok(json!({"runtimeInstanceID":self.active_runtime_id,"connections":self.connections()}))
    }

    pub(super) fn connect(&mut self, runtime_id: &str) -> Result<(), RuntimeError> {
        let runtime = self
            .runtime_instances
            .iter()
            .find(|item| item.id == runtime_id)
            .cloned()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        let gateway = self
            .gateways
            .iter()
            .find(|item| item.id == runtime.gateway_id)
            .cloned()
            .ok_or_else(|| RuntimeError::invalid("gateway id"))?;
        let model_record_key = runtime
            .model_record_key
            .as_deref()
            .ok_or_else(|| RuntimeError::invalid("runtime model id"))?;
        runnable_pi_model(&gateway, model_record_key)?;
        let physical_model_id = gateway
            .pi_binding_id(
                model_record_key,
                self.authentication_resources
                    .revision(&gateway, model_record_key),
            )
            .map_err(RuntimeError::invalid)?;
        let route_aliases = gateway
            .routes
            .iter()
            .map(|route| {
                gateway
                    .pi_binding_id(
                        &route.model_record_key,
                        self.authentication_resources
                            .revision(&gateway, &route.model_record_key),
                    )
                    .map(|alias| (alias, route.model_record_key.clone()))
                    .map_err(RuntimeError::invalid)
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        self.shutdown_active()?;
        let runner = Runner::start(
            gateway.to_gateway_config()?,
            crate::authentication_resolver::RegistryResolver::capture(
                &self.authentication_resources,
                &self.options,
            ),
            route_aliases,
        )
        .map_err(|_| RuntimeError::invalid("gateway startup"))?;
        let projection_dir = self
            .options
            .home_directory
            .join("runtime-projections")
            .join(format!("{:x}", sha2::Sha256::digest(runtime.id.as_bytes())));
        let config = PiConfig {
            binary: setting_path(&runtime, "binary")?,
            node_binary: setting_path_optional(&runtime, "nodeBinary"),
            sdk_helper: setting_path_optional(&runtime, "sdkHelper")
                .or_else(|| Some(self.options.resources_directory.join("pi_sessions.mjs"))),
            extension: Some(
                self.options
                    .resources_directory
                    .join("pi_virtual_model.mjs"),
            ),
            agent_dir: Some(setting_path(&runtime, "agentDir")?),
            working_dir: None,
            provider: Some("velune-gateway".into()),
            model: Some("velune/auto".into()),
            rpc_entry: Some(self.options.resources_directory.join("pi_rpc.mjs")),
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
            .validate_dispatch(model_record_key)
            .map_err(RuntimeError::invalid)?
            .protocol
        {
            GatewayProtocol::ChatCompletionsV1 => "openai-completions",
            GatewayProtocol::ResponsesV1 => "openai-responses",
            GatewayProtocol::MessagesV1 => return Err(RuntimeError::invalid("provider protocol")),
        };
        if let Err(error) = materialize_models(
            &config,
            &gateway,
            runner.endpoint(),
            protocol,
            &self.authentication_resources,
        ) {
            drop(runner);
            return Err(error);
        }
        let subscription_capability = self
            .authentication_resources
            .subscription(&gateway, model_record_key);
        write_selection_file(
            &config,
            model_record_key,
            &physical_model_id,
            subscription_capability,
        )?;
        self.gateway_runner = Some(runner);
        self.pi_config = Some(config);
        self.active_runtime_id = Some(runtime_id.into());
        self.physical_model_id = None;
        self.model_record_key = None;
        self.subscription_capability = false;
        Ok(())
    }

    pub(super) fn start_pi_for_session(
        &mut self,
        cwd: &Path,
        session: Option<&Path>,
        model_record_key: Option<&str>,
        physical_model_id: Option<&str>,
        subscription_capability: bool,
    ) -> Result<(), RuntimeError> {
        validate_session_cwd(cwd)?;
        let cwd = fs::canonicalize(cwd)
            .map_err(|_| RuntimeError::invalid("conversation working directory"))?;
        let mut config = self
            .pi_config
            .clone()
            .ok_or_else(|| RuntimeError::invalid("runtime is not connected"))?;
        self.shutdown_pi()?;
        config.working_dir = Some(cwd.clone());
        config.session = session.map(Path::to_owned);
        config.name = None;
        if let (Some(logical), Some(physical)) = (model_record_key, physical_model_id) {
            write_selection_file(&config, logical, physical, subscription_capability)?;
        }
        let mut client = pi::Client::spawn(config.clone())
            .map_err(|_| RuntimeError::invalid("runtime startup"))?;
        // Establish readiness through RPC before exposing bootstrap events to
        // the projection. Pi can emit initialization records before its first
        // command response; those records must not settle a new turn.
        client
            .state()
            .map_err(|_| RuntimeError::invalid("runtime startup"))?;
        let runtime_id = self.active_runtime_id.as_deref().unwrap_or_default();
        let mut projection = PiProjection::new(ConversationSummary {
            id: session
                .map(|path| format!("{runtime_id}:{}", path.display()))
                .unwrap_or_else(|| "active".into()),
            title: session
                .and_then(|path| path.file_stem())
                .and_then(|item| item.to_str())
                .unwrap_or("当前会话")
                .into(),
            updated_at: None,
            runtime_id: runtime_id.into(),
            cwd: Some(cwd.to_string_lossy().into_owned()),
        });
        if let Some(snapshot) = projection.snapshot.as_mut() {
            snapshot.model_record_key = model_record_key.map(str::to_owned);
        }
        self.pi = Some(client);
        self.pi_config = Some(config);
        self.projection = Some(projection);
        self.model_record_key = model_record_key.map(str::to_owned);
        self.physical_model_id = physical_model_id.map(str::to_owned);
        self.subscription_capability = subscription_capability;
        self.drain_pi();
        Ok(())
    }

    pub(super) fn shutdown_pi(&mut self) -> Result<(), RuntimeError> {
        if let Some(mut client) = self.pi.take() {
            client
                .shutdown()
                .map_err(|_| RuntimeError::invalid("runtime shutdown"))?;
        }
        self.pi_busy = false;
        self.pi_turn_started = false;
        self.projection = None;
        self.physical_model_id = None;
        self.model_record_key = None;
        self.subscription_capability = false;
        if let Some(config) = self.pi_config.as_mut() {
            config.working_dir = None;
            config.session = None;
        }
        Ok(())
    }

    pub(super) fn connections(&self) -> Vec<Value> {
        self.active_runtime_id.as_ref()
            .map(|id| {
                let name = self.runtime_instances.iter().find(|runtime| &runtime.id == id)
                    .map(|runtime| runtime.name.as_str()).expect("active runtime instance is configured");
                vec![json!({"id":id,"name":name,"state":"connected","capabilities":["chat","cancel"]})]
            })
            .unwrap_or_default()
    }

    pub(super) fn shutdown_active(&mut self) -> Result<(), RuntimeError> {
        if let Some(mut client) = self.pi.take() {
            client
                .shutdown()
                .map_err(|_| RuntimeError::invalid("runtime shutdown"))?;
        }
        self.pi_config = None;
        self.pi_busy = false;
        self.pi_turn_started = false;
        self.projection = None;
        self.gateway_runner = None;
        self.active_runtime_id = None;
        self.physical_model_id = None;
        self.model_record_key = None;
        self.subscription_capability = false;
        Ok(())
    }
}
