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
pub use codex::{
    ThreadChange, manage_thread, manage_thread_at, resume_provider_matches, update_read_state,
};
mod codex_history;
pub mod codex_projects;
mod codex_retry;
mod codex_side;
mod codex_title;
pub use codex_history::{
    codex_context_provider, export_codex_context, list_codex_history, list_codex_history_at,
    read_codex_history, read_codex_history_at, read_codex_summary_at,
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
    pub fn code(&self) -> &'static str {
        self.code
    }
    pub(crate) fn detail(&self) -> &str {
        &self.detail
    }
}
impl From<crate::execution::Error> for Error {
    fn from(error: crate::execution::Error) -> Self {
        let code = error.code();
        Self::with_code(error.into_detail(), code)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NativeKind {
    Codex,
    Codex166,
    DeepSeek,
    ClaudeCode,
}
impl NativeKind {
    pub fn runtime_kind(self) -> crate::version::RuntimeKind {
        use crate::version::RuntimeKind;
        match self {
            Self::Codex => RuntimeKind::Codex,
            Self::Codex166 => RuntimeKind::Codex166,
            Self::DeepSeek => RuntimeKind::DeepSeek,
            Self::ClaudeCode => RuntimeKind::ClaudeCode,
        }
    }
    pub fn of(kind: crate::version::RuntimeKind) -> Option<Self> {
        use crate::version::RuntimeKind;
        match kind {
            RuntimeKind::Codex => Some(Self::Codex),
            RuntimeKind::Codex166 => Some(Self::Codex166),
            RuntimeKind::DeepSeek => Some(Self::DeepSeek),
            RuntimeKind::ClaudeCode => Some(Self::ClaudeCode),
            RuntimeKind::Pi | RuntimeKind::Acp => None,
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

/// The display name of a native model selection id: a Codex speed variant
/// reads as its base model and tier, anything else is itself.
pub fn display_model_name(kind: crate::version::RuntimeKind, model_id: &str) -> String {
    match NativeKind::of(kind) {
        Some(NativeKind::Codex | NativeKind::Codex166) => codex::speed_model_name(model_id),
        _ => model_id.to_owned(),
    }
}
/// A model a runtime offers, with the reasoning levels the runtime accepts
/// for it, in the runtime's own vocabulary; empty when it takes none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelOption {
    pub model: RuntimeModel,
    pub reasoning_efforts: Vec<String>,
}

/// A fork whose native file is created by its first turn. Its source and
/// boundary survive replacing the execution configuration; clients do not
/// interpret the runtime's resume point.
#[derive(Clone)]
pub struct PendingFork {
    conversation_id: String,
    from: String,
    at: Option<String>,
}

impl PendingFork {
    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
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
    compacting: bool,
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
    compaction_message_id: Option<String>,
    /// Conversations of the native sub-agents this conversation's tool
    /// calls started; their later events belong to this conversation.
    sub_agents: std::collections::BTreeSet<String>,
    restart_required: bool,
    /// The runtime process was ended on purpose; there is nothing to read.
    process_ended: bool,
    gateway_turn: bool,
    /// A side chat: a temporary copy of a conversation that is never saved.
    side: bool,
    /// What only a Codex app-server session tracks.
    codex: codex::State,
    /// What only a Claude Code session tracks.
    claude: claude::State,
    tunnel: Option<crate::execution::LoopbackTunnel>,
    _mcp: crate::mcp::Prepared,
}
impl NativeSession {
    /// A loaded native handle belongs to this execution owner until its process ends.
    pub fn owns_control(&self, target: &crate::control::Target) -> bool {
        !self.process_ended
            && !self.failed
            && self.native_id.as_deref() == Some(&target.native_id)
            && self.config.kind.runtime_kind() == target.kind
            && self.config.execution == target.execution
            && self.config.agent_dir == target.agent_dir
            && self.config.os_home == target.os_home
    }

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
            config.kind.runtime_kind(),
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
            let bytes = config
                .execution
                .output(crate::process_tree::command("pwd"), None, None)?;
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
        let tunnel = if matches!(config.kind, NativeKind::Codex | NativeKind::Codex166) {
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
            compaction_message_id: None,
            compacting: false,
            sub_agents: std::collections::BTreeSet::new(),
            restart_required: false,
            process_ended: false,
            gateway_turn: tunnel.is_some(),
            side: false,
            codex: Default::default(),
            claude: Default::default(),
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
            NativeKind::Codex | NativeKind::Codex166 => {
                let mut command = crate::process_tree::command(&config.binary);
                command.arg("app-server");
                if config.kind == NativeKind::Codex166
                    && let Some(gateway) = &config.gateway
                {
                    // Admission compares against startup config, before request overrides.
                    command.args([
                        "-c",
                        "model_provider=\"velune\"",
                        "-c",
                        "model_providers.velune.name=\"Velune LLM Gateway\"",
                        "-c",
                        "model_providers.velune.env_key=\"VELUNE_GATEWAY_TOKEN\"",
                        "-c",
                        "model_providers.velune.wire_api=\"responses\"",
                        "-c",
                        "model_providers.velune.requires_openai_auth=false",
                    ]);
                    command.arg("-c").arg(format!(
                        "model_providers.velune.base_url={}",
                        json!(gateway.endpoint)
                    ));
                }
                command
            }
            NativeKind::DeepSeek => Self::dsh_command(config)?,
            NativeKind::ClaudeCode => {
                Self::claude_command(config, selection, session, effort, permission_mode)?
            }
        };
        let home_key = match config.kind {
            NativeKind::Codex | NativeKind::Codex166 => "CODEX_HOME",
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
            NativeKind::Codex | NativeKind::Codex166 => self.codex_initialize(),
            NativeKind::DeepSeek => self.dsh_initialize(),
            NativeKind::ClaudeCode => self.claude_initialize(),
        }
    }
    pub fn create(&mut self, cwd: Option<&Path>, runtime_id: &str) -> Result<ConversationSnapshot> {
        self.create_with_project(cwd, runtime_id, None)
    }
    /// A project is an explicit workspace choice, separate from the command cwd.
    pub fn create_in_project(
        &mut self,
        cwd: Option<&Path>,
        runtime_id: &str,
        project: &codex_projects::NativeProject,
    ) -> Result<ConversationSnapshot> {
        if self.config.kind != NativeKind::Codex166 {
            return Err(Error::new("此运行时不支持选择整个 Codex 项目"));
        }
        self.create_with_project(cwd, runtime_id, Some(project))
    }
    fn create_with_project(
        &mut self,
        cwd: Option<&Path>,
        runtime_id: &str,
        project: Option<&codex_projects::NativeProject>,
    ) -> Result<ConversationSnapshot> {
        self.ensure_idle()?;
        self.validate_cwd(cwd)?;
        let gateway = if self.gateway_turn {
            None
        } else {
            self.config.gateway.take()
        };
        let outcome: Result<()> = (|| {
            match self.config.kind {
                NativeKind::Codex | NativeKind::Codex166 => {
                    self.codex_create(cwd, runtime_id, project)?
                }
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
    /// Start a disposable side context after changing execution configuration.
    /// Reference history is never imported into the original native session.
    pub fn create_side_context(
        &mut self,
        cwd: Option<&Path>,
        runtime_id: &str,
        context: &str,
    ) -> Result<ConversationSnapshot> {
        if self.config.kind == NativeKind::DeepSeek {
            return Err(Error::new("此运行时不提供侧聊"));
        }
        self.side = true;
        self.codex.side_context = Some(context.into());
        self.create(cwd, runtime_id)?;
        if self.config.kind == NativeKind::ClaudeCode {
            let directory = self
                .config
                .projection_directory
                .join(format!("side-context-{}", self.epoch));
            self.config.execution.create_dir_all(&directory)?;
            let path = directory.join("context.txt");
            self.config
                .execution
                .write_file_atomic(&path, context.as_bytes())?;
            self.claude.start = claude::ClaudeStart::SideContext { path };
        }
        self.snapshot
            .clone()
            .ok_or_else(|| Error::new("侧聊上下文不可用"))
    }
    /// A persistent Codex fork whose earlier messages are reference context.
    pub fn reference_context(&mut self, context: String) -> Result<()> {
        if !matches!(self.config.kind, NativeKind::Codex | NativeKind::Codex166)
            || self.native_id.is_some()
        {
            return Err(Error::new("此运行时不支持新会话引用上下文"));
        }
        self.codex.side_context = Some(context);
        Ok(())
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
                NativeKind::Codex | NativeKind::Codex166 => {
                    self.codex_open(native_id, cwd, runtime_id, history)?
                }
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
    /// Opens a side chat on native conversation `parent_id`, in the
    /// runtime's own way: Claude Code answers side questions on the
    /// conversation's context, Codex forks it into an ephemeral thread told
    /// that what came before is reference only. Nothing of it is saved; it
    /// lasts as long as this session. The snapshot shows only what is asked
    /// and answered in it.
    pub fn open_side(
        &mut self,
        parent_id: &str,
        cwd: Option<&Path>,
        runtime_id: &str,
    ) -> Result<ConversationSnapshot> {
        self.ensure_idle()?;
        self.validate_cwd(cwd)?;
        if parent_id.is_empty() {
            return Err(Error::new("会话身份为空"));
        }
        let gateway = if self.gateway_turn {
            None
        } else {
            self.config.gateway.take()
        };
        let outcome = match self.config.kind {
            NativeKind::Codex | NativeKind::Codex166 => {
                self.codex_open_side(parent_id, cwd, runtime_id)
            }
            NativeKind::ClaudeCode => self.claude_open_side(parent_id, cwd, runtime_id),
            NativeKind::DeepSeek => Err(Error::new("DeepSeek 运行时不提供侧聊")),
        };
        if let Some(gateway) = gateway {
            self.config.gateway = Some(gateway);
        }
        outcome?;
        self.side = true;
        self.restart_required = false;
        self.snapshot
            .clone()
            .ok_or_else(|| Error::new("运行时未打开侧聊"))
    }
    pub fn send(&mut self, input: &crate::TurnInput) -> Result<()> {
        self.start_turn(input, false)
    }

    pub fn can_compact(&self) -> bool {
        crate::version::variant(self.config.kind.runtime_kind()).can_compact
    }
    /// Maintain the current native context without adding a user turn.
    pub fn compact(&mut self) -> Result<()> {
        self.ensure_idle()?;
        if !self.can_compact() || self.side {
            return Err(Error::new("此运行时不支持主动压缩上下文"));
        }
        if !self.work().is_empty() {
            return Err(Error::new("请先等待子 Agent 结束"));
        }
        self.compacting = true;
        self.compaction_message_id = None;
        let result = (|| {
            self.prepare_turn(None, None)?;
            if let Some(snapshot) = &mut self.snapshot {
                snapshot
                    .executions
                    .begin(ExecutionKind::ContextMaintenance, false);
            }
            match self.config.kind {
                NativeKind::Codex | NativeKind::Codex166 => self.codex_compact(),
                NativeKind::ClaudeCode => self.claude_compact(),
                NativeKind::DeepSeek => unreachable!(),
            }
        })();
        if result.is_err() {
            self.compacting = false;
        }
        result
    }
    /// Starts a native turn from the existing context without a user message.
    pub fn resume(
        &mut self,
        effort: Option<String>,
        permission_mode: Option<String>,
    ) -> Result<()> {
        if self.config.kind == NativeKind::DeepSeek || self.side {
            return Err(Error::new("此运行时不支持无新消息恢复"));
        }
        self.start_turn(
            &crate::TurnInput {
                text: String::new(),
                effort,
                permission_mode,
                images: Vec::new(),
            },
            true,
        )
    }
    fn start_turn(&mut self, input: &crate::TurnInput, resuming: bool) -> Result<()> {
        self.ensure_idle()?;
        if !resuming && input.text.trim().is_empty() && input.images.is_empty() {
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
        self.codex.retry = codex_retry::Retry {
            effort: input.effort.clone(),
            permission_mode: input.permission_mode.clone(),
            ..Default::default()
        };
        let result = self
            .prepare_turn(input.effort.as_deref(), input.permission_mode.as_deref())
            .and_then(|()| {
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.executions.begin(ExecutionKind::UserTurn, resuming);
                }
                match self.config.kind {
                    NativeKind::Codex | NativeKind::Codex166 => self.codex_send(input),
                    NativeKind::DeepSeek => self.dsh_send(&input.text),
                    NativeKind::ClaudeCode => self.claude_send(input),
                }
            });
        if result.is_err() {
            self.fail_connection();
        }
        result
    }
    /// Whether the runtime takes a message into the running turn.
    pub fn can_steer(&self) -> bool {
        self.config.kind != NativeKind::DeepSeek
            && self.codex.retry.can_steer()
            && !(self.side && self.config.kind == NativeKind::ClaudeCode)
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
            NativeKind::Codex | NativeKind::Codex166 => self.codex_steer(input),
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
        if self.side {
            return Err(Error::new("侧聊不提供消息编辑"));
        }
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
            NativeKind::Codex | NativeKind::Codex166 => self.codex_branch(index, placement),
            NativeKind::ClaudeCode => self.claude_branch(index, placement),
            NativeKind::DeepSeek => Err(Error::new("DeepSeek 运行时不提供消息编辑")),
        }
    }
    /// Continue in a new native conversation holding everything through the
    /// turn the user message `message_id` starts: up to the next user
    /// message, or the whole conversation when there is none. The original
    /// stays as it is; this session then presents the copy, ready for the
    /// next turn.
    pub fn fork_through(&mut self, message_id: &str) -> Result<()> {
        self.ensure_idle()?;
        if self.side {
            return Err(Error::new("侧聊不提供会话分叉"));
        }
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
            NativeKind::Codex | NativeKind::Codex166 => self.codex_fork(index),
            NativeKind::ClaudeCode => self.claude_fork(index),
            NativeKind::DeepSeek => Err(Error::new("DeepSeek 运行时不提供会话分叉")),
        }
    }

    pub fn pending_fork(&self) -> Option<PendingFork> {
        let claude::ClaudeStart::Fork { from, at } = &self.claude.start else {
            return None;
        };
        Some(PendingFork {
            conversation_id: self.snapshot.as_ref()?.conversation.id.clone(),
            from: from.clone(),
            at: at.clone(),
        })
    }

    /// Restore only the matching fork after its model/provider configuration
    /// has changed. Opening the not-yet-created target as a normal resume
    /// would discard its inherited context.
    pub fn restore_pending_fork(&mut self, pending: &PendingFork) -> Result<()> {
        self.ensure_idle()?;
        if self.config.kind != NativeKind::ClaudeCode
            || self
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.conversation.id.as_str())
                != Some(pending.conversation_id())
        {
            return Err(Error::new("待创建的分支与当前原生会话不匹配"));
        }
        self.claude.start = claude::ClaudeStart::Fork {
            from: pending.from.clone(),
            at: pending.at.clone(),
        };
        Ok(())
    }
    /// Drop the messages from `index` on from the snapshot.
    fn truncate_snapshot(&mut self, index: usize) {
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.messages.truncate(index);
            let kept: std::collections::BTreeSet<_> = snapshot
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect();
            for record in &mut snapshot.executions.records {
                if record
                    .message_ids
                    .iter()
                    .any(|id| !kept.contains(id.as_str()))
                {
                    record.outcome = None;
                }
            }
            snapshot.executions.retain(&snapshot.messages);
            snapshot.revision += 1;
        }
    }
    pub fn cancel(&mut self) -> Result<()> {
        if !self.turn_running() {
            return Ok(());
        }
        if matches!(self.config.kind, NativeKind::Codex | NativeKind::Codex166) && !self.compacting
        {
            return self.codex_cancel_recovery();
        }
        match self.config.kind {
            NativeKind::Codex | NativeKind::Codex166 => self.codex_cancel(),
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
        let model_id = if matches!(self.config.kind, NativeKind::Codex | NativeKind::Codex166) {
            let (base, tier) = codex::split_speed(&model_id);
            self.codex.service_tier = tier;
            base
        } else {
            model_id
        };
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
            NativeKind::Codex | NativeKind::Codex166 => self.codex_models(),
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
        // An ephemeral fork lives only in its app-server process.
        if self.side && matches!(self.config.kind, NativeKind::Codex | NativeKind::Codex166) {
            return Err(Error::new("侧聊不能更换模型"));
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
        let title_eligible = self.codex.title.eligible;
        self.codex.title = Default::default();
        self.codex.title.eligible = title_eligible;
        let mut active = self.config.clone();
        if let Some(gateway) = active.gateway.as_mut() {
            let tunnel =
                crate::execution::LoopbackTunnel::open(&active.execution, &gateway.endpoint)?;
            gateway.endpoint = tunnel.endpoint().to_owned();
            self.tunnel = Some(tunnel);
        }
        self.codex.threads.clear();
        self.codex.ran_threads.clear();
        self.process_ended = false;
        self.rpc = Self::spawn_rpc(
            &active,
            self.native_selection.as_ref(),
            Some((&id, cwd.as_deref(), &self.claude.start)),
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
                NativeKind::Codex | NativeKind::Codex166 => self.codex_open(
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
        let revision = self.snapshot.as_ref().map(|s| s.revision);
        let outcome = (|| {
            let records = self.rpc.poll()?;
            for record in records {
                match self.config.kind {
                    NativeKind::Codex | NativeKind::Codex166 => self.codex_handle_record(record)?,
                    NativeKind::DeepSeek => self.dsh_handle_record(record)?,
                    NativeKind::ClaudeCode => self.claude_handle_record(record)?,
                }
            }
            if self.config.kind == NativeKind::ClaudeCode {
                self.claude_check_terminal_timeout();
            }
            if matches!(self.config.kind, NativeKind::Codex | NativeKind::Codex166) {
                self.codex_poll_retry()?;
                self.codex_poll_title();
            }
            if let Some(snapshot) = &mut self.snapshot
                && revision != Some(snapshot.revision)
            {
                snapshot
                    .executions
                    .confirm(&snapshot.message_identity_confirmations);
                snapshot.executions.retain(&snapshot.messages);
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
        if matches!(self.config.kind, NativeKind::Codex | NativeKind::Codex166) && !self.compacting
        {
            self.codex_recovery_failure(
                "运行时控制连接已终止；自动恢复已停止，未重新派发。".into(),
            );
        }
        let compacting = self.compacting;
        let pending_compaction = self.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .messages
                .iter()
                .find(|message| {
                    !message.completed && Some(&message.id) == self.compaction_message_id.as_ref()
                })
                .map(|message| message.id.clone())
        });
        self.rpc.shutdown();
        self.process_ended = true;
        self.codex.threads.clear();
        self.interactions.clear();
        self.failed = true;
        self.settle(RunState::Failed);
        let text = "运行时控制连接已终止，请重新连接；已有消息仍保留。";
        if compacting {
            self.message(Message {
                id: pending_compaction
                    .unwrap_or_else(|| format!("compaction-connection-{}", self.epoch)),
                role: MessageRole::System,
                blocks: vec![MessageBlock::ContextCompaction {
                    text: format!("上下文压缩已中断：{text}"),
                }],
                completed: true,
                timestamp_unix_ms: None,
                turn_settings: None,
                turn_usage: None,
                branch_point: None,
            });
        } else {
            self.notice(text);
        }
    }

    pub fn snapshot(&self) -> Option<&ConversationSnapshot> {
        self.snapshot.as_ref()
    }

    /// Native title commands are independent of turn execution and approvals.
    pub fn rename(&mut self, title: &str) -> Result<()> {
        if matches!(self.config.kind, NativeKind::Codex | NativeKind::Codex166) {
            self.codex_cancel_title();
        }
        let id = self
            .native_id
            .as_deref()
            .ok_or_else(|| Error::new("原生会话尚未创建"))?;
        match self.config.kind {
            NativeKind::Codex | NativeKind::Codex166 => {
                self.rpc
                    .request("thread/name/set", &json!({"threadId":id,"name":title}))?;
            }
            NativeKind::ClaudeCode => {
                self.rpc.claude_control(json!({"subtype":"rename_session","title":title,"source":"host","session_id":id}))?;
            }
            NativeKind::DeepSeek => return Err(Error::new("此运行时没有原生会话重命名接口")),
        }
        if let Some(snapshot) = self.snapshot.as_mut() {
            snapshot.conversation.title = conversation_title(Some(title), None);
            snapshot.revision = snapshot.revision.saturating_add(1);
        }
        Ok(())
    }
    /// Best-effort metadata still uses the process and its model connection,
    /// but does not make the visible conversation busy or disable input.
    pub fn has_pending_metadata(&self) -> bool {
        self.codex.title.pending()
    }

    /// Native sub-agents and tasks, excluding client metadata requests.
    pub fn work(&self) -> Vec<BackgroundTask> {
        match self.config.kind {
            NativeKind::Codex | NativeKind::Codex166 => {
                let Some(runtime_id) = self
                    .snapshot
                    .as_ref()
                    .map(|snapshot| &snapshot.conversation.runtime_id)
                else {
                    return Vec::new();
                };
                self.codex
                    .threads
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
            NativeKind::Codex | NativeKind::Codex166 => self.codex_stop_background(),
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
            NativeKind::Codex | NativeKind::Codex166 => self.codex_reply(&pending, reply)?,
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
    /// 0.159 process ran are put back on the runtime's own provider.
    /// 0.166 retains each context's provider and uses a new context to switch.
    pub fn release_override(&mut self) -> Result<()> {
        self.rpc.shutdown();
        self.process_ended = true;
        self.codex.threads.clear();
        if !std::mem::take(&mut self.gateway_turn) || self.config.kind != NativeKind::Codex {
            return Ok(());
        }
        self.config.gateway = None;
        self.codex_restore_provider()
    }
    pub fn shutdown(&mut self) -> Result<()> {
        if matches!(self.config.kind, NativeKind::Codex | NativeKind::Codex166) {
            self.codex_end_recovery(Some(TurnOutcome::Cancelled), false);
        }
        // A side question's child is let end on its own, as at a stop.
        if self.side && self.config.kind == NativeKind::ClaudeCode {
            self.rpc.retire();
        } else {
            self.rpc.shutdown();
        }
        self.process_ended = true;
        self.codex.threads.clear();
        self.interactions.clear();
        self.pending_rpc.clear();
        if let claude::ClaudeStart::SideContext { path } = &self.claude.start
            && let Some(directory) = path.parent()
        {
            let _ = self.config.execution.remove_dir_all(directory);
        }
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
        self.codex.turn_error = None;
        self.interactions.clear();
        self.pending_rpc.clear();
        self.sub_agents = sub_agent_conversations(&history).collect();
        self.snapshot = Some(ConversationSnapshot {
            context_runtime_id: runtime_id.into(),
            revision: 1,
            conversation: ConversationSummary {
                native_read_state: None,
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
            executions: Default::default(),
            transcript_turns: Vec::new(),
            transcript_items: Vec::new(),
            transcript_outline: Vec::new(),
            transcript_message_identities: Vec::new(),
            message_identity_confirmations: Vec::new(),
            pending_interactions: Vec::new(),
            follow_up: Default::default(),
            sub_agents: Vec::new(),
            collaborators: Vec::new(),
            actions: ConversationActions {
                input_access: Default::default(),
                can_send: true,
                can_cancel: false,
                can_switch: true,
            },
        });
    }
    pub fn restore_context(&mut self, source: &ConversationSnapshot) {
        if let Some(snapshot) = &mut self.snapshot {
            snapshot
                .executions
                .restore(&source.executions, &snapshot.messages);
            let receipt = snapshot.conversation.native_read_state.take();
            snapshot.conversation = source.conversation.clone();
            // The live resume's receipt accompanies its freshly read messages.
            snapshot.conversation.native_read_state = receipt;
        }
    }

    /// Preserve the read-only source metadata when preparing the same native session.
    pub fn set_read_state(&mut self, receipt: &NativeReadState) {
        if let Some(snapshot) = &mut self.snapshot
            && snapshot.conversation.id == receipt.conversation_id
        {
            snapshot.conversation.native_read_state = Some(receipt.clone());
            snapshot.revision += 1;
        }
    }
    pub fn set_conversation(&mut self, conversation: ConversationSummary) {
        if let Some(snapshot) = self.snapshot.as_mut() {
            snapshot.conversation = conversation;
        }
    }
    fn running(&mut self) {
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.run_state = RunState::Running;
            snapshot.actions = ConversationActions {
                input_access: Default::default(),
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
        if let Some(snapshot) = &mut self.snapshot {
            snapshot
                .executions
                .finish((state == RunState::Failed).then_some(TurnOutcome::Failed));
        }
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
                if !message.completed {
                    for block in &mut message.blocks {
                        if let MessageBlock::ContextCompaction { text } = block {
                            *text = if snapshot.run_state == RunState::Stopping {
                                "上下文压缩已取消"
                            } else if state == RunState::Failed {
                                "上下文压缩失败"
                            } else {
                                "上下文压缩已结束（未收到完成确认）"
                            }
                            .into();
                        }
                    }
                }
                message.completed = true;
            }
            snapshot.run_state = if self.compacting && !self.failed {
                RunState::Idle
            } else {
                state
            };
            snapshot
                .pending_interactions
                .retain(|interaction| background.contains(&interaction.id));
            snapshot.actions = ConversationActions {
                input_access: Default::default(),
                can_send: !self.failed,
                can_cancel: false,
                can_switch: true,
            };
            snapshot.revision += 1;
        }
        self.compacting = false;
        self.codex.cancel_pending = false;
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
            snapshot.executions.attach(&message.id);
            if let Some(old) = snapshot.messages.iter_mut().find(|m| m.id == message.id) {
                // What the turn used arrives apart from its user message.
                let usage = old.turn_usage.take();
                let timestamp = old.timestamp_unix_ms;
                *old = message;
                if old.timestamp_unix_ms.is_none() {
                    old.timestamp_unix_ms = timestamp;
                }
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
        if self.compacting {
            return;
        }
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

#[cfg(all(test, unix))]
mod compaction_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn compaction_connection_failures_preserve_reply_and_confirmed_success() {
        for phase in ["prepare", "pending", "completed"] {
            let mut nonce = [0; 8];
            getrandom::fill(&mut nonce).unwrap();
            let root = std::env::temp_dir()
                .join(format!("velune-compact-{:x}", u64::from_le_bytes(nonce)));
            std::fs::create_dir(&root).unwrap();
            let binary = root.join("codex");
            std::fs::write(
                &binary,
                r#"#!/bin/sh
if [ "$1" = --version ]; then echo 'codex-cli 0.159.3'; exit 0; fi
if [ -f "$CODEX_HOME/fail-initialize" ]; then exit 1; fi
while IFS= read -r line; do
  case "$line" in
    *'"id":'*) printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9]*\).*/{"id":\1,"result":{}}/p' ;;
  esac
done
"#,
            )
            .unwrap();
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut session = NativeSession::connect(NativeConfig {
                execution: crate::execution::ExecutionTarget::Local,
                os_home: Some(root.clone()),
                kind: NativeKind::Codex,
                binary,
                node_binary: None,
                agent_dir: root.clone(),
                resources_directory: root.clone(),
                projection_directory: root.clone(),
                gateway: None,
                mcp_servers: vec![],
                session_agents: vec![],
            })
            .unwrap();
            let messages = serde_json::from_value(json!([
                {"id":"user","role":"user","completed":true,"blocks":[{"kind":"text","text":"hello"}]},
                {"id":"answer","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"done"}]}
            ])).unwrap();
            session.replace_snapshot("synthetic", Some(&root), "codex", messages);
            if phase == "prepare" {
                session.restart_required = true;
                std::fs::write(root.join("fail-initialize"), "synthetic").unwrap();
                assert!(session.compact().is_err());
            } else {
                session.compact().unwrap();
                session.codex_handle_record(json!({"method":if phase == "completed" {"item/completed"} else {"item/started"},
                    "params":{"item":{"id":"compact","type":"contextCompaction"}}})).unwrap();
                session.fail_connection();
            }
            assert!(!session.compacting);
            let snapshot = session.snapshot().unwrap();
            assert_eq!(snapshot.run_state, RunState::Failed);
            assert!(snapshot.messages.iter().all(|message| message.completed));
            assert!(
                matches!(&snapshot.messages.last().unwrap().blocks[0], MessageBlock::ContextCompaction { text } if text.contains("连接"))
            );
            if phase == "completed" {
                let confirmed = snapshot
                    .messages
                    .iter()
                    .find(|message| message.id == "compact")
                    .unwrap();
                assert!(
                    matches!(&confirmed.blocks[0], MessageBlock::ContextCompaction { text } if text == "上下文已压缩")
                );
            }
            let mut transcript = crate::transcript::TranscriptProjection::default();
            transcript.apply(crate::transcript::TranscriptEvent::Replace {
                messages: &snapshot.messages,
                executions: &snapshot.executions,
            });
            assert_eq!(
                transcript.turns()[0].last_message_id.as_deref(),
                Some("answer")
            );
            drop(session);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
