//! Generic ACP v1 over stdio. Agent launch, authentication and provider ownership stay external.
pub use crate::native::{Error, Result};
use crate::{
    conversation::*,
    execution::ExecutionTarget,
    native::rpc::{CONTROL_TIMEOUT, RpcClient, observe_request},
};
use agent_client_protocol_schema::v1 as wire;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Instant,
};
mod config;
mod port;
mod projection;
pub use config::{ConfigChoice, ConfigOption, ConfigValue};
pub use port::{
    AcpAdmissionError, AcpCommand, AcpNativeError, AcpReceipt, AcpReceiptStage, AcpTicket,
};

/// Arguments and environment belong to the execution target. Environment values may be secrets.
#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    pub binary: PathBuf,
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub execution: ExecutionTarget,
    pub os_home: Option<PathBuf>,
    pub mcp_servers: Vec<crate::mcp::Server>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub mcp_http: bool,
    pub load_session: bool,
    pub list_sessions: bool,
    pub resume_session: bool,
    pub close_session: bool,
    pub delete_session: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub name: String,
    pub title: Option<String>,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Clone)]
struct Permission {
    parent_deadline: Option<Instant>,
    rpc_id: Value,
    options: Vec<String>,
}

struct Elicitation {
    parent_deadline: Option<Instant>,
    rpc_id: Value,
    params: Value,
}

pub struct Client {
    config: Config,
    rpc: RpcClient,
    capabilities: Capabilities,
    /// Whether the Agent accepts image blocks in a prompt.
    prompt_image: bool,
    agent_info: Option<AgentInfo>,
    auth_methods: Vec<AuthMethod>,
    options: Vec<ConfigOption>,
    snapshot: Option<ConversationSnapshot>,
    native_id: Option<String>,
    prompt_id: Option<u64>,
    permissions: BTreeMap<String, Permission>,
    elicitations: BTreeMap<String, Elicitation>,
    epoch: String,
    sequence: u64,
    message_sequence: u64,
    pending_user_echo: Option<String>,
    failed: bool,
    establishing: bool,
    initial_updates: crate::transport::DeferredRecords,
    connection_generation: u64,
    port: Option<port::Port>,
    cleaned_up: bool,
    stream: Option<projection::Stream>,
    mcp_servers: Vec<Value>,
    _mcp: crate::mcp::Prepared,
    // These typed fields live for the client, not merely until the next poll.
    _initialization_data: Option<crate::creation::Retained<()>>,
    _conversation_data: Option<crate::creation::DataRetention>,
    _session_data: Option<crate::creation::Retained<()>>,
    _options_data: Option<crate::creation::Retained<()>>,
    _snapshot_data: Vec<crate::creation::Retained<()>>,
    _interaction_data: BTreeMap<String, crate::creation::Retained<()>>,
    _initial_update_data: std::collections::VecDeque<crate::creation::DataRetention>,
    // Last field: creation resource/HOME ownership outlives all native fields,
    // MCP cleanup and the RPC owner, plus actual reader/writer jobs via Arc.
    _creation_cleanup: Option<std::sync::Arc<crate::creation::ReadAccount>>,
}

impl Client {
    /// Negotiate ACP v1 directly; no assumptions about a CLI's name or version command.
    pub fn connect(config: Config) -> Result<Self> {
        config.execution.validate()?;
        if !config.binary.is_absolute() {
            return Err(Error::new("ACP 可执行文件必须为目标机器上的绝对路径"));
        }
        if config.args.iter().any(|arg| arg.contains('\0'))
            || config.environment.iter().any(|(key, value)| {
                key.is_empty() || key.contains(['=', '\0']) || value.contains('\0')
            })
        {
            return Err(Error::new("ACP 启动参数或环境变量无效"));
        }
        if config.os_home.is_some() && config.environment.contains_key("HOME") {
            return Err(Error::new(
                "ACP HOME 已由运行时执行配置指定，不能在环境变量中重复指定",
            ));
        }
        let mcp = crate::mcp::Prepared::new(
            &config.mcp_servers,
            &config.execution,
            config.os_home.as_deref(),
        )?;
        let mut command = crate::process_tree::command(&config.binary);
        command.args(&config.args).envs(&config.environment);
        let rpc = RpcClient::spawn_at(command, &config.execution, config.os_home.as_deref())?;
        let mut bytes = [0; 8];
        getrandom::fill(&mut bytes).map_err(|error| Error::new(error.to_string()))?;
        let mut client = Self {
            config,
            rpc,
            capabilities: Capabilities::default(),
            prompt_image: false,
            agent_info: None,
            auth_methods: Vec::new(),
            options: Vec::new(),
            snapshot: None,
            native_id: None,
            prompt_id: None,
            permissions: BTreeMap::new(),
            elicitations: BTreeMap::new(),
            epoch: format!("{:x}", u64::from_le_bytes(bytes)),
            sequence: 0,
            message_sequence: 0,
            pending_user_echo: None,
            failed: false,
            establishing: false,
            initial_updates: Default::default(),
            connection_generation: port::generation()?,
            port: None,
            cleaned_up: false,
            stream: None,
            mcp_servers: Vec::new(),
            _mcp: mcp,
            _initialization_data: None,
            _conversation_data: None,
            _session_data: None,
            _options_data: None,
            _snapshot_data: Vec::new(),
            _interaction_data: BTreeMap::new(),
            _initial_update_data: std::collections::VecDeque::new(),
            _creation_cleanup: None,
        };
        let response: crate::creation::Retained<wire::InitializeResponse> = client.request(
            "initialize",
            &json!({
                "protocolVersion":1,
                "clientInfo":{"name":"velune","version":env!("CARGO_PKG_VERSION")},
                "clientCapabilities":{"session":{"configOptions":{"boolean":{}}},"elicitation":{"form":{}}}
            }),
        )?;
        let (response, initialization_retention) = response.into_parts();
        if response.protocol_version != agent_client_protocol_schema::ProtocolVersion::V1 {
            return Err(Error::with_code(
                "ACP Agent 未协商可支持的协议版本 1",
                "unsupported",
            ));
        }
        let capabilities = &response.agent_capabilities;
        let sessions = &capabilities.session_capabilities;
        client.capabilities = Capabilities {
            mcp_http: capabilities.mcp_capabilities.http,
            load_session: capabilities.load_session,
            list_sessions: sessions.list.is_some(),
            resume_session: sessions.resume.is_some(),
            close_session: sessions.close.is_some(),
            delete_session: sessions.delete.is_some(),
        };
        client.prompt_image = capabilities.prompt_capabilities.image;
        client.mcp_servers = client._mcp.acp_servers(client.capabilities.mcp_http)?;
        client.agent_info = response.agent_info.map(|info| AgentInfo {
            name: info.name,
            title: info.title,
            version: info.version,
        });
        client.auth_methods = response
            .auth_methods
            .into_iter()
            .map(|method| AuthMethod {
                id: method.id().to_string(),
                name: method.name().to_owned(),
                description: method.description().map(str::to_owned),
            })
            .collect();
        client._initialization_data = Some(crate::creation::Retained::from_parts(
            (),
            initialization_retention,
        ));
        Ok(client)
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }
    pub fn agent_info(&self) -> Option<&AgentInfo> {
        self.agent_info.as_ref()
    }
    pub fn auth_methods(&self) -> &[AuthMethod] {
        &self.auth_methods
    }
    pub fn config_options(&self) -> &[ConfigOption] {
        &self.options
    }
    pub fn snapshot(&self) -> Option<&ConversationSnapshot> {
        self.snapshot.as_ref()
    }
    fn snapshot_retention_copy(&self) -> Result<crate::creation::DataRetention> {
        let mut ownership = self
            ._conversation_data
            .as_ref()
            .map(crate::creation::DataRetention::try_clone)
            .transpose()
            .map_err(|error| Error::with_code(error.to_string(), "queue_full"))?
            .unwrap_or_default();
        for source in self
            ._session_data
            .iter()
            .chain(&self._options_data)
            .chain(&self._snapshot_data)
            .chain(self._interaction_data.values())
        {
            let (_, retained) = source
                .try_clone()
                .map_err(|error| Error::with_code(error.to_string(), "queue_full"))?
                .into_parts();
            ownership = ownership.merge(retained);
        }
        Ok(ownership)
    }
    pub fn owned_snapshot(&self) -> Result<crate::creation::Retained<ConversationSnapshot>> {
        let ownership = self.snapshot_retention_copy()?;
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or_else(|| Error::new("ACP 会话尚未打开"))?;
        Ok(crate::creation::Retained::from_parts(
            snapshot.clone(),
            ownership,
        ))
    }
    pub fn turn_running(&self) -> bool {
        self.prompt_id.is_some()
    }

    pub fn create(
        &mut self,
        cwd: &Path,
        runtime_id: &str,
    ) -> Result<crate::creation::Retained<ConversationSnapshot>> {
        self.prepare_session(cwd)?;
        self.establishing = true;
        let result = self.request::<wire::NewSessionResponse>(
            "session/new",
            &json!({"cwd":cwd,"mcpServers":self.mcp_servers}),
        );
        self.establishing = false;
        let outcome = result.and_then(|response| {
            let (response, ownership) = response.into_parts();
            self._session_data = Some(crate::creation::Retained::from_parts((), ownership));
            let id = response.session_id.to_string();
            if id.is_empty() {
                return Err(Error::new("ACP 会话身份为空"));
            }
            self.replace_snapshot(&id, cwd, runtime_id);
            self.options = config::project(response.config_options.unwrap_or_default())?;
            for record in self.initial_updates.drain() {
                let ownership = self
                    ._initial_update_data
                    .pop_front()
                    .expect("each initial ACP update retains its data charge");
                self.handle(crate::creation::Retained::from_parts(record, ownership))?;
            }
            self.owned_snapshot()
        });
        if outcome.is_err() {
            self.fail();
        }
        outcome
    }

    /// `session/load` replays authoritative history. Resume alone is not a history-reading capability.
    pub fn load(
        &mut self,
        native_id: &str,
        cwd: &Path,
        runtime_id: &str,
    ) -> Result<crate::creation::Retained<ConversationSnapshot>> {
        if !self.capabilities.load_session {
            return Err(unsupported("session/load"));
        }
        if native_id.is_empty() {
            return Err(Error::new("ACP 会话身份为空"));
        }
        self.prepare_session(cwd)?;
        self.replace_snapshot(native_id, cwd, runtime_id);
        let response = self.request::<wire::LoadSessionResponse>(
            "session/load",
            &json!({"sessionId":native_id,"cwd":cwd,"mcpServers":self.mcp_servers}),
        );
        let outcome = response.and_then(|response| {
            let (response, ownership) = response.into_parts();
            self._session_data = Some(crate::creation::Retained::from_parts((), ownership));
            self.options = config::project(response.config_options.unwrap_or_default())?;
            self.complete_messages();
            self.owned_snapshot()
        });
        if outcome.is_err() {
            self.fail();
        }
        outcome
    }

