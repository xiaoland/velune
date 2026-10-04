//! Cross-platform application runtime.
//!
//! This module owns ordinary application configuration and the active
//! Pi/gateway projection. It has no filesystem socket, SQLite, environment, or
//! platform credential access beyond explicit paths supplied at open.

use crate::{
    authentication,
    conversation::{ConversationSummary, PiProjection, RunState},
    gateway::{GatewayConfig, GatewayProtocol, ModelDefinition, RuntimeInstance},
    gateway_runtime::Runner,
    pi::{self, Config as PiConfig},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

const CONTRACT_VERSION: u64 = 3;

fn runtime_types(resources_directory: &Path) -> Value {
    let bundled_binary =
        resources_directory.join("node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js");
    let binary_value = if bundled_binary.is_file() {
        bundled_binary.to_string_lossy().into_owned()
    } else {
        String::new()
    };
    json!([{
        "id":"pi",
        "name":"Pi Agent 运行时",
        "fields":[
            {"key":"binary","label":"运行时入口","kind":"filePath","required":true,"value":binary_value,"options":[],"help":"Pi CLI 的绝对路径；应用不会使用全局 PATH。"},
            {"key":"nodeBinary","label":"Node 可执行文件","kind":"filePath","required":false,"value":"","options":[],"help":"Node 22.19+ 的绝对路径；用于启动 JavaScript CLI。"},
            {"key":"agentDir","label":"运行时目录","kind":"directoryPath","required":true,"value":"","options":[],"help":"配置与状态根目录，不是会话项目目录。"},
            {"key":"sessionDir","label":"会话目录","kind":"directoryPath","required":false,"value":"","options":[],"help":"可选的 Pi 会话目录。"}
        ],
        "actions":[{"id":"connect","label":"连接运行时"}]
    }])
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeOptions {
    pub home_directory: PathBuf,
    pub resources_directory: PathBuf,
    #[serde(default)]
    pub credential_resolver: Option<PathBuf>,
}

impl RuntimeOptions {
    fn validate(&self) -> Result<(), RuntimeError> {
        for path in [
            Some(&self.home_directory),
            Some(&self.resources_directory),
            self.credential_resolver.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if !path.is_absolute() {
                return Err(RuntimeError::invalid("runtime paths must be absolute"));
            }
        }
        if !self.home_directory.exists() {
            fs::create_dir_all(&self.home_directory)
                .map_err(|_| RuntimeError::invalid("runtime home directory"))?;
        }
        if !self.resources_directory.is_dir() {
            return Err(RuntimeError::invalid("runtime resources directory"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedConfig {
    schema_version: u32,
    #[serde(default)]
    gateways: Vec<GatewayConfig>,
    #[serde(default)]
    runtime_instances: Vec<RuntimeInstance>,
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("invalid runtime: {0}")]
    Invalid(String),
    #[error("unsupported runtime action: {0}")]
    Unsupported(String),
    #[error("runtime I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("runtime JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

impl RuntimeError {
    fn invalid(message: &'static str) -> Self {
        Self::Invalid(message.into())
    }

    pub(crate) fn envelope(&self) -> Value {
        let code = match self {
            Self::Invalid(_) => "invalid",
            Self::Unsupported(_) => "unsupported",
            Self::Io(_) => "io",
            Self::Json(_) => "json",
        };
        json!({"ok":false,"error":{"code":code,"message":self.to_string()}})
    }
}

pub struct CoreRuntime {
    options: RuntimeOptions,
    _home_lock: File,
    gateways: Vec<GatewayConfig>,
    runtime_instances: Vec<RuntimeInstance>,
    pi: Option<pi::Client>,
    pi_config: Option<PiConfig>,
    pi_busy: bool,
    projection: Option<PiProjection>,
    gateway_runner: Option<Runner>,
    active_runtime_id: Option<String>,
    authentication: Option<authentication::Login>,
    physical_model_id: Option<String>,
    logical_model_id: Option<String>,
    subscription_capability: bool,
    pi_turn_started: bool,
}

impl CoreRuntime {
    pub fn open(options: RuntimeOptions) -> Result<Self, RuntimeError> {
        options.validate()?;
        let lock_path = options.home_directory.join("runtime.lock");
        let home_lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            home_lock.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        match home_lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(RuntimeError::invalid("runtime home is already open"));
            }
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        let persisted_path = options.home_directory.join("generic-config.json");
        let persisted = match fs::read(&persisted_path) {
            Ok(bytes) => {
                let value: PersistedConfig = serde_json::from_slice(&bytes)?;
                if value.schema_version != 2 {
                    return Err(RuntimeError::invalid("unsupported configuration schema"));
                }
                for gateway in &value.gateways {
                    gateway.validate().map_err(RuntimeError::invalid)?;
                }
                for runtime in &value.runtime_instances {
                    if runtime.id.is_empty()
                        || runtime.name.is_empty()
                        || runtime.type_id != "pi"
                        || !value
                            .gateways
                            .iter()
                            .any(|item| item.id == runtime.gateway_id)
                    {
                        return Err(RuntimeError::invalid("invalid persisted runtime instance"));
                    }
                }
                value
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => PersistedConfig {
                schema_version: 2,
                ..PersistedConfig::default()
            },
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            options,
            _home_lock: home_lock,
            gateways: persisted.gateways,
            runtime_instances: persisted.runtime_instances,
            pi: None,
            pi_config: None,
            pi_busy: false,
            projection: None,
            gateway_runner: None,
            active_runtime_id: None,
            authentication: None,
            physical_model_id: None,
            logical_model_id: None,
            subscription_capability: false,
            pi_turn_started: false,
        })
    }

    pub fn request(&mut self, request: &Value) -> Value {
        self.drain_pi();
        match self.request_inner(request) {
            Ok(data) => json!({"version":CONTRACT_VERSION,"ok":true,"data":data}),
            Err(error) => error.envelope(),
        }
    }

    pub fn close_if_idle(&mut self) -> Result<(), RuntimeError> {
        self.drain_pi();
        if self
            .authentication
            .as_ref()
            .is_some_and(authentication::Login::is_running)
        {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        if self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        self.shutdown_active()?;
        // Release the lock explicitly; a concurrent child spawn can briefly
        // inherit the open file description before exec closes its descriptors.
        self._home_lock.unlock()?;
        Ok(())
    }

    fn request_inner(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if request["version"] != CONTRACT_VERSION {
            return Err(RuntimeError::Unsupported(
                "projection contract version".into(),
            ));
        }
        let action = request["action"].as_str().unwrap_or_default();
        if self
            .authentication
            .as_ref()
            .is_some_and(authentication::Login::is_running)
            && !matches!(action, "authentication" | "list" | "getSnapshot")
        {
            return Err(RuntimeError::invalid("authentication is running"));
        }
        match action {
            "list" => self.list(),
            "gateways" => self.gateway_action(request),
            "runtimeInstances" => self.runtime_action(request),
            "runtimeAction" | "connect" => self.connect_action(request),
            "create" => self.create_conversation(request),
            "open" => self.open_conversation(request),
            "getSnapshot" => {
                self.ensure_active(request)?;
                self.sync_projection()?;
                Ok(
                    json!({"snapshot":self.projection.as_ref().and_then(|item| item.snapshot.as_ref())}),
                )
            }
            "send" => self.send_action(request),
            "cancel" => self.cancel_action(request),
            "selectModel" => self.select_model(request),
            "authentication" => self.authentication_action(&request["payload"]),
            "resources" | "settings" | "beginAuthorization" => {
                Err(RuntimeError::Unsupported("legacy action".into()))
            }
            action => Err(RuntimeError::Unsupported(action.into())),
        }
    }

    fn authentication_action(&mut self, payload: &Value) -> Result<Value, RuntimeError> {
        if payload["operation"].as_str() == Some("start") && self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let source_before = self
            .authentication
            .as_ref()
            .map(|login| login.source().clone());
        let mut data = authentication::handle(
            &mut self.authentication,
            &self.options.resources_directory,
            &self.options.home_directory,
            payload,
        )
        .map_err(RuntimeError::Invalid)?;
        let succeeded = data["events"].as_array().is_some_and(|events| {
            events
                .iter()
                .any(|event| event["type"] == "result" && event["ok"] == true)
        });
        let Some(source) = source_before.filter(|_| succeeded) else {
            return Ok(data);
        };
        let mut changed = false;
        for gateway in &mut self.gateways {
            for provider in &mut gateway.providers {
                let matches = provider
                    .credential_source
                    .as_ref()
                    .is_some_and(|candidate| {
                        candidate.kind == source.kind
                            && candidate.harness_type_id == source.harness_type_id
                            && candidate.provider_id == source.provider_id
                            && candidate.settings.get("authPath") == source.settings.get("authPath")
                    });
                if matches {
                    provider.credential_generation = provider
                        .credential_generation
                        .checked_add(1)
                        .ok_or_else(|| RuntimeError::invalid("credential generation overflow"))?;
                    changed = true;
                }
            }
        }
        if changed {
            self.persist()?;
            if !self.pi_busy {
                self.shutdown_active()?;
            }
            data["gateways"] = serde_json::to_value(&self.gateways)?;
            data["requiresReconnect"] = Value::Bool(true);
        }
        Ok(data)
    }

    fn list(&self) -> Result<Value, RuntimeError> {
        Ok(json!({
            "conversations": self.session_summaries()?,
            "connections": self.connections(),
            "models": self.gateways.iter().flat_map(|item| item.models.iter()).collect::<Vec<_>>(),
            "gateways": self.gateways,
            "runtimeInstances": self.runtime_instances,
            "runtimeTypes": runtime_types(&self.options.resources_directory),
            "credentialSourceTypes": [{
                "id":"pi",
                "name":"Pi Agent 认证来源",
                "fields":[
                    {"key":"providerId","label":"认证提供商","kind":"choice","required":true,"value":"openai","options":[{"id":"openai","label":"ChatGPT"}],"help":"此适配器接入 Pi 的 OpenAI 订阅登录，不接入 legacy Codex backend。"},
                    {"key":"authPath","label":"认证文件","kind":"filePath","required":true,"value":"","options":[],"help":"保留 Pi 原生认证来源文件路径；Velune 不复制凭据。"},
                    {"key":"nodeBinary","label":"Node 可执行文件","kind":"filePath","required":true,"value":"","options":[],"help":"Node 22.19+ 的绝对路径。"}
                ],
                "actions":[{"id":"login","label":"登录…"}],
                "capability":"读取指定 Pi 认证来源；Velune 不复制认证资料。"
            }],
            "protocols": [
                {"id":"chatCompletionsV1","name":"OpenAI Chat Completions v1","supported":true},
                {"id":"responsesV1","name":"OpenAI Responses v1","supported":true}
            ],
            "activeRuntimeInstanceID": self.active_runtime_id,
        }))
    }

    fn create_conversation(&mut self, request: &Value) -> Result<Value, RuntimeError> {
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
            .model_id
            .as_deref()
            .ok_or_else(|| RuntimeError::invalid("runtime model id"))?
            .to_owned();
        self.logical_model_id = Some(default_model.clone());
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
        self.subscription_capability = subscription_capability(gateway, &default_model);
        let default_physical_model_id = gateway
            .pi_binding_id(&default_model)
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

    fn open_conversation(&mut self, request: &Value) -> Result<Value, RuntimeError> {
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
        let restored_model = saved["virtualState"]["state"]["logicalModelId"]
            .as_str()
            .or_else(|| saved["virtualState"]["state"]["modelId"].as_str())
            .or_else(|| saved["model"]["modelId"].as_str());
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
        let model_id = restored_model
            .filter(|id| {
                ((saved["virtualState"]["provider"] == "velune"
                    && saved["virtualState"]["state"]["provider"] == "velune-gateway")
                    || saved["virtualState"]["provider"] == "velune-gateway"
                    || saved["model"]["provider"] == "velune-gateway")
                    && runnable_pi_model(gateway, id).is_ok()
            })
            .map(str::to_owned);
        self.physical_model_id = None;
        self.logical_model_id = model_id.clone();
        self.subscription_capability = model_id
            .as_deref()
            .is_some_and(|id| subscription_capability(gateway, id));
        if let Some(model_id) = model_id {
            let physical_model_id = gateway
                .pi_binding_id(&model_id)
                .map_err(RuntimeError::invalid)?;
            self.physical_model_id = Some(physical_model_id.clone());
            self.start_pi_for_session(
                &cwd,
                Some(&path),
                Some(&model_id),
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

    fn gateway_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let previous = self.gateways.clone();
        let mut changed = false;
        match request["payload"]["operation"].as_str().unwrap_or("list") {
            "upsert" => {
                let raw = request["payload"]["gateway"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("gateway"))?;
                let gateway: GatewayConfig = serde_json::from_str(raw)?;
                gateway.validate().map_err(RuntimeError::invalid)?;
                self.gateways.retain(|item| item.id != gateway.id);
                self.gateways.push(gateway);
                changed = true;
                if let Err(error) = self.persist() {
                    self.gateways = previous.clone();
                    return Err(error);
                }
            }
            "delete" => {
                let id = request["payload"]["gatewayID"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("gateway id"))?;
                if self
                    .runtime_instances
                    .iter()
                    .any(|item| item.gateway_id == id)
                {
                    return Err(RuntimeError::invalid("gateway is used by a runtime"));
                }
                self.gateways.retain(|item| item.id != id);
                changed = true;
                if let Err(error) = self.persist() {
                    self.gateways = previous.clone();
                    return Err(error);
                }
            }
            "list" => {}
            _ => return Err(RuntimeError::invalid("gateway operation")),
        }
        if changed && self.active_runtime_id.is_some() {
            self.shutdown_active()?;
        }
        Ok(
            json!({"gateways":self.gateways,"models":self.gateways.iter().flat_map(|item| item.models.iter()).collect::<Vec<_>>(),"requiresReconnect":changed}),
        )
    }

    fn runtime_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let previous = self.runtime_instances.clone();
        let mut changed = false;
        match request["payload"]["operation"].as_str().unwrap_or("list") {
            "upsert" => {
                let raw = request["payload"]["runtimeInstance"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("runtime instance"))?;
                let runtime: RuntimeInstance = serde_json::from_str(raw)?;
                if runtime.id.is_empty()
                    || runtime.name.is_empty()
                    || runtime.type_id != "pi"
                    || !self
                        .gateways
                        .iter()
                        .any(|item| item.id == runtime.gateway_id)
                {
                    return Err(RuntimeError::invalid("runtime instance"));
                }
                self.runtime_instances.retain(|item| item.id != runtime.id);
                self.runtime_instances.push(runtime);
                changed = true;
                if let Err(error) = self.persist() {
                    self.runtime_instances = previous.clone();
                    return Err(error);
                }
            }
            "delete" => {
                let id = request["payload"]["runtimeInstanceID"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
                self.runtime_instances.retain(|item| item.id != id);
                changed = true;
                if let Err(error) = self.persist() {
                    self.runtime_instances = previous.clone();
                    return Err(error);
                }
            }
            "list" => {}
            _ => return Err(RuntimeError::invalid("runtime operation")),
        }
        if changed && self.active_runtime_id.is_some() {
            self.shutdown_active()?;
        }
        Ok(
            json!({"runtimeInstances":self.runtime_instances,"runtimeTypes":runtime_types(&self.options.resources_directory),"requiresReconnect":changed}),
        )
    }

    fn connect_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
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

    fn connect(&mut self, runtime_id: &str) -> Result<(), RuntimeError> {
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
        let logical_model_id = runtime
            .model_id
            .as_deref()
            .ok_or_else(|| RuntimeError::invalid("runtime model id"))?;
        runnable_pi_model(&gateway, logical_model_id)?;
        let physical_model_id = gateway
            .pi_binding_id(logical_model_id)
            .map_err(RuntimeError::invalid)?;
        self.shutdown_active()?;
        let runner = Runner::start(gateway.clone(), self.options.credential_resolver.clone())
            .map_err(|_| RuntimeError::invalid("gateway startup"))?;
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
            protocol: Some(
                match gateway
                    .validate_dispatch(logical_model_id)
                    .map_err(RuntimeError::invalid)?
                    .protocol
                {
                    GatewayProtocol::ChatCompletionsV1 => "openai-completions",
                    GatewayProtocol::ResponsesV1 => "openai-responses",
                    GatewayProtocol::MessagesV1 => {
                        return Err(RuntimeError::invalid("provider protocol"));
                    }
                }
                .into(),
            ),
            endpoint: Some(runner.endpoint().into()),
            credential_ref: None,
            credential_resolver: None,
            gateway_token: Some(runner.token().into()),
            session_dir: setting_path_optional(&runtime, "sessionDir"),
            session: None,
            // A session's Pi name belongs to its persisted session_info entry.
            // Runtime names must not overwrite it when reopening a session.
            name: None,
        };
        if let Err(error) = materialize_models(&config, &gateway) {
            drop(runner);
            return Err(error);
        }
        let subscription_capability = subscription_capability(&gateway, logical_model_id);
        write_selection_file(
            &config,
            logical_model_id,
            &physical_model_id,
            subscription_capability,
        )?;
        self.gateway_runner = Some(runner);
        self.pi_config = Some(config);
        self.active_runtime_id = Some(runtime_id.into());
        self.physical_model_id = None;
        self.logical_model_id = None;
        self.subscription_capability = false;
        Ok(())
    }

    fn start_pi_for_session(
        &mut self,
        cwd: &Path,
        session: Option<&Path>,
        logical_model_id: Option<&str>,
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
        if let (Some(logical), Some(physical)) = (logical_model_id, physical_model_id) {
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
            snapshot.model_id = logical_model_id.map(str::to_owned);
        }
        self.pi = Some(client);
        self.pi_config = Some(config);
        self.projection = Some(projection);
        self.logical_model_id = logical_model_id.map(str::to_owned);
        self.physical_model_id = physical_model_id.map(str::to_owned);
        self.subscription_capability = subscription_capability;
        self.drain_pi();
        Ok(())
    }

    fn shutdown_pi(&mut self) -> Result<(), RuntimeError> {
        if let Some(mut client) = self.pi.take() {
            client
                .shutdown()
                .map_err(|_| RuntimeError::invalid("runtime shutdown"))?;
        }
        self.pi_busy = false;
        self.pi_turn_started = false;
        self.projection = None;
        self.physical_model_id = None;
        self.logical_model_id = None;
        self.subscription_capability = false;
        if let Some(config) = self.pi_config.as_mut() {
            config.working_dir = None;
            config.session = None;
        }
        Ok(())
    }

    fn send_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
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

    fn bind_gateway_model(&mut self) -> Result<(), RuntimeError> {
        let model_id = self
            .pi_config
            .as_ref()
            .and_then(|config| config.agent_dir.as_ref())
            .map(|agent_dir| agent_dir.join("velune-selection.json"))
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|value| value["modelId"].as_str().map(str::to_owned))
            .ok_or_else(|| RuntimeError::invalid("runtime model id"))?;
        self.pi
            .as_mut()
            .ok_or_else(|| RuntimeError::invalid("runtime is not connected"))?
            .request(json!({"type":"set_model","provider":"velune","modelId":"auto"}))
            .map_err(|_| RuntimeError::invalid("gateway model binding"))?;
        let _ = model_id;
        Ok(())
    }

    fn cancel_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        self.pi
            .as_mut()
            .ok_or_else(|| RuntimeError::invalid("runtime is not connected"))?
            .cancel()
            .map_err(|_| RuntimeError::invalid("message cancel"))?;
        self.drain_pi();
        Ok(json!({"snapshot":self.projection.as_ref().and_then(|item| item.snapshot.as_ref())}))
    }

    fn select_model(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        if self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        if self.pi.is_none() {
            return Err(RuntimeError::invalid("conversation is not active"));
        }
        let model_id = request["payload"]["modelID"]
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
        runnable_pi_model(gateway, model_id)?;
        let selected_subscription_capability = subscription_capability(gateway, model_id);
        let selected_physical_model_id = gateway
            .pi_binding_id(model_id)
            .map_err(RuntimeError::invalid)?;
        self.write_selection(
            model_id,
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

    fn write_selection(
        &self,
        logical_model_id: &str,
        physical_model_id: &str,
        subscription_capability: bool,
    ) -> Result<(), RuntimeError> {
        let config = self
            .pi_config
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid("runtime is not connected"))?;
        write_selection_file(
            config,
            logical_model_id,
            physical_model_id,
            subscription_capability,
        )
    }

    fn ensure_active(&self, request: &Value) -> Result<(), RuntimeError> {
        let requested = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        if self.active_runtime_id.as_deref() != Some(requested) {
            return Err(RuntimeError::invalid("runtime instance is not active"));
        }
        Ok(())
    }

    fn sync_projection(&mut self) -> Result<(), RuntimeError> {
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
                snapshot.model_id = self.logical_model_id.clone();
                snapshot.actions.can_send = snapshot.model_id.is_some() && !self.pi_busy;
                snapshot.actions.can_cancel = self.pi_busy;
            }
        }
        self.drain_pi();
        Ok(())
    }

    fn drain_pi(&mut self) {
        let events = self
            .pi
            .as_mut()
            .map(|client| client.poll())
            .unwrap_or_default();
        for event in events {
            let is_settled = event["type"] == "agent_settled";
            let matched_settled = is_settled && self.pi_turn_started;
            if matches!(
                event["type"].as_str(),
                Some("agent_start" | "turn_start" | "message_start")
            ) {
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

    fn connections(&self) -> Vec<Value> {
        self.active_runtime_id.as_ref()
            .map(|id| {
                let name = self.runtime_instances.iter().find(|runtime| &runtime.id == id)
                    .map(|runtime| runtime.name.as_str()).expect("active runtime instance is configured");
                vec![json!({"id":id,"name":name,"state":"connected","capabilities":["chat","cancel"]})]
            })
            .unwrap_or_default()
    }

    fn session_summaries(&self) -> Result<Vec<ConversationSummary>, RuntimeError> {
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

    fn shutdown_active(&mut self) -> Result<(), RuntimeError> {
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
        self.logical_model_id = None;
        self.subscription_capability = false;
        Ok(())
    }

    fn persist(&self) -> Result<(), RuntimeError> {
        let path = self.options.home_directory.join("generic-config.json");
        let bytes = serde_json::to_vec_pretty(&PersistedConfig {
            schema_version: 2,
            gateways: self.gateways.clone(),
            runtime_instances: self.runtime_instances.clone(),
        })?;
        let temporary = path.with_extension("json.tmp");
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(temporary, &path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

fn setting_path(runtime: &RuntimeInstance, key: &str) -> Result<PathBuf, RuntimeError> {
    setting_path_optional(runtime, key).ok_or_else(|| RuntimeError::invalid("runtime setting path"))
}

fn setting_path_optional(runtime: &RuntimeInstance, key: &str) -> Option<PathBuf> {
    runtime
        .settings
        .get(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn runnable_pi_model<'a>(
    gateway: &'a GatewayConfig,
    id: &str,
) -> Result<&'a ModelDefinition, RuntimeError> {
    gateway
        .validate_dispatch(id)
        .map_err(RuntimeError::invalid)?;
    let model = gateway.model(id).expect("validated model");
    if model.context_window.is_none() {
        return Err(RuntimeError::invalid(
            "请先在模型设置中填写上下文窗口（tokens）",
        ));
    }
    if model.reasoning_levels.iter().any(|level| {
        !["off", "minimal", "low", "medium", "high", "xhigh", "max"].contains(&level.as_str())
    }) {
        return Err(RuntimeError::invalid(
            "当前 Pi 适配器不支持该模型的推理等级",
        ));
    }
    Ok(model)
}

fn materialize_models(config: &PiConfig, gateway: &GatewayConfig) -> Result<(), RuntimeError> {
    let agent_dir = config
        .agent_dir
        .as_ref()
        .ok_or_else(|| RuntimeError::invalid("runtime directory"))?;
    let provider = config
        .provider
        .as_deref()
        .ok_or_else(|| RuntimeError::invalid("gateway provider"))?;
    let endpoint = config
        .endpoint
        .as_deref()
        .ok_or_else(|| RuntimeError::invalid("gateway endpoint"))?;
    let protocol = config
        .protocol
        .as_deref()
        .ok_or_else(|| RuntimeError::invalid("gateway protocol"))?;
    fs::create_dir_all(agent_dir)?;
    let marker = agent_dir.join(".velune-managed");
    let target = agent_dir.join("models.json");
    if target.exists() && !marker.exists() {
        return Err(RuntimeError::invalid("refusing unmanaged Pi models.json"));
    }
    // Pi caches its available model catalog at startup. Include every explicit
    // route so subsequent set_model calls use the same immutable gateway revision.
    let entries = gateway
        .models
        .iter()
        .filter(|model| runnable_pi_model(gateway, &model.id).is_ok())
        .map(|model| -> Result<Value, RuntimeError> {
            let physical_id = gateway
                .pi_binding_id(&model.id)
                .map_err(RuntimeError::invalid)?;
            let api =
                gateway
                    .validate_dispatch(&model.id)
                    .ok()
                    .map(|provider| match provider.protocol {
                        GatewayProtocol::ChatCompletionsV1 => "openai-completions",
                        GatewayProtocol::ResponsesV1 => "openai-responses",
                        GatewayProtocol::MessagesV1 => "unsupported",
                    });
            let mut entry = json!({
                "id": physical_id,
                "logicalModelId": model.id,
                "name": model.nickname,
                "input": ["text"],
                "maxTokens": model.max_output_tokens,
                "contextWindow": model.context_window.expect("runnable model context window"),
            });
            if let Some(api) = api {
                entry["api"] = Value::String(api.into());
            }
            if !model.reasoning_levels.is_empty() {
                entry["reasoning"] = Value::Bool(true);
                entry["compat"] = json!({"supportsReasoningEffort": true});
                let levels = ["off", "minimal", "low", "medium", "high", "xhigh", "max"]
                    .into_iter()
                    .map(|level| {
                        let value = if model.reasoning_levels.iter().any(|item| item == level) {
                            Value::String(level.into())
                        } else {
                            Value::Null
                        };
                        (level.to_owned(), value)
                    })
                    .collect::<serde_json::Map<_, _>>();
                entry["thinkingLevelMap"] = Value::Object(levels);
            }
            Ok(entry)
        })
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    let mut value = json!({"providers":{}});
    value["providers"][provider] = json!({
        "baseUrl": endpoint,
        "api": protocol,
        "models": entries,
        "apiKey": if config.gateway_token.is_some() {
            Value::String("$VELUNE_GATEWAY_TOKEN".into())
        } else {
            Value::Null
        },
    });
    let bytes = serde_json::to_vec_pretty(&value)?;
    let temporary = agent_dir.join("models.json.velune.tmp");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, target)?;
    fs::write(marker, b"velune managed models config v1\n")?;
    Ok(())
}

fn write_selection_file(
    config: &PiConfig,
    logical_model_id: &str,
    physical_model_id: &str,
    subscription_capability: bool,
) -> Result<(), RuntimeError> {
    let agent_dir = config
        .agent_dir
        .as_ref()
        .ok_or_else(|| RuntimeError::invalid("runtime directory"))?;
    fs::create_dir_all(agent_dir)?;
    let path = agent_dir.join("velune-selection.json");
    let temporary = agent_dir.join("velune-selection.json.tmp");
    let bytes = serde_json::to_vec(&json!({
        "provider": "velune-gateway",
        "logicalModelId": logical_model_id,
        "physicalModelId": physical_model_id,
        "modelId": physical_model_id,
        "thinkingLevel": "off",
        "subscriptionCapability": subscription_capability
    }))?;
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn subscription_capability(gateway: &GatewayConfig, model_id: &str) -> bool {
    let Some(route) = gateway
        .routes
        .iter()
        .find(|route| route.model_id == model_id)
    else {
        return false;
    };
    gateway
        .providers
        .iter()
        .find(|provider| provider.id == route.provider_id)
        .and_then(|provider| {
            provider
                .credential_source
                .as_ref()
                .map(|source| (provider, source))
        })
        .is_some_and(|(provider, source)| {
            matches!(provider.protocol, GatewayProtocol::ResponsesV1)
                && source.harness_type_id == "pi"
                && source.provider_id == "openai"
        })
}

fn validate_session_cwd(cwd: &Path) -> Result<(), RuntimeError> {
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err(RuntimeError::invalid("conversation working directory"));
    }
    Ok(())
}

fn pi_session_helper(config: &PiConfig, session: Option<&Path>) -> Result<Value, RuntimeError> {
    let helper = config
        .sdk_helper
        .as_ref()
        .ok_or_else(|| RuntimeError::invalid("Pi session helper is not configured"))?;
    let mut command = Command::new(
        config
            .node_binary
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("node")),
    );
    command.arg(helper);
    if let Some(path) = session {
        command.arg("--inspect-session").arg(path);
    } else {
        command.arg("--all");
    }
    if let Some(cwd) = &config.working_dir {
        // Pi records the physical process cwd; use the same path for SDK filtering.
        command.arg("--cwd").arg(fs::canonicalize(cwd)?);
    }
    if let Some(session_dir) = &config.session_dir {
        command.arg("--session-dir").arg(session_dir);
    }
    if let Some(agent_dir) = &config.agent_dir {
        command.env("PI_CODING_AGENT_DIR", agent_dir);
    }
    let output = command.output().map_err(RuntimeError::Io)?;
    if !output.status.success() {
        return Err(RuntimeError::invalid("Pi session helper failed"));
    }
    serde_json::from_slice(&output.stdout).map_err(RuntimeError::Json)
}

pub fn runtime_options(value: &Value) -> Result<RuntimeOptions, RuntimeError> {
    Ok(serde_json::from_value(value.clone())?)
}
