//! Historical projection through native Codex APIs and the installed DSH reader.
use crate::{
    conversation::*,
    native::{Error, Result as NativeResult},
};
use serde::{Deserialize, Serialize};
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
    detail: String,
}
impl HistoryError {
    fn new(kind: HistoryFailureKind, phase: &'static str) -> Self {
        Self {
            kind,
            phase,
            detail: String::new(),
        }
    }
    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
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
    pub fn full_message(&self) -> String {
        let base = format!(
            "会话历史读取失败（阶段：{}，类别：{}）",
            self.phase(),
            self.code()
        );
        if self.detail.is_empty() {
            base
        } else {
            format!("{base} 原始错误：{}", self.detail)
        }
    }
}

pub struct HistoryConfig {
    pub execution: crate::execution::ExecutionTarget,
    pub os_home: Option<PathBuf>,
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
    /// A native sub-agent's session names the session that started it.
    #[serde(default)]
    parent_native_id: Option<String>,
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
    code: String,
    #[serde(default)]
    detail: String,
}
fn bridge_error(code: &str, detail: &str, phase: &'static str) -> HistoryError {
    let kind = match code {
        "session_not_found" => HistoryFailureKind::SessionNotFound,
        "ambiguous_session" => HistoryFailureKind::AmbiguousSession,
        "provider_read" => HistoryFailureKind::ProviderRead,
        _ => HistoryFailureKind::BridgeContract,
    };
    HistoryError::new(kind, phase).with_detail(detail)
}
fn execute(
    config: &HistoryConfig,
    operation: &str,
    runtime_id: &str,
    native_id: Option<&str>,
) -> Result<Vec<u8>, HistoryError> {
    if !matches!(config.provider.as_str(), "deepseek" | "claude-code") {
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
        if !(config.provider == "claude-code" && path == &config.node_binary) && !path.is_absolute()
        {
            return Err(HistoryError::new(
                HistoryFailureKind::InvalidConfiguration,
                "validate",
            ));
        }
    }
    let request = json!({"operation":operation,"provider":config.provider,"home":config.home,"roots":[config.root],"runtimeId":runtime_id,"nativeId":native_id});
    let input = serde_json::to_vec(&request).map_err(|error| {
        HistoryError::new(HistoryFailureKind::BridgeContract, "request")
            .with_detail(error.to_string())
    })?;
    let mut command = if config.provider == "claude-code" {
        let mut command = Command::new("python3");
        command.args(["-c", include_str!("../resources/claude_history.py")]);
        command
    } else {
        let mut command = Command::new(&config.node_binary);
        command.arg(config.resources_directory.join("huihua_sessions.mjs"));
        command
    };
    command.env("CLAUDE_CONFIG_DIR", &config.home);
    // The helper stdout is the history payload and is read completely. The
    // limit applies only to retained stderr diagnostics.
    config
        .execution
        .output(command, Some(input), config.os_home.as_deref())
        .map_err(Error::from)
        .map_err(|error| {
            let kind = match error.code() {
                "spawn" => HistoryFailureKind::Spawn,
                "timeout" => HistoryFailureKind::Timeout,
                "process_exit" => HistoryFailureKind::ProcessExit,
                _ => HistoryFailureKind::Transport,
            };
            HistoryError::new(kind, "helper").with_detail(error.detail())
        })
}
pub fn list(
    config: &HistoryConfig,
    runtime_id: &str,
) -> Result<Vec<ConversationSummary>, HistoryError> {
    if config.provider == "codex" {
        return crate::native::list_codex_history_at(
            &config.binary,
            &config.home,
            runtime_id,
            &config.execution,
            config.os_home.as_deref(),
        )
        .map_err(|error| native_history_error(error, "native_list"));
    }
    let bytes = execute(config, "list", runtime_id, None)?;
    let result: List = serde_json::from_slice(&bytes).map_err(|error| {
        HistoryError::new(HistoryFailureKind::BridgeContract, "list").with_detail(error.to_string())
    })?;
    if result.contract_version != 1 {
        return Err(HistoryError::new(
            HistoryFailureKind::BridgeContract,
            "list",
        ));
    }
    if !result.failures.is_empty() {
        tracing::warn!(target: "velune_agent_runtime", event = "history_entries_skipped", phase = "list", skipped = result.failures.len(), causes = ?result.failures.iter().map(|failure| (&failure.code, &failure.detail)).collect::<Vec<_>>());
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
            parent_conversation_id: s
                .parent_native_id
                .map(|parent| format!("{runtime_id}:{parent}")),
        })
        .collect())
}
pub fn read(
    config: &HistoryConfig,
    runtime_id: &str,
    native_id: &str,
) -> Result<History, HistoryError> {
    if config.provider == "codex" {
        return crate::native::read_codex_history_at(
            &config.binary,
            &config.home,
            runtime_id,
            native_id,
            &config.execution,
            config.os_home.as_deref(),
        )
        .map_err(|error| native_history_error(error, "native_read"));
    }
    let bytes = execute(config, "read", runtime_id, Some(native_id))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        HistoryError::new(HistoryFailureKind::BridgeContract, "read").with_detail(error.to_string())
    })?;
    if let Some(code) = value
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(|code| code.as_str())
    {
        let detail = value
            .get("error")
            .and_then(|error| error.get("detail"))
            .and_then(|detail| detail.as_str())
            .unwrap_or("");
        return Err(bridge_error(code, detail, "read"));
    }
    let result: Read = serde_json::from_value(value).map_err(|error| {
        HistoryError::new(HistoryFailureKind::BridgeContract, "read").with_detail(error.to_string())
    })?;
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
    HistoryError::new(HistoryFailureKind::ProviderRead, phase).with_detail(error.detail())
}