    /// Restore agent context without transcript replay. Use `load` when history is needed.
    pub fn resume(
        &mut self,
        native_id: &str,
        cwd: &Path,
        runtime_id: &str,
    ) -> Result<crate::creation::Retained<ConversationSnapshot>> {
        if !self.capabilities.resume_session {
            return Err(unsupported("session/resume"));
        }
        if native_id.is_empty() {
            return Err(Error::new("ACP 会话身份为空"));
        }
        self.prepare_session(cwd)?;
        self.replace_snapshot(native_id, cwd, runtime_id);
        let response = self.request::<wire::ResumeSessionResponse>(
            "session/resume",
            &json!({"sessionId":native_id,"cwd":cwd,"mcpServers":self.mcp_servers}),
        );
        let outcome = response.and_then(|response| {
            let (response, ownership) = response.into_parts();
            self._session_data = Some(crate::creation::Retained::from_parts((), ownership));
            self.options = config::project(response.config_options.unwrap_or_default())?;
            self.notice("Agent 已恢复原生上下文；session/resume 不回放会话历史。");
            self.owned_snapshot()
        });
        if outcome.is_err() {
            self.fail();
        }
        outcome
    }

    pub fn close_session(&mut self) -> Result<()> {
        self.ensure_idle()?;
        if !self.capabilities.close_session {
            return Err(unsupported("session/close"));
        }
        let id = self.session_id()?.to_owned();
        self.request::<wire::CloseSessionResponse>("session/close", &json!({"sessionId":id}))?;
        self.clear_session();
        Ok(())
    }

    pub fn list_sessions(
        &mut self,
        cwd: Option<&Path>,
        runtime_id: &str,
    ) -> Result<crate::creation::Retained<Vec<ConversationSummary>>> {
        self.ensure_idle()?;
        if !self.capabilities.list_sessions {
            return Err(unsupported("session/list"));
        }
        if cwd.is_some_and(|cwd| !cwd.is_absolute()) {
            return Err(Error::new("ACP 会话筛选目录必须为绝对路径"));
        }
        let mut cursor = None::<String>;
        let mut cursors = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        let mut result: Option<crate::creation::Retained<Vec<ConversationSummary>>> = None;
        loop {
            let mut params = json!({});
            if let Some(cwd) = cwd {
                params["cwd"] = json!(cwd);
            }
            if let Some(cursor) = &cursor {
                params["cursor"] = json!(cursor);
            }
            let response: crate::creation::Retained<wire::ListSessionsResponse> =
                self.request("session/list", &params)?;
            let page = response.try_map(|response| {
                let mut page = Vec::new();
                for value in response.sessions {
                    let id = value.session_id.to_string();
                    if id.is_empty() || !value.cwd.is_absolute() {
                        return Err(Error::new("ACP 返回的会话身份或目录无效"));
                    }
                    let cwd = value.cwd.to_string_lossy().into_owned();
                    if !ids.insert(id.clone()) {
                        return Err(Error::new("ACP 会话目录返回重复身份"));
                    }
                    page.push(ConversationSummary {
                        native_read_state: None,
                        id: format!("{runtime_id}:{id}"),
                        runtime_id: runtime_id.into(),
                        cwd: Some(cwd),
                        title: conversation_title(value.title.as_deref(), None),
                        created_at_unix_ms: None,
                        updated_at_unix_ms: value.updated_at.as_deref().and_then(timestamp),
                        can_rename: false,
                        can_delete: self.capabilities.delete_session,
                        parent_conversation_id: None,
                    });
                }
                Ok((page, response.next_cursor))
            })?;
            let (page, next) = page.split_map(|value| value);
            result = Some(match result.take() {
                Some(previous) => previous.zip(page).map(|(mut all, page)| {
                    all.extend(page);
                    all
                }),
                None => page,
            });
            let Some(next) = next.as_ref().clone() else {
                break;
            };
            if !cursors.insert(next.clone()) {
                return Err(Error::new("ACP 会话目录分页游标重复"));
            }
            cursor = Some(next);
        }
        Ok(result.expect("session list always receives a first page"))
    }

    pub fn delete_session(&mut self, native_id: &str) -> Result<()> {
        self.ensure_idle()?;
        if !self.capabilities.delete_session {
            return Err(unsupported("session/delete"));
        }
        if native_id.is_empty() {
            return Err(Error::new("ACP 会话身份为空"));
        }
        self.request::<wire::DeleteSessionResponse>(
            "session/delete",
            &json!({"sessionId":native_id}),
        )?;
        if self.native_id.as_deref() == Some(native_id) {
            self.clear_session();
        }
        Ok(())
    }

    /// Reasoning is a session configuration option of its own in ACP.
    pub fn models(&self) -> Result<Vec<crate::native::ModelOption>> {
        Ok(self
            .options
            .iter()
            .find(|option| option.category.as_deref() == Some("model"))
            .into_iter()
            .flat_map(|option| &option.choices)
            .map(|choice| crate::native::ModelOption {
                model: crate::native::RuntimeModel {
                    provider_id: None,
                    id: choice.value.clone(),
                    name: choice.name.clone(),
                },
                reasoning_efforts: Vec::new(),
            })
            .collect())
    }

    /// Composer convenience for the first model selector in the agent's preferred order.
    pub fn select_model(&mut self, value: &str) -> Result<()> {
        let id = self
            .options
            .iter()
            .find(|option| option.category.as_deref() == Some("model"))
            .map(|option| option.id.clone())
            .ok_or_else(|| unsupported("模型选择"))?;
        self.set_config_option(&id, ConfigValue::Select(value.into()))
    }

    pub fn set_config_option(&mut self, config_id: &str, value: ConfigValue) -> Result<()> {
        self.ensure_idle()?;
        let id = self.session_id()?.to_owned();
        let option = self
            .options
            .iter()
            .find(|option| option.id == config_id)
            .ok_or_else(|| Error::new("ACP 会话未公布此配置项"))?;
        let mut params = json!({"sessionId":id,"configId":config_id});
        match (&option.value, value) {
            (ConfigValue::Select(_), ConfigValue::Select(value))
                if option.choices.iter().any(|choice| choice.value == value) =>
            {
                params["value"] = json!(value)
            }
            (ConfigValue::Boolean(_), ConfigValue::Boolean(value)) => {
                params["type"] = json!("boolean");
                params["value"] = json!(value);
            }
            _ => return Err(Error::new("ACP 配置值不属于当前公布的选项或类型")),
        }
        let response: crate::creation::Retained<wire::SetSessionConfigOptionResponse> =
            self.request("session/set_config_option", &params)?;
        let (response, ownership) = response.into_parts();
        self._options_data = Some(crate::creation::Retained::from_parts((), ownership));
        self.options = match config::project(response.config_options) {
            Ok(options) => options,
            Err(error) => {
                self.fail();
                return Err(error);
            }
        };
        Ok(())
    }

    pub fn send(&mut self, text: &str, images: &[crate::TurnImage]) -> Result<()> {
        self.ensure_idle()?;
        if text.trim().is_empty() && images.is_empty() {
            return Err(Error::new("消息内容为空"));
        }
        if !images.is_empty() && !self.prompt_image {
            return Err(Error::with_code("此 ACP Agent 不接受图片", "unsupported"));
        }
        let id = self.session_id()?.to_owned();
        let prompt = (!text.is_empty())
            .then(|| json!({"type":"text","text":text}))
            .into_iter()
            .chain(images.iter().map(|image| {
                json!({"type":"image","mimeType":image.kind.media_type(),"data":image.base64()})
            }))
            .collect::<Vec<_>>();
        let result = self
            .rpc
            .begin("session/prompt", &json!({"sessionId":id,"prompt":prompt}));
        let request = match result {
            Ok(id) => id,
            Err(error) => {
                self.fail();
                return Err(error);
            }
        };
        self.project_prompt(text, images, request);
        Ok(())
    }

