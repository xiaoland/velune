//! Read-only browsing and deferred execution preparation share one active view.
use super::*;
use crate::conversation::{ConversationActions, ConversationSnapshot, Message, MessageBlock};
use std::collections::BTreeSet;
use velune_agent_runtime::creation::{DataRetention, Retained};

pub(super) fn history_failure(
    runtime: &RuntimeInstance,
    error: velune_agent_runtime::history::HistoryError,
) -> RuntimeError {
    let family = velune_agent_runtime::version::variant(runtime.type_id).family_id;
    tracing::warn!(target: "velune_application", event = "runtime_history_read_failed", phase = error.phase(), failure_kind = error.code(), runtime_family = family, runtime_id=%runtime.id, runtime_name=%runtime.name, runtime_type_id=%runtime.type_id, binary=?runtime.settings.get("binary"), node_binary=?runtime.settings.get("nodeBinary"), agent_dir=?runtime.settings.get("agentDir"), detail=%error);
    RuntimeError::History(error)
}

pub(super) fn runtime_context(runtime: &RuntimeInstance) -> String {
    format!(
        "运行时 {}（id={}，type={}）；binary={}；nodeBinary={}；agentDir={}；sessionDir={}",
        runtime.name,
        runtime.id,
        runtime.type_id,
        runtime
            .settings
            .get("binary")
            .map(String::as_str)
            .unwrap_or("未配置"),
        runtime
            .settings
            .get("nodeBinary")
            .map(String::as_str)
            .unwrap_or("未配置"),
        runtime
            .settings
            .get("agentDir")
            .map(String::as_str)
            .unwrap_or("未配置"),
        runtime
            .settings
            .get("sessionDir")
            .map(String::as_str)
            .unwrap_or("默认"),
    )
}
/// The latest index of one runtime instance's native history, valid while
/// its configuration is unchanged. Claude Code and Pi sessions are indexed
/// from their files and read only when needed; other runtimes list read
/// summaries directly.
pub(super) struct HistoryListing {
    source: runtime_source::RuntimeSourceKey,
    entries: Vec<IndexedConversation>,
}

/// Cloning an index entry shares the actual immutable summary allocation, not
/// a deep copy whose charge would disappear with another view's cache.
#[derive(Clone)]
pub(super) struct SharedSummary(std::sync::Arc<Retained<ConversationSummary>>);
impl SharedSummary {
    pub(super) fn retained(summary: Retained<ConversationSummary>) -> Self {
        Self(std::sync::Arc::new(summary))
    }
    pub(super) fn into_retained(self) -> Result<Retained<ConversationSummary>, RuntimeError> {
        match std::sync::Arc::try_unwrap(self.0) {
            Ok(summary) => Ok(summary),
            Err(summary) => summary
                .try_clone()
                .map_err(|error| RuntimeError::context("history summary credit", error)),
        }
    }
    fn update_from<T: Clone>(
        &mut self,
        source: &Retained<T>,
        update: impl FnOnce(&mut ConversationSummary, T),
    ) -> Result<(), RuntimeError> {
        let source = source
            .try_clone()
            .map_err(|error| RuntimeError::context("history receipt credit", error))?;
        let summary = self
            .0
            .try_clone()
            .map_err(|error| RuntimeError::context("history summary credit", error))?;
        *self = Self::retained(summary.zip(source).map(|(mut summary, source)| {
            update(&mut summary, source);
            summary
        }));
        Ok(())
    }
    pub(super) fn update(
        &mut self,
        update: &impl Fn(&mut ConversationSummary) -> bool,
    ) -> Result<(), RuntimeError> {
        let owned = self
            .0
            .try_clone()
            .map_err(|error| RuntimeError::context("history summary credit", error))?;
        *self = Self::retained(owned.map(|mut summary| {
            update(&mut summary);
            summary
        }));
        Ok(())
    }
}
impl std::ops::Deref for SharedSummary {
    type Target = ConversationSummary;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().as_ref()
    }
}
impl AsRef<ConversationSummary> for SharedSummary {
    fn as_ref(&self) -> &ConversationSummary {
        self
    }
}
impl From<ConversationSummary> for SharedSummary {
    fn from(summary: ConversationSummary) -> Self {
        Self::retained(Retained::from_parts(summary, DataRetention::default()))
    }
}
/// Split moved entries. Every piece shares the original page charge until
/// the last actually shared/moved summary is released.
pub(super) fn retained_values<T>(mut values: Retained<Vec<T>>) -> Vec<Retained<T>> {
    let mut entries = Vec::with_capacity(values.len());
    while !values.is_empty() {
        let (entry, rest) =
            values.split_map(|mut values| (values.pop().expect("nonempty"), values));
        entries.push(entry);
        values = rest;
    }
    entries.reverse();
    entries
}
pub(super) fn shared_summaries(
    summaries: Retained<Vec<ConversationSummary>>,
) -> Vec<SharedSummary> {
    retained_values(summaries)
        .into_iter()
        .map(SharedSummary::retained)
        .collect()
}

/// Fallback projections retain the same provenance as ordinary indexes, including across client views.
#[derive(Clone)]
pub(super) struct CachedSource {
    pub(super) source: runtime_source::RuntimeSourceKey,
    pub(super) value: organization::ListedSource,
}

#[derive(Clone)]
pub(super) struct HistoryArchive {
    pub(super) source: runtime_source::RuntimeSourceKey,
    pub(super) summaries: Vec<SharedSummary>,
}
#[derive(Clone)]
pub(super) struct IndexedConversation {
    pub(super) id: String,
    /// The native record's state; a read summary holds while it is unchanged.
    pub(super) identity: String,
    /// What the files tell before the session is read.
    pub(super) hint: ConversationHint,
    pub(super) state: EntryState,
}
#[derive(Clone, Default)]
pub(super) struct ConversationHint {
    pub(super) parent_conversation_id: Option<String>,
    pub(super) session_id: Option<String>,
    pub(super) cwd: Option<String>,
    /// The file's modification time: which sessions to read first, never shown.
    pub(super) activity_unix_ms: Option<i64>,
    pub(super) created_unix_ms: Option<i64>,
}
#[derive(Clone)]
pub(super) enum EntryState {
    Unread,
    Read(SharedSummary),
    /// The runtime could not read it as a session; not listed until it changes.
    Unreadable,
}
/// A read job carries only the source identity and native read inputs.
pub(super) struct HistoryRead {
    source: runtime_source::RuntimeSourceKey,
    preparation: runtime_execution::ExecutionPreparation,
    summary: Option<SharedSummary>,
    native_id: String,
    acp_cwd: Option<PathBuf>,
    pi_context: Option<String>,
}

