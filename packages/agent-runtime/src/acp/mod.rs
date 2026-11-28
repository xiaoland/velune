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
mod auto_compaction;
mod config;
mod projection;
pub use config::{ConfigChoice, ConfigOption, ConfigValue};
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

struct Permission {
    rpc_id: Value,
    options: Vec<String>,
}

struct Elicitation {
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
    auto_compaction: Option<auto_compaction::Applied>,
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
            auto_compaction: None,
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
        self.restore_auto_compaction()?;
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
        let switches_model = self
            .options
            .iter()
            .find(|option| option.category.as_deref() == Some("model"))
            .is_some_and(|option| {
                option.id == config_id && option.value != value && option.accepts(&value)
            });
        if switches_model {
            self.restore_auto_compaction()?;
        }
        self.write_config_option(config_id, value)
    }

    fn write_config_option(&mut self, config_id: &str, value: ConfigValue) -> Result<()> {
        let id = self.session_id()?.to_owned();
        let option = self
            .options
            .iter()
            .find(|option| option.id == config_id)
            .ok_or_else(|| Error::new("ACP 会话未公布此配置项"))?;
        let mut params = json!({"sessionId":id,"configId":config_id});
        if !option.accepts(&value) {
            return Err(Error::new("ACP 配置值不属于当前公布的选项或类型"));
        }
        match value {
            ConfigValue::Select(value) => params["value"] = json!(value),
            ConfigValue::Boolean(value) => {
                params["type"] = json!("boolean");
                params["value"] = json!(value);
            }
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
        let outcome = if !self.failed && !self.turn_running() {
            self.restore_auto_compaction().and_then(|()| {
                if let Some(id) = self
                    .native_id
                    .clone()
                    .filter(|_| self.capabilities.close_session)
                {
                    self.request::<wire::CloseSessionResponse>(
                        "session/close",
                        &json!({"sessionId":id}),
                    )
                    .map(|_| ())
                } else {
                    Ok(())
                }
            })
        } else {
            Ok(())
        };
        if outcome.is_ok() && !self.failed && !self.turn_running() {
            self.rpc.finish();
        } else {
            self.rpc.shutdown();
        }
        self.failed = true;
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
                if params["sessionId"].as_str() == self.native_id.as_deref()
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
                    self.rpc
                        .respond(id, &json!({"outcome":{"outcome":"cancelled"}}))?;
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
                    self.reply(&id, RuntimeInteractionReply::Cancel)?;
                }
            } else if let Some(id) = record.get("id") {
                self.rpc.reject(id, "Unsupported client capability")?;
            }
            return Ok(());
        }
        if self.prompt_id.is_some() && record["id"].as_u64() == self.prompt_id {
            self.prompt_id = None;
            self.cancel_permissions()?;
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
        }
        Ok(())
    }

    fn cancel_permissions(&mut self) -> Result<()> {
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

    fn interaction_capacity(&self) -> Result<()> {
        if self.permissions.len() + self.elicitations.len() >= 32 {
            return Err(Error::with_code(
                "ACP interaction capacity full",
                "transport",
            ));
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
            return self
                .rpc
                .reject(rpc_id, "Unsupported or inactive elicitation scope");
        }
        let questions = match crate::elicitation::questions(params) {
            Ok(questions) => questions,
            Err(error) => return self.rpc.reject(rpc_id, &error.to_string()),
        };
        let id = self.next_id("elicitation");
        self.elicitations.insert(
            id.clone(),
            Elicitation {
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

    fn ensure_idle(&self) -> Result<()> {
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
        if self.native_id.is_some() {
            if !self.capabilities.close_session {
                return Err(Error::new(
                    "此 ACP Agent 不支持关闭会话，请为新会话创建连接",
                ));
            }
            return self.close_session();
        }
        self.clear_session();
        Ok(())
    }
    fn clear_session(&mut self) {
        self.native_id = None;
        self.snapshot = None;
        self.options.clear();
        self.auto_compaction = None;
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
        self.rpc.shutdown();
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
        self.permissions.clear();
        self.elicitations.clear();
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.pending_interactions.clear();
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
mod tests;