    fn project_prompt(&mut self, text: &str, images: &[crate::TurnImage], request: u64) {
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.executions.begin(ExecutionKind::UserTurn, false);
        }
        self.prompt_id = Some(request);
        self.stream = None;
        let message_id = self.next_message_id();
        self.pending_user_echo = Some(message_id.clone());
        self.message(Message {
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
            id: message_id,
            role: MessageRole::User,
            completed: true,
            timestamp_unix_ms: None,
            blocks: (!text.is_empty())
                .then(|| MessageBlock::Text { text: text.into() })
                .into_iter()
                .chain(images.iter().map(|image| MessageBlock::Image {
                    media_type: Some(image.kind.media_type().into()),
                    source: crate::conversation::ImageSource::Path {
                        path: image.path.to_string_lossy().into_owned(),
                    },
                }))
                .collect(),
        });
        self.state(RunState::Running, false, true);
    }

    pub fn cancel(&mut self) -> Result<()> {
        self.ensure_sync()?;
        if !self.turn_running() {
            return Ok(());
        }
        let id = self.session_id()?.to_owned();
        let result = self.cancel_permissions().and_then(|()| {
            self.rpc.notify_with_class(
                "session/cancel",
                &json!({"sessionId":id}),
                crate::transport::WriteClass::ProtocolReplyCancel,
            )
        });
        if let Err(error) = result {
            self.fail();
            return Err(error);
        }
        self.state(RunState::Stopping, false, true);
        Ok(())
    }

    pub fn reply(&mut self, interaction_id: &str, reply: RuntimeInteractionReply) -> Result<()> {
        self.ensure_sync()?;
        if let Some(elicitation) = self.elicitations.get(interaction_id) {
            let result = crate::elicitation::reply(&elicitation.params, reply)?;
            if let Err(error) = self.rpc.respond(&elicitation.rpc_id, &result) {
                self.fail();
                return Err(error);
            }
            self.elicitations.remove(interaction_id);
            self.remove_interaction(interaction_id);
            return Ok(());
        }
        let permission = self
            .permissions
            .get(interaction_id)
            .ok_or_else(|| Error::new("ACP 权限请求已失效"))?;
        let outcome = match reply {
            RuntimeInteractionReply::Decision { option_id }
                if permission.options.contains(&option_id) =>
            {
                json!({"outcome":"selected","optionId":option_id})
            }
            RuntimeInteractionReply::Cancel => json!({"outcome":"cancelled"}),
            _ => return Err(Error::new("ACP 权限回复不属于当前公布的选项")),
        };
        if let Err(error) = self
            .rpc
            .respond(&permission.rpc_id, &json!({"outcome":outcome}))
        {
            self.fail();
            return Err(error);
        }
        self.permissions.remove(interaction_id);
        self.remove_interaction(interaction_id);
        Ok(())
    }

    fn remove_interaction(&mut self, interaction_id: &str) {
        if let Some(snapshot) = &mut self.snapshot {
            snapshot
                .pending_interactions
                .retain(|interaction| interaction.id != interaction_id);
            snapshot.revision += 1;
        }
        self._interaction_data.remove(interaction_id);
    }

    pub fn poll(&mut self) -> Result<()> {
        if self.port.is_some() {
            return self.poll_port();
        }
        if self.failed {
            return Ok(());
        }
        let outcome = self.rpc.poll().and_then(|records| {
            for record in records {
                self.handle(record)?;
            }
            Ok(())
        });
        if outcome.is_err() {
            self.fail();
        }
        outcome
    }

    pub fn set_conversation(
        &mut self,
        conversation: crate::creation::Retained<ConversationSummary>,
    ) {
        let (conversation, data) = conversation.into_parts();
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.conversation = conversation;
            self._conversation_data = Some(data);
        }
    }

    /// Gracefully close an idle session when supported, then always release its process.
    /// A busy or failed connection is aborted; Drop never performs a protocol request.
    pub fn shutdown(&mut self) -> Result<()> {
        let outcome = if self.port.is_none()
            && !self.failed
            && !self.turn_running()
            && self.capabilities.close_session
        {
            if let Some(id) = self.native_id.clone() {
                self.request::<wire::CloseSessionResponse>(
                    "session/close",
                    &json!({"sessionId":id}),
                )
                .map(|_| ())
            } else {
                Ok(())
            }
        } else {
            Ok(())
        };
        if outcome.is_ok() && !self.failed && !self.turn_running() {
            self.rpc.finish();
        } else {
            self.rpc.shutdown();
        }
        self.settle_port_shutdown();
        self.failed = true;
        self.cleaned_up = true;
        self.prompt_id = None;
        self.permissions.clear();
        self.elicitations.clear();
        self.finish(if outcome.is_ok() {
            RunState::Idle
        } else {
            RunState::Failed
        });
        outcome
    }

    fn request<T: serde::de::DeserializeOwned>(
        &mut self,
        method: &str,
        params: &Value,
    ) -> Result<crate::creation::Retained<T>> {
        observe_request(method, || self.request_response(method, params))
    }

    fn request_response<T: serde::de::DeserializeOwned>(
        &mut self,
        method: &str,
        params: &Value,
    ) -> Result<crate::creation::Retained<T>> {
        let deadline = Instant::now() + CONTROL_TIMEOUT;
        let id = match self.rpc.begin_until(method, params, deadline) {
            Ok(id) => id,
            Err(error) => {
                self.fail();
                return Err(error);
            }
        };
        loop {
            let record = match self.rpc.receive(method, deadline) {
                Ok(record) => record,
                Err(error) => {
                    self.fail();
                    return Err(error);
                }
            };
            if record["jsonrpc"] != "2.0" {
                self.fail();
                return Err(Error::new("ACP 返回的 JSON-RPC 版本无效"));
            }
            if record.get("method").is_none() && record["id"].as_u64() == Some(id) {
                if let Some(error) = record.get("error") {
                    let error = match rpc_error(error) {
                        Ok(error) => error,
                        Err(error) => {
                            self.fail();
                            error
                        }
                    };
                    return Err(error);
                }
                let outcome = record.try_map(|mut record| {
                    record
                        .get_mut("result")
                        .filter(|value| value.is_object())
                        .map(Value::take)
                        .ok_or_else(|| Error::new("ACP 响应缺少对象结果"))
                        .and_then(|value| {
                            serde_json::from_value(value).map_err(|error| {
                                Error::new(format!("ACP {method} 响应无效：{error}"))
                            })
                        })
                });
                if outcome.is_err() {
                    self.fail();
                }
                return outcome;
            }
            if let Err(error) = self.handle(record) {
                self.fail();
                return Err(error);
            }
        }
    }

    fn handle(&mut self, record: crate::creation::Retained<Value>) -> Result<()> {
        if record["method"] == "session/update" && self.establishing {
            let (value, ownership) = record.into_parts();
            self.initial_updates.push(value).map_err(|error| {
                Error::with_code(
                    format!("ACP initial update buffer full: {error}"),
                    "transport",
                )
            })?;
            self._initial_update_data.push_back(ownership);
            return Ok(());
        }
        // Only records that become projected content retain an origin charge.
        // Ordinary ACKs, unsupported requests and unrelated notifications drop
        // their receive owner immediately after handling.
        let projects_content = record["method"] == "session/update"
            && record["params"]["sessionId"].as_str() == self.native_id.as_deref()
            && self.native_id.is_some();
        let prompt_error = self.prompt_id.is_some()
            && record["id"].as_u64() == self.prompt_id
            && record.get("error").is_some();
        let request_id = record.get("id").cloned();
        let (value, ownership) = record.into_parts();
        let result = self.handle_value(value);
        if (projects_content || prompt_error) && self.snapshot.is_some() {
            self._snapshot_data
                .push(crate::creation::Retained::from_parts((), ownership));
        } else if let Some(rpc_id) = request_id {
            let key = self
                .permissions
                .iter()
                .find(|(_, entry)| entry.rpc_id == rpc_id)
                .map(|(id, _)| id.clone())
                .or_else(|| {
                    self.elicitations
                        .iter()
                        .find(|(_, entry)| entry.rpc_id == rpc_id)
                        .map(|(id, _)| id.clone())
                });
            if let Some(key) = key {
                self._interaction_data
                    .insert(key, crate::creation::Retained::from_parts((), ownership));
            }
        }
        self._interaction_data.retain(|id, _| {
            self.permissions.contains_key(id)
                || self.elicitations.contains_key(id)
                || self.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot
                        .pending_interactions
                        .iter()
                        .any(|entry| entry.id == *id)
                })
        });
        result
    }

    fn handle_value(&mut self, record: Value) -> Result<()> {
        if record["jsonrpc"] != "2.0" {
            return Err(Error::new("ACP 返回的 JSON-RPC 版本无效"));
        }
        if let Some(method) = record["method"].as_str() {
            let params = &record["params"];
            if method == "session/update" {
                if self.establishing {
                    self.initial_updates.push(record).map_err(|error| {
                        Error::with_code(
                            format!("ACP initial update buffer full: {error}"),
                            "transport",
                        )
                    })?;
                } else if params["sessionId"].as_str() == self.native_id.as_deref()
                    && self.native_id.is_some()
                {
                    self.update(&params["update"])?;
                }
            } else if method == "session/request_permission" {
                let id = record
                    .get("id")
                    .ok_or_else(|| Error::new("ACP 权限请求缺少身份"))?;
                self.ensure_agent_request_id_available(id)?;
                if !self.turn_running()
                    || self
                        .snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.run_state == RunState::Stopping)
                    || params["sessionId"].as_str() != self.native_id.as_deref()
                {
                    self.respond_auto(id, &json!({"outcome":{"outcome":"cancelled"}}))?;
                } else {
                    self.permission(id, params)?;
                }
            } else if method == "elicitation/create" {
                let id = record
                    .get("id")
                    .ok_or_else(|| Error::new("ACP 提问缺少身份"))?;
                self.elicitation(id, params)?;
            } else if method == "$/cancel_request" {
                let expired: Vec<_> = self
                    .permissions
                    .iter()
                    .filter(|(_, permission)| permission.rpc_id == params["requestId"])
                    .map(|(id, _)| id.clone())
                    .chain(
                        self.elicitations
                            .iter()
                            .filter(|(_, elicitation)| elicitation.rpc_id == params["requestId"])
                            .map(|(id, _)| id.clone()),
                    )
                    .collect();
                for id in expired {
                    if self.port.is_some() {
                        self.cancel_one_interaction(&id)?;
                    } else {
                        self.reply(&id, RuntimeInteractionReply::Cancel)?;
                    }
                }
            } else if let Some(id) = record.get("id") {
                self.reject_auto(id, "Unsupported client capability")?;
            }
            return Ok(());
        }
        if self.prompt_id.is_some() && record["id"].as_u64() == self.prompt_id {
            self.prompt_id = None;
            if self.port.is_none() {
                self.cancel_permissions()?;
            }
            if let Some(error) = record.get("error") {
                self.notice(&rpc_error(error)?.to_string());
                self.finish(RunState::Failed);
            } else {
                let response: wire::PromptResponse =
                    serde_json::from_value(record["result"].clone())
                        .map_err(|error| Error::new(format!("ACP prompt 响应无效：{error}")))?;
                match response.stop_reason {
                    wire::StopReason::EndTurn => {}
                    wire::StopReason::Cancelled => self.notice("本轮已取消。"),
                    wire::StopReason::MaxTokens => self.notice("Agent 达到本轮输出上限。"),
                    wire::StopReason::MaxTurnRequests => {
                        self.notice("Agent 达到本轮请求次数上限。")
                    }
                    wire::StopReason::Refusal => {
                        self.notice("Agent 拒绝本轮请求；本轮内容不会进入其下一轮上下文。")
                    }
                    _ => return Err(Error::new("ACP prompt 响应缺少可识别的结束原因")),
                }
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot
                        .executions
                        .finish(prompt_outcome(&response.stop_reason));
                }
                self.finish(RunState::Idle);
            }
            if self.port.is_some() {
                self.cancel_permissions()?;
            }
        }
        Ok(())
    }

    fn cancel_permissions(&mut self) -> Result<()> {
        if self.port.is_some() {
            return self.port_cancel_interactions();
        }
        for permission in std::mem::take(&mut self.permissions).into_values() {
            self.rpc.respond(
                &permission.rpc_id,
                &json!({"outcome":{"outcome":"cancelled"}}),
            )?;
        }
        for elicitation in std::mem::take(&mut self.elicitations).into_values() {
            self.rpc
                .respond(&elicitation.rpc_id, &json!({"action":"cancel"}))?;
        }
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.pending_interactions.clear();
            snapshot.revision += 1;
        }
        Ok(())
    }

    fn ensure_agent_request_id_available(&self, rpc_id: &Value) -> Result<()> {
        if self
            .permissions
            .values()
            .any(|pending| pending.rpc_id == *rpc_id)
            || self
                .elicitations
                .values()
                .any(|pending| pending.rpc_id == *rpc_id)
        {
            return Err(Error::new("ACP Agent 请求身份重复"));
        }
        Ok(())
    }

    fn elicitation(&mut self, rpc_id: &Value, params: &Value) -> Result<()> {
        self.interaction_capacity()?;
        self.ensure_agent_request_id_available(rpc_id)?;
        let parsed = serde_json::from_value::<wire::CreateElicitationRequest>(params.clone());
        let valid_scope = parsed.as_ref().is_ok_and(|request| match request.scope() {
            wire::ElicitationScope::Session(scope) => {
                self.native_id.as_deref() == Some(scope.session_id.0.as_ref())
                    && self
                        .snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.run_state != RunState::Stopping)
            }
            _ => false,
        });
        // Auth/configuration forms have no conversation surface yet. Explicitly decline
        // their unsupported scope rather than blocking the synchronous control request.
        if !valid_scope {
            return self.reject_auto(rpc_id, "Unsupported or inactive elicitation scope");
        }
        let questions = match crate::elicitation::questions(params) {
            Ok(questions) => questions,
            Err(error) => return self.reject_auto(rpc_id, &error.to_string()),
        };
        let id = self.next_id("elicitation");
        self.elicitations.insert(
            id.clone(),
            Elicitation {
                parent_deadline: self.interaction_parent_deadline(),
                rpc_id: rpc_id.clone(),
                params: params.clone(),
            },
        );
        let snapshot = self
            .snapshot
            .as_mut()
            .expect("validated elicitation session");
        snapshot.pending_interactions.push(RuntimeInteraction {
            id,
            kind: InteractionKind::UserInput { questions },
        });
        snapshot.revision += 1;
        Ok(())
    }

    fn permission(&mut self, rpc_id: &Value, params: &Value) -> Result<()> {
        self.interaction_capacity()?;
        self.ensure_agent_request_id_available(rpc_id)?;
        let _: wire::RequestId = serde_json::from_value(rpc_id.clone())
            .map_err(|error| Error::new(format!("ACP 请求身份无效：{error}")))?;
        let params: wire::RequestPermissionRequest = serde_json::from_value(params.clone())
            .map_err(|error| Error::new(format!("ACP 权限请求无效：{error}")))?;
        let mut seen = std::collections::BTreeSet::new();
        let options = params
            .options
            .into_iter()
            .map(|option| {
                let option = project_permission_option(option);
                if !seen.insert(option.id.clone()) {
                    return Err(Error::new("ACP 权限选项身份重复"));
                }
                Ok(option)
            })
            .collect::<Result<Vec<_>>>()?;
        if options.is_empty() {
            return Err(Error::new("ACP 权限请求没有选项"));
        }
        let id = self.next_id("permission");
        self.permissions.insert(
            id.clone(),
            Permission {
                parent_deadline: self.interaction_parent_deadline(),
                rpc_id: rpc_id.clone(),
                options: options.iter().map(|option| option.id.clone()).collect(),
            },
        );
        let snapshot = self
            .snapshot
            .as_mut()
            .expect("permission belongs to active prompt");
        snapshot.pending_interactions.push(RuntimeInteraction {
            id,
            kind: InteractionKind::Approval {
                title: "运行时请求权限".into(),
                detail: params
                    .tool_call
                    .fields
                    .title
                    .unwrap_or_else(|| "请决定是否允许工具操作".into()),
                options,
            },
        });
        snapshot.revision += 1;
        Ok(())
    }

    fn ensure_sync(&self) -> Result<()> {
        if self.port.is_some() {
            return Err(Error::with_code(
                "ACP command port owns this connection; use typed enqueue",
                "needs_preparation",
            ));
        }
        Ok(())
    }
    fn ensure_idle(&self) -> Result<()> {
        self.ensure_sync()?;
        if self.failed {
            return Err(Error::new("ACP 连接已关闭，请重新连接"));
        }
        if self.turn_running() {
            return Err(Error::new("ACP 会话仍在执行"));
        }
        Ok(())
    }
    fn session_id(&self) -> Result<&str> {
        self.native_id
            .as_deref()
            .ok_or_else(|| Error::new("尚未创建或加载 ACP 会话"))
    }
    fn prepare_session(&mut self, cwd: &Path) -> Result<()> {
        self.ensure_idle()?;
        if !cwd.is_absolute() || !self.config.execution.is_directory(cwd)? {
            return Err(Error::new("ACP 工作目录必须是目标机器上的已有绝对目录"));
        }
        if let Some(id) = self.native_id.clone() {
            if !self.capabilities.close_session {
                return Err(Error::new(
                    "此 ACP Agent 不支持关闭会话，请为新会话创建连接",
                ));
            }
            self.request::<wire::CloseSessionResponse>("session/close", &json!({"sessionId":id}))?;
        }
        self.clear_session();
        Ok(())
    }
    fn clear_session(&mut self) {
        self.native_id = None;
        self.snapshot = None;
        self.options.clear();
        self.stream = None;
        self.initial_updates.drain();
        self.permissions.clear();
        self.elicitations.clear();
        self.pending_user_echo = None;
        self._conversation_data = None;
        self._session_data = None;
        self._options_data = None;
        self._snapshot_data.clear();
        self._interaction_data.clear();
        self._initial_update_data.clear();
    }
    fn fail(&mut self) {
        if self.port.is_none() {
            self.rpc.shutdown();
        }
        self.failed = true;
        self.prompt_id = None;
        self.finish(RunState::Failed);
        self.notice("ACP 控制连接已终止；已有消息仍保留，请重新连接。");
    }
    fn finish(&mut self, state: RunState) {
        if let Some(snapshot) = &mut self.snapshot {
            snapshot
                .executions
                .finish((state == RunState::Failed).then_some(TurnOutcome::Failed));
        }
        if self.port.is_none() {
            self.permissions.clear();
            self.elicitations.clear();
            if let Some(snapshot) = &mut self.snapshot {
                snapshot.pending_interactions.clear();
            }
        }
        self.complete_messages();
        self.stream = None;
        self.pending_user_echo = None;
        self.state(state, !self.failed, false);
    }
}

