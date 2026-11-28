//! Historical projection through native Codex APIs and the installed DSH reader.
pub use crate::native::codex_history::{materialize_codex_import, materialize_owned_codex_import};
use crate::{
    conversation::*,
    creation::{DataRetention, Retained},
    native::{Error, Result as NativeResult, codex_history},
};
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};

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

#[derive(Clone, PartialEq, Eq)]
pub struct HistoryConfig {
    pub kind: RuntimeKind,
    pub execution: crate::execution::ExecutionTarget,
    pub os_home: Option<PathBuf>,
    pub binary: PathBuf,
    pub node_binary: PathBuf,
    pub resources_directory: PathBuf,
    pub root: PathBuf,
    pub home: PathBuf,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    #[serde(default)]
    pub native_read_state: Option<NativeReadState>,
    pub native_id: String,
    pub title: ConversationTitle,
    pub cwd: Option<String>,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub pending_interactions: Vec<RuntimeInteraction>,
    #[serde(default)]
    pub executions: Executions,
    /// The native history ends with a stopped or failed continuable turn.
    #[serde(default)]
    pub can_resume: bool,
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
impl Summary {
    fn for_runtime(self, runtime_id: &str) -> ConversationSummary {
        ConversationSummary {
            native_read_state: None,
            can_rename: false,
            can_delete: false,
            id: format!("{runtime_id}:{}", self.native_id),
            title: self.title,
            updated_at_unix_ms: self.updated_at_unix_ms,
            created_at_unix_ms: self.created_at_unix_ms,
            runtime_id: runtime_id.into(),
            cwd: self.cwd,
            parent_conversation_id: self
                .parent_native_id
                .map(|parent| format!("{runtime_id}:{parent}")),
        }
    }
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
    summary: Option<Summary>,
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
    native_ids: &[String],
) -> Result<Vec<u8>, HistoryError> {
    execute_before(config, operation, runtime_id, native_id, native_ids, None)
}
fn execute_before(
    config: &HistoryConfig,
    operation: &str,
    runtime_id: &str,
    native_id: Option<&str>,
    native_ids: &[String],
    before_message_id: Option<&str>,
) -> Result<Vec<u8>, HistoryError> {
    execute_selection(
        config,
        operation,
        runtime_id,
        native_id,
        native_ids,
        before_message_id,
        &[],
    )
}
#[allow(clippy::too_many_arguments)]
fn execute_selection(
    config: &HistoryConfig,
    operation: &str,
    runtime_id: &str,
    native_id: Option<&str>,
    native_ids: &[String],
    before_message_id: Option<&str>,
    without_message_ids: &[String],
) -> Result<Vec<u8>, HistoryError> {
    if !matches!(
        config.kind,
        crate::version::RuntimeKind::DeepSeek | crate::version::RuntimeKind::ClaudeCode
    ) {
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
        if !(config.kind == crate::version::RuntimeKind::ClaudeCode && path == &config.node_binary)
            && !path.is_absolute()
        {
            return Err(HistoryError::new(
                HistoryFailureKind::InvalidConfiguration,
                "validate",
            ));
        }
    }
    // The helper names the readers it implements; the kind check above
    // admits only those two.
    let provider = if config.kind == crate::version::RuntimeKind::ClaudeCode {
        "claude-code"
    } else {
        "deepseek"
    };
    let request = json!({"operation":operation,"provider":provider,"home":config.home,"roots":[config.root],"runtimeId":runtime_id,"nativeId":native_id,"nativeIds":native_ids,"beforeMessageId":before_message_id,"withoutMessageIds":without_message_ids});
    let input = serde_json::to_vec(&request).map_err(|error| {
        HistoryError::new(HistoryFailureKind::BridgeContract, "request")
            .with_detail(error.to_string())
    })?;
    let mut command = if config.kind == crate::version::RuntimeKind::ClaudeCode {
        let mut command = config.execution.python();
        command.args([
            "-c",
            concat!(
                include_str!("../resources/claude_context.py"),
                "\n",
                include_str!("../resources/claude_history.py")
            ),
        ]);
        command
    } else {
        let mut command = crate::process_tree::command(&config.node_binary);
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
) -> Result<Retained<Vec<ConversationSummary>>, HistoryError> {
    if config.kind == crate::version::RuntimeKind::Codex166 {
        return codex_history::list_codex_history_at(
            config.kind,
            &config.binary,
            &config.home,
            runtime_id,
            false,
            &config.execution,
            config.os_home.as_deref(),
        )
        .map_err(|error| native_history_error(error, "native_list"));
    }
    let bytes = execute(config, "list", runtime_id, None, &[])?;
    listed_summaries(&bytes, runtime_id, "list")
        .map(|summaries| Retained::from_parts(summaries, DataRetention::default()))
}
fn listed_summaries(
    bytes: &[u8],
    runtime_id: &str,
    phase: &'static str,
) -> Result<Vec<ConversationSummary>, HistoryError> {
    let result: List = serde_json::from_slice(bytes).map_err(|error| {
        HistoryError::new(HistoryFailureKind::BridgeContract, phase).with_detail(error.to_string())
    })?;
    if result.contract_version != 1 {
        return Err(HistoryError::new(HistoryFailureKind::BridgeContract, phase));
    }
    if !result.failures.is_empty() {
        tracing::warn!(target: "velune_agent_runtime", event = "history_entries_skipped", phase, skipped = result.failures.len(), causes = ?result.failures.iter().map(|failure| (&failure.code, &failure.detail)).collect::<Vec<_>>());
    }
    Ok(result
        .sessions
        .into_iter()
        .map(|s| s.for_runtime(runtime_id))
        .collect())
}

/// A Claude Code session by its file: when it changed and the working
/// directory its project folder records, without reading the session.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexedSession {
    pub native_id: String,
    pub size: u64,
    pub modified_at_unix_ms: Option<i64>,
    pub created_at_unix_ms: Option<i64>,
    pub cwd: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Index {
    contract_version: u32,
    entries: Vec<IndexedSession>,
}

/// The sessions a Claude Code root holds, by file state only.
pub fn index(config: &HistoryConfig) -> Result<Vec<IndexedSession>, HistoryError> {
    if config.kind != crate::version::RuntimeKind::ClaudeCode {
        return Err(HistoryError::new(
            HistoryFailureKind::UnsupportedProvider,
            "validate",
        ));
    }
    let bytes = execute(config, "index", "", None, &[])?;
    let result: Index = serde_json::from_slice(&bytes).map_err(|error| {
        HistoryError::new(HistoryFailureKind::BridgeContract, "index")
            .with_detail(error.to_string())
    })?;
    if result.contract_version != 1 {
        return Err(HistoryError::new(
            HistoryFailureKind::BridgeContract,
            "index",
        ));
    }
    Ok(result.entries)
}

/// Summaries of the named Claude Code sessions (or their sub-agents'); a
/// session that is gone is left out.
pub fn summaries(
    config: &HistoryConfig,
    runtime_id: &str,
    native_ids: &[String],
) -> Result<Vec<ConversationSummary>, HistoryError> {
    let bytes = execute(config, "summaries", runtime_id, None, native_ids)?;
    listed_summaries(&bytes, runtime_id, "summaries")
}

/// Naming metadata Claude Code persists separately from its live event stream.
pub fn claude_title(
    config: &HistoryConfig,
    native_id: &str,
) -> Result<Option<String>, HistoryError> {
    if config.kind != RuntimeKind::ClaudeCode {
        return Err(HistoryError::new(
            HistoryFailureKind::UnsupportedProvider,
            "title",
        ));
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Title {
        Error { error: BridgeFailure },
        Value { title: Option<String> },
    }
    let bytes = execute(config, "title", "", Some(native_id), &[])?;
    match serde_json::from_slice(&bytes) {
        Ok(Title::Value { title }) => Ok(title),
        Ok(Title::Error { error }) => Err(bridge_error(&error.code, &error.detail, "title")),
        Err(error) => Err(
            HistoryError::new(HistoryFailureKind::BridgeContract, "title")
                .with_detail(error.to_string()),
        ),
    }
}

/// The conversations a runtime archived itself; only Codex has an archive.
pub fn list_archived(
    config: &HistoryConfig,
    runtime_id: &str,
) -> Result<Retained<Vec<ConversationSummary>>, HistoryError> {
    if config.kind != crate::version::RuntimeKind::Codex166 {
        return Ok(Retained::from_parts(Vec::new(), DataRetention::default()));
    }
    codex_history::list_codex_history_at(
        config.kind,
        &config.binary,
        &config.home,
        runtime_id,
        true,
        &config.execution,
        config.os_home.as_deref(),
    )
    .map_err(|error| native_history_error(error, "native_list"))
}
/// One conversation its runtime describes by identity, with whether the
/// runtime archived it; only Codex can, for others there is none.
pub fn read_summary(
    config: &HistoryConfig,
    runtime_id: &str,
    native_id: &str,
) -> Result<Option<Retained<(ConversationSummary, bool)>>, HistoryError> {
    if config.kind != crate::version::RuntimeKind::Codex166 {
        return Ok(None);
    }
    codex_history::read_codex_summary_at(
        config.kind,
        &config.binary,
        &config.home,
        runtime_id,
        native_id,
        &config.execution,
        config.os_home.as_deref(),
    )
    .map(Some)
    .map_err(|error| native_history_error(error, "native_summary"))
}
/// Export model input after native compaction, independently of the UI projection.
pub fn export_context(
    config: &HistoryConfig,
    native_id: &str,
) -> NativeResult<Retained<crate::import::Context>> {
    if config.kind == crate::version::RuntimeKind::Codex166 {
        return codex_history::export_codex_context(config, native_id);
    }
    let bytes = execute(config, "export_context", "", Some(native_id), &[])
        .map_err(|error| Error::new(error.full_message()))?;
    crate::import::Context::parse(&bytes)
        .map(|context| Retained::from_parts(context, DataRetention::default()))
}

/// Exports Codex's effective native context with a generated portable summary
/// inserted at its compaction boundary.
pub fn export_codex_context_with_summary(
    config: &HistoryConfig,
    native_id: &str,
    summary: &str,
) -> NativeResult<Retained<crate::import::Context>> {
    if config.kind != crate::version::RuntimeKind::Codex166 {
        return Err(Error::with_code(
            "portable compaction summaries are only supported for Codex",
            "unsupported",
        ));
    }
    codex_history::export_codex_context_with_summary(config, native_id, summary)
}

/// Whether Codex context export needs a portable summary for encrypted
/// compaction. This metadata-only result does not expose rollout contents.
pub fn codex_requires_portable_summary(
    config: &HistoryConfig,
    native_id: &str,
) -> NativeResult<bool> {
    if config.kind != crate::version::RuntimeKind::Codex166 {
        return Err(Error::with_code(
            "portable compaction summary checks are only supported for Codex",
            "unsupported",
        ));
    }
    codex_history::codex_requires_portable_summary(config, native_id)
}

/// Gets the effective Codex model/provider identity without exposing history.
pub fn codex_thread_model(
    config: &HistoryConfig,
    native_id: &str,
) -> NativeResult<Retained<(String, String)>> {
    if config.kind != crate::version::RuntimeKind::Codex166 {
        return Err(Error::with_code(
            "Codex thread model lookup is only supported for Codex",
            "unsupported",
        ));
    }
    codex_history::codex_thread_model(config, native_id)
}

/// Lists the model options visible to a source Codex provider, including its
/// reasoning efforts. Run on a history worker; the result is provider-scoped.
pub fn codex_models_for_provider(
    config: &HistoryConfig,
    provider_id: &str,
) -> NativeResult<Retained<Vec<crate::native::ModelOption>>> {
    if config.kind != crate::version::RuntimeKind::Codex166 {
        return Err(Error::with_code(
            "Codex model catalog is only supported for Codex",
            "unsupported",
        ));
    }
    codex_history::codex_models_for_provider(config, provider_id)
}

/// Archives a crash-abandoned summary fork only when its exact marker,
/// source lineage, registered thread identity, fingerprint and child-free
/// state still match.
pub fn recover_codex_handoff_summary(
    config: &HistoryConfig,
    source_native_id: &str,
    work_id: &str,
    expected_temporary_thread_id: Option<&str>,
    expected_temporary_fingerprint: Option<&str>,
) -> NativeResult<crate::native::HandoffSummaryRecovery> {
    codex_history::recover_codex_handoff_summary(
        config,
        source_native_id,
        work_id,
        expected_temporary_thread_id,
        expected_temporary_fingerprint,
    )
}

/// Checks whether a prepared summary's source is still the same immediately
/// before target creation. Run this RPC/file check on a history worker.
pub fn verify_codex_handoff_summary_source(
    config: &HistoryConfig,
    source_native_id: &str,
    source_provider_id: &str,
    source_model_id: &str,
    source_turn_id: &str,
    source_fingerprint: &str,
) -> NativeResult<bool> {
    codex_history::verify_codex_handoff_summary_source(
        config,
        source_native_id,
        source_provider_id,
        source_model_id,
        source_turn_id,
        source_fingerprint,
    )
}

/// A displayed Claude record may precede the current compacted replay chain.
pub(crate) fn claude_replay_contains(
    config: &HistoryConfig,
    native_id: &str,
    message_id: &str,
) -> NativeResult<bool> {
    #[derive(Deserialize)]
    struct ReplayBoundary {
        found: bool,
    }
    let bytes = execute_before(
        config,
        "contains_replay_message",
        "",
        Some(native_id),
        &[],
        Some(message_id),
    )
    .map_err(|error| Error::new(error.full_message()))?;
    serde_json::from_slice::<ReplayBoundary>(&bytes)
        .map(|boundary| boundary.found)
        .map_err(|error| Error::new(format!("Claude 原生分支检查响应无效：{error}")))
}

/// Effective model context immediately before one exact native user message.
/// The boundary is resolved before compaction, so later summaries cannot
/// leak the replaced message or its replies back into the rebuilt session.
pub fn export_context_before(
    config: &HistoryConfig,
    native_id: &str,
    message_id: &str,
) -> NativeResult<Retained<crate::import::Context>> {
    if config.kind == crate::version::RuntimeKind::Codex166 {
        return codex_history::export_codex_context_before(config, native_id, message_id);
    }
    if config.kind != crate::version::RuntimeKind::ClaudeCode {
        return Err(Error::with_code(
            "此运行时尚未接入历史边界导出",
            "unsupported",
        ));
    }
    let bytes = execute_before(
        config,
        "export_context",
        "",
        Some(native_id),
        &[],
        Some(message_id),
    )
    .map_err(|error| Error::new(error.full_message()))?;
    crate::import::Context::parse_before(&bytes)
        .map(|context| Retained::from_parts(context, DataRetention::default()))
}

/// Effective context with exactly one native user prompt omitted. Later
/// replies and tool records remain native context; no model turn is started.
pub fn export_context_without(
    config: &HistoryConfig,
    native_id: &str,
    message_id: &str,
) -> NativeResult<Retained<crate::import::Context>> {
    if config.kind == crate::version::RuntimeKind::Codex166 {
        return codex_history::export_codex_context_without(config, native_id, message_id);
    }
    if config.kind != crate::version::RuntimeKind::ClaudeCode {
        return Err(Error::with_code(
            "此运行时尚未接入单条消息导出",
            "unsupported",
        ));
    }
    let bytes = execute_before(
        config,
        "export_context_without",
        "",
        Some(native_id),
        &[],
        Some(message_id),
    )
    .map_err(|error| Error::new(error.full_message()))?;
    crate::import::Context::parse(&bytes)
        .map(|context| Retained::from_parts(context, DataRetention::default()))
}

pub fn export_context_without_many(
    config: &HistoryConfig,
    native_id: &str,
    message_ids: &[String],
) -> NativeResult<Retained<crate::import::Context>> {
    if config.kind == crate::version::RuntimeKind::Codex166 {
        return codex_history::export_codex_context_without_many(config, native_id, message_ids);
    }
    if config.kind != crate::version::RuntimeKind::ClaudeCode || message_ids.is_empty() {
        return Err(Error::with_code(
            "此运行时尚未接入精确消息导出",
            "unsupported",
        ));
    }
    let bytes = execute_selection(
        config,
        "export_context_without_many",
        "",
        Some(native_id),
        &[],
        None,
        message_ids,
    )
    .map_err(|error| Error::new(error.full_message()))?;
    crate::import::Context::parse(&bytes)
        .map(|context| Retained::from_parts(context, DataRetention::default()))
}

/// Resolve an exact Codex thread and read its display history in one
/// app-server connection, using the same native receipt for both.
pub fn read_codex_with_summary(
    config: &HistoryConfig,
    runtime_id: &str,
    native_id: &str,
) -> Result<Retained<(ConversationSummary, History)>, HistoryError> {
    if config.kind != crate::version::RuntimeKind::Codex166 {
        return Err(HistoryError::new(
            HistoryFailureKind::UnsupportedProvider,
            "validate",
        ));
    }
    codex_history::read_codex_with_summary_at(config, runtime_id, native_id)
        .map_err(|error| native_history_error(error, "native_read"))
}

pub fn read(
    config: &HistoryConfig,
    runtime_id: &str,
    native_id: &str,
) -> Result<Retained<History>, HistoryError> {
    if config.kind == crate::version::RuntimeKind::Codex166 {
        return codex_history::read_codex_history_at(
            config.kind,
            &config.binary,
            &config.home,
            runtime_id,
            native_id,
            &config.execution,
            config.os_home.as_deref(),
        )
        .map_err(|error| native_history_error(error, "native_read"));
    }
    Ok(Retained::from_parts(
        read_bridge(config, runtime_id, native_id)?.history,
        DataRetention::default(),
    ))
}

/// A cold Claude read returns the history and its metadata in one helper.
pub fn read_claude_with_summary(
    config: &HistoryConfig,
    runtime_id: &str,
    native_id: &str,
) -> Result<Retained<(ConversationSummary, History)>, HistoryError> {
    let read = read_bridge(config, runtime_id, native_id)?;
    let summary = read
        .summary
        .filter(|summary| summary.native_id == native_id)
        .ok_or_else(|| HistoryError::new(HistoryFailureKind::BridgeContract, "read_summary"))?;
    Ok(Retained::from_parts(
        (summary.for_runtime(runtime_id), read.history),
        DataRetention::default(),
    ))
}

fn read_bridge(
    config: &HistoryConfig,
    runtime_id: &str,
    native_id: &str,
) -> Result<Read, HistoryError> {
    let bytes = execute(config, "read", runtime_id, Some(native_id), &[])?;
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
    Ok(result)
}

fn native_history_error(error: Error, phase: &'static str) -> HistoryError {
    tracing::warn!(target:"velune_agent_runtime",event="history_native_failed",phase,runtime_family="codex",native_code=error.code());
    HistoryError::new(HistoryFailureKind::ProviderRead, phase).with_detail(error.detail())
}

/// A Pi session file and what its first line records. Pi appends to its
/// session files, so an unchanged size and modification time mean unchanged
/// metadata. Size and nanosecond modification time are opaque decimal strings,
/// so no precision is lost crossing JSON.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiSessionFile {
    pub path: String,
    #[serde(default)]
    pub parent_path: Option<String>,
    pub session_id: Option<String>,
    pub size: String,
    pub mtime_ns: String,
    pub modified_at_unix_ms: Option<i64>,
    /// The session header's working directory and start; a hint until Pi
    /// reads the session.
    pub cwd: Option<String>,
    pub created_at_unix_ms: Option<i64>,
}
/// Pi SessionManager metadata, read only for the files asked for.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiSessionSummary {
    pub path: String,
    #[serde(default)]
    pub parent_path: Option<String>,
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
    pub files: Vec<PiSessionFile>,
    pub sessions: Vec<PiSessionSummary>,
}

/// Every Pi session file, with Pi's metadata for the files in `read`.
pub fn list_pi(config: &crate::Config, read: &[String]) -> NativeResult<PiListing> {
    let input = serde_json::to_vec(&json!({ "want": read }))
        .map_err(|error| Error::new(format!("Pi 历史列表请求无效：{error}")))?;
    let listing: PiListing =
        serde_json::from_value(execute_pi(config, PiOperation::List, Some(input))?)
            .map_err(|error| Error::new(format!("Pi 历史 SDK 格式不匹配：{error}")))?;
    if listing.contract_version != 3 {
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
    execute_pi(config, PiOperation::Inspect(session, false), None)
}

/// Inspect a cold Pi history and its metadata without constructing an index.
pub fn read_pi_with_summary(
    config: &crate::Config,
    session: &Path,
) -> NativeResult<(PiSessionSummary, serde_json::Value)> {
    let mut value = execute_pi(config, PiOperation::Inspect(session, true), None)?;
    let summary: PiSessionSummary = serde_json::from_value(
        value
            .get_mut("summary")
            .map(serde_json::Value::take)
            .unwrap_or_default(),
    )
    .map_err(|error| Error::new(format!("Pi 历史摘要格式不匹配：{error}")))?;
    if Path::new(&summary.path).as_os_str() != session.as_os_str() {
        return Err(Error::new("Pi 返回不同的原生会话"));
    }
    Ok((summary, value))
}
pub fn export_pi_context(
    config: &crate::Config,
    session: &std::path::Path,
) -> NativeResult<crate::import::Context> {
    let value = execute_pi(config, PiOperation::ExportContext(session), None)?;
    crate::import::Context::parse(
        &serde_json::to_vec(&value).map_err(|error| Error::new(error.to_string()))?,
    )
}

pub fn export_pi_context_before(
    config: &crate::Config,
    session: &std::path::Path,
    message_id: &str,
) -> NativeResult<crate::import::Context> {
    let value = execute_pi(config, PiOperation::ExportBefore(session, message_id), None)?;
    crate::import::Context::parse_before(
        &serde_json::to_vec(&value).map_err(|error| Error::new(error.to_string()))?,
    )
}
/// A stable source entry, never a live Agent/RPC host to switch.
#[derive(Clone, Copy)]
pub enum PiForkBoundary<'a> {
    ThroughEntry(&'a str),
    BeforeEntry(&'a str),
    /// An in-place edit fallback retains active-branch context edits on the prefix.
    BeforeEditedEntry {
        entry: &'a str,
        expected_digest: &'a str,
    },
}

pub fn fork_pi_session(
    config: &crate::Config,
    source: &std::path::Path,
    boundary: PiForkBoundary<'_>,
) -> NativeResult<std::path::PathBuf> {
    let (position, entry, expected_digest) = match boundary {
        PiForkBoundary::ThroughEntry(entry) => ("at", entry, None),
        PiForkBoundary::BeforeEntry(entry) => ("before", entry, None),
        PiForkBoundary::BeforeEditedEntry {
            entry,
            expected_digest,
        } => ("before", entry, Some(expected_digest)),
    };
    let input = serde_json::to_vec(&json!({"position":position,"entryId":entry,"preservePrefixEdits":expected_digest.is_some(),"expectedDigest":expected_digest}))
        .map_err(|error| Error::new(error.to_string()))?;
    let value = execute_pi(config, PiOperation::Fork(source), Some(input))
        .map_err(|error| Error::with_code(error.to_string(), "fork_outcome_unknown"))?;
    if let Some(error) = value["error"].as_str() {
        let code = match value["code"].as_str() {
            Some("fork_rejected") => "fork_rejected",
            Some("fork_created") => "fork_created",
            _ => "fork_outcome_unknown",
        };
        return Err(Error::with_code(error, code));
    }
    let path = value["path"]
        .as_str()
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| Error::with_code("Pi 分叉没有返回原生身份", "fork_outcome_unknown"))?;
    Ok(path)
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
/// Delete from Pi's effective native context without starting a model turn.
pub fn delete_pi_message(
    config: &crate::Config,
    session: &std::path::Path,
    message_id: &str,
    truncate: bool,
    expected_digest: &str,
    backup_directory: &std::path::Path,
) -> NativeResult<()> {
    let input = serde_json::to_vec(&json!({
        "messageId":message_id, "truncate":truncate,
        "expectedDigest":expected_digest, "backupDirectory":backup_directory
    }))
    .map_err(|error| Error::new(error.to_string()))?;
    let result = execute_pi(config, PiOperation::DeleteMessage(session), Some(input))?;
    if result["ok"] != true {
        return Err(Error::new("Pi 消息删除未确认"));
    }
    Ok(())
}
enum PiOperation<'a> {
    List,
    Inspect(&'a std::path::Path, bool),
    ExportContext(&'a std::path::Path),
    ExportBefore(&'a std::path::Path, &'a str),
    Fork(&'a std::path::Path),
    Rename(&'a std::path::Path),
    Delete(&'a std::path::Path),
    DeleteMessage(&'a std::path::Path),
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
    let mut command = crate::process_tree::command(node);
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
        .arg(crate::version::RuntimeKind::Pi.id());
    match operation {
        PiOperation::List => {}
        PiOperation::ExportContext(session) => {
            command.arg("--export-context").arg(session);
        }
        PiOperation::ExportBefore(session, message_id) => {
            command
                .arg("--export-context")
                .arg(session)
                .arg("--before-message-id")
                .arg(message_id);
        }
        PiOperation::Fork(session) => {
            command.arg("--fork-session").arg(session);
        }
        PiOperation::Inspect(session, with_summary) => {
            command.arg("--inspect-session").arg(session);
            if with_summary {
                command.arg("--with-summary");
            }
        }
        PiOperation::Rename(session) => {
            command.arg("--rename-session").arg(session);
        }
        PiOperation::Delete(session) => {
            command.arg("--delete-session").arg(session);
        }
        PiOperation::DeleteMessage(session) => {
            command.arg("--delete-message").arg(session);
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