/// Internal read-side owner. The wire snapshot has no retention fields.
/// Moving this holder moves its receive credit; replacing/evicting History drops both.
#[derive(Debug)]
pub(super) struct HistorySnapshot {
    pub(super) snapshot: ConversationSnapshot,
    pub(super) retention: DataRetention,
}
impl HistorySnapshot {
    pub(super) fn into_retained(self) -> Retained<ConversationSnapshot> {
        Retained::from_parts(self.snapshot, self.retention)
    }
    pub(super) fn try_clone(&self) -> Result<Self, RuntimeError> {
        // Reserve the independent copy before allocating its transcript.
        let retention = self
            .retention
            .try_clone()
            .map_err(|error| RuntimeError::context("history retained-data credit", error))?;
        Ok(Self {
            snapshot: self.snapshot.clone(),
            retention,
        })
    }
    pub(super) fn from_retained(snapshot: Retained<ConversationSnapshot>) -> Self {
        let (snapshot, retention) = snapshot.into_parts();
        Self {
            snapshot,
            retention,
        }
    }
}
impl From<Retained<ConversationSnapshot>> for HistorySnapshot {
    fn from(snapshot: Retained<ConversationSnapshot>) -> Self {
        Self::from_retained(snapshot)
    }
}
impl From<ConversationSnapshot> for HistorySnapshot {
    fn from(snapshot: ConversationSnapshot) -> Self {
        Self {
            snapshot,
            retention: DataRetention::default(),
        }
    }
}
impl std::ops::Deref for HistorySnapshot {
    type Target = ConversationSnapshot;
    fn deref(&self) -> &Self::Target {
        &self.snapshot
    }
}
impl std::ops::DerefMut for HistorySnapshot {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.snapshot
    }
}

pub(super) struct LoadedHistory {
    source: runtime_source::RuntimeSourceKey,
    prepared_resources: Option<String>,
    snapshot: HistorySnapshot,
    acp_capabilities: Option<crate::AcpCapabilities>,
}

pub(super) fn history_read_changed() -> RuntimeError {
    let mut failure = crate::Failure::new(
        crate::FailureKind::Conflict,
        "读取期间会话来源已改变，请重新打开",
    );
    failure.code = "history_source_changed".into();
    failure.phase = "history_read".into();
    RuntimeError::Contract(failure)
}

impl HistoryRead {
    pub(super) fn source(&self) -> &runtime_source::RuntimeSourceKey {
        &self.source
    }

    pub(super) fn load(self) -> Result<LoadedHistory, RuntimeError> {
        let logical_id = self.native_id.clone();
        self.load_with_context(&logical_id, None)
    }

    /// Keep the logical conversation and physical native source together in
    /// local diagnostics without recording native paths or conversation text.
    pub(super) fn load_with_context(
        self,
        logical_id: &str,
        segment_index: Option<usize>,
    ) -> Result<LoadedHistory, RuntimeError> {
        let started = std::time::Instant::now();
        let runtime_type = self.source.kind();
        let runtime_id = self.source.runtime_id().to_owned();
        let identity = diagnostic_session_identity(runtime_type, logical_id, &self.native_id);
        tracing::info!(
            target: "velune_application",
            event = "runtime_history_read_started",
            phase = "native_read",
            runtime_id = %runtime_id,
            runtime_type = %runtime_type,
            logical_conversation_id_sha256 = %identity.logical_conversation_id_sha256,
            logical_session_id = %identity.logical_session_id,
            native_identity_sha256 = %identity.native_identity_sha256,
            native_session_id = %identity.native_session_id,
            segment_index,
        );
        let result = self.load_native();
        tracing::info!(
            target: "velune_application",
            event = "runtime_history_read",
            phase = "native_read",
            runtime_id = %runtime_id,
            runtime_type = %runtime_type,
            logical_conversation_id_sha256 = %identity.logical_conversation_id_sha256,
            logical_session_id = %identity.logical_session_id,
            native_identity_sha256 = %identity.native_identity_sha256,
            native_session_id = %identity.native_session_id,
            segment_index,
            elapsed_ms = started.elapsed().as_millis() as u64,
            success = result.is_ok(),
        );
        result
    }

    fn load_native(self) -> Result<LoadedHistory, RuntimeError> {
        use runtime_source::Reader;
        let Self {
            source,
            preparation,
            summary,
            native_id,
            acp_cwd,
            pi_context,
        } = self;
        let prepared_resources = preparation.prepare()?;
        let runtime_id = source.runtime_id();
        let reader = source.reader();
        let mut pi_history = None;
        let mut native_history = None;
        let mut retention = DataRetention::default();
        let summary = summary
            .map(SharedSummary::into_retained)
            .transpose()?
            .map(|summary| {
                let (summary, guard) = summary.into_parts();
                retention = std::mem::take(&mut retention).merge(guard);
                summary
            });
        let summary = match (summary, reader) {
            (Some(summary), _) => Some(summary),
            (None, _) if acp_cwd.is_some() => None,
            (None, Reader::Pi(config)) => {
                let (summary, saved) = history::read_pi_with_summary(config, Path::new(&native_id))
                    .map_err(|error| {
                        RuntimeError::context(pi_context.as_deref().unwrap_or("Pi 历史读取"), error)
                    })?;
                pi_history = Some(saved);
                Some(pi_history_summary(runtime_id, summary))
            }
            (None, Reader::Native(config)) if config.provider == "codex" => {
                let ((summary, saved), guard) =
                    history::read_codex_with_summary(config, runtime_id, &native_id)?.into_parts();
                retention = retention.merge(guard);
                native_history = Some(saved);
                Some(summary)
            }
            (None, Reader::Native(config)) if config.provider == "claude-code" => {
                let ((summary, saved), guard) =
                    history::read_claude_with_summary(config, runtime_id, &native_id)?.into_parts();
                retention = retention.merge(guard);
                native_history = Some(saved);
                Some(summary)
            }
            (None, _) => {
                let (summary, guard) = load_history_summary(reader, runtime_id, &native_id).map_err(|error| {
                    tracing::warn!(target: "velune_application", event = "runtime_history_read_failed", phase = "history_lookup", runtime_type_id = %source.kind(), detail = %error);
                    error
                })?.into_parts();
                retention = retention.merge(guard);
                Some(summary)
            }
        };
        if let Reader::Acp(config) = reader {
            let cwd = acp_cwd
                .as_deref()
                .or_else(|| {
                    summary
                        .as_ref()
                        .and_then(|summary| summary.cwd.as_deref())
                        .map(Path::new)
                })
                .ok_or_else(|| RuntimeError::invalid("ACP 会话列表缺少工作目录"))?;
            let (snapshot, capabilities) = super::acp_composition::load_acp_history(
                config.clone(),
                runtime_id,
                &native_id,
                cwd,
            )?;
            return Ok(LoadedHistory {
                source,
                prepared_resources,
                snapshot: HistorySnapshot::from_retained(snapshot.map(|snapshot| snapshot)),
                acp_capabilities: Some(capabilities),
            });
        }

        let summary = summary.expect("native history resolves source membership before reading");
        let mut snapshot = ConversationSnapshot {
            retained_reply_ids: Vec::new(),
            work_notification_ids: Vec::new(),
            revision: 1,
            context_runtime_id: runtime_id.to_owned(),
            conversation: summary,
            resource_id: None,
            model_record_key: None,
            run_state: RunState::Idle,
            messages: Vec::new(),
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
                can_send: false,
                can_cancel: false,
                can_switch: true,
            },
        };
        match reader {
            Reader::Pi(config) => {
                let saved = match pi_history {
                    Some(saved) => saved,
                    None => pi_session_helper(config, Path::new(&native_id)).map_err(|error| {
                        RuntimeError::context(pi_context.as_deref().unwrap_or("Pi 历史读取"), error)
                    })?,
                };
                let mut projection = PiProjection::new(snapshot.conversation.clone());
                projection.replace_history(&saved);
                let projected = projection.snapshot.expect("history projection");
                snapshot.messages = projected.messages;
                snapshot.retained_reply_ids = projected.retained_reply_ids;
                snapshot.work_notification_ids = projected.work_notification_ids;
                snapshot.executions = projected.executions;
                snapshot.sub_agents = projected.sub_agents;
                snapshot.conversation.cwd = saved["cwd"].as_str().map(str::to_owned);
                let marker = &saved["virtualState"];
                if marker["provider"] == "velune" || marker["provider"] == "velune-gateway" {
                    snapshot.model_record_key = marker["state"]["modelRecordKey"]
                        .as_str()
                        .map(str::to_owned);
                }
            }
            Reader::Native(config) => {
                let read = match native_history {
                    Some(saved) => saved,
                    None => {
                        let (read, guard) =
                            history::read(config, runtime_id, &native_id)?.into_parts();
                        retention = retention.merge(guard);
                        read
                    }
                };
                snapshot.conversation.native_read_state = read.native_read_state;
                snapshot.conversation.cwd = read.cwd;
                snapshot.messages = read.messages;
                snapshot.executions = read.executions;
                snapshot.pending_interactions = read.pending_interactions;
                snapshot.follow_up.can_resume = read.can_resume;
            }
            Reader::Acp(_) => unreachable!("ACP history has its own snapshot"),
        }
        // The next-turn model is supplied when the user submits. Sub-agent
        // conversations remain read-only.
        snapshot.actions.can_send = snapshot.conversation.parent_conversation_id.is_none();
        Ok(LoadedHistory {
            source,
            prepared_resources,
            snapshot: HistorySnapshot {
                snapshot,
                retention,
            },
            acp_capabilities: None,
        })
    }
}