fn project_permission_option(option: wire::PermissionOption) -> InteractionOption {
    InteractionOption {
        id: option.option_id.to_string(),
        label: option.name,
        approval_kind: approval_option_kind(option.kind),
    }
}

pub(crate) fn approval_option_kind(kind: wire::PermissionOptionKind) -> Option<ApprovalOptionKind> {
    use wire::PermissionOptionKind;
    Some(match kind {
        PermissionOptionKind::AllowOnce => ApprovalOptionKind::AllowOnce,
        PermissionOptionKind::RejectOnce => ApprovalOptionKind::RejectOnce,
        PermissionOptionKind::AllowAlways => ApprovalOptionKind::AllowAlways,
        PermissionOptionKind::RejectAlways => ApprovalOptionKind::RejectAlways,
        _ => return None,
    })
}

fn unsupported(method: &str) -> Error {
    Error::with_code(format!("此 ACP Agent 未公布 {method} 能力"), "unsupported")
}
fn rpc_error(error: &Value) -> Result<Error> {
    let error: wire::Error = serde_json::from_value(error.clone())
        .map_err(|error| Error::new(format!("ACP 错误响应无效：{error}")))?;
    let category = match error.code {
        wire::ErrorCode::MethodNotFound => "unsupported",
        wire::ErrorCode::AuthRequired => "authentication_required",
        _ => "rpc",
    };
    Ok(Error::with_code(
        format!("ACP 请求失败（{}）：{error}", i32::from(error.code)),
        category,
    ))
}
fn timestamp(value: &str) -> Option<i64> {
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .ok()
        .and_then(|time| i64::try_from(time.unix_timestamp_nanos() / 1_000_000).ok())
}

