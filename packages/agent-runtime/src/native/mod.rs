//! Native control protocols. Historical evidence is acquired separately through huihua.
use crate::conversation::*;
use crate::work::{BackgroundState, BackgroundTask};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};
mod claude;
pub use claude::{
    CLAUDE_EFFORT_LEVELS, claude_login_directory, claude_user_config_directory,
    rename_claude_session_at,
};
mod codex;
pub use codex::{ThreadChange, manage_thread, manage_thread_at};
mod codex_history;
pub use codex_history::{
    list_codex_history, list_codex_history_at, read_codex_history, read_codex_history_at,
    read_codex_summary_at,
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
            Self::Codex => "codex-0.159",
            Self::DeepSeek => "dsh-acp-0.2",
            Self::ClaudeCode => "claude-code-2.1",
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
    /// Sub-agents the host supplies to gateway turns only; nothing of them is
    /// written where the Harness keeps its own definitions, so a turn on the
    /// Harness's own provider never sees them.
    pub session_agents: Vec<SessionAgent>,
}

/// A sub-agent definition for one execution. The model is a name the
/// turn's gateway routes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionAgent {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub tools: Vec<String>,
    pub skills: Vec<String>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RuntimeModel {
    pub provider_id: Option<String>,
    pub id: String,
    pub name: String,
}
/// A model a runtime offers, with the reasoning levels the runtime accepts
/// for it, in the runtime's own vocabulary; empty when it takes none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelOption {
    pub model: RuntimeModel,
    pub reasoning_efforts: Vec<String>,
}
#[derive(Clone)]
struct PendingRequest {
    id: Value,
    /// Asked by a native sub-agent, whose work may outlive the turn.
    background: bool,
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
    /// Injected MCP services the Harness already has under the same name.
    shadowed_mcp_servers: std::collections::BTreeSet<String>,
    claude_result: Option<(bool, std::time::Instant)>,
    claude_tasks: std::collections::BTreeSet<String>,
    /// Conversations of the native sub-agents this conversation's tool
    /// calls started; their later events belong to this conversation.
    sub_agents: std::collections::BTreeSet<String>,
    /// Codex threads of this app-server besides the conversation's own,
    /// by thread id, while they work.
    codex_threads: BTreeMap<String, codex::CodexThread>,
    /// Every Codex thread besides the conversation's own that this
    /// app-server has run.
    codex_ran_threads: std::collections::BTreeSet<String>,
    claude_requires_action: bool,
    claude_stream_id: Option<String>,
    /// The model and context tokens of the turn's latest reply.
    claude_context: Option<(String, u64)>,
    restart_required: bool,
    /// The runtime process was ended on purpose; there is nothing to read.
    process_ended: bool,
    /// Messages steered into Claude Code turns, for their local identities.
    steer_sequence: u64,
    gateway_turn: bool,
    claude_start: claude::ClaudeStart,
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
                Self::codex_write_roles(&active)?;
                Some(tunnel)
            } else {
                None
            }
        } else {
            None
        };
        let rpc = Self::spawn_rpc(&active, None, None, None, None)?;
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
            shadowed_mcp_servers: std::collections::BTreeSet::new(),
            claude_result: None,
            claude_tasks: std::collections::BTreeSet::new(),
            sub_agents: std::collections::BTreeSet::new(),
            codex_threads: BTreeMap::new(),
            codex_ran_threads: std::collections::BTreeSet::new(),
            claude_requires_action: false,
            claude_stream_id: None,
            claude_context: None,
            restart_required: false,
            process_ended: false,
            steer_sequence: 0,
            gateway_turn: tunnel.is_some(),
            claude_start: Default::default(),
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
        session: Option<(&str, Option<&Path>, &claude::ClaudeStart)>,
        effort: Option<&str>,
        permission_mode: Option<&str>,
    ) -> Result<rpc::RpcClient> {
        let mut command = match config.kind {
            NativeKind::Codex => {
                let mut command = Command::new(&config.binary);
                command.arg("app-server");
                command
            }
            NativeKind::DeepSeek => Self::dsh_command(config)?,
            NativeKind::ClaudeCode => {
                Self::claude_command(config, selection, session, effort, permission_mode)?
            }
        };
        let home_key = match config.kind {
            NativeKind::Codex => "CODEX_HOME",
            NativeKind::DeepSeek => "DSH_HOME",
            NativeKind::ClaudeCode => "CLAUDE_CONFIG_DIR",
        };
        if config.kind == NativeKind::ClaudeCode && Self::claude_default_config(config) {
            // Setting CLAUDE_CONFIG_DIR at all, even to its default, makes the CLI
            // suffix its macOS Keychain service name, hiding the user's own login.
            command.env_remove(home_key);
        } else {
            command.env(home_key, &config.agent_dir);
        }
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
    pub fn send(&mut self, input: &crate::TurnInput) -> Result<()> {
        self.ensure_idle()?;
        if input.text.trim().is_empty() && input.images.is_empty() {
            return Err(Error::new("消息内容为空"));
        }
        if self.native_id.is_none() {
            return Err(Error::new("没有活动会话"));
        }
        if self.config.kind == NativeKind::DeepSeek && input.effort.is_some() {
            return Err(Error::new("DeepSeek 运行时不提供推理强度设置"));
        }
        if self.config.kind == NativeKind::DeepSeek && input.permission_mode.is_some() {
            return Err(Error::new("DeepSeek 运行时不提供权限模式设置"));
        }
        if self.config.kind == NativeKind::DeepSeek && !input.images.is_empty() {
            return Err(Error::new("DeepSeek 运行时不接受图片"));
        }
        if self.restart_needed() && !self.work().is_empty() {
            return Err(Error::with_code(
                "子 Agent 仍在运行；此轮需要重新启动运行时，请等待其结束或先停止子 Agent",
                "outstanding_work",
            ));
        }
        let result = self
            .prepare_turn(input.effort.as_deref(), input.permission_mode.as_deref())
            .and_then(|()| match self.config.kind {
                NativeKind::Codex => self.codex_send(input),
                NativeKind::DeepSeek => self.dsh_send(&input.text),
                NativeKind::ClaudeCode => self.claude_send(input),
            });
        if result.is_err() {
            self.fail_connection();
        }
        result
    }
    /// Whether the runtime takes a message into the running turn.
    pub fn can_steer(&self) -> bool {
        self.config.kind != NativeKind::DeepSeek
            && self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.run_state == RunState::Running)
    }
    /// Deliver a user message into the running turn through the runtime's
    /// own channel; it does not start a turn.
    pub fn steer(&mut self, input: &crate::TurnInput) -> Result<()> {
        if !self.can_steer() {
            return Err(Error::new("此运行时当前不能插入消息"));
        }
        if input.text.trim().is_empty() && input.images.is_empty() {
            return Err(Error::new("消息内容为空"));
        }
        match self.config.kind {
            NativeKind::Codex => self.codex_steer(input),
            NativeKind::ClaudeCode => self.claude_steer(input),
            NativeKind::DeepSeek => Err(Error::new("DeepSeek 运行时不提供插入消息")),
        }
    }
    /// Make the next send replace the user message `message_id` and all
    /// after it. In place, the conversation goes back to just before the
    /// message; a fork continues in a new native conversation holding what
    /// precedes it, which this session then presents, and leaves the
    /// original as it is. The snapshot no longer shows the replaced messages.
    pub fn branch(&mut self, message_id: &str, placement: EditPlacement) -> Result<()> {
        self.ensure_idle()?;
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or_else(|| Error::new("没有活动会话"))?;
        let index = snapshot
            .messages
            .iter()
            .position(|message| message.id == message_id && message.role == MessageRole::User)
            .ok_or_else(|| Error::new("会话中没有这条用户消息"))?;
        match self.config.kind {
            NativeKind::Codex => self.codex_branch(index, placement),
            NativeKind::ClaudeCode => self.claude_branch(index, placement),
            NativeKind::DeepSeek => Err(Error::new("DeepSeek 运行时不提供消息编辑")),
        }
    }
    /// Drop the messages from `index` on from the snapshot.
    fn truncate_snapshot(&mut self, index: usize) {
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.messages.truncate(index);
            snapshot.revision += 1;
        }
    }
    pub fn cancel(&mut self) -> Result<()> {
        if !self.turn_running() {
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
    pub fn models(&mut self) -> Result<Vec<ModelOption>> {
        self.ensure_idle()?;
        match self.config.kind {
            NativeKind::Codex => self.codex_models(),
            NativeKind::DeepSeek => self.dsh_models(),
            NativeKind::ClaudeCode => self.claude_models(),
        }
    }
    /// Whether the next turn needs a new runtime process, which ends the
    /// current one with any work it still does: the turn's model moves
    /// between the runtime's own provider and Velune's gateway, or another
    /// gateway injection or native model was selected. Claude Code takes the
    /// turn's reasoning level and permission mode only at launch, so its
    /// turn child is always replaced.
    pub fn restart_needed(&self) -> bool {
        self.config.kind == NativeKind::ClaudeCode
            || (self.config.gateway.is_some() && !self.gateway_turn)
            || self.restart_required
    }
    fn prepare_turn(&mut self, effort: Option<&str>, permission_mode: Option<&str>) -> Result<()> {
        if !self.restart_needed() {
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
        self.codex_threads.clear();
        self.codex_ran_threads.clear();
        self.process_ended = false;
        self.rpc = Self::spawn_rpc(
            &active,
            self.native_selection.as_ref(),
            Some((&id, cwd.as_deref(), &self.claude_start)),
            effort,
            permission_mode,
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
        if self.failed || self.process_ended {
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
            self.end_spent_process();
            Ok(())
        })();
        if outcome.is_err() {
            self.fail_connection();
        }
        outcome
    }

    fn fail_connection(&mut self) {
        self.rpc.shutdown();
        self.process_ended = true;
        self.codex_threads.clear();
        self.interactions.clear();
        self.failed = true;
        self.settle(RunState::Failed);
        self.notice("运行时控制连接已终止，请重新连接；已有消息仍保留。");
    }

    pub fn snapshot(&self) -> Option<&ConversationSnapshot> {
        self.snapshot.as_ref()
    }
    /// The work this session's runtime process does besides the turn, as
    /// its events tell: the sub-agents Codex runs as other threads of the
    /// app-server, Claude Code's unfinished tasks.
    pub fn work(&self) -> Vec<BackgroundTask> {
        match self.config.kind {
            NativeKind::Codex => {
                let Some(runtime_id) = self
                    .snapshot
                    .as_ref()
                    .map(|snapshot| &snapshot.conversation.runtime_id)
                else {
                    return Vec::new();
                };
                self.codex_threads
                    .iter()
                    .map(|(thread, entry)| BackgroundTask {
                        conversation_id: format!("{runtime_id}:{thread}"),
                        state: entry.state,
                    })
                    .collect()
            }
            NativeKind::ClaudeCode => self
                .claude_running_sub_agents()
                .into_iter()
                .map(|conversation_id| BackgroundTask {
                    conversation_id,
                    state: BackgroundState::Running,
                })
                .collect(),
            NativeKind::DeepSeek => Vec::new(),
        }
    }
    /// Ask the work besides the turn to stop. Claude Code's tasks belong to
    /// its turn, which stays running until they end; stopping the turn stops them.
    pub fn stop_background(&mut self) -> Result<()> {
        match self.config.kind {
            NativeKind::Codex => self.codex_stop_background(),
            NativeKind::ClaudeCode | NativeKind::DeepSeek => Ok(()),
        }
    }
    /// The conversation's own turn runs. Work it started may go on after it; see `work`.
    pub fn turn_running(&self) -> bool {
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
    /// Ends a gateway turn's process once the turn and its work have ended.
    /// Codex keeps the provider a thread last ran with, so the threads the
    /// process ran are put back on the runtime's own provider, and a later
    /// native session does not look for the gateway.
    pub fn release_override(&mut self) -> Result<()> {
        self.rpc.shutdown();
        self.process_ended = true;
        self.codex_threads.clear();
        if !std::mem::take(&mut self.gateway_turn) || self.config.kind != NativeKind::Codex {
            return Ok(());
        }
        self.config.gateway = None;
        self.codex_restore_provider()
    }
    pub fn shutdown(&mut self) -> Result<()> {
        self.rpc.shutdown();
        self.process_ended = true;
        self.codex_threads.clear();
        self.interactions.clear();
        self.pending_rpc.clear();
        self.settle(RunState::Idle);
        Ok(())
    }
    fn ensure_idle(&self) -> Result<()> {
        if self.failed {
            return Err(Error::new("运行时连接已关闭，请重新连接"));
        }
        if self.turn_running() {
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
        self.sub_agents = sub_agent_conversations(&history).collect();
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
                parent_conversation_id: None,
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
            follow_up: Default::default(),
            sub_agents: Vec::new(),
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
        self.settle(state);
        Ok(())
    }
    /// Ends the turn. The process stays while it does other work; the
    /// host releases the turn's gateway once that work has ended too.
    fn settle(&mut self, state: RunState) {
        if self.config.kind == NativeKind::ClaudeCode {
            self.restart_required = true;
        }
        self.turn_id = None;
        // A sub-agent's request still waits for its answer.
        let background = self
            .interactions
            .iter()
            .filter(|(_, pending)| pending.background)
            .map(|(id, _)| id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        self.interactions.retain(|id, _| background.contains(id));
        if let Some(snapshot) = &mut self.snapshot {
            for message in &mut snapshot.messages {
                message.completed = true;
            }
            snapshot.run_state = state;
            snapshot
                .pending_interactions
                .retain(|interaction| background.contains(&interaction.id));
            snapshot.actions = ConversationActions {
                can_send: !self.failed,
                can_cancel: false,
                can_switch: true,
            };
            snapshot.revision += 1;
        }
        self.end_spent_process();
    }
    /// A Claude Code turn's process serves only that turn: it ends once the
    /// turn and its tasks have.
    fn end_spent_process(&mut self) {
        if self.config.kind == NativeKind::ClaudeCode
            && self.restart_required
            && !self.process_ended
            && !self.turn_running()
            && self.work().is_empty()
        {
            self.rpc.finish();
            self.process_ended = true;
        }
    }
    fn message(&mut self, message: Message) {
        self.sub_agents
            .extend(sub_agent_conversations(std::slice::from_ref(&message)));
        if let Some(snapshot) = &mut self.snapshot {
            if let Some(old) = snapshot.messages.iter_mut().find(|m| m.id == message.id) {
                // What the turn used arrives apart from its user message.
                let usage = old.turn_usage.take();
                *old = message;
                if old.turn_usage.is_none() {
                    old.turn_usage = usage;
                }
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
    /// Updates the usage of the turn the last user message starts.
    fn turn_usage(&mut self, update: impl FnOnce(&mut Option<TurnUsage>)) {
        if let Some(snapshot) = &mut self.snapshot
            && let Some(user) = snapshot
                .messages
                .iter_mut()
                .rev()
                .find(|message| message.role == MessageRole::User)
        {
            update(&mut user.turn_usage);
            snapshot.revision += 1;
        }
    }
    /// Appends an application status notice to the current projection, if a session exists.
    pub fn notice(&mut self, text: &str) {
        if let Some(snapshot) = &mut self.snapshot {
            let id = format!("notice-{}", snapshot.revision);
            self.message(Message {
                turn_settings: None,
                turn_usage: None,
                branch_point: None,
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

/// The conversations that messages' tool calls started or addressed.
fn sub_agent_conversations(messages: &[Message]) -> impl Iterator<Item = String> + '_ {
    messages
        .iter()
        .flat_map(|message| &message.blocks)
        .filter_map(|block| match block {
            MessageBlock::Tool {
                conversation_id: Some(conversation),
                ..
            } => Some(conversation.clone()),
            _ => None,
        })
}