pub(super) struct DiagnosticSessionIdentity {
    pub(super) logical_conversation_id_sha256: String,
    pub(super) logical_session_id: String,
    pub(super) native_identity_sha256: String,
    pub(super) native_session_id: String,
}

/// Stable, privacy-safe session labels shared by history and projection
/// diagnostics. Known native UUIDs remain searchable; unknown identifiers are
/// represented only by a digest. Native paths and conversation text are never
/// copied into a diagnostic field.
pub(super) fn diagnostic_session_identity(
    runtime_type: RuntimeKind,
    logical_id: &str,
    native_id: &str,
) -> DiagnosticSessionIdentity {
    let logical_session_id = logical_session_id_label(runtime_type, logical_id);
    let native_session_id = session_id_label(runtime_type, native_id);
    DiagnosticSessionIdentity {
        logical_conversation_id_sha256: identity_digest(logical_id),
        logical_session_id,
        native_identity_sha256: identity_digest(native_id),
        native_session_id,
    }
}

fn logical_session_id_label(runtime_type: RuntimeKind, identity: &str) -> String {
    let uuid = match runtime_type {
        RuntimeKind::Pi => pi_session_uuid(identity),
        RuntimeKind::ClaudeCode | RuntimeKind::Codex166 => runtime_session_uuid(identity),
        RuntimeKind::Acp | RuntimeKind::DeepSeek => None,
    }
    .or_else(|| {
        identity
            .rsplit_once(':')
            .and_then(|(_, token)| normalized_uuid(token))
    });
    uuid.unwrap_or_else(|| format!("sha256:{}", identity_digest(identity)))
}

fn session_id_label(runtime_type: RuntimeKind, identity: &str) -> String {
    let uuid = match runtime_type {
        RuntimeKind::Pi => pi_session_uuid(identity),
        RuntimeKind::ClaudeCode | RuntimeKind::Codex166 => runtime_session_uuid(identity),
        RuntimeKind::Acp | RuntimeKind::DeepSeek => None,
    };
    uuid.unwrap_or_else(|| format!("sha256:{}", identity_digest(identity)))
}

fn runtime_session_uuid(identity: &str) -> Option<String> {
    let token = identity
        .strip_prefix("thread-")
        .or_else(|| identity.strip_prefix("thread_"))
        .unwrap_or(identity);
    normalized_uuid(token)
}

fn pi_session_uuid(identity: &str) -> Option<String> {
    let basename = Path::new(identity).file_name()?.to_str()?;
    let stem = basename.strip_suffix(".jsonl")?;
    let token = stem.rsplit_once('_').map_or(stem, |(_, token)| token);
    normalized_uuid(token)
}

fn normalized_uuid(token: &str) -> Option<String> {
    let bytes = token.as_bytes();
    (bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        }))
    .then(|| token.to_ascii_lowercase())
}

fn identity_digest(identity: &str) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(identity.as_bytes()))
}

fn pi_history_summary(runtime_id: &str, session: history::PiSessionSummary) -> ConversationSummary {
    ConversationSummary {
        native_read_state: None,
        can_rename: false,
        can_delete: false,
        id: format!("{runtime_id}:{}", session.path),
        title: velune_conversation::conversation_title(
            session.name.as_deref(),
            session.first_message.as_deref(),
        ),
        updated_at_unix_ms: session.modified_unix_ms,
        created_at_unix_ms: session.created_unix_ms,
        runtime_id: runtime_id.to_owned(),
        cwd: session.cwd,
        parent_conversation_id: session
            .parent_path
            .map(|parent| format!("{runtime_id}:{parent}")),
    }
}

/// Lookup belongs to the bounded history job too, not its owner-side preparation.
fn load_history_summary(
    reader: &runtime_source::Reader,
    runtime_id: &str,
    native_id: &str,
) -> Result<Retained<ConversationSummary>, RuntimeError> {
    use runtime_source::Reader;
    let id = format!("{runtime_id}:{native_id}");
    let summary = match reader {
        Reader::Pi(_) => unreachable!("Pi cold read already includes its summary"),
        Reader::Native(config) => history::list(config, runtime_id)?
            .map(|summaries| summaries.into_iter().find(|summary| summary.id == id)),
        Reader::Acp(config) => {
            let mut session = velune_agent_runtime::acp::Client::connect(config.clone())
                .map_err(|error| RuntimeError::context("ACP 初始化", error))?;
            let listed = session
                .list_sessions(None, runtime_id)
                .map_err(|error| RuntimeError::context("ACP 会话列表", error));
            let closed = session
                .shutdown()
                .map_err(|error| RuntimeError::context("ACP 关闭", error));
            let summaries = listed?;
            closed?;
            summaries.map(|summaries| summaries.into_iter().find(|summary| summary.id == id))
        }
    };
    summary.try_map(|summary| {
        summary.ok_or_else(|| RuntimeError::invalid("此会话不属于所选运行时历史目录"))
    })
}