/// Explicit stop reasons; limits and refusals ended the request without proving success.
pub(crate) fn prompt_outcome(reason: &wire::StopReason) -> Option<TurnOutcome> {
    match reason {
        wire::StopReason::EndTurn => Some(TurnOutcome::Completed),
        wire::StopReason::Cancelled => Some(TurnOutcome::Cancelled),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_projection_uses_effect_declared_by_agent() {
        let projected = |id: &'static str, kind| {
            let option = project_permission_option(wire::PermissionOption::new(id, id, kind));
            assert_eq!(option.id, id);
            option.approval_kind
        };
        assert_eq!(
            projected("allow", wire::PermissionOptionKind::AllowAlways),
            Some(ApprovalOptionKind::AllowAlways)
        );
        assert_eq!(
            projected("not-an-allow-id", wire::PermissionOptionKind::AllowOnce),
            Some(ApprovalOptionKind::AllowOnce)
        );
    }

    #[test]
    fn rpc_error_keeps_code_message_and_native_data() {
        let error = rpc_error(&json!({"code":-32000,"message":"synthetic unavailable",
            "data":{"upstreamCode":"server_overloaded","requestId":"synthetic"}}))
        .unwrap();
        let text = error.to_string();
        for value in [
            "-32000",
            "synthetic unavailable",
            "server_overloaded",
            "requestId",
        ] {
            assert!(text.contains(value));
        }
        assert!(rpc_error(&json!({"code":"invalid","message":"bad"})).is_err());
    }

    pub(super) fn synthetic_client(mode: &str) -> (Client, PathBuf) {
        let mut random = [0; 8];
        getrandom::fill(&mut random).unwrap();
        let root =
            std::env::temp_dir().join(format!("velune-acp-form-{:x}", u64::from_le_bytes(random)));
        std::fs::create_dir(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let helper = root.join("rpc.py");
        std::fs::write(&helper,r#"
import json,sys,os,time
mode=sys.argv[1]
root=os.path.dirname(__file__)
def marker(name): return os.path.exists(os.path.join(root,name))
def wait(name):
    while not marker(name): time.sleep(0.005)
pending = None
count = 0
seen = set()
def emit(value): print(json.dumps(dict(jsonrpc='2.0',**value)),flush=True)
def chunk(text,session='synthetic'):
    emit({'method':'session/update','params':{'sessionId':session,'update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':text}}}})
def form():
    emit({'id':'q','method':'elicitation/create','params':{'mode':'form','sessionId':'synthetic','message':'Value?', 'requestedSchema':{'type':'object','required':['value'],'properties':{'value':{'type':'string'}}}}})
def interactions():
    emit({'id':'p','method':'session/request_permission','params':{'sessionId':'synthetic','toolCall':{'toolCallId':'t','title':'Synthetic tool'},'options':[{'optionId':'yes','name':'Yes','kind':'allow_once'}]}})
    emit({'id':'q','method':'elicitation/create','params':{'mode':'form','sessionId':'synthetic','message':'Value?', 'requestedSchema':{'type':'object','required':['value'],'properties':{'value':{'type':'string'}}}}})
def options(value='model/a'):
    return [{'id':'opaque/model-id','name':'Model','category':'model','type':'select','currentValue':value,'options':[{'value':'model/a','name':'A'},{'value':'model/b','name':'B'}]}]

for line in sys.stdin:
    request=json.loads(line)
    method=request.get('method')
    params=request.get('params',{})
    if method=='initialize':
        assert params['clientCapabilities']['elicitation']['form']=={}
        result={'protocolVersion':1,'agentCapabilities':{'loadSession':True,'sessionCapabilities':{'list':{},'resume':{},'close':{},'delete':{}}},'authMethods':[]}
    elif method=='session/new':
        if mode=='history':
            chunk('first')
            chunk('ignored','unknown')
            chunk(' second')
        result={'sessionId':'synthetic','configOptions':options()}
        if mode=='idle-form':
            emit({'id':request['id'],'result':result})
            form()
            wait('release')
            continue
        if mode=='blocked':
            emit({'id':request['id'],'result':result})
            wait('updates')
            chunk('progress while stdin is blocked')
            interactions()
            wait('terminal')
            with open(os.path.join(root,'prompt-id')) as f: prompt_id=json.load(f)
            emit({'id':prompt_id,'result':{'stopReason':'end_turn'}})
            wait('release')
            continue
    elif method in ('session/load','session/resume') and mode=='reject-open':
        chunk('unconfirmed history',params['sessionId'])
        emit({'id':request['id'],'error':{'code':-32000,'message':'not opened','data':{'target':params['sessionId']}}})
        continue
    elif method=='session/load':
        chunk('authoritative replay',params['sessionId'])
        result={'configOptions':options('model/b')}
    elif method=='session/resume': result={'configOptions':options()}
    elif method=='session/list':
        result={'sessions':[]}
        if mode=='live':
            emit({'id':request['id'],'result':result})
            chunk('|')
            continue
    elif method=='session/set_config_option':
        assert params['configId']=='opaque/model-id'
        if mode=='control-budget':
            form()
            wait('release')
        if mode=='late-control':
            wait('release')
            emit({'id':request['id'],'result':{'configOptions':options('model/b')}})
            emit({'id':'after-deadline','method':'synthetic/unsupported','params':{}})
            with open(os.path.join(root,'terminal-emitted'),'w') as f: f.write('control')
            continue
        if mode=='reject':
            emit({'id':request['id'],'error':{'code':-32000,'message':'synthetic rejected','data':{'native':'kept'}}})
            continue
        result={'configOptions':options('model/b')}
        if mode=='accepted-disconnect':
            emit({'id':request['id'],'result':result})
            emit({'id':'unsupported','method':'synthetic/unsupported','params':{}})
            sys.exit(0)

    elif method=='session/prompt':
        if mode=='blocked': continue
        if mode=='prompt-error':
            chunk('output before error')
            emit({'id':request['id'],'error':{'code':-32000,'message':'after execution','data':{'toolRan':True}}})
            continue
        if mode=='disconnect':
            chunk('output before disconnect')
            sys.exit(0)
        if mode=='auto-short-write':
            form()
            wait('automatic')
            emit({'id':'wrong-session','method':'session/request_permission','params':{'sessionId':'other-session','toolCall':{'toolCallId':'t'},'options':[{'optionId':'yes','name':'Yes','kind':'allow_once'}]}})
            chunk('automatic cancellation required')
            wait('release')
            continue
        if mode in ('late-complete','late-error'):
            wait('release')
            if mode=='late-error': emit({'id':request['id'],'error':{'code':-32000,'message':'late after deadline','data':{'executed':True}}})
            else: emit({'id':request['id'],'result':{'stopReason':'end_turn'}})
            # Also request an automatic reply in this batch. The latched timeout
            # must forbid it without losing the actual terminal result above.
            emit({'id':'after-deadline','method':'synthetic/unsupported','params':{}})
            with open(os.path.join(root,'terminal-emitted'),'w') as f: f.write(mode)
            continue
        if mode=='hold':
            wait('release')
            emit({'id':request['id'],'result':{'stopReason':'end_turn'}})
            continue
        pending=request['id']
        if mode.startswith('budget-'):
            wait('after-write')
            chunk('late output')
            interactions()
            continue
        if mode in ('duplicate-permission-first','duplicate-elicitation-first'):
            permission={'id':'same','method':'session/request_permission','params':{'sessionId':'synthetic','toolCall':{'toolCallId':'t'},'options':[{'optionId':'yes','name':'Yes','kind':'allow_once'}]}}
            form={'id':'same','method':'elicitation/create','params':{'mode':'form','sessionId':'synthetic','message':'Value?', 'requestedSchema':{'type':'object','properties':{'value':{'type':'string'}}}}}
            for value in ([permission,form] if mode=='duplicate-permission-first' else [form,permission]): emit(value)
            continue
        if mode=='live':
            interactions()
            continue
        count+=1
        form={'mode':'form','sessionId':'synthetic','message':'How many?',
              'requestedSchema':{'type':'object','required':['count'],'properties':{'count':{'type':'integer','minimum':1}}}}
        emit({'id':'q','method':'elicitation/create','params':form})
        if count==2: emit({'method':'$/cancel_request','params':{'requestId':'q'}})
        continue
    elif method=='session/cancel':
        if mode=='live':
            seen.add('cancel')
            chunk('cancel delivered')
            assert seen=={'p','q','cancel'}
            emit({'id':pending,'result':{'stopReason':'cancelled'}})
        continue
    elif method is None and mode.startswith('late-'):
        with open(os.path.join(root,'unexpected-reply'),'w') as f: f.write('new dispatch after expiry')
        continue
    elif method is None and mode=='idle-form':
        assert request['id']=='q' and request['result']['content']['value']
        count+=1
        with open(os.path.join(root,'reply-count'),'w') as f: f.write(str(count))
        chunk('idle answer delivered')
        continue
    elif method is None and mode.startswith('budget-'):
        assert request['id'] not in seen
        seen.add(request['id'])
        chunk(request['id']+' delivered')
        if seen=={'p','q'}:
            wait('release')
            if mode=='budget-error': emit({'id':pending,'error':{'code':-32000,'message':'late turn error','data':{'executed':True}}})
            else: emit({'id':pending,'result':{'stopReason':'end_turn'}})
        continue
    elif method is None and mode.startswith('duplicate-'):
        with open(os.path.join(root,'duplicate-reply'),'w') as f: f.write('unexpected response')
        continue
    elif method is None and mode=='live':
        seen.add(request['id'])
        if request['id']=='p': assert request['result']['outcome']['optionId']=='yes'
        else: assert request['result']['content']['value']=='answer'
        chunk(request['id']+' delivered')
        continue
    elif method is None and mode=='blocked': continue
    elif method is None and request['id']=='q':
        if count==1: assert request['result']['content']['count']==2
        else: assert request['result']['action']=='cancel'
        emit({'id':pending,'result':{'stopReason':'end_turn'}})
        continue
    else: result={}
    emit({'id':request['id'],'result':result})
"#).unwrap();
        let client = Client::connect(Config {
            binary: "/usr/bin/python3".into(),
            args: vec![helper.to_string_lossy().into_owned(), mode.into()],
            environment: Default::default(),
            execution: ExecutionTarget::Local,
            os_home: Some(root.clone()),
            mcp_servers: vec![],
        })
        .unwrap();
        (client, root)
    }

    #[test]
    fn elicitation_form_negotiates_and_routes_typed_reply_and_cancellation() {
        let (mut client, root) = synthetic_client("form");
        client.create(&root, "synthetic-acp").unwrap();
        for turn in 0..3 {
            client.send("question", &[]).unwrap();
            let deadline = Instant::now() + std::time::Duration::from_secs(2);
            while client.snapshot().unwrap().pending_interactions.is_empty()
                && client.turn_running()
                && Instant::now() < deadline
            {
                client.poll().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            if turn != 1 {
                let interaction = client.snapshot().unwrap().pending_interactions[0].clone();
                assert!(matches!(
                    interaction.kind,
                    InteractionKind::UserInput { .. }
                ));
                if turn == 0 {
                    client
                        .reply(
                            &interaction.id,
                            RuntimeInteractionReply::Answers {
                                answers: vec![InteractionAnswer {
                                    question_id: "count".into(),
                                    values: vec!["2".into()],
                                }],
                            },
                        )
                        .unwrap();
                } else {
                    client.cancel().unwrap();
                }
            }
            while client.turn_running() && Instant::now() < deadline {
                client.poll().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(!client.turn_running());
            assert!(client.snapshot().unwrap().pending_interactions.is_empty());
        }
        // A request whose RPC connection disappears cannot survive as an
        // answerable form or be converted into an ordinary conversation input.
        client.send("question", &[]).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while client.snapshot().unwrap().pending_interactions.is_empty()
            && Instant::now() < deadline
        {
            client.poll().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let expired = client.snapshot().unwrap().pending_interactions[0]
            .id
            .clone();
        client.fail();
        assert!(client.snapshot().unwrap().pending_interactions.is_empty());
        assert!(
            client
                .reply(&expired, RuntimeInteractionReply::Cancel)
                .is_err()
        );
        client.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    fn command_deadline() -> Instant {
        Instant::now() + std::time::Duration::from_secs(4)
    }

    fn drive_port(
        client: &mut Client,
        done: impl Fn(&Client, &[crate::creation::Retained<AcpReceipt>]) -> bool,
    ) -> Vec<crate::creation::Retained<AcpReceipt>> {
        let deadline = command_deadline();
        let mut receipts = Vec::new();
        loop {
            client.poll().unwrap();
            receipts.extend(client.take_receipts());
            if done(client, &receipts) {
                return receipts;
            }
            assert!(
                Instant::now() < deadline,
                "ACP synthetic port did not progress: {receipts:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn acp_port_blocked_stdin_keeps_poll_and_replies_live_until_written() {
        let (mut client, root) = synthetic_client("blocked");
        client.create(&root, "synthetic-acp").unwrap();
        client.enable_command_port().unwrap();
        let before = Instant::now();
        let prompt = client
            .enqueue(
                AcpCommand::Prompt {
                    text: "x".repeat(200_000),
                    images: vec![],
                    run_deadline: None,
                },
                command_deadline(),
            )
            .unwrap();
        assert!(before.elapsed() < std::time::Duration::from_millis(500));
        std::fs::write(
            root.join("prompt-id"),
            serde_json::to_vec(&prompt.wire_id).unwrap(),
        )
        .unwrap();
        std::fs::write(root.join("updates"), []).unwrap();
        let receipts = drive_port(&mut client, |client, _| {
            client.snapshot().unwrap().pending_interactions.len() == 2
        });
        assert!(
            !receipts.iter().any(
                |receipt| receipt.ticket == prompt && receipt.stage == AcpReceiptStage::Written
            )
        );
        assert!(client.snapshot().unwrap().messages.iter().any(|message| message.blocks.iter().any(|block|
            matches!(block, MessageBlock::Text { text } if text == "progress while stdin is blocked"))));
        let interactions = client.snapshot().unwrap().pending_interactions.clone();
        let permission = interactions
            .iter()
            .find(|i| matches!(i.kind, InteractionKind::Approval { .. }))
            .unwrap()
            .id
            .clone();
        let elicitation = interactions
            .iter()
            .find(|i| matches!(i.kind, InteractionKind::UserInput { .. }))
            .unwrap()
            .id
            .clone();
        let answer = client
            .enqueue(
                AcpCommand::ReplyElicitation {
                    interaction_id: elicitation.clone(),
                    reply: RuntimeInteractionReply::Answers {
                        answers: vec![InteractionAnswer {
                            question_id: "value".into(),
                            values: vec!["answer".into()],
                        }],
                    },
                },
                command_deadline(),
            )
            .unwrap();
        assert!(
            client
                .enqueue(
                    AcpCommand::ReplyElicitation {
                        interaction_id: elicitation,
                        reply: RuntimeInteractionReply::Cancel
                    },
                    command_deadline()
                )
                .is_err()
        );
        let cancel = client
            .enqueue(AcpCommand::Cancel, command_deadline())
            .unwrap();
        client.poll().unwrap();
        assert_eq!(client.snapshot().unwrap().pending_interactions.len(), 2);
        std::fs::write(root.join("terminal"), []).unwrap();
        let terminal = drive_port(&mut client, |client, receipts| {
            !client.turn_running()
                && receipts.iter().any(|receipt| {
                    receipt.ticket == prompt
                        && matches!(receipt.stage, AcpReceiptStage::TurnCompleted { .. })
                })
        });
        assert!(
            !terminal
                .iter()
                .any(|receipt| receipt.stage == AcpReceiptStage::Written)
        );
        // Terminal parent is not permission delivery. Both interactions and their
        // write leases stay tracked until the blocked stream is actually drained.
        assert_eq!(client.snapshot().unwrap().pending_interactions.len(), 2);
        assert!(matches!(
            client.enqueue(AcpCommand::Close, command_deadline()),
            Err(AcpAdmissionError::NeedsPreparation(_))
        ));
        assert!(
            client
                .enqueue(
                    AcpCommand::ReplyPermission {
                        interaction_id: permission,
                        reply: RuntimeInteractionReply::Cancel
                    },
                    command_deadline()
                )
                .is_err()
        );
        std::fs::write(root.join("release"), []).unwrap();
        let settled = drive_port(&mut client, |client, receipts| {
            client.snapshot().unwrap().pending_interactions.is_empty()
                && receipts
                    .iter()
                    .any(|r| r.ticket == cancel && r.stage == AcpReceiptStage::Written)
                && receipts
                    .iter()
                    .any(|r| r.ticket == answer && r.stage == AcpReceiptStage::Written)
        });
        assert!(
            settled
                .iter()
                .any(|r| r.ticket == prompt && r.stage == AcpReceiptStage::Written)
        );
        assert!(!client.needs_cleanup());
        client.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn acp_port_native_rejection_is_not_prompt_turn_error_or_disconnect() {
        for mode in ["reject", "prompt-error", "disconnect"] {
            let (mut client, root) = synthetic_client(mode);
            client.create(&root, "synthetic-acp").unwrap();
            client.enable_command_port().unwrap();
            assert!(client.send("sync must not mutate", &[]).is_err());
            assert!(client.cancel().is_err());
            let command = if mode == "reject" {
                AcpCommand::SetConfig {
                    config_id: "opaque/model-id".into(),
                    value: ConfigValue::Select("model/b".into()),
                }
            } else {
                AcpCommand::Prompt {
                    text: "synthetic".into(),
                    images: vec![],
                    run_deadline: None,
                }
            };
            let ticket = client.enqueue(command, command_deadline()).unwrap();
            let deadline = command_deadline();
            let mut receipts = Vec::new();
            let mut failure = None;
            while Instant::now() < deadline {
                let outcome = client.poll();
                receipts.extend(client.take_receipts());
                if let Err(error) = outcome {
                    failure = Some(error);
                    break;
                }
                if receipts
                    .iter()
                    .any(|r| r.ticket == ticket && !matches!(r.stage, AcpReceiptStage::Written))
                {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(
                receipts
                    .iter()
                    .any(|r| r.ticket == ticket && r.stage == AcpReceiptStage::Written),
                "{mode}: {receipts:?}"
            );
            match mode {
                "reject" => {
                    assert!(receipts.iter().any(|r| matches!(&r.stage, AcpReceiptStage::ProtocolRejected { error }
                        if error.code == -32000 && error.message == "synthetic rejected" && error.data == Some(json!({"native":"kept"})))));
                    assert_eq!(
                        client.config_options()[0].value,
                        ConfigValue::Select("model/a".into())
                    );
                    assert!(!client.needs_cleanup());
                }
                "prompt-error" => {
                    assert!(
                        receipts
                            .iter()
                            .any(|r| matches!(&r.stage, AcpReceiptStage::TurnError { error }
                        if error.data == Some(json!({"toolRan":true}))))
                    );
                    assert!(
                        !receipts
                            .iter()
                            .any(|r| matches!(r.stage, AcpReceiptStage::ProtocolRejected { .. }))
                    );
                }
                _ => {
                    assert!(failure.is_some());
                    assert!(
                        receipts
                            .iter()
                            .any(|r| matches!(r.stage, AcpReceiptStage::Indeterminate { .. }))
                    );
                    assert!(client.needs_cleanup());
                    client.poll().unwrap();
                    assert!(client.needs_cleanup());
                    assert!(matches!(
                        client.enqueue(AcpCommand::Cancel, command_deadline()),
                        Err(AcpAdmissionError::NeedsPreparation(_))
                    ));
                }
            }
            client.shutdown().unwrap();
            assert!(!client.needs_cleanup());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn acp_port_new_load_replay_and_opaque_config_are_authoritative() {
        let (mut client, root) = synthetic_client("history");
        client.enable_command_port().unwrap();
        let ticket = client
            .enqueue(
                AcpCommand::New {
                    cwd: root.clone(),
                    runtime_id: "synthetic-acp".into(),
                },
                command_deadline(),
            )
            .unwrap();
        drive_port(&mut client, |_, receipts| {
            receipts.iter().any(|r| {
                r.ticket == ticket && matches!(r.stage, AcpReceiptStage::ProtocolSuccess { .. })
            })
        });
        let messages = &client.snapshot().unwrap().messages;
        assert_eq!(messages.len(), 1);
        assert!(
            matches!(&messages[0].blocks[0], MessageBlock::Text { text } if text == "first second")
        );
        assert_eq!(client.models().unwrap().len(), 2);
        let config = client
            .enqueue(
                AcpCommand::SetConfig {
                    config_id: "opaque/model-id".into(),
                    value: ConfigValue::Select("model/a".into()),
                },
                command_deadline(),
            )
            .unwrap();
        drive_port(&mut client, |_, receipts| {
            receipts.iter().any(|r| {
                r.ticket == config && matches!(r.stage, AcpReceiptStage::ProtocolSuccess { .. })
            })
        });
        // Returned options, not the submitted value, are authoritative; no restart/session probe.
        assert_eq!(
            client.config_options()[0].value,
            ConfigValue::Select("model/b".into())
        );
        assert_eq!(
            client.snapshot().unwrap().conversation.id,
            "synthetic-acp:synthetic"
        );
        let revision = client.snapshot().unwrap().revision;
        assert!(matches!(
            client.enqueue(
                AcpCommand::Load {
                    native_id: "loaded".into(),
                    cwd: root.clone(),
                    runtime_id: "synthetic-acp".into()
                },
                command_deadline()
            ),
            Err(AcpAdmissionError::NeedsPreparation(_))
        ));
        assert_eq!(client.snapshot().unwrap().revision, revision);
        for resume in [false, true] {
            let close = client
                .enqueue(AcpCommand::Close, command_deadline())
                .unwrap();
            drive_port(&mut client, |_, receipts| {
                receipts.iter().any(|r| {
                    r.ticket == close && matches!(r.stage, AcpReceiptStage::ProtocolSuccess { .. })
                })
            });
            let command = if resume {
                AcpCommand::Resume {
                    native_id: "loaded".into(),
                    cwd: root.clone(),
                    runtime_id: "synthetic-acp".into(),
                }
            } else {
                AcpCommand::Load {
                    native_id: "loaded".into(),
                    cwd: root.clone(),
                    runtime_id: "synthetic-acp".into(),
                }
            };
            let load = client.enqueue(command, command_deadline()).unwrap();
            drive_port(&mut client, |_, receipts| {
                receipts.iter().any(|r| {
                    r.ticket == load && matches!(r.stage, AcpReceiptStage::ProtocolSuccess { .. })
                })
            });
            let messages = &client.snapshot().unwrap().messages;
            if resume {
                assert!(!messages.iter().any(|message| {
                    message
                        .blocks
                        .iter()
                        .any(|block| matches!(block, MessageBlock::Text { .. }))
                }));
            } else {
                assert_eq!(messages.len(), 1);
                assert!(messages[0].completed);
                assert!(
                    matches!(&messages[0].blocks[0], MessageBlock::Text { text } if text == "authoritative replay")
                );
            }
        }
        client.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn acp_port_reused_wire_ids_stay_on_their_connection_generation() {
        let (mut old, old_root) = synthetic_client("hold");
        let (mut new, new_root) = synthetic_client("hold");
        old.create(&old_root, "synthetic-acp").unwrap();
        new.create(&new_root, "synthetic-acp").unwrap();
        old.enable_command_port().unwrap();
        new.enable_command_port().unwrap();
        let old_ticket = old
            .enqueue(
                AcpCommand::Prompt {
                    text: "old".into(),
                    images: vec![],
                    run_deadline: None,
                },
                command_deadline(),
            )
            .unwrap();
        let new_ticket = new
            .enqueue(
                AcpCommand::Prompt {
                    text: "new".into(),
                    images: vec![],
                    run_deadline: None,
                },
                command_deadline(),
            )
            .unwrap();
        assert_eq!(old_ticket.wire_id, new_ticket.wire_id);
        assert_ne!(old_ticket.generation, new_ticket.generation);
        assert_ne!(old_ticket.session, new_ticket.session);
        // Consuming Written cannot remove either still-live prompt operation.
        drive_port(&mut old, |_, receipts| {
            receipts.iter().any(|r| r.stage == AcpReceiptStage::Written)
        });
        drive_port(&mut new, |_, receipts| {
            receipts.iter().any(|r| r.stage == AcpReceiptStage::Written)
        });
        std::fs::write(old_root.join("release"), []).unwrap();
        drive_port(&mut old, |_, receipts| {
            receipts.iter().any(|r| {
                r.ticket == old_ticket && matches!(r.stage, AcpReceiptStage::TurnCompleted { .. })
            })
        });
        new.poll().unwrap();
        assert!(new.turn_running());
        assert!(new.take_receipts().is_empty());
        std::fs::write(new_root.join("release"), []).unwrap();
        drive_port(&mut new, |_, receipts| {
            receipts.iter().any(|r| {
                r.ticket == new_ticket && matches!(r.stage, AcpReceiptStage::TurnCompleted { .. })
            })
        });
        old.shutdown().unwrap();
        new.shutdown().unwrap();
        std::fs::remove_dir_all(old_root).unwrap();
        std::fs::remove_dir_all(new_root).unwrap();
    }
    #[test]
    fn acp_port_permission_elicitation_and_cancel_are_delivered_before_prompt_result() {
        let (mut client, root) = synthetic_client("live");
        client.create(&root, "synthetic-acp").unwrap();
        client.enable_command_port().unwrap();
        // Leave 23 completed control results unconsumed. Their receipt budget
        // must remain reserved, with separate space for replies to a live turn.
        for pages in 1..=23 {
            client
                .enqueue(
                    AcpCommand::List {
                        cwd: None,
                        cursor: None,
                    },
                    command_deadline(),
                )
                .unwrap();
            let deadline = command_deadline();
            loop {
                client.poll().unwrap();
                if client.snapshot().unwrap().messages.iter().any(|m| {
                    m.blocks
                        .iter()
                        .any(|b| matches!(b, MessageBlock::Text { text } if text.len() == pages))
                }) {
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let prompt = client
            .enqueue(
                AcpCommand::Prompt {
                    text: "question".into(),
                    images: vec![],
                    run_deadline: None,
                },
                command_deadline(),
            )
            .unwrap();
        let deadline = command_deadline();
        while client.snapshot().unwrap().pending_interactions.len() != 2 {
            client.poll().unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            matches!(client.enqueue(AcpCommand::List { cwd: None, cursor: None }, command_deadline()),
            Err(AcpAdmissionError::NotDispatched { code, .. }) if code == "queue_full")
        );
        let interactions = client.snapshot().unwrap().pending_interactions.clone();
        let mut replies = Vec::new();
        for interaction in interactions {
            let command = match interaction.kind {
                InteractionKind::Approval { .. } => AcpCommand::ReplyPermission {
                    interaction_id: interaction.id,
                    reply: RuntimeInteractionReply::Decision {
                        option_id: "yes".into(),
                    },
                },
                _ => AcpCommand::ReplyElicitation {
                    interaction_id: interaction.id,
                    reply: RuntimeInteractionReply::Answers {
                        answers: vec![InteractionAnswer {
                            question_id: "value".into(),
                            values: vec!["answer".into()],
                        }],
                    },
                },
            };
            replies.push(client.enqueue(command, command_deadline()).unwrap());
        }
        let written = drive_port(&mut client, |client, receipts| {
            client.snapshot().unwrap().pending_interactions.is_empty()
            && client.snapshot().unwrap().messages.iter().any(|m| m.blocks.iter().any(|b| matches!(b, MessageBlock::Text { text } if text.contains("q delivered"))))
            && replies.iter().all(|ticket| receipts.iter().any(|r| &r.ticket == ticket && r.stage == AcpReceiptStage::Written))
        });
        assert!(client.turn_running());
        assert!(
            !written
                .iter()
                .any(|r| (r.ticket == prompt || replies.contains(&r.ticket))
                    && matches!(
                        r.stage,
                        AcpReceiptStage::ProtocolSuccess { .. }
                            | AcpReceiptStage::TurnCompleted { .. }
                    ))
        );
        assert!(
            client.has_pending_operations(),
            "consuming Written does not free the prompt lifecycle"
        );
        let cancel = client
            .enqueue(AcpCommand::Cancel, command_deadline())
            .unwrap();
        let terminal = drive_port(&mut client, |client, receipts| {
            !client.turn_running()
            && receipts.iter().any(|r| r.ticket == prompt && matches!(&r.stage, AcpReceiptStage::TurnCompleted { stop_reason } if stop_reason == "cancelled"))
            && receipts.iter().any(|r| r.ticket == cancel && r.stage == AcpReceiptStage::Written)
        });
        assert!(
            !terminal.iter().any(|r| r.ticket == cancel
                && matches!(r.stage, AcpReceiptStage::ProtocolSuccess { .. }))
        );
        assert!(!client.has_pending_operations());
        client.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn acp_port_recorded_success_survives_terminal_barrier_and_optional_reply_failure() {
        let (mut client, root) = synthetic_client("accepted-disconnect");
        client.create(&root, "synthetic-acp").unwrap();
        client.enable_command_port().unwrap();
        let ticket = client
            .enqueue(
                AcpCommand::SetConfig {
                    config_id: "opaque/model-id".into(),
                    value: ConfigValue::Select("model/a".into()),
                },
                command_deadline(),
            )
            .unwrap();
        let deadline = command_deadline();
        let mut receipts = Vec::new();
        let mut failed = false;
        while Instant::now() < deadline {
            let result = client.poll();
            receipts.extend(client.take_receipts());
            if result.is_err() {
                failed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(failed);
        assert!(client.needs_cleanup());
        assert_eq!(
            client.config_options()[0].value,
            ConfigValue::Select("model/b".into())
        );
        assert!(
            receipts.iter().any(|r| r.ticket == ticket
                && matches!(r.stage, AcpReceiptStage::ProtocolSuccess { .. }))
        );
        assert!(
            receipts
                .iter()
                .any(|r| r.ticket == ticket && r.stage == AcpReceiptStage::Written)
        );
        assert!(!receipts.iter().any(
            |r| r.ticket == ticket && matches!(r.stage, AcpReceiptStage::Indeterminate { .. })
        ));
        client.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn acp_port_absolute_deadline_does_not_release_blocked_writer_or_interactions() {
        let (mut client, root) = synthetic_client("blocked");
        client.create(&root, "synthetic-acp").unwrap();
        client.enable_command_port().unwrap();
        let deadline = Instant::now() + std::time::Duration::from_millis(200);
        let prompt = client
            .enqueue(
                AcpCommand::Prompt {
                    text: "x".repeat(200_000),
                    images: vec![],
                    run_deadline: Some(deadline),
                },
                deadline,
            )
            .unwrap();
        std::fs::write(root.join("updates"), []).unwrap();
        drive_port(&mut client, |client, _| {
            client.snapshot().unwrap().pending_interactions.len() == 2
        });
        let interaction_id = client.snapshot().unwrap().pending_interactions[0]
            .id
            .clone();
        std::thread::sleep(
            deadline.saturating_duration_since(Instant::now())
                + std::time::Duration::from_millis(10),
        );
        assert!(
            matches!(client.enqueue(AcpCommand::ReplyPermission { interaction_id, reply: RuntimeInteractionReply::Cancel }, command_deadline()),
            Err(AcpAdmissionError::NotDispatched { code, .. }) if code == "timeout")
        );
        assert!(client.poll().is_err());
        assert!(client.needs_cleanup());
        assert!(client.has_pending_operations());
        assert_eq!(client.snapshot().unwrap().pending_interactions.len(), 2);
        let receipts = client.take_receipts();
        assert!(receipts.iter().any(
            |r| r.ticket == prompt && matches!(r.stage, AcpReceiptStage::Indeterminate { .. })
        ));
        assert!(!receipts.iter().any(|r| r.stage == AcpReceiptStage::Written));
        assert!(
            client.has_pending_operations(),
            "timeout is not cancellation of a real blocked write"
        );
        client.shutdown().unwrap();
        assert!(!client.has_pending_operations());
        std::fs::remove_dir_all(root).unwrap();
    }
    fn rejected_open_rolls_back(resume: bool) {
        let (mut client, root) = synthetic_client("reject-open");
        client.enable_command_port().unwrap();
        let command = if resume {
            AcpCommand::Resume {
                native_id: "rejected".into(),
                cwd: root.clone(),
                runtime_id: "synthetic-acp".into(),
            }
        } else {
            AcpCommand::Load {
                native_id: "rejected".into(),
                cwd: root.clone(),
                runtime_id: "synthetic-acp".into(),
            }
        };
        let ticket = client.enqueue(command, command_deadline()).unwrap();
        drive_port(&mut client, |_, receipts| {
            receipts.iter().any(|r| {
                r.ticket == ticket && matches!(r.stage, AcpReceiptStage::ProtocolRejected { .. })
            })
        });
        assert!(
            client.snapshot().is_none(),
            "rejected identity/history must not remain published"
        );
        assert!(
            client
                .enqueue(
                    AcpCommand::Prompt {
                        text: "must not reach rejected ID".into(),
                        images: vec![],
                        run_deadline: None
                    },
                    command_deadline()
                )
                .is_err()
        );
        assert!(!client.needs_cleanup());
        let retry = client
            .enqueue(
                AcpCommand::New {
                    cwd: root.clone(),
                    runtime_id: "synthetic-acp".into(),
                },
                command_deadline(),
            )
            .unwrap();
        drive_port(&mut client, |_, receipts| {
            receipts.iter().any(|r| {
                r.ticket == retry && matches!(r.stage, AcpReceiptStage::ProtocolSuccess { .. })
            })
        });
        assert_eq!(
            client.snapshot().unwrap().conversation.id,
            "synthetic-acp:synthetic"
        );
        client.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn acp_port_rejected_load_revokes_unconfirmed_identity_and_history() {
        rejected_open_rolls_back(false);
    }

    #[test]
    fn acp_port_rejected_resume_revokes_unconfirmed_identity_and_history() {
        rejected_open_rolls_back(true);
    }

    #[test]
    fn acp_port_agent_request_ids_are_unique_across_interaction_kinds_in_both_orders() {
        for mode in ["duplicate-permission-first", "duplicate-elicitation-first"] {
            let (mut client, root) = synthetic_client(mode);
            client.create(&root, "synthetic-acp").unwrap();
            client.enable_command_port().unwrap();
            client
                .enqueue(
                    AcpCommand::Prompt {
                        text: "question".into(),
                        images: vec![],
                        run_deadline: None,
                    },
                    command_deadline(),
                )
                .unwrap();
            let deadline = command_deadline();
            let error = loop {
                if let Err(error) = client.poll() {
                    break error;
                }
                assert!(
                    Instant::now() < deadline,
                    "second duplicate interaction must fail"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            assert!(error.to_string().contains("身份重复"));
            assert!(client.needs_cleanup());
            assert_eq!(client.snapshot().unwrap().pending_interactions.len(), 1);
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert!(!root.join("duplicate-reply").exists());
            client.shutdown().unwrap();
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn acp_idle_human_form_has_no_implicit_ttl_in_port_or_sync_facade() {
        let (mut port, port_root) = synthetic_client("idle-form");
        let (mut sync, sync_root) = synthetic_client("idle-form");
        port.create(&port_root, "synthetic-acp").unwrap();
        port.enable_command_port().unwrap();
        sync.create(&sync_root, "synthetic-acp").unwrap();
        drive_port(&mut port, |client, _| {
            !client.snapshot().unwrap().pending_interactions.is_empty()
        });
        let deadline = command_deadline();
        while sync.snapshot().unwrap().pending_interactions.is_empty() {
            sync.poll().unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            port.elicitations.values().next().unwrap().parent_deadline,
            None
        );
        assert_eq!(
            sync.elicitations.values().next().unwrap().parent_deadline,
            None
        );
        // Real elapsed time is intentional: this is the original 15s TTL regression.
        std::thread::sleep(CONTROL_TIMEOUT + std::time::Duration::from_millis(100));
        port.poll().unwrap();
        sync.poll().unwrap();
        assert_eq!(port.snapshot().unwrap().pending_interactions.len(), 1);
        assert_eq!(sync.snapshot().unwrap().pending_interactions.len(), 1);
        assert!(!port.needs_cleanup());
        let id = port.snapshot().unwrap().pending_interactions[0].id.clone();
        let ticket = port
            .enqueue(
                AcpCommand::ReplyElicitation {
                    interaction_id: id,
                    reply: RuntimeInteractionReply::Answers {
                        answers: vec![InteractionAnswer {
                            question_id: "value".into(),
                            values: vec!["x".repeat(200_000)],
                        }],
                    },
                },
                command_deadline(),
            )
            .unwrap();
        port.poll().unwrap();
        assert_eq!(
            port.snapshot().unwrap().pending_interactions.len(),
            1,
            "a queued answer is not a written answer"
        );
        assert!(!port.take_receipts().iter().any(|r| r.ticket == ticket));
        std::fs::write(port_root.join("release"), []).unwrap();
        let receipts = drive_port(&mut port, |client, receipts| {
            client.snapshot().unwrap().pending_interactions.is_empty()
                && receipts
                    .iter()
                    .any(|r| r.ticket == ticket && r.stage == AcpReceiptStage::Written)
                // Written does not prove the peer has published its counter:
                // opening/truncating the file can make it visible while empty.
                && std::fs::read_to_string(port_root.join("reply-count"))
                    .is_ok_and(|contents| contents == "1")
        });
        assert_eq!(receipts.iter().filter(|r| r.ticket == ticket).count(), 1);
        assert_eq!(
            std::fs::read_to_string(port_root.join("reply-count")).unwrap(),
            "1"
        );
        std::fs::write(sync_root.join("release"), []).unwrap();
        let id = sync.snapshot().unwrap().pending_interactions[0].id.clone();
        sync.reply(
            &id,
            RuntimeInteractionReply::Answers {
                answers: vec![InteractionAnswer {
                    question_id: "value".into(),
                    values: vec!["legacy answer".into()],
                }],
            },
        )
        .unwrap();
        assert!(sync.snapshot().unwrap().pending_interactions.is_empty());
        port.shutdown().unwrap();
        sync.shutdown().unwrap();
        std::fs::remove_dir_all(port_root).unwrap();
        std::fs::remove_dir_all(sync_root).unwrap();
    }

    #[test]
    fn acp_prompt_written_only_settles_dispatch_budget_not_run_or_human_input() {
        for mode in ["budget-complete", "budget-error"] {
            let (mut client, root) = synthetic_client(mode);
            client.create(&root, "synthetic-acp").unwrap();
            client.enable_command_port().unwrap();
            let write_deadline = Instant::now() + std::time::Duration::from_millis(350);
            let ticket = client
                .enqueue(
                    AcpCommand::Prompt {
                        text: "once".into(),
                        images: vec![],
                        run_deadline: None,
                    },
                    write_deadline,
                )
                .unwrap();
            let written = drive_port(&mut client, |_, receipts| {
                receipts
                    .iter()
                    .any(|r| r.ticket == ticket && r.stage == AcpReceiptStage::Written)
            });
            std::thread::sleep(
                write_deadline.saturating_duration_since(Instant::now())
                    + std::time::Duration::from_millis(20),
            );
            std::fs::write(root.join("after-write"), []).unwrap();
            drive_port(&mut client, |client, _| {
                client.snapshot().unwrap().pending_interactions.len() == 2
            });
            assert!(client.turn_running());
            assert!(client.has_pending_operations());
            assert!(matches!(
                client.enqueue(
                    AcpCommand::Prompt {
                        text: "not a retry".into(),
                        images: vec![],
                        run_deadline: None
                    },
                    command_deadline()
                ),
                Err(AcpAdmissionError::NeedsPreparation(_))
            ));
            assert!(client.snapshot().unwrap().messages.iter().any(|m| {
                m.blocks
                    .iter()
                    .any(|b| matches!(b, MessageBlock::Text { text } if text == "late output"))
            }));
            assert!(
                client
                    .permissions
                    .values()
                    .all(|pending| pending.parent_deadline.is_none())
            );
            assert!(
                client
                    .elicitations
                    .values()
                    .all(|pending| pending.parent_deadline.is_none())
            );
            let interactions = client.snapshot().unwrap().pending_interactions.clone();
            let mut tickets = Vec::new();
            for interaction in interactions {
                let command = if matches!(interaction.kind, InteractionKind::Approval { .. }) {
                    AcpCommand::ReplyPermission {
                        interaction_id: interaction.id,
                        reply: RuntimeInteractionReply::Decision {
                            option_id: "yes".into(),
                        },
                    }
                } else {
                    AcpCommand::ReplyElicitation {
                        interaction_id: interaction.id,
                        reply: RuntimeInteractionReply::Answers {
                            answers: vec![InteractionAnswer {
                                question_id: "value".into(),
                                values: vec!["answer".into()],
                            }],
                        },
                    }
                };
                tickets.push(client.enqueue(command, command_deadline()).unwrap());
            }
            drive_port(&mut client, |client, receipts| {
                client.snapshot().unwrap().pending_interactions.is_empty()
                    && tickets.iter().all(|ticket| {
                        receipts
                            .iter()
                            .any(|r| &r.ticket == ticket && r.stage == AcpReceiptStage::Written)
                    })
            });
            assert!(client.turn_running());
            assert!(client.has_pending_operations());
            std::fs::write(root.join("release"), []).unwrap();
            let terminal = drive_port(&mut client, |client, receipts| {
                !client.turn_running()
                    && receipts.iter().any(|r| {
                        r.ticket == ticket
                            && matches!(
                                r.stage,
                                AcpReceiptStage::TurnCompleted { .. }
                                    | AcpReceiptStage::TurnError { .. }
                            )
                    })
            });
            assert_eq!(written.iter().filter(|r| r.ticket == ticket).count(), 1);
            assert_eq!(terminal.iter().filter(|r| r.ticket == ticket).count(), 1);
            assert!(
                matches!(
                    &terminal.iter().find(|r| r.ticket == ticket).unwrap().stage,
                    AcpReceiptStage::TurnCompleted { .. }
                ) == (mode == "budget-complete")
            );
            assert!(!client.has_pending_operations());
            assert!(!client.needs_cleanup());
            client.shutdown().unwrap();
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn acp_explicit_run_and_control_deadlines_preserve_written_and_require_cleanup() {
        for control in [false, true] {
            let (mut client, root) = synthetic_client(if control {
                "control-budget"
            } else {
                "budget-explicit"
            });
            client.create(&root, "synthetic-acp").unwrap();
            client.enable_command_port().unwrap();
            let deadline = Instant::now() + std::time::Duration::from_millis(600);
            let command = if control {
                AcpCommand::SetConfig {
                    config_id: "opaque/model-id".into(),
                    value: ConfigValue::Select("model/b".into()),
                }
            } else {
                AcpCommand::Prompt {
                    text: "explicit budget".into(),
                    images: vec![],
                    run_deadline: Some(deadline),
                }
            };
            let ticket = client
                .enqueue(
                    command,
                    if control {
                        deadline
                    } else {
                        command_deadline()
                    },
                )
                .unwrap();
            let written = drive_port(&mut client, |_, receipts| {
                receipts
                    .iter()
                    .any(|r| r.ticket == ticket && r.stage == AcpReceiptStage::Written)
            });
            if !control {
                std::fs::write(root.join("after-write"), []).unwrap();
            }
            drive_port(&mut client, |client, _| {
                !client.snapshot().unwrap().pending_interactions.is_empty()
            });
            let parent = client.elicitations.values().next().unwrap().parent_deadline;
            assert_eq!(
                parent,
                if control { None } else { Some(deadline) },
                "session scope must not guess a control parent"
            );
            if !control {
                let permission_id = client.permissions.keys().next().unwrap().clone();
                let reply = client
                    .enqueue(
                        AcpCommand::ReplyPermission {
                            interaction_id: permission_id.clone(),
                            reply: RuntimeInteractionReply::Decision {
                                option_id: "yes".into(),
                            },
                        },
                        command_deadline(),
                    )
                    .unwrap();
                assert!(matches!(client.enqueue(AcpCommand::ReplyPermission {
                    interaction_id: permission_id,
                    reply: RuntimeInteractionReply::Cancel,
                }, command_deadline()), Err(AcpAdmissionError::NotDispatched { code, .. }) if code == "already_pending"));
                drive_port(&mut client, |_, receipts| {
                    receipts
                        .iter()
                        .any(|r| r.ticket == reply && r.stage == AcpReceiptStage::Written)
                });
                assert_eq!(
                    client.elicitations.values().next().unwrap().parent_deadline,
                    Some(deadline)
                );
            }
            std::thread::sleep(
                deadline.saturating_duration_since(Instant::now())
                    + std::time::Duration::from_millis(20),
            );
            if !control {
                let interaction_id = client.elicitations.keys().next().unwrap().clone();
                assert!(
                    matches!(client.enqueue(AcpCommand::ReplyElicitation { interaction_id,
                    reply: RuntimeInteractionReply::Cancel }, command_deadline()),
                    Err(AcpAdmissionError::NotDispatched { code, .. }) if code == "timeout")
                );
            }
            assert!(client.poll().is_err());
            assert!(client.needs_cleanup());
            assert_eq!(
                written
                    .iter()
                    .filter(|r| r.ticket == ticket && r.stage == AcpReceiptStage::Written)
                    .count(),
                1
            );
            let terminal = client.take_receipts();
            assert!(
                terminal.iter().any(|r| r.ticket == ticket
                    && matches!(r.stage, AcpReceiptStage::Indeterminate { .. }))
            );
            assert!(
                client.has_pending_operations(),
                "failure/receipt consumption cannot release the live owner lease"
            );
            assert!(!client.snapshot().unwrap().pending_interactions.is_empty());
            client.poll().unwrap();
            assert!(client.needs_cleanup());
            client.shutdown().unwrap();
            client.take_receipts();
            assert!(!client.has_pending_operations());
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn acp_expired_budget_latches_before_late_turn_or_control_terminal() {
        for mode in ["late-complete", "late-error", "late-control"] {
            let (mut client, root) = synthetic_client(mode);
            client.create(&root, "synthetic-acp").unwrap();
            client.enable_command_port().unwrap();
            let deadline = Instant::now() + std::time::Duration::from_millis(350);
            let command = if mode == "late-control" {
                AcpCommand::SetConfig {
                    config_id: "opaque/model-id".into(),
                    value: ConfigValue::Select("model/b".into()),
                }
            } else {
                AcpCommand::Prompt {
                    text: "once".into(),
                    images: vec![],
                    run_deadline: Some(deadline),
                }
            };
            let ticket = client
                .enqueue(
                    command,
                    if mode == "late-control" {
                        deadline
                    } else {
                        command_deadline()
                    },
                )
                .unwrap();
            let mut receipts = drive_port(&mut client, |_, receipts| {
                receipts
                    .iter()
                    .any(|r| r.ticket == ticket && r.stage == AcpReceiptStage::Written)
            });
            std::thread::sleep(
                deadline.saturating_duration_since(Instant::now())
                    + std::time::Duration::from_millis(20),
            );
            std::fs::write(root.join("release"), []).unwrap();
            // No owner poll until the peer has emitted its genuinely late terminal.
            let gate = command_deadline();
            while !root.join("terminal-emitted").exists() {
                assert!(Instant::now() < gate);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
            let result = client.poll();
            receipts.extend(client.take_receipts());
            assert!(
                result.is_err(),
                "late terminal must not erase the expired {mode} budget"
            );
            assert_eq!(result.unwrap_err().code(), "timeout");
            assert!(client.needs_cleanup());
            // The peer gate and drain interval put terminal evidence into the
            // input queue before this first post-deadline owner observation.
            let stage = &receipts
                .iter()
                .find(|r| r.ticket == ticket && r.stage != AcpReceiptStage::Written)
                .unwrap()
                .stage;
            assert!(
                match mode {
                    "late-complete" => matches!(stage, AcpReceiptStage::TurnCompleted { .. }),
                    "late-error" => matches!(stage, AcpReceiptStage::TurnError { .. }),
                    _ => matches!(stage, AcpReceiptStage::ProtocolSuccess { .. }),
                },
                "native terminal evidence must survive expiry: {receipts:?}"
            );
            assert!(
                client.has_pending_operations(),
                "consuming evidence cannot release the failed owner before cleanup"
            );
            assert!(matches!(
                client.enqueue(
                    AcpCommand::List {
                        cwd: None,
                        cursor: None
                    },
                    command_deadline()
                ),
                Err(AcpAdmissionError::NeedsPreparation(_))
            ));
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert!(!root.join("unexpected-reply").exists());
            client.shutdown().unwrap();
            client.take_receipts();
            assert!(!client.has_pending_operations());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn acp_automatic_wrong_session_reply_keeps_short_write_budget_under_long_run() {
        let (mut client, root) = synthetic_client("auto-short-write");
        client.create(&root, "synthetic-acp").unwrap();
        client.enable_command_port().unwrap();
        let long = Instant::now() + std::time::Duration::from_secs(3600);
        let prompt = client
            .enqueue(
                AcpCommand::Prompt {
                    text: "once".into(),
                    images: vec![],
                    run_deadline: Some(long),
                },
                command_deadline(),
            )
            .unwrap();
        let written = drive_port(&mut client, |client, receipts| {
            !client.snapshot().unwrap().pending_interactions.is_empty()
                && receipts
                    .iter()
                    .any(|r| r.ticket == prompt && r.stage == AcpReceiptStage::Written)
        });
        let id = client.elicitations.keys().next().unwrap().clone();
        // This legal, long-budget answer really blocks stdin. No cancel command
        // or mark_cancellation path participates in this regression.
        let answer = client
            .enqueue(
                AcpCommand::ReplyElicitation {
                    interaction_id: id,
                    reply: RuntimeInteractionReply::Answers {
                        answers: vec![InteractionAnswer {
                            question_id: "value".into(),
                            values: vec!["x".repeat(200_000)],
                        }],
                    },
                },
                long,
            )
            .unwrap();
        std::fs::write(root.join("automatic"), []).unwrap();
        let queued = drive_port(&mut client, |client, _| {
            client.snapshot().unwrap().messages.iter().any(|m| {
                m.blocks.iter().any(|b|
            matches!(b, MessageBlock::Text { text } if text == "automatic cancellation required"))
            })
        });
        assert!(
            !queued
                .iter()
                .any(|r| r.ticket == answer && r.stage == AcpReceiptStage::Written)
        );
        assert_eq!(client.snapshot().unwrap().pending_interactions.len(), 1);
        std::thread::sleep(CONTROL_TIMEOUT + std::time::Duration::from_millis(100));
        let result = client.poll();
        assert!(
            result.is_err(),
            "automatic cancellation must use its short write budget, not the hour-long run"
        );
        assert_eq!(result.unwrap_err().code(), "timeout");
        assert!(client.needs_cleanup());
        let receipts = client.take_receipts();
        assert!(
            receipts
                .iter()
                .any(|r| r.ticket.wire_id == json!("wrong-session")
                    && matches!(r.stage, AcpReceiptStage::Indeterminate { .. }))
        );
        assert!(
            written
                .iter()
                .any(|r| r.ticket == prompt && r.stage == AcpReceiptStage::Written)
        );
        assert!(client.has_pending_operations());
        client.shutdown().unwrap();
        client.take_receipts();
        assert!(!client.has_pending_operations());
        std::fs::remove_dir_all(root).unwrap();
    }
}
