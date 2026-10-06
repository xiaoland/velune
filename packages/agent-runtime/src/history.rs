//! Historical projection through native Codex APIs and the installed DSH reader.
use crate::{
    conversation::*,
    native::{Error, Result as NativeResult, rpc::bounded_process},
};
use serde::Deserialize;
use serde_json::json;
use std::{path::PathBuf, process::Command};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryFailureKind {
    InvalidConfiguration,
    UnsupportedProvider,
    Transport,
    Spawn,
    Timeout,
    ProcessExit,
    OutputLimit,
    BridgeContract,
    SessionNotFound,
    AmbiguousSession,
    ProviderRead,
}
impl HistoryFailureKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "invalid_configuration",
            Self::UnsupportedProvider => "unsupported_provider",
            Self::Transport => "transport",
            Self::Spawn => "spawn",
            Self::Timeout => "timeout",
            Self::ProcessExit => "process_exit",
            Self::OutputLimit => "output_limit",
            Self::BridgeContract => "bridge_contract",
            Self::SessionNotFound => "session_not_found",
            Self::AmbiguousSession => "ambiguous_session",
            Self::ProviderRead => "provider_read",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("history {phase}: {kind:?}")]
pub struct HistoryError {
    kind: HistoryFailureKind,
    phase: &'static str,
}
impl HistoryError {
    fn new(kind: HistoryFailureKind, phase: &'static str) -> Self {
        Self { kind, phase }
    }
    pub fn kind(&self) -> HistoryFailureKind {
        self.kind
    }
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }
    pub fn phase(&self) -> &'static str {
        self.phase
    }
    pub fn safe_message(&self) -> String {
        format!("会话历史读取失败（{}）", self.code())
    }
}