/// Runtimes whose sessions are files read one by one.
fn reads_lazily(runtime: &RuntimeInstance) -> bool {
    matches!(runtime.type_id, RuntimeKind::Pi | RuntimeKind::ClaudeCode)
}
impl CoreRuntime {
    pub(super) fn runtime_instance(&self, id: &str) -> Result<&RuntimeInstance, RuntimeError> {
        self.runtime_instances
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))
    }
    pub(crate) fn select_runtime(
        &mut self,
        id: String,
    ) -> Result<crate::ConfigurationSnapshot, RuntimeError> {
        if self.session.executing() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let id = id.as_str();
        if !self.runtime_instance(id)?.enabled {
            return Err(RuntimeError::invalid("此运行时已停用，请先启用"));
        }
        if self.next_turn_runtime_id.as_deref() != Some(id) {
            self.next_turn_runtime_id = Some(id.into());
        }
        self.list()
    }
    pub(super) fn pi_history_config(
        &self,
        runtime: &RuntimeInstance,
    ) -> Result<PiConfig, RuntimeError> {
        let config = self.pi_history_configuration(runtime)?;
        self.prepare_execution_context(runtime)?;
        Ok(config)
    }

    pub(super) fn pi_history_configuration(
        &self,
        runtime: &RuntimeInstance,
    ) -> Result<PiConfig, RuntimeError> {
        let context = self.execution_context(runtime)?;
        Ok(PiConfig {
            mcp_servers: Vec::new(),
            execution: context.target,
            os_home: context.os_home,
            binary: setting_path(runtime, "binary")?,
            node_binary: Some(setting_path(runtime, "nodeBinary")?),
            sdk_helper: Some(context.resources.join("pi_sessions.mjs")),
            agent_dir: Some(context.agent_dir),
            session_dir: setting_path_optional(runtime, "sessionDir"),
            working_dir: None,
            provider: None,
            model: None,
            session: None,
            side: false,
            name: None,
            extension: None,
            rpc_entry: None,
            models_path: None,
            selection_file: None,
            gateway_token: None,
        })
    }
    pub(super) fn store_history_index(
        &self,
        source: runtime_source::RuntimeSourceKey,
        mut entries: Vec<IndexedConversation>,
    ) -> Result<(), RuntimeError> {
        if !source.is_current(self) {
            return Err(RuntimeError::invalid("运行时历史来源已改变，请刷新后重试"));
        }
        let previous = self
            .history_listings
            .borrow_mut()
            .remove(source.runtime_id())
            .filter(|listing| listing.source == source)
            .map(|listing| {
                listing
                    .entries
                    .into_iter()
                    .map(|entry| (entry.id.clone(), entry))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        for entry in &mut entries {
            if matches!(entry.state, EntryState::Unread)
                && let Some(known) = previous.get(&entry.id)
                && known.identity == entry.identity
            {
                entry.state = known.state.clone();
            }
        }
        self.history_listings.borrow_mut().insert(
            source.runtime_id().to_owned(),
            HistoryListing { source, entries },
        );
        self.advance_history_index_generation();
        Ok(())
    }
    /// The runtime's indexed conversations as last indexed.
    pub(super) fn indexed(&self, runtime: &RuntimeInstance) -> Vec<IndexedConversation> {
        self.history_listings
            .borrow()
            .get(&runtime.id)
            .filter(|listing| listing.source.matches(self, runtime))
            .map(|listing| listing.entries.clone())
            .unwrap_or_default()
    }
    pub(super) fn indexed_session_id(&self, runtime: &RuntimeInstance, id: &str) -> Option<String> {
        self.history_listings
            .borrow()
            .get(&runtime.id)
            .filter(|listing| listing.source.matches(self, runtime))?
            .entries
            .iter()
            .find(|entry| entry.id == id)?
            .hint
            .session_id
            .clone()
    }
    /// Read the named conversations of an indexed runtime. One the runtime
    /// cannot read is not listed again until its record changes; a Claude
    /// Code sub-agent, outside the index, joins it once read.
    pub(super) fn read_indexed(
        &self,
        runtime: &RuntimeInstance,
        ids: &[String],
    ) -> Result<(), RuntimeError> {
        if !reads_lazily(runtime) || ids.is_empty() {
            return Ok(());
        }
        let source = self.runtime_source_key(runtime)?;
        let prefix = format!("{}:", runtime.id);
        let natives: Vec<String> = ids
            .iter()
            .filter_map(|id| id.strip_prefix(&prefix).map(str::to_owned))
            .collect();
        let (summaries, identities): (
            BTreeMap<String, ConversationSummary>,
            BTreeMap<String, String>,
        ) = if runtime.type_id == RuntimeKind::Pi {
            let listing =
                history::list_pi(&self.pi_history_config(runtime)?, &natives).map_err(|error| {
                    RuntimeError::context("Pi 历史无法读取；请检查实例路径与 SDK 配置后重试", error)
                })?;
            let identities = listing
                .files
                .iter()
                .map(|file| {
                    (
                        format!("{prefix}{}", file.path),
                        format!("{}:{}", file.size, file.mtime_ns),
                    )
                })
                .collect();
            let summaries = listing
                .sessions
                .into_iter()
                .map(|session| {
                    let summary = pi_history_summary(&runtime.id, session);
                    (summary.id.clone(), summary)
                })
                .collect();
            (summaries, identities)
        } else {
            let summaries =
                history::summaries(&self.history_config(runtime)?, &runtime.id, &natives)
                    .map_err(|error| history_failure(runtime, error))?
                    .into_iter()
                    .map(|summary| (summary.id.clone(), summary))
                    .collect();
            (summaries, BTreeMap::new())
        };
        let mut listings = self.history_listings.borrow_mut();
        let Some(listing) = listings
            .get_mut(&runtime.id)
            .filter(|listing| listing.source == source && source.matches(self, runtime))
        else {
            return Ok(());
        };
        let mut summaries = summaries;
        for id in ids {
            let summary = summaries.remove(id);
            match listing.entries.iter_mut().find(|entry| entry.id == *id) {
                Some(entry) => {
                    if let Some(identity) = identities.get(id) {
                        entry.identity = identity.clone();
                    }
                    entry.state = summary.map_or(EntryState::Unreadable, |summary| {
                        EntryState::Read(summary.into())
                    });
                }
                None => {
                    if let Some(summary) = summary {
                        listing.entries.push(IndexedConversation {
                            id: id.clone(),
                            identity: String::new(),
                            hint: ConversationHint::default(),
                            state: EntryState::Read(summary.into()),
                        });
                    }
                }
            }
        }
        Ok(())
    }
    /// Drop a runtime's index, so a runtime that cannot be read lists nothing.
    pub(super) fn forget_index(&self, runtime: &RuntimeInstance) {
        let current = self
            .runtime_instance(&runtime.id)
            .ok()
            .filter(|current| current.enabled)
            .and_then(|current| self.runtime_source_key(current).ok());
        if current.is_none()
            || self
                .runtime_source_key(runtime)
                .is_ok_and(|source| Some(source) == current)
        {
            self.discard_runtime_source_caches(&runtime.id);
        }
    }

    pub(super) fn discard_runtime_source_caches(&self, runtime_id: &str) {
        self.advance_history_index_generation();
        *self.last_listed.borrow_mut() = organization::ListedProjection::default();
        self.history_listings.borrow_mut().remove(runtime_id);
        self.codex_active_indexes.borrow_mut().remove(runtime_id);
        self.native_projects.borrow_mut().remove(runtime_id);
        self.reached
            .borrow_mut()
            .retain(|_, cached| cached.source.runtime_id() != runtime_id);
        self.archived_listing.borrow_mut().remove(runtime_id);
    }
    /// Read the native conversations continued ones are made of, so a
    /// continued conversation lists with its own title and activity.
    pub(super) fn read_link_segments(&self, segments: &BTreeSet<&str>) {
        if segments.is_empty() {
            return;
        }
        for runtime in self.runtime_instances.iter().filter(|r| r.enabled) {
            if let Err(error) = self.read_unread_where(runtime, |id| segments.contains(id)) {
                tracing::warn!(target:"velune_application",event="runtime_history_read_failed",phase="link_segments",runtime_id=%runtime.id,runtime_type_id=%runtime.type_id,detail=%error);
            }
        }
    }
    /// Read what the runtime's index has not read yet.
    pub(super) fn read_unread(&self, runtime: &RuntimeInstance) -> Result<(), RuntimeError> {
        self.read_unread_where(runtime, |_| true)
    }

    pub(super) fn read_unread_where(
        &self,
        runtime: &RuntimeInstance,
        wanted: impl Fn(&str) -> bool,
    ) -> Result<(), RuntimeError> {
        if !reads_lazily(runtime) {
            return Ok(());
        }
        let unread: Vec<String> = self
            .indexed(runtime)
            .into_iter()
            .filter(|entry| matches!(entry.state, EntryState::Unread) && wanted(&entry.id))
            .map(|entry| entry.id)
            .collect();
        self.read_indexed(runtime, &unread)
    }
    /// One conversation of a runtime, indexed again and read if needed;
    /// none when the runtime does not have it.
    pub(super) fn conversation_summary(
        &self,
        runtime: &RuntimeInstance,
        id: &str,
    ) -> Result<Option<SharedSummary>, RuntimeError> {
        self.index_history(runtime)?;
        let state = self
            .indexed(runtime)
            .into_iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.state);
        if matches!(state, Some(EntryState::Unread)) || (state.is_none() && reads_lazily(runtime)) {
            self.read_indexed(runtime, &[id.to_owned()])?;
        }
        Ok(self.listed_summary(runtime, id))
    }
    /// A conversation this instance's index has read; no native read.
    pub(super) fn listed_summary(
        &self,
        runtime: &RuntimeInstance,
        id: &str,
    ) -> Option<SharedSummary> {
        let source = self.runtime_source_key(runtime).ok()?;
        if !source.is_current(self) {
            return None;
        }
        self.history_listings
            .borrow()
            .get(&runtime.id)
            .filter(|listing| listing.source == source)
            .and_then(|listing| {
                listing.entries.iter().find_map(|entry| match &entry.state {
                    EntryState::Read(summary) if entry.id == id => Some(summary.clone()),
                    _ => None,
                })
            })
            .or_else(|| {
                // The runtime's own archive is listed apart from its history.
                self.archived_listing
                    .borrow()
                    .get(&runtime.id)
                    .filter(|archive| archive.source == source)
                    .and_then(|archive| archive.summaries.iter().find(|summary| summary.id == id))
                    .cloned()
            })
            .or_else(|| {
                self.reached
                    .borrow()
                    .get(id)
                    .filter(|cached| cached.source == source)
                    .map(|cached| &cached.value)
                    .filter(|source| source.summary.runtime_id == runtime.id)
                    .map(|source| source.summary.clone())
            })
    }
    pub(super) fn validate_session_model(
        &self,
        runtime: &RuntimeInstance,
        key: &str,
    ) -> Result<(), RuntimeError> {
        let gateway = self
            .gateways
            .iter()
            .find(|g| g.id == runtime.gateway_id)
            .ok_or_else(|| RuntimeError::invalid("runtime gateway"))?;
        let provider = gateway
            .resolve_dispatch(key)
            .map_err(|_| RuntimeError::invalid("所选会话模型已不存在，请重新选择"))?
            .provider();
        let protocols = crate::config::runtime_provider_protocols(runtime.type_id);
        if !protocols.contains(&provider.protocol) {
            return Err(RuntimeError::invalid("所选模型协议不适用于此运行时版本"));
        }
        Ok(())
    }
    pub(super) fn read_history(
        &self,
        runtime_id: &str,
        id: &str,
    ) -> Result<HistorySnapshot, RuntimeError> {
        self.finish_history_read(
            self.prepare_history_read(runtime_id, id)?
                .load_with_context(id, None)?,
        )
    }

    pub(super) fn prepare_history_read(
        &self,
        runtime_id: &str,
        id: &str,
    ) -> Result<HistoryRead, RuntimeError> {
        let runtime = self.runtime_instance(runtime_id)?;
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用，请先启用"));
        }
        let native_id = id
            .strip_prefix(&format!("{runtime_id}:"))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        let source = self.runtime_source_key(runtime)?;
        let preparation = if runtime.type_id == RuntimeKind::Acp {
            self.runtime_preparation(runtime)?
        } else {
            self.execution_preparation(runtime)?.1
        };
        Ok(HistoryRead {
            source,
            preparation,
            summary: (runtime.type_id != RuntimeKind::Codex166)
                .then(|| self.listed_summary(runtime, id))
                .flatten(),
            native_id: native_id.to_owned(),
            acp_cwd: None,
            pi_context: (runtime.type_id == RuntimeKind::Pi).then(|| runtime_context(runtime)),
        })
    }

    pub(super) fn prepare_acp_history(
        &self,
        runtime: &RuntimeInstance,
        native_id: &str,
        cwd: &Path,
    ) -> Result<HistoryRead, RuntimeError> {
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用，请先启用"));
        }
        let source = self.runtime_source_key(runtime)?;
        let preparation = self.runtime_preparation(runtime)?;
        Ok(HistoryRead {
            source,
            preparation,
            summary: None,
            native_id: native_id.to_owned(),
            acp_cwd: Some(cwd.to_owned()),
            pi_context: None,
        })
    }

    pub(super) fn finish_history_read(
        &self,
        loaded: LoadedHistory,
    ) -> Result<HistorySnapshot, RuntimeError> {
        let LoadedHistory {
            source,
            prepared_resources,
            mut snapshot,
            acp_capabilities,
        } = loaded;
        if !source.is_current(self) {
            return Err(history_read_changed());
        }
        if let Some(key) = prepared_resources {
            self.prepared_remote_resources.borrow_mut().insert(key);
        }
        let runtime = self.runtime_instance(source.runtime_id())?;
        if let Some(capabilities) = acp_capabilities {
            self.acp_capability_cache
                .borrow_mut()
                .insert(runtime.id.clone(), capabilities);
        }
        if snapshot
            .model_record_key
            .as_deref()
            .is_some_and(|key| self.validate_session_model(runtime, key).is_err())
        {
            snapshot.model_record_key = None;
        }
        Ok(snapshot)
    }

    pub(super) fn invalidate_execution(&mut self) -> Result<(), RuntimeError> {
        self.ensure_work_ended()?;
        let can_restore = !self.session.requires_live_connection();
        let mut snapshot = self.owned_native_snapshot()?;
        // Closing execution leaves its native history view usable. A read-only
        // view has no execution owner, so restoration must follow its source.
        let source = snapshot
            .as_ref()
            .map(|view| view.conversation.runtime_id.clone());
        let shutdown_result = self.shutdown_active();
        if let Some(id) = source.filter(|id| self.runtime_instance(id).is_ok_and(|r| r.enabled)) {
            if let Some(view) = snapshot.as_mut() {
                if view.model_record_key.as_deref().is_some_and(|key| {
                    self.validate_session_model(
                        self.runtime_instance(&id).expect("configured instance"),
                        key,
                    )
                    .is_err()
                }) {
                    view.model_record_key = None;
                }
                view.run_state = RunState::Idle;
                view.pending_interactions
                    .retain(|interaction| !interaction.requires_live_reply());
                view.actions = ConversationActions {
                    input_access: Default::default(),
                    can_send: can_restore && view.conversation.parent_conversation_id.is_none(),
                    can_cancel: false,
                    can_switch: true,
                };
                view.revision = view.revision.saturating_add(1);
                self.session.model_record_key = view.model_record_key.clone();
            }
            if let Some(snapshot) = snapshot {
                self.session.set_history(snapshot);
            }
        }
        shutdown_result
    }
    pub(super) fn prepare_snapshot(
        &mut self,
        snapshot: impl Into<HistorySnapshot>,
        key: &str,
        start: super::model_selection::ConversationStart<'_>,
        project: Option<&Retained<velune_agent_runtime::native::codex_projects::NativeProject>>,
    ) -> Result<(), RuntimeError> {
        use super::model_selection::ConversationStart;
        let snapshot = snapshot.into().into_retained();
        let is_new = matches!(start, ConversationStart::New);
        let previous = self.owned_native_snapshot()?;
        let previous_runtime = self.next_turn_runtime_id.clone();
        let runtime_id = snapshot.conversation.runtime_id.clone();
        let runtime = self.runtime_instance(&runtime_id)?.clone();
        self.validate_session_model(&runtime, key)?;
        let cwd = snapshot.conversation.cwd.as_deref().map(PathBuf::from);
        if let Some(cwd) = &cwd {
            self.validate_runtime_cwd(&runtime, cwd)?;
        }
        let gateway = self
            .gateways
            .iter()
            .find(|g| g.id == runtime.gateway_id)
            .expect("configured gateway");
        // A route runs while any member can authenticate; the gateway skips the others.
        let configured = gateway.members(key).iter().any(|member| {
            gateway
                .providers
                .iter()
                .find(|p| p.models.iter().any(|m| &m.record_key == member))
                .is_some_and(|p| p.authentication.description().configured)
        });
        if !configured {
            return Err(RuntimeError::invalid(
                "此模型的提供商尚未配置认证，请先完成提供商配置",
            ));
        }
        let context = self.prepare_execution_context(&runtime)?;
        velune_agent_runtime::version::check_version_at(
            runtime.type_id,
            &setting_path(&runtime, "binary")?,
            setting_path_optional(&runtime, "nodeBinary").as_deref(),
            &context.target,
            context.os_home.as_deref(),
        )
        .map_err(|error| RuntimeError::Invalid(error.to_string()))?;
        if runtime.type_id == RuntimeKind::Pi {
            runnable_pi_model(gateway, key)?;
        }
        gateway.to_gateway_config()?;
        let result = (|| {
            self.prepare_runtime(&runtime_id, key)?;
            if runtime.type_id != RuntimeKind::Pi {
                match start {
                    ConversationStart::New => self.native_create(cwd.as_deref(), key, project)?,
                    ConversationStart::Resume => self.native_resume(&snapshot, key)?,
                    ConversationStart::CodexFork(through_turn) => {
                        self.native_fork_source(&snapshot, key, through_turn)?;
                    }
                }
                return Ok(());
            }
            let gateway = self
                .gateways
                .iter()
                .find(|g| g.id == runtime.gateway_id)
                .expect("prepared gateway");
            let physical = gateway
                .pi_binding_id(key, gateway.authentication_revision(key))
                .map_err(RuntimeError::invalid)?;
            self.start_pi_for_session(
                cwd.as_deref(),
                (!is_new).then_some(&snapshot),
                Some(key),
                Some(&physical),
            )?;
            if self.session.side.is_some() {
                self.session.pi()?.restore_context(&snapshot);
            }
            self.session.model_record_key = Some(key.into());
            self.bind_gateway_model()?;
            self.sync_projection()?;
            Ok(())
        })();
        if let Err(error) = result {
            // Preparation never submits a prompt. Failed setup is retryable;
            // an accepted send is never automatically dispatched a second time.
            let _ = self.shutdown_active();
            self.next_turn_runtime_id = previous_runtime;
            self.session.model_record_key =
                previous.as_ref().and_then(|s| s.model_record_key.clone());
            if let Some(mut view) = previous {
                let failure_id = format!("prepare-failed:{}", view.revision);
                view.messages.push(Message {
                    turn_settings: None,
                    turn_usage: None,
                    branch_point: None,
                    completed: true,
                    id: failure_id,
                    role: velune_conversation::MessageRole::System,
                    timestamp_unix_ms: None,
                    blocks: vec![MessageBlock::Notice {
                        text: "运行时准备失败；会话内容已保留，可修正配置后重试。".into(),
                    }],
                });
                view.revision = view.revision.saturating_add(1);
                self.session.set_history(view);
            }
            return Err(error);
        }
        Ok(())
    }
}

