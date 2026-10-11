//! Native control protocols. Historical evidence is acquired separately through huihua.
use crate::conversation::*;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};
mod claude;
mod codex;
pub use codex::{manage_thread, manage_thread_at};
mod codex_history;
pub use codex_history::{
    list_codex_history, list_codex_history_at, read_codex_history, read_codex_history_at,
};
mod deepseek;
pub(crate) mod rpc;
#[derive(thiserror::Error, Debug)]
#[error("{detail}")]
pub struct Error {
    detail: String,
    code: &'static str,
}
impl Error {
    pub(crate) fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            code: "internal",
        }
    }
    pub(crate) fn with_code(detail: impl Into<String>, code: &'static str) -> Self {
        Self {
            detail: detail.into(),
            code,
        }
    }
    pub(crate) fn code(&self) -> &'static str {
        self.code
    }
    pub(crate) fn detail(&self) -> &str {
        &self.detail
    }
}
impl From<crate::execution::Error> for Error {
    fn from(error: crate::execution::Error) -> Self {
        Self::new(error.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NativeKind {
    Codex,
    DeepSeek,
    ClaudeCode,
}
impl NativeKind {
    pub fn type_id(self) -> &'static str {
        match self {
            Self::Codex => "codex-0.159.3",
            Self::DeepSeek => "dsh-acp-0.2.0-rc.2",
            Self::ClaudeCode => "claude-code-2.1.286",
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct GatewayInjection {
    pub endpoint: String,
    pub token: String,
    pub model_alias: String,
    pub protocol: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Option<Vec<String>>,
}
#[derive(Clone)]
pub struct NativeConfig {
    pub execution: crate::execution::ExecutionTarget,
    pub os_home: Option<PathBuf>,
    pub kind: NativeKind,
    pub binary: PathBuf,
    pub node_binary: Option<PathBuf>,
    pub agent_dir: PathBuf,
    pub resources_directory: PathBuf,
    pub projection_directory: PathBuf,
    pub gateway: Option<GatewayInjection>,
    pub mcp_servers: Vec<crate::mcp::Server>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RuntimeModel {
    pub provider_id: Option<String>,
    pub id: String,
    pub name: String,
}
#[derive(Clone)]
struct PendingRequest {
    id: Value,
    method: String,
    options: Vec<String>,
    question_ids: Vec<String>,
    payload: Value,
}
pub struct NativeSession {
    config: NativeConfig,
    default_cwd: PathBuf,
    rpc: rpc::RpcClient,
    snapshot: Option<ConversationSnapshot>,
    native_id: Option<String>,
    turn_id: Option<String>,
    pending_rpc: BTreeMap<u64, String>,
    interactions: BTreeMap<String, PendingRequest>,
    available_models: Vec<Value>,
    interaction_sequence: u64,
    epoch: String,
    failed: bool,
    native_selection: Option<RuntimeModel>,
    native_provider: Option<String>,
    claude_result: Option<(bool, std::time::Instant)>,
    claude_tasks: std::collections::BTreeSet<String>,
    claude_requires_action: bool,
    claude_stream_id: Option<String>,
    restart_required: bool,
    gateway_turn: bool,
    claude_fresh: bool,
    tunnel: Option<crate::execution::LoopbackTunnel>,
    _mcp: crate::mcp::Prepared,
}
impl NativeSession {
    pub fn connect(mut config: NativeConfig) -> Result<Self> {
        for path in [
            &config.binary,
            &config.agent_dir,
            &config.resources_directory,
            &config.projection_directory,
        ] {
            if !path.is_absolute() {
                return Err(Error::new("运行时路径必须为绝对路径"));
            }
        }
        crate::version::check_version_at(
            config.kind.type_id(),
            &config.binary,
            config.node_binary.as_deref(),
            &config.execution,
            config.os_home.as_deref(),
        )?;
        let mcp = crate::mcp::Prepared::new(
            &config.mcp_servers,
            &config.execution,
            config.os_home.as_deref(),
        )?;
        config.mcp_servers = mcp.servers.clone();
        let default_cwd = if config.execution.is_local() {
            std::env::current_dir()
                .map_err(|error| Error::new(format!("运行时默认工作目录不可用：{error}")))?
        } else if let Some(home) = &config.os_home {
            home.clone()
        } else {
            let bytes = config.execution.output(Command::new("pwd"), None, None)?;
            let cwd = std::str::from_utf8(&bytes)
                .map_err(|_| Error::new("远端工作目录不是有效文本"))?
                .trim_end_matches(['\r', '\n']);
            let path = PathBuf::from(cwd);
            if !path.is_absolute() {
                return Err(Error::new("远端工作目录不是绝对路径"));
            }
            path
        };
        let selected_gateway = config.gateway.take();
        let mut active = config.clone();
        let tunnel = if config.kind == NativeKind::Codex {
            if let Some(gateway) = selected_gateway.as_ref() {
                let tunnel =
                    crate::execution::LoopbackTunnel::open(&config.execution, &gateway.endpoint)?;
                let mut injection = gateway.clone();
                injection.endpoint = tunnel.endpoint().into();
                active.gateway = Some(injection);
                Some(tunnel)
            } else {
                None
            }
        } else {
            None
        };
        let rpc = Self::spawn_rpc(&active, None, None)?;
        let mut bytes = [0; 8];
        getrandom::fill(&mut bytes)
            .map_err(|error| Error::new(format!("运行时会话身份生成失败：{error}")))?;
        let mut session = Self {
            config,
            default_cwd,
            rpc,
            snapshot: None,
            native_id: None,
            turn_id: None,
            pending_rpc: BTreeMap::new(),
            interactions: BTreeMap::new(),
            available_models: Vec::new(),
            interaction_sequence: 0,
            failed: false,
            native_selection: None,
            native_provider: None,
            claude_result: None,
            claude_tasks: std::collections::BTreeSet::new(),
            claude_requires_action: false,
            claude_stream_id: None,
            restart_required: false,
            gateway_turn: tunnel.is_some(),
            claude_fresh: false,
            tunnel,
            _mcp: mcp,
            epoch: format!("{:x}", u64::from_le_bytes(bytes)),
        };
        session.initialize()?;
        session.config.gateway = selected_gateway;
        Ok(session)
    }
    fn spawn_rpc(
        config: &NativeConfig,
        selection: Option<&RuntimeModel>,
        session: Option<(&str, Option<&Path>, bool)>,
    ) -> Result<rpc::RpcClient> {
        let mut command = match config.kind {
            NativeKind::Codex => {
                let mut command = Command::new(&config.binary);
                command.arg("app-server");
                command
            }
            NativeKind::DeepSeek => Self::dsh_command(config)?,
            NativeKind::ClaudeCode => Self::claude_command(config, selection, session)?,
        };
        let home_key = match config.kind {
            NativeKind::Codex => "CODEX_HOME",
            NativeKind::DeepSeek => "DSH_HOME",
            NativeKind::ClaudeCode => "CLAUDE_CONFIG_DIR",
        };
        command.env(home_key, &config.agent_dir);
        crate::mcp::set_environment(&mut command, &config.mcp_servers);
        if let Some(gateway) = &config.gateway {
            command.env("VELUNE_GATEWAY_TOKEN", &gateway.token);
        } else {
            command.env_remove("VELUNE_GATEWAY_TOKEN");
        }
        rpc::RpcClient::spawn_at(command, &config.execution, config.os_home.as_deref())
    }
    fn initialize(&mut self) -> Result<()> {
        match self.config.kind {
            NativeKind::Codex => self.codex_initialize(),
            NativeKind::DeepSeek => self.dsh_initialize(),
            NativeKind::ClaudeCode => self.claude_initialize(),
        }
    }
    pub fn create(&mut self, cwd: Option<&Path>, runtime_id: &str) -> Result<ConversationSnapshot> {
        self.ensure_idle()?;
        self.validate_cwd(cwd)?;
        let gateway = if self.gateway_turn {
            None
        } else {
            self.config.gateway.take()
        };
        let outcome: Result<()> = (|| {
            match self.config.kind {
                NativeKind::Codex => self.codex_create(cwd, runtime_id)?,
                NativeKind::DeepSeek => self.dsh_create(cwd, runtime_id)?,
                NativeKind::ClaudeCode => self.claude_create(cwd, runtime_id)?,
            };
            Ok(())
        })();
        if let Some(gateway) = gateway {
            self.config.gateway = Some(gateway);
        }
        outcome?;
        self.restart_required = false;
        self.snapshot
            .clone()
            .ok_or_else(|| Error::new("运行时未创建会话"))
    }
    pub fn open(
        &mut self,
        native_id: &str,
        cwd: Option<&Path>,
        runtime_id: &str,
        history: Vec<Message>,
    ) -> Result<ConversationSnapshot> {
        self.ensure_idle()?;
        self.validate_cwd(cwd)?;
        if native_id.is_empty() {
            return Err(Error::new("会话身份为空"));
        }
        let gateway = if self.gateway_turn {
            None
        } else {
            self.config.gateway.take()
        };
        let outcome: Result<()> = (|| {
            match self.config.kind {
                NativeKind::Codex => self.codex_open(native_id, cwd, runtime_id, history)?,
                NativeKind::DeepSeek => self.dsh_open(native_id, cwd, runtime_id, history)?,
                NativeKind::ClaudeCode => self.claude_open(native_id, cwd, runtime_id, history)?,
            };
            Ok(())
        })();
        if let Some(gateway) = gateway {
            self.config.gateway = Some(gateway);
        }
        outcome?;
        self.restart_required = false;
        self.snapshot
            .clone()
            .ok_or_else(|| Error::new("运行时未恢复会话"))
    }
    pub fn send(&mut self, text: &str) -> Result<()> {
        self.ensure_idle()?;
        if text.trim().is_empty() {
            return Err(Error::new("消息内容为空"));
        }
        if self.native_id.is_none() {
            return Err(Error::new("没有活动会话"));
        }
        let result = self.prepare_turn().and_then(|()| match self.config.kind {
            NativeKind::Codex => self.codex_send(text),
            NativeKind::DeepSeek => self.dsh_send(text),
            NativeKind::ClaudeCode => self.claude_send(text),
        });
        if result.is_err() {
            self.fail_connection();
        }
        result
    }
    pub fn cancel(&mut self) -> Result<()> {
        if !self.busy() {
            return Ok(());
        }
        match self.config.kind {
            NativeKind::Codex => self.codex_cancel(),
            NativeKind::DeepSeek => self.dsh_cancel(),
            NativeKind::ClaudeCode => self.claude_cancel(),
        }
    }
    /// Select a gateway for the next turn. Existing native provider settings are never rewritten.
    pub fn select_model(&mut self, injection: GatewayInjection) -> Result<()> {
        self.ensure_idle()?;
        if self.config.gateway.as_ref() != Some(&injection) {
            self.restart_required = true;
        }
        self.config.gateway = Some(injection);
        Ok(())
    }
    /// DeepSeek requires the provider identity returned by its native catalog.
    /// Claude Code retains its configured provider and accepts no provider identity.
    pub fn select_native_model(
        &mut self,
        provider_id: Option<String>,
        model_id: String,
    ) -> Result<()> {
        self.ensure_idle()?;
        if model_id.trim().is_empty()
            || model_id.chars().any(char::is_control)
            || provider_id
                .as_deref()
                .is_some_and(|id| id.trim().is_empty() || id.chars().any(char::is_control))
        {
            return Err(Error::new("原生模型身份不能为空"));
        }
        if self.config.kind == NativeKind::ClaudeCode && provider_id.is_some() {
            return Err(Error::new(
                "Claude Code 使用运行时既有提供商，不能指定另一份提供商身份",
            ));
        }
        if self.config.kind == NativeKind::DeepSeek && provider_id.is_none() {
            return Err(Error::new("DeepSeek 原生模型选择必须包含提供商身份"));
        }
        self.native_selection = Some(RuntimeModel {
            provider_id,
            name: model_id.clone(),
            id: model_id,
        });
        self.config.gateway = None;
        self.restart_required = true;
        Ok(())
    }
    pub fn models(&mut self) -> Result<Vec<RuntimeModel>> {
        self.ensure_idle()?;
        match self.config.kind {
            NativeKind::Codex => self.codex_models(),
            NativeKind::DeepSeek => self.dsh_models(),
            NativeKind::ClaudeCode => self.claude_models(),
        }
    }
    fn prepare_turn(&mut self) -> Result<()> {
        if self.config.kind != NativeKind::ClaudeCode
            && (self.config.gateway.is_none() || self.gateway_turn)
            && !self.restart_required
        {
            return Ok(());
        }
        let saved = self
            .snapshot
            .clone()
            .ok_or_else(|| Error::new("没有活动会话"))?;
        let id = self
            .native_id
            .clone()
            .ok_or_else(|| Error::new("没有原生会话身份"))?;
        let cwd = saved.conversation.cwd.as_deref().map(PathBuf::from);
        self.rpc.shutdown();
        self.tunnel = None;
        let mut active = self.config.clone();
        if let Some(gateway) = active.gateway.as_mut() {
            let tunnel =
                crate::execution::LoopbackTunnel::open(&active.execution, &gateway.endpoint)?;
            gateway.endpoint = tunnel.endpoint().to_owned();
            self.tunnel = Some(tunnel);
        }
        self.rpc = Self::spawn_rpc(
            &active,
            self.native_selection.as_ref(),
            Some((&id, cwd.as_deref(), self.claude_fresh)),
        )?;
        // The active endpoint may be a remote loopback tunnel, only valid for this child.
        let desired = self.config.gateway.take();
        self.config.gateway = active.gateway;
        self.gateway_turn = self.config.gateway.is_some();
        let outcome = (|| {
            self.initialize()?;
            // The previous process owned this loaded-session handle; the replacement has none yet.
            self.native_id = None;
            match self.config.kind {
                NativeKind::Codex => self.codex_open(
                    &id,
                    cwd.as_deref(),
                    &saved.context_runtime_id,
                    saved.messages.clone(),
                )?,
                NativeKind::DeepSeek => self.dsh_open(
                    &id,
                    cwd.as_deref(),
                    &saved.context_runtime_id,
                    saved.messages.clone(),
                )?,
                NativeKind::ClaudeCode => {
                    self.native_id = Some(id.clone());
                }
            }
            Ok(())
        })();
        self.config.gateway = desired;
        self.snapshot = Some(saved);
        self.restart_required = false;
        if outcome.is_err() {
            self.fail_connection();
        }
        outcome
    }
    pub fn poll(&mut self) -> Result<()> {
        if self.failed || self.restart_required {
            return Ok(());
        }
        let outcome = (|| {
            let records = self.rpc.poll()?;
            for record in records {
                match self.config.kind {
                    NativeKind::Codex => self.codex_handle_record(record)?,
                    NativeKind::DeepSeek => self.dsh_handle_record(record)?,
                    NativeKind::ClaudeCode => self.claude_handle_record(record)?,
                }
            }
            if self.config.kind == NativeKind::ClaudeCode {
                self.claude_check_terminal_timeout();
            }
            Ok(())
        })();
        if outcome.is_err() {
            self.fail_connection();
        }
        outcome
    }

    fn fail_connection(&mut self) {
        self.rpc.shutdown();
        self.failed = true;
        self.settle(RunState::Failed);
        self.notice("运行时控制连接已终止，请重新连接；已有消息仍保留。");
    }

    pub fn snapshot(&self) -> Option<&ConversationSnapshot> {
        self.snapshot.as_ref()
    }
    pub fn busy(&self) -> bool {
        self.snapshot
            .as_ref()
            .is_some_and(|s| matches!(s.run_state, RunState::Running | RunState::Stopping))
    }
    pub fn reply(&mut self, interaction_id: &str, reply: RuntimeInteractionReply) -> Result<()> {
        let pending = self
            .interactions
            .get(interaction_id)
            .cloned()
            .ok_or_else(|| Error::new("此交互请求已经失效"))?;
        match self.config.kind {
            NativeKind::Codex => self.codex_reply(&pending, reply)?,
            NativeKind::DeepSeek => self.dsh_reply(&pending, reply)?,
            NativeKind::ClaudeCode => self.claude_reply(&pending, reply)?,
        };
        self.interactions.remove(interaction_id);
        if let Some(snapshot) = &mut self.snapshot {
            snapshot
                .pending_interactions
                .retain(|i| i.id != interaction_id);
            snapshot.revision += 1;
        }
        Ok(())
    }
    pub fn shutdown(&mut self) -> Result<()> {
        self.rpc.shutdown();
        self.interactions.clear();
        self.pending_rpc.clear();
        self.settle(RunState::Idle);
        Ok(())
    }
    fn ensure_idle(&self) -> Result<()> {
        if self.failed {
            return Err(Error::new("运行时连接已关闭，请重新连接"));
        }
        if self.busy() {
            Err(Error::new("运行时仍在执行"))
        } else {
            Ok(())
        }
    }
    fn validate_cwd(&self, cwd: Option<&Path>) -> Result<()> {
        if let Some(path) = cwd
            && (!path.is_absolute() || !self.config.execution.is_directory(path)?)
        {
            return Err(Error::new("会话工作目录必须是目标主机上的已有绝对目录"));
        }
        Ok(())
    }
    fn replace_snapshot(
        &mut self,
        id: &str,
        cwd: Option<&Path>,
        runtime_id: &str,
        history: Vec<Message>,
    ) {
        self.native_id = Some(id.into());
        self.turn_id = None;
        self.interactions.clear();
        self.pending_rpc.clear();
        self.snapshot = Some(ConversationSnapshot {
            context_runtime_id: runtime_id.into(),
            revision: 1,
            conversation: ConversationSummary {
                can_rename: false,
                can_delete: false,
                id: format!("{runtime_id}:{id}"),
                title: crate::conversation::conversation_title(
                    None,
                    crate::conversation::first_user_text(&history),
                ),
                updated_at_unix_ms: None,
                created_at_unix_ms: None,
                runtime_id: runtime_id.into(),
                cwd: cwd.map(|path| path.to_string_lossy().into_owned()),
            },
            resource_id: None,
            model_record_key: None,
            run_state: RunState::Idle,
            messages: history,
            transcript_turns: Vec::new(),
            transcript_items: Vec::new(),
            transcript_outline: Vec::new(),
            transcript_message_identities: Vec::new(),
            message_identity_confirmations: Vec::new(),
            pending_interactions: Vec::new(),
            actions: ConversationActions {
                can_send: true,
                can_cancel: false,
                can_switch: true,
            },
        });
    }
    /// Preserve the read-only source metadata when preparing the same native session.
    pub fn set_conversation(&mut self, conversation: ConversationSummary) {
        if let Some(snapshot) = self.snapshot.as_mut() {
            snapshot.conversation = conversation;
        }
    }
    fn running(&mut self) {
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.run_state = RunState::Running;
            snapshot.actions = ConversationActions {
                can_send: false,
                can_cancel: true,
                can_switch: false,
            };
            snapshot.revision += 1;
        }
    }
    fn finish_turn(&mut self, state: RunState) -> Result<()> {
        if self.gateway_turn && self.config.kind != NativeKind::ClaudeCode {
            let id = self
                .native_id
                .clone()
                .ok_or_else(|| Error::new("缺少原生会话身份"))?;
            let snapshot = self
                .snapshot
                .clone()
                .ok_or_else(|| Error::new("缺少会话投影"))?;
            let cwd = snapshot.conversation.cwd.as_deref().map(PathBuf::from);
            self.config.gateway = None;
            match self.config.kind {
                NativeKind::Codex => self.codex_open(
                    &id,
                    cwd.as_deref(),
                    &snapshot.context_runtime_id,
                    snapshot.messages.clone(),
                )?,
                NativeKind::DeepSeek => {}
                NativeKind::ClaudeCode => {}
            }
            self.snapshot = Some(snapshot);
        }
        self.settle(state);
        Ok(())
    }
    fn settle(&mut self, state: RunState) {
        if self.gateway_turn || self.config.kind == NativeKind::ClaudeCode {
            self.rpc.finish();
            self.tunnel = None;
            self.restart_required = true;
            self.gateway_turn = false;
            self.config.gateway = None;
        }
        self.turn_id = None;
        self.interactions.clear();
        if let Some(snapshot) = &mut self.snapshot {
            for message in &mut snapshot.messages {
                message.completed = true;
            }
            snapshot.run_state = state;
            snapshot.pending_interactions.clear();
            snapshot.actions = ConversationActions {
                can_send: !self.failed,
                can_cancel: false,
                can_switch: true,
            };
            snapshot.revision += 1;
        }
    }
    fn message(&mut self, message: Message) {
        if let Some(snapshot) = &mut self.snapshot {
            if let Some(old) = snapshot.messages.iter_mut().find(|m| m.id == message.id) {
                *old = message;
            } else {
                snapshot.messages.push(message);
            }
            snapshot
                .conversation
                .title
                .refresh(first_user_text(&snapshot.messages));
            snapshot.revision += 1;
        }
    }
    /// Appends an application status notice to the current projection, if a session exists.
    pub fn notice(&mut self, text: &str) {
        if let Some(snapshot) = &mut self.snapshot {
            let id = format!("notice-{}", snapshot.revision);
            self.message(Message {
                completed: true,
                id,
                role: crate::conversation::MessageRole::System,
                timestamp_unix_ms: None,
                blocks: vec![MessageBlock::Notice { text: text.into() }],
            });
        }
    }
    fn add_interaction(&mut self, pending: PendingRequest, kind: InteractionKind) {
        self.interaction_sequence += 1;
        let id = format!("{}-{}", self.epoch, self.interaction_sequence);
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.pending_interactions.push(RuntimeInteraction {
                id: id.clone(),
                kind,
            });
            snapshot.revision += 1;
            self.interactions.insert(id, pending);
        }
    }
}