pub struct HistoryConfig {
    pub provider: String,
    pub binary: PathBuf,
    pub node_binary: PathBuf,
    pub resources_directory: PathBuf,
    pub root: PathBuf,
    pub home: PathBuf,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub native_id: String,
    pub title: ConversationTitle,
    pub cwd: Option<String>,
    pub messages: Vec<Message>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    native_id: String,
    title: ConversationTitle,
    cwd: Option<String>,
    updated_at_unix_ms: Option<i64>,
    created_at_unix_ms: Option<i64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct List {
    contract_version: u32,
    sessions: Vec<Summary>,
    #[serde(default)]
    failures: Vec<BridgeFailure>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Read {
    contract_version: u32,
    history: History,
}
#[derive(Deserialize)]
struct BridgeFailure {
    #[serde(rename = "code")]
    _code: String,
}
fn bridge_error(code: &str, phase: &'static str) -> HistoryError {
    let kind = match code {
        "session_not_found" => HistoryFailureKind::SessionNotFound,
        "ambiguous_session" => HistoryFailureKind::AmbiguousSession,
        "provider_read" => HistoryFailureKind::ProviderRead,
        _ => HistoryFailureKind::BridgeContract,
    };
    HistoryError::new(kind, phase)
}
fn execute(
    config: &HistoryConfig,
    operation: &str,
    native_id: Option<&str>,
) -> Result<Vec<u8>, HistoryError> {
    if config.provider != "deepseek" {
        return Err(HistoryError::new(
            HistoryFailureKind::UnsupportedProvider,
            "validate",
        ));
    }
    for path in [
        &config.node_binary,
        &config.resources_directory,
        &config.root,
        &config.home,
    ] {
        if !path.is_absolute() {
            return Err(HistoryError::new(
                HistoryFailureKind::InvalidConfiguration,
                "validate",
            ));
        }
    }
    let request = json!({"operation":operation,"provider":config.provider,"home":config.home,"roots":[config.root],"nativeId":native_id});
    let input = serde_json::to_vec(&request)
        .map_err(|_| HistoryError::new(HistoryFailureKind::BridgeContract, "request"))?;
    let mut command = Command::new(&config.node_binary);
    command.arg(config.resources_directory.join("huihua_sessions.mjs"));
    bounded_process(command, Some(input), 16 * 1024 * 1024).map_err(|error| {
        let kind = match error.code() {
            "spawn" => HistoryFailureKind::Spawn,
            "timeout" => HistoryFailureKind::Timeout,
            "process_exit" => HistoryFailureKind::ProcessExit,
            "output_limit" => HistoryFailureKind::OutputLimit,
            _ => HistoryFailureKind::Transport,
        };
        HistoryError::new(kind, "helper")
    })
}
pub fn list(
    config: &HistoryConfig,
    runtime_id: &str,
) -> Result<Vec<ConversationSummary>, HistoryError> {
    if config.provider == "codex" {
        return crate::native::list_codex_history(&config.binary, &config.home, runtime_id)
            .map_err(|error| native_history_error(error, "native_list"));
    }
    let bytes = execute(config, "list", None)?;
    let result: List = serde_json::from_slice(&bytes)
        .map_err(|_| HistoryError::new(HistoryFailureKind::BridgeContract, "list"))?;
    if result.contract_version != 1 {
        return Err(HistoryError::new(
            HistoryFailureKind::BridgeContract,
            "list",
        ));
    }
    if !result.failures.is_empty() {
        tracing::warn!(target: "velune_agent_runtime", event = "history_entries_skipped", phase = "list", skipped = result.failures.len());
    }
    Ok(result
        .sessions
        .into_iter()
        .map(|s| ConversationSummary {
            can_rename: false,
            can_delete: false,
            id: format!("{runtime_id}:{}", s.native_id),
            title: s.title,
            updated_at_unix_ms: s.updated_at_unix_ms,
            created_at_unix_ms: s.created_at_unix_ms,
            runtime_id: runtime_id.into(),
            cwd: s.cwd,
        })
        .collect())
}
pub fn read(config: &HistoryConfig, native_id: &str) -> Result<History, HistoryError> {
    if config.provider == "codex" {
        return crate::native::read_codex_history(&config.binary, &config.home, native_id)
            .map_err(|error| native_history_error(error, "native_read"));
    }
    let bytes = execute(config, "read", Some(native_id))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| HistoryError::new(HistoryFailureKind::BridgeContract, "read"))?;
    if let Some(code) = value
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(|code| code.as_str())
    {
        return Err(bridge_error(code, "read"));
    }
    let result: Read = serde_json::from_value(value)
        .map_err(|_| HistoryError::new(HistoryFailureKind::BridgeContract, "read"))?;
    if result.contract_version != 1 || result.history.native_id != native_id {
        return Err(HistoryError::new(
            HistoryFailureKind::BridgeContract,
            "read",
        ));
    }
    Ok(result.history)
}

fn native_history_error(error: Error, phase: &'static str) -> HistoryError {
    tracing::warn!(target:"velune_agent_runtime",event="history_native_failed",phase,runtime_family="codex",native_code=error.code());
    HistoryError::new(
        if error.code() == "output_limit" {
            HistoryFailureKind::OutputLimit
        } else {
            HistoryFailureKind::ProviderRead
        },
        phase,
    )
}

/// Execute the fixed Pi SDK history helper without starting the Agent. Output
/// and wall time share the bounded native helper transport; stderr is discarded.
pub fn read_pi(
    config: &crate::Config,
    session: Option<&std::path::Path>,
) -> NativeResult<serde_json::Value> {
    execute_pi(config, session, None)
}
/// Modify Pi's native source with the fixed SDK; no Velune metadata overlay.
pub fn manage_pi(
    config: &crate::Config,
    session: &std::path::Path,
    title: Option<&str>,
) -> NativeResult<()> {
    let result = execute_pi(config, Some(session), Some(title))?;
    if result["ok"] != true {
        return Err(Error::new("Pi 会话修改未确认"));
    }
    Ok(())
}
fn execute_pi(
    config: &crate::Config,
    session: Option<&std::path::Path>,
    mutation: Option<Option<&str>>,
) -> NativeResult<serde_json::Value> {
    let node = config
        .node_binary
        .as_ref()
        .filter(|p| p.is_absolute())
        .ok_or_else(|| Error::new("Pi 历史 Node 路径无效"))?;
    let helper = config
        .sdk_helper
        .as_ref()
        .filter(|p| p.is_absolute())
        .ok_or_else(|| Error::new("Pi 历史 SDK helper 路径无效"))?;
    let mut command = Command::new(node);
    let agent_dir = config
        .agent_dir
        .as_ref()
        .filter(|p| p.is_absolute())
        .ok_or_else(|| Error::new("Pi 历史来源目录无效"))?;
    command.arg(helper).arg("--agent-dir").arg(agent_dir);
    if let Some(session) = session {
        command
            .arg(match mutation {
                Some(Some(_)) => "--rename-session",
                Some(None) => "--delete-session",
                None => "--inspect-session",
            })
            .arg(session);
    } else {
        command.arg("--all");
    }
    if let Some(root) = &config.session_dir {
        command.arg("--session-dir").arg(root);
    }
    let input = mutation
        .flatten()
        .map(|title| serde_json::to_vec(&json!({"title":title})))
        .transpose()
        .map_err(|_| Error::new("Pi 会话修改请求无效"))?;
    let output = bounded_process(command, input, 16 * 1024 * 1024)?;
    serde_json::from_slice(&output).map_err(|_| Error::new("Pi 历史 SDK 格式不匹配"))
}