#[cfg(test)]
mod source_cache_tests {
    use super::*;
    use crate::config::NewConversationChoice;
    use velune_agent_runtime::native::codex_projects::{
        NativeProject, ProjectListing, ProjectRoot,
    };

    #[test]
    fn history_diagnostic_identity_is_stable_and_does_not_expose_native_path() {
        let native_path = "/private/native/session.jsonl";
        let digest = identity_digest(native_path);
        assert_eq!(
            digest,
            "72f621c3abd0f85a9f384abd742599ce44ab7bd33b0a3b11d91c8ca00cf24aa3"
        );
        assert_eq!(digest.len(), 64);
        assert!(!digest.contains(native_path));
    }

    #[test]
    fn history_diagnostic_session_identity_keeps_only_recognized_native_uuids() {
        let uuid = "dec68bcd-1f99-4d0e-a82d-0f04e86d4ea7";
        let pi_path = format!("/private/sessions/2026-11-16T11-26-02-240Z_{uuid}.jsonl");
        let pi = diagnostic_session_identity(RuntimeKind::Pi, &pi_path, &pi_path);
        assert_eq!(pi.logical_session_id, uuid);
        assert_eq!(pi.native_session_id, uuid);
        assert!(!pi.native_session_id.contains("/private"));

        let claude = diagnostic_session_identity(RuntimeKind::ClaudeCode, uuid, uuid);
        assert_eq!(claude.logical_session_id, uuid);
        assert_eq!(claude.native_session_id, uuid);

        let logical_from_pi_link = diagnostic_session_identity(
            RuntimeKind::Pi,
            "model_record_18e43e8e5109d8d0_4:0d7a90df-259e-4886-ab34-713bf04ba6f9",
            &pi_path,
        );
        assert_eq!(
            logical_from_pi_link.logical_session_id,
            "0d7a90df-259e-4886-ab34-713bf04ba6f9"
        );
        assert_eq!(logical_from_pi_link.native_session_id, uuid);

        let unknown_path = "/private/sessions/not-a-standard-name.jsonl";
        let unknown = diagnostic_session_identity(RuntimeKind::Pi, unknown_path, unknown_path);
        assert!(unknown.logical_session_id.starts_with("sha256:"));
        assert!(unknown.native_session_id.starts_with("sha256:"));
        assert!(!unknown.native_session_id.contains("/private"));
    }

