//! Generic ACP v1 over stdio. Agent launch, authentication and provider ownership stay external.
pub use crate::native::{Error, Result};
use crate::{conversation::*, execution::ExecutionTarget, native::rpc::RpcClient};
use agent_client_protocol_schema::v1 as wire;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
mod config;
mod projection;
pub use config::{ConfigChoice, ConfigOption, ConfigValue};

/// Arguments and environment belong to the execution target. Environment values may be secrets.
#[derive(Clone)]
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
    rpc_id: Value,
    options: Vec<String>,
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
    epoch: String,
    sequence: u64,
    message_sequence: u64,
    pending_user_echo: Option<String>,
    failed: bool,
    establishing: bool,
    initial_updates: Vec<Value>,
    stream: Option<projection::Stream>,
    mcp_servers: Vec<Value>,
    _mcp: crate::mcp::Prepared,
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
        let mut command = Command::new(&config.binary);
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
            epoch: format!("{:x}", u64::from_le_bytes(bytes)),
            sequence: 0,
            message_sequence: 0,
            pending_user_echo: None,
            failed: false,
            establishing: false,
            initial_updates: Vec::new(),
            stream: None,
            mcp_servers: Vec::new(),
            _mcp: mcp,
        };
        let response: wire::InitializeResponse = client.request(
            "initialize",
            &json!({
                "protocolVersion":1,
                "clientInfo":{"name":"velune","version":env!("CARGO_PKG_VERSION")},
                "clientCapabilities":{"session":{"configOptions":{"boolean":{}}}}
            }),
        )?;
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
    pub fn turn_running(&self) -> bool {
        self.prompt_id.is_some()
    }

    pub fn create(&mut self, cwd: &Path, runtime_id: &str) -> Result<ConversationSnapshot> {
        self.prepare_session(cwd)?;
        self.establishing = true;
        let result = self.request::<wire::NewSessionResponse>(
            "session/new",
            &json!({"cwd":cwd,"mcpServers":self.mcp_servers}),
        );
        self.establishing = false;
        let outcome = result.and_then(|response| {
            let id = response.session_id.to_string();
            if id.is_empty() {
                return Err(Error::new("ACP 会话身份为空"));
            }
            self.replace_snapshot(&id, cwd, runtime_id);
            self.options = config::project(response.config_options.unwrap_or_default())?;
            for record in std::mem::take(&mut self.initial_updates) {
                self.handle(record)?;
            }
            Ok(self.snapshot.clone().expect("created ACP session"))
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
    ) -> Result<ConversationSnapshot> {
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
            self.options = config::project(response.config_options.unwrap_or_default())?;
            self.complete_messages();
            Ok(self.snapshot.clone().expect("loaded ACP session"))
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
    ) -> Result<ConversationSnapshot> {
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
            self.options = config::project(response.config_options.unwrap_or_default())?;
            self.notice("Agent 已恢复原生上下文；session/resume 不回放会话历史。");
            Ok(self.snapshot.clone().expect("resumed ACP session"))
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
    ) -> Result<Vec<ConversationSummary>> {
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
        let mut result = Vec::new();
        loop {
            let mut params = json!({});
            if let Some(cwd) = cwd {
                params["cwd"] = json!(cwd);
            }
            if let Some(cursor) = &cursor {
                params["cursor"] = json!(cursor);
            }
            let response: wire::ListSessionsResponse = self.request("session/list", &params)?;
            for value in response.sessions {
                let id = value.session_id.to_string();
                if id.is_empty() || !value.cwd.is_absolute() {
                    return Err(Error::new("ACP 返回的会话身份或目录无效"));
                }
                let cwd = value.cwd.to_string_lossy().into_owned();
                if !ids.insert(id.clone()) {
                    return Err(Error::new("ACP 会话目录返回重复身份"));
                }
                result.push(ConversationSummary {
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
            let Some(next) = response.next_cursor else {
                break;
            };
            if !cursors.insert(next.clone()) {
                return Err(Error::new("ACP 会话目录分页游标重复"));
            }
            cursor = Some(next);
        }
        Ok(result)
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
        self.ensure_idle()?;
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
        let response: wire::SetSessionConfigOptionResponse =
            self.request("session/set_config_option", &params)?;
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
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<()> {
        if !self.turn_running() {
            return Ok(());
        }
        let id = self.session_id()?.to_owned();
        let result = self
            .cancel_permissions()
            .and_then(|()| self.rpc.notify("session/cancel", &json!({"sessionId":id})));
        if let Err(error) = result {
            self.fail();
            return Err(error);
        }
        self.state(RunState::Stopping, false, true);
        Ok(())
    }

    pub fn reply(&mut self, interaction_id: &str, reply: RuntimeInteractionReply) -> Result<()> {
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
        if let Some(snapshot) = &mut self.snapshot {
            snapshot
                .pending_interactions
                .retain(|interaction| interaction.id != interaction_id);
            snapshot.revision += 1;
        }
        Ok(())
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

    pub fn set_conversation(&mut self, conversation: ConversationSummary) {
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.conversation = conversation;
        }
    }

    /// Gracefully close an idle session when supported, then always release its process.
    /// A busy or failed connection is aborted; Drop never performs a protocol request.
    pub fn shutdown(&mut self) -> Result<()> {
        let outcome = if !self.failed && !self.turn_running() && self.capabilities.close_session {
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
        self.failed = true;
        self.prompt_id = None;
        self.permissions.clear();
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
    ) -> Result<T> {
        let id = match self.rpc.begin(method, params) {
            Ok(id) => id,
            Err(error) => {
                self.fail();
                return Err(error);
            }
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let record = match self.rpc.receive(deadline) {
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
                let outcome = record
                    .get("result")
                    .filter(|value| value.is_object())
                    .cloned()
                    .ok_or_else(|| Error::new("ACP 响应缺少对象结果"))
                    .and_then(|value| {
                        serde_json::from_value(value)
                            .map_err(|error| Error::new(format!("ACP {method} 响应无效：{error}")))
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

    fn handle(&mut self, record: Value) -> Result<()> {
        if record["jsonrpc"] != "2.0" {
            return Err(Error::new("ACP 返回的 JSON-RPC 版本无效"));
        }
        if let Some(method) = record["method"].as_str() {
            let params = &record["params"];
            if method == "session/update" {
                if self.establishing {
                    self.initial_updates.push(record);
                } else if params["sessionId"].as_str() == self.native_id.as_deref()
                    && self.native_id.is_some()
                {
                    self.update(&params["update"])?;
                }
            } else if method == "session/request_permission" {
                let id = record
                    .get("id")
                    .ok_or_else(|| Error::new("ACP 权限请求缺少身份"))?;
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
            } else if method == "$/cancel_request" {
                let expired: Vec<_> = self
                    .permissions
                    .iter()
                    .filter(|(_, permission)| permission.rpc_id == params["requestId"])
                    .map(|(id, _)| id.clone())
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
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.pending_interactions.clear();
            snapshot.revision += 1;
        }
        Ok(())
    }

    fn permission(&mut self, rpc_id: &Value, params: &Value) -> Result<()> {
        let _: wire::RequestId = serde_json::from_value(rpc_id.clone())
            .map_err(|error| Error::new(format!("ACP 请求身份无效：{error}")))?;
        let params: wire::RequestPermissionRequest = serde_json::from_value(params.clone())
            .map_err(|error| Error::new(format!("ACP 权限请求无效：{error}")))?;
        let mut seen = std::collections::BTreeSet::new();
        let options = params
            .options
            .into_iter()
            .map(|option| {
                let id = option.option_id.to_string();
                if !seen.insert(id.clone()) {
                    return Err(Error::new("ACP 权限选项身份重复"));
                }
                Ok(InteractionOption {
                    id,
                    label: option.name,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if options.is_empty() {
            return Err(Error::new("ACP 权限请求没有选项"));
        }
        if self
            .permissions
            .values()
            .any(|permission| permission.rpc_id == *rpc_id)
        {
            return Err(Error::new("ACP 权限请求身份重复"));
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
        self.initial_updates.clear();
        self.permissions.clear();
        self.pending_user_echo = None;
    }
    fn fail(&mut self) {
        self.rpc.shutdown();
        self.failed = true;
        self.prompt_id = None;
        self.finish(RunState::Failed);
        self.notice("ACP 控制连接已终止；已有消息仍保留，请重新连接。");
    }
    fn finish(&mut self, state: RunState) {
        self.permissions.clear();
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.pending_interactions.clear();
        }
        self.complete_messages();
        self.stream = None;
        self.pending_user_echo = None;
        self.state(state, !self.failed, false);
    }
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
        format!(
            "ACP 请求失败（{}）：{}",
            i32::from(error.code),
            error.message
        ),
        category,
    ))
}
fn timestamp(value: &str) -> Option<i64> {
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .ok()
        .and_then(|time| i64::try_from(time.unix_timestamp_nanos() / 1_000_000).ok())
}