/// A Pi session file as observed by a previous listing. Pi appends to its
/// session files, so an unchanged size and modification time mean unchanged metadata.
/// Size and nanosecond modification time are opaque decimal strings compared
/// only by the helper, so no precision is lost crossing JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PiSessionFile {
    pub path: String,
    pub size: String,
    pub mtime_ns: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiListedFile {
    #[serde(flatten)]
    pub file: PiSessionFile,
    /// False only for a file reported in `known` with the same identity.
    pub changed: bool,
}
/// Pi SessionManager metadata, read only for new or changed files.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiSessionSummary {
    pub path: String,
    pub cwd: Option<String>,
    pub name: Option<String>,
    pub created_unix_ms: Option<i64>,
    pub modified_unix_ms: Option<i64>,
    pub first_message: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiListing {
    contract_version: u32,
    pub files: Vec<PiListedFile>,
    pub sessions: Vec<PiSessionSummary>,
}

/// List Pi sessions, re-reading only files that differ from `known`.
pub fn list_pi(config: &crate::Config, known: &[PiSessionFile]) -> NativeResult<PiListing> {
    let input = serde_json::to_vec(&json!({ "files": known }))
        .map_err(|error| Error::new(format!("Pi 历史列表请求无效：{error}")))?;
    let listing: PiListing =
        serde_json::from_value(execute_pi(config, PiOperation::List, Some(input))?)
            .map_err(|error| Error::new(format!("Pi 历史 SDK 格式不匹配：{error}")))?;
    if listing.contract_version != 2 {
        return Err(Error::new("Pi 历史列表契约版本不匹配"));
    }
    Ok(listing)
}
/// Execute the fixed Pi SDK history helper without starting the Agent. Output
/// and wall time share the bounded native helper transport; stderr is discarded.
pub fn read_pi(
    config: &crate::Config,
    session: &std::path::Path,
) -> NativeResult<serde_json::Value> {
    execute_pi(config, PiOperation::Inspect(session), None)
}
/// Modify Pi's native source with the fixed SDK; no Velune metadata overlay.
pub fn manage_pi(
    config: &crate::Config,
    session: &std::path::Path,
    title: Option<&str>,
) -> NativeResult<()> {
    let (operation, input) = match title {
        Some(title) => (
            PiOperation::Rename(session),
            Some(
                serde_json::to_vec(&json!({"title":title}))
                    .map_err(|error| Error::new(format!("Pi 会话修改请求无效：{error}")))?,
            ),
        ),
        None => (PiOperation::Delete(session), None),
    };
    let result = execute_pi(config, operation, input)?;
    if result["ok"] != true {
        return Err(Error::new("Pi 会话修改未确认"));
    }
    Ok(())
}
enum PiOperation<'a> {
    List,
    Inspect(&'a std::path::Path),
    Rename(&'a std::path::Path),
    Delete(&'a std::path::Path),
}
fn execute_pi(
    config: &crate::Config,
    operation: PiOperation<'_>,
    input: Option<Vec<u8>>,
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
    command
        .arg(helper)
        .arg("--agent-dir")
        .arg(agent_dir)
        .arg("--cli")
        .arg(&config.binary)
        .arg("--runtime-type")
        .arg(&config.runtime_type_id);
    match operation {
        PiOperation::List => {}
        PiOperation::Inspect(session) => {
            command.arg("--inspect-session").arg(session);
        }
        PiOperation::Rename(session) => {
            command.arg("--rename-session").arg(session);
        }
        PiOperation::Delete(session) => {
            command.arg("--delete-session").arg(session);
        }
    }
    if let Some(root) = &config.session_dir {
        command.arg("--session-dir").arg(root);
    }
    let output = config
        .execution
        .output(command, input, config.os_home.as_deref())?;
    serde_json::from_slice(&output)
        .map_err(|error| Error::new(format!("Pi 历史 SDK 格式不匹配：{error}")))
}