    fn runtime(kind: RuntimeKind) -> RuntimeInstance {
        RuntimeInstance {
            enabled: true,
            id: "synthetic".into(),
            name: "Synthetic".into(),
            type_id: kind,
            gateway_id: "default".into(),
            settings: BTreeMap::from([
                ("binary".into(), "/synthetic/agent".into()),
                ("nodeBinary".into(), "/synthetic/node".into()),
                ("agentDir".into(), "/synthetic/state".into()),
            ]),
        }
    }

    fn summary(id: &str) -> ConversationSummary {
        ConversationSummary {
            id: id.into(),
            runtime_id: "synthetic".into(),
            title: velune_conversation::ConversationTitle::Untitled,
            updated_at_unix_ms: None,
            created_at_unix_ms: None,
            cwd: Some("/synthetic/project".into()),
            can_rename: false,
            can_delete: false,
            parent_conversation_id: None,
            native_read_state: None,
        }
    }

    fn entries() -> Vec<IndexedConversation> {
        vec![IndexedConversation {
            id: "synthetic:thread".into(),
            identity: "unchanged-file".into(),
            hint: ConversationHint::default(),
            state: EntryState::Read(summary("synthetic:thread").into()),
        }]
    }

    fn projects(name: &str) -> ProjectListing {
        ProjectListing {
            projects: vec![NativeProject {
                id: "project".into(),
                name: name.into(),
                roots: vec![ProjectRoot {
                    path: "/synthetic/project".into(),
                }],
            }],
            assignments: BTreeMap::from([("synthetic:thread".into(), "project".into())]),
            conversations: vec![summary("synthetic:thread")],
        }
    }

