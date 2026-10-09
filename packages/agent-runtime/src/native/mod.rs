//! Native control protocols. Historical evidence is acquired separately through huihua.
use crate::conversation::*;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};
mod codex;
pub use codex::manage_thread;
mod codex_history;
pub use codex_history::{list_codex_history, read_codex_history};
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
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NativeKind {
    Codex,
    DeepSeek,
}
impl NativeKind {
    pub fn type_id(self) -> &'static str {
        match self {
            Self::Codex => "codex-0.159.3",
            Self::DeepSeek => "dsh-acp-0.2.0-rc.2",
        }
    }
}
#[derive(Clone)]
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
    pub kind: NativeKind,
    pub binary: PathBuf,
    pub node_binary: Option<PathBuf>,
    pub agent_dir: PathBuf,
    pub resources_directory: PathBuf,
    pub projection_directory: PathBuf,
    pub gateway: GatewayInjection,
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
}
impl NativeSession {
    pub fn connect(config: NativeConfig) -> Result<Self> {
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
        crate::version::check_version(
            config.kind.type_id(),
            &config.binary,
            config.node_binary.as_deref(),
        )?;
        let default_cwd = std::env::current_dir()
            .map_err(|error| Error::new(format!("运行时默认工作目录不可用：{error}")))?;
        let rpc = Self::spawn_rpc(&config)?;
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
            epoch: format!("{:x}", u64::from_le_bytes(bytes)),
        };
        session.initialize()?;
        Ok(session)
    }
    fn spawn_rpc(config: &NativeConfig) -> Result<rpc::RpcClient> {
        let command = match config.kind {
            NativeKind::Codex => {
                let mut c = Command::new(&config.binary);
                c.arg("app-server");
                c
            }
            NativeKind::DeepSeek => Self::dsh_command(config)?,
        };
        let home_key = match config.kind {
            NativeKind::Codex => "CODEX_HOME",
            NativeKind::DeepSeek => "DSH_HOME",
        };
        rpc::RpcClient::spawn(
            command,
            &[
                (
                    home_key.into(),
                    config.agent_dir.to_string_lossy().into_owned(),
                ),
                ("VELUNE_GATEWAY_TOKEN".into(), config.gateway.token.clone()),
            ],
        )
    }
    fn initialize(&mut self) -> Result<()> {
        match self.config.kind {
            NativeKind::Codex => self.codex_initialize(),
            NativeKind::DeepSeek => self.dsh_initialize(),
        }
    }
    pub fn create(&mut self, cwd: Option<&Path>, runtime_id: &str) -> Result<ConversationSnapshot> {
        self.ensure_idle()?;
        self.validate_cwd(cwd)?;
        match self.config.kind {
            NativeKind::Codex => self.codex_create(cwd, runtime_id)?,
            NativeKind::DeepSeek => self.dsh_create(cwd, runtime_id)?,
        };
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
        match self.config.kind {
            NativeKind::Codex => self.codex_open(native_id, cwd, runtime_id, history)?,
            NativeKind::DeepSeek => self.dsh_open(native_id, cwd, runtime_id, history)?,
        };
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
        match self.config.kind {
            NativeKind::Codex => self.codex_send(text),
            NativeKind::DeepSeek => self.dsh_send(text),
        }
    }
    pub fn cancel(&mut self) -> Result<()> {
        if !self.busy() {
            return Ok(());
        }
        match self.config.kind {
            NativeKind::Codex => self.codex_cancel(),
            NativeKind::DeepSeek => self.dsh_cancel(),
        }
    }
    pub fn select_model(&mut self, injection: GatewayInjection) -> Result<()> {
        self.ensure_idle()?;
        if injection.protocol != self.config.gateway.protocol
            || injection.endpoint != self.config.gateway.endpoint
            || injection.token != self.config.gateway.token
        {
            return Err(Error::new("模型协议或网关实例已变化，请重新连接运行时"));
        }
        match self.config.kind {
            NativeKind::Codex => {
                self.config.gateway = injection;
                Ok(())
            }
            NativeKind::DeepSeek => {
                let outcome = self.dsh_select_model(injection);
                if outcome.is_err() {
                    // DSH selection replaces its control process; a partial replacement
                    // cannot safely continue under the previous model selection.
                    self.fail_connection();
                }
                outcome
            }
        }
    }
    pub fn poll(&mut self) -> Result<()> {
        if self.failed {
            return Ok(());
        }
        let outcome = (|| {
            let records = self.rpc.poll()?;
            for record in records {
                match self.config.kind {
                    NativeKind::Codex => self.codex_handle_record(record)?,
                    NativeKind::DeepSeek => self.dsh_handle_record(record)?,
                }
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
        if cwd.is_some_and(|path| !path.is_absolute() || !path.is_dir()) {
            Err(Error::new("会话工作目录必须是已有绝对目录"))
        } else {
            Ok(())
        }
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
    fn settle(&mut self, state: RunState) {
        self.turn_id = None;
        self.interactions.clear();
        if let Some(snapshot) = &mut self.snapshot {
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