    fn open() -> (crate::Application, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("velune-source-cache-{}", crate::new_record_key()));
        fs::create_dir(&root).unwrap();
        let app = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        (app, root)
    }

    #[test]
    fn cold_history_preparation_captures_sources_without_starting_io() {
        let (mut app, root) = open();
        let core = &mut app.runtime;
        for kind in [
            RuntimeKind::Pi,
            RuntimeKind::ClaudeCode,
            RuntimeKind::DeepSeek,
            RuntimeKind::Codex166,
            RuntimeKind::Acp,
        ] {
            let mut runtime = runtime(kind);
            runtime
                .set_execution(crate::RuntimeExecution {
                    home: crate::RuntimeHome::Managed { remote_root: None },
                    ..Default::default()
                })
                .unwrap();
            core.runtime_instances = vec![runtime.clone()];
            let request = core
                .prepare_history_read(&runtime.id, "synthetic:missing")
                .unwrap();
            assert!(request.source.is_current(core));
            assert!(request.summary.is_none());
            assert!(!root.join("runtime-homes").exists());

            // These paths and the host do not exist. Capturing a read must not
            // attempt native membership, SSH, helper installation or HOME IO.
            runtime
                .set_execution(crate::RuntimeExecution {
                    transport: crate::RuntimeTransport::Ssh {
                        binary: "/synthetic/ssh".into(),
                        destination: "synthetic.invalid".into(),
                        port: Some(2222),
                    },
                    home: crate::RuntimeHome::Managed {
                        remote_root: Some("/synthetic/homes".into()),
                    },
                    remote_resources_directory: Some("/synthetic/helpers".into()),
                })
                .unwrap();
            core.runtime_instances = vec![runtime.clone()];
            assert!(!request.source.is_current(core));
            let remote = core
                .prepare_history_read(&runtime.id, "synthetic:missing")
                .unwrap();
            assert!(remote.source.is_current(core));
            assert!(!root.join("runtime-homes").exists());
        }
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn native_caches_keep_non_source_edits_and_reject_old_results_and_fallbacks() {
        let (mut app, root) = open();
        let core = &mut app.runtime;
        let old = runtime(RuntimeKind::Codex166);
        core.upsert_runtime(old.clone()).unwrap();
        let source = core.runtime_source_key(&old).unwrap();
        core.store_history_index(source.clone(), entries()).unwrap();
        core.store_native_projects(
            source.clone(),
            Retained::from_parts(projects("original"), DataRetention::default()),
            false,
        )
        .unwrap();
        let stale_archive = HistoryArchive {
            source: source.clone(),
            summaries: vec![summary("synthetic:archived").into()],
        };
        let stale_reached = CachedSource {
            source: source.clone(),
            value: organization::ListedSource {
                summary: summary("synthetic:reached").into(),
                archived_natively: true,
                provisional: false,
            },
        };
        core.archived_listing
            .borrow_mut()
            .insert(old.id.clone(), stale_archive.clone());
        core.reached
            .borrow_mut()
            .insert("synthetic:reached".into(), stale_reached.clone());
        core.gateways.push(GatewayConfig {
            id: "other".into(),
            name: "Other".into(),
            providers: Vec::new(),
            routes: Vec::new(),
        });
        let mut changed = old.clone();
        changed.name = "Renamed".into();
        changed.gateway_id = "other".into();
        changed
            .set_new_conversation_choice(NewConversationChoice::Chosen {
                choice: Default::default(),
            })
            .unwrap();
        core.upsert_runtime(changed.clone()).unwrap();
        assert_eq!(core.indexed(&changed).len(), 1);
        assert_eq!(core.native_project_keys().len(), 1);
        assert!(
            core.native_conversation_project(&changed.id, "synthetic:thread")
                .is_some()
        );
        assert!(
            core.listed_summary(&changed, "synthetic:archived")
                .is_some()
        );
        core.gateways
            .iter_mut()
            .find(|gateway| gateway.id == "other")
            .unwrap()
            .providers
            .push(crate::config::ProviderDefinition {
                id: "synthetic-provider".into(),
                name: "Synthetic".into(),
                protocol: Protocol::Responses,
                endpoint: "http://127.0.0.1:1/v1".into(),
                execution_backend: crate::config::ExecutionBackend::Http,
                request_profile: crate::config::RequestProfile::Native,
                quota_source: crate::config::ProviderQuotaSource::None,
                authentication: Default::default(),
                quota_authentication: Default::default(),
                billing: Default::default(),
                models: Vec::new(),
                quota_limits: Vec::new(),
            });
        core.invalidate_gateway_execution("other").unwrap();
        assert!(core.runtime_source_key(&changed).unwrap() == source);
        assert_eq!(core.native_project_keys().len(), 1);
        let mut new = changed.clone();
        new.settings
            .insert("agentDir".into(), "/synthetic/other-state".into());
        core.upsert_runtime(new.clone()).unwrap();
        assert!(core.history_listings.borrow().is_empty());
        assert!(core.native_projects.borrow().is_empty());
        assert!(core.reached.borrow().is_empty());
        assert!(core.archived_listing.borrow().is_empty());
        // A client view may restore an archive projection captured before the source edit.
        core.archived_listing
            .borrow_mut()
            .insert(new.id.clone(), stale_archive);
        core.reached
            .borrow_mut()
            .insert("synthetic:reached".into(), stale_reached);
        assert!(core.cached_sources().is_empty());
        assert!(core.listed_summary(&new, "synthetic:archived").is_none());
        assert!(core.listed_summary(&new, "synthetic:reached").is_none());
        let fresh = core.runtime_source_key(&new).unwrap();
        core.store_history_index(fresh.clone(), entries()).unwrap();
        core.store_native_projects(
            fresh.clone(),
            Retained::from_parts(projects("fresh"), DataRetention::default()),
            false,
        )
        .unwrap();
        assert!(core.store_history_index(source.clone(), entries()).is_err());
        assert!(
            core.store_native_projects(
                source,
                Retained::from_parts(projects("stale"), DataRetention::default()),
                true
            )
            .is_err()
        );
        core.forget_index(&old); // A delayed failure from the old source cannot remove fresh data.
        assert_eq!(core.indexed(&new).len(), 1);
        let project_key = core.native_project_keys().pop().unwrap();
        assert_eq!(core.native_project(&project_key).unwrap().1.name, "fresh");
        core.runtime_instances[0]
            .settings
            .insert("execution".into(), "broken".into());
        core.forget_index(&core.runtime_instances[0]);
        assert!(core.history_listings.borrow().is_empty());
        assert!(core.native_projects.borrow().is_empty());
        assert!(core.archived_listing.borrow().is_empty());
        assert!(core.reached.borrow().is_empty());
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parked_client_listing_retains_its_source_provenance() {
        let (mut app, root) = open();
        let core = &mut app.runtime;
        let mut runtime = runtime(RuntimeKind::Codex166);
        core.upsert_runtime(runtime.clone()).unwrap();
        let mut client = core.client_state();
        core.enter_client_state(&mut client, false);
        *core.last_listed.borrow_mut() = organization::ListedProjection {
            sources: vec![core.runtime_source_key(&runtime).unwrap()],
            entries: BTreeMap::from([(
                "synthetic:thread".into(),
                organization::ListedSource {
                    summary: summary("synthetic:thread").into(),
                    archived_natively: false,
                    provisional: false,
                },
            )]),
        };
        core.leave_client_state(&mut client);
        runtime.name = "Renamed".into();
        core.upsert_runtime(runtime.clone()).unwrap();
        core.enter_client_state(&mut client, false);
        assert!(core.listed_sources().contains_key("synthetic:thread"));
        core.leave_client_state(&mut client);
        runtime
            .settings
            .insert("agentDir".into(), "/synthetic/other-state".into());
        core.upsert_runtime(runtime).unwrap();
        core.enter_client_state(&mut client, false);
        assert!(core.listed_sources().is_empty());
        core.leave_client_state(&mut client);
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reader_identity_uses_resolved_configs_and_keeps_execution_dependencies_separate() {
        let (app, root) = open();
        let core = &app.runtime;
        let mut pi = runtime(RuntimeKind::Pi);
        let source = core.runtime_source_key(&pi).unwrap();
        let execution = pi.execution_configuration();
        pi.settings.insert(
            "sdkHelper".into(),
            "/synthetic/custom-execution-helper.mjs".into(),
        );
        assert!(core.runtime_source_key(&pi).unwrap() == source);
        assert_ne!(pi.execution_configuration(), execution);
        pi.set_execution(crate::RuntimeExecution::default())
            .unwrap();
        assert!(core.runtime_source_key(&pi).unwrap() == source);
        pi.settings
            .insert("sessionDir".into(), "/synthetic/other-sessions".into());
        assert!(core.runtime_source_key(&pi).unwrap() != source);
        pi.set_execution(crate::RuntimeExecution {
            transport: crate::RuntimeTransport::Ssh {
                binary: "/usr/bin/ssh".into(),
                destination: "synthetic.invalid".into(),
                port: Some(2222),
            },
            home: crate::RuntimeHome::Managed {
                remote_root: Some("/synthetic/homes".into()),
            },
            remote_resources_directory: Some("/synthetic/helpers".into()),
        })
        .unwrap();
        let remote = core.runtime_source_key(&pi).unwrap();
        pi.settings
            .insert("agentDir".into(), "/ignored-with-managed-home".into());
        assert!(core.runtime_source_key(&pi).unwrap() == remote);
        let mut acp = runtime(RuntimeKind::Acp);
        let plain = core.runtime_source_key(&acp).unwrap();
        acp.set_acp_launch(crate::AcpLaunch {
            args: vec!["--other-source".into()],
            environment: BTreeMap::new(),
        })
        .unwrap();
        assert!(core.runtime_source_key(&acp).unwrap() != plain);
        // All identity construction above is pure, including SSH and managed HOME resolution.
        assert!(!root.join("runtime-homes").exists());
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    mod retention_tests {
        include!("browsing/retention_tests.rs");
    }
}

impl CoreRuntime {
    pub(super) fn keep_native_read_state(
        &mut self,
        receipt: Retained<velune_conversation::NativeReadState>,
    ) -> Result<(), RuntimeError> {
        let needs_update = |summary: &ConversationSummary| {
            summary.native_read_state.as_ref() != Some(receipt.as_ref())
                && (summary.id == receipt.conversation_id
                    || summary
                        .native_read_state
                        .as_ref()
                        .is_some_and(|s| s.conversation_id == receipt.conversation_id))
        };
        let update_shared = |summary: &mut SharedSummary| {
            if needs_update(summary) {
                summary.update_from(&receipt, |summary, value| {
                    summary.native_read_state = Some(value);
                })?;
            }
            Ok::<(), RuntimeError>(())
        };
        for source in self.last_listed.borrow_mut().entries.values_mut() {
            update_shared(&mut source.summary)?;
        }
        for source in self.reached.borrow_mut().values_mut() {
            update_shared(&mut source.value.summary)?;
        }
        for archive in self.archived_listing.borrow_mut().values_mut() {
            for summary in &mut archive.summaries {
                update_shared(summary)?;
            }
        }
        for listing in self.history_listings.borrow_mut().values_mut() {
            for entry in &mut listing.entries {
                if let EntryState::Read(summary) = &mut entry.state {
                    update_shared(summary)?;
                }
            }
        }
        self.for_each_session(|core| {
            match &mut core.session.active_state {
                ActiveState::History(snapshot) if needs_update(&snapshot.conversation) => {
                    // Read state changes the serialized history header; cached
                    // size must be recomputed when this parked view is trimmed.
                    let owned = receipt.try_clone().map_err(|error| {
                        RuntimeError::context("history read-state credit", error)
                    })?;
                    let (value, guard) = owned.into_parts();
                    snapshot.conversation.native_read_state = Some(value);
                    snapshot.retention = std::mem::take(&mut snapshot.retention).merge(guard);
                    core.session.invalidate_history_cache_weight();
                }
                ActiveState::Native(session)
                    if session.snapshot().is_some_and(|snapshot| {
                        snapshot.conversation.id == receipt.conversation_id
                            && needs_update(&snapshot.conversation)
                    }) =>
                {
                    session.set_read_state(receipt.try_clone().map_err(|error| {
                        RuntimeError::context("native read-state credit", error)
                    })?)
                }
                _ => {}
            }
            Ok(())
        })
    }
}
