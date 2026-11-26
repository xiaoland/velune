//! Owned native index work. The application owner applies it to shared caches,
//! then builds each requesting view from its current presentation state.
use super::browsing::{
    ConversationHint, EntryState, IndexedConversation, SharedSummary, history_failure,
    shared_summaries,
};
use super::runtime_execution::ExecutionPreparation;
use super::runtime_source::{Reader, RuntimeSourceKey};
use super::*;
use sha2::{Digest, Sha256};
use velune_agent_runtime::creation::Retained;
use velune_agent_runtime::native::codex_projects::{self, ActiveListIndex, ProjectListing};

pub(crate) struct ListRefresh {
    index_generation: u64,
    requests: Vec<HistoryIndexRead>,
    telemetry: Option<Arc<velune_telemetry::ClientTelemetry>>,
}

pub(crate) struct LoadedListRefresh {
    index_generation: u64,
    indexes: Vec<LoadedHistoryIndex>,
}

/// A startup read populates only the adapter's source-scoped index proof.
pub(crate) struct CodexIndexWarmup(Vec<HistoryIndexRead>);

pub(super) struct CodexActiveIndex {
    source: RuntimeSourceKey,
    index: Arc<ActiveListIndex>,
}

struct HistoryIndexRead {
    runtime: RuntimeInstance,
    source: Option<RuntimeSourceKey>,
    preparation: Result<ExecutionPreparation, RuntimeError>,
    codex_active_index: Option<Arc<ActiveListIndex>>,
}

struct LoadedHistoryIndex {
    runtime: RuntimeInstance,
    source: Option<RuntimeSourceKey>,
    prepared_resources: Option<String>,
    acp_capabilities: Option<crate::AcpCapabilities>,
    index: Result<NativeIndex, RuntimeError>,
}

enum NativeIndex {
    Entries(Vec<IndexedConversation>),
    Summaries(Retained<Vec<ConversationSummary>>),
    Codex(Result<Retained<ProjectListing>, RuntimeError>),
}

impl ListRefresh {
    pub(crate) fn load(self) -> LoadedListRefresh {
        let start = std::time::Instant::now();
        let mut known_resources = BTreeSet::new();
        let mut indexes = Vec::with_capacity(self.requests.len());
        for request in self.requests {
            let index = request.load(self.telemetry.as_ref(), &known_resources);
            if let Some(key) = &index.prepared_resources {
                known_resources.insert(key.clone());
            }
            indexes.push(index);
        }
        let elapsed = start.elapsed();
        if elapsed.as_millis() >= 100 {
            tracing::info!(target: "velune_application", event = "conversation_list_index",
                runtime_count = indexes.len(), refresh_ms = elapsed.as_millis() as u64);
        }
        LoadedListRefresh {
            index_generation: self.index_generation,
            indexes,
        }
    }
}

impl CodexIndexWarmup {
    pub(crate) fn run(self) {
        let started = std::time::Instant::now();
        let sources = self.0.len();
        let known_resources = BTreeSet::new();
        let mut failures = 0;
        for request in self.0 {
            if request.warm(&known_resources).is_err() {
                failures += 1;
            }
        }
        tracing::info!(target: "velune_application", event = "codex_active_index_warmup",
            sources, failures, elapsed_ms = started.elapsed().as_millis() as u64);
    }
}

impl CoreRuntime {
    pub(crate) fn prepare_codex_index_warmup(&self) -> Option<CodexIndexWarmup> {
        let requests = self
            .runtime_instances
            .iter()
            .filter(|runtime| runtime.enabled && runtime.type_id == RuntimeKind::Codex166)
            .map(|runtime| {
                self.prepare_history_index(runtime)
                    .with_local_codex_index(self)
            })
            .filter(|request| request.codex_active_index.is_some())
            .collect::<Vec<_>>();
        (!requests.is_empty()).then_some(CodexIndexWarmup(requests))
    }

    pub(crate) fn prepare_list_refresh(&self) -> Option<ListRefresh> {
        if self.shows_archived() {
            return None;
        }
        Some(ListRefresh {
            index_generation: self.history_index_generation.get(),
            requests: self
                .runtime_instances
                .iter()
                .filter(|runtime| runtime.enabled)
                .map(|runtime| {
                    self.prepare_history_index(runtime)
                        .with_local_codex_index(self)
                })
                .collect(),
            telemetry: self.telemetry.clone(),
        })
    }

    pub(crate) fn install_list_refresh(
        &self,
        loaded: LoadedListRefresh,
    ) -> Result<(), RuntimeError> {
        let mut runtimes = self
            .runtime_instances
            .iter()
            .filter(|runtime| runtime.enabled);
        // A synchronous reader can replace the indexes while this worker
        // runs, even when the runtime configuration is unchanged.
        if self.history_index_generation.get() != loaded.index_generation
            || !loaded.indexes.iter().all(|index| {
                runtimes
                    .next()
                    .is_some_and(|runtime| index.matches(self, runtime))
            })
            || runtimes.next().is_some()
        {
            return Err(RuntimeError::invalid("运行时历史来源已改变，请刷新后重试"));
        }

        let mut failures = Vec::new();
        for ((slot, runtime), index) in self
            .runtime_instances
            .iter()
            .enumerate()
            .filter(|(_, runtime)| runtime.enabled)
            .zip(loaded.indexes)
        {
            if let Err(error) = index.install(self) {
                failures.push(self.list_history_failure(slot, runtime, error));
            }
        }
        *self.history_failures.borrow_mut() = failures;
        Ok(())
    }

    pub(super) fn advance_history_index_generation(&self) {
        self.history_index_generation
            .set(self.history_index_generation.get().wrapping_add(1));
    }

    /// Synchronous callers use the same reader and source-scoped updates.
    pub(super) fn index_history(&self, runtime: &RuntimeInstance) -> Result<(), RuntimeError> {
        self.prepare_history_index(runtime)
            .load(self.telemetry.as_ref(), &BTreeSet::new())
            .install(self)
    }

    fn prepare_history_index(&self, runtime: &RuntimeInstance) -> HistoryIndexRead {
        let (source, preparation) = match self.runtime_source_key(runtime) {
            Ok(source) => {
                let preparation = match source.reader() {
                    Reader::Acp(_) => self.runtime_preparation(runtime),
                    _ => self
                        .execution_preparation(runtime)
                        .map(|(_, preparation)| preparation),
                };
                (Some(source), preparation)
            }
            Err(error) => (None, Err(error)),
        };
        HistoryIndexRead {
            runtime: runtime.clone(),
            source,
            preparation,
            codex_active_index: None,
        }
    }
}

impl HistoryIndexRead {
    fn with_local_codex_index(mut self, core: &CoreRuntime) -> Self {
        if self.preparation.is_ok()
            && let Some(source) = &self.source
            && source.kind() == RuntimeKind::Codex166
            && let Reader::Native(config) = source.reader()
            && ActiveListIndex::supports(config)
        {
            let storage_path = codex_active_index_path(&core.options.home_directory, source.runtime_id());
            let mut indexes = core.codex_active_indexes.borrow_mut();
            let entry = indexes
                .entry(source.runtime_id().to_owned())
                .or_insert_with(|| CodexActiveIndex {
                    source: source.clone(),
                    index: Arc::new(ActiveListIndex::with_storage(storage_path.clone())),
                });
            if entry.source != *source {
                entry.source = source.clone();
                entry.index = Arc::new(ActiveListIndex::with_storage(storage_path));
            }
            self.codex_active_index = Some(entry.index.clone());
        }
        self
    }

    fn warm(self, known_resources: &BTreeSet<String>) -> Result<(), RuntimeError> {
        let Self {
            source,
            preparation,
            codex_active_index,
            ..
        } = self;
        let _ = preparation.and_then(|preparation| preparation.prepare_with_known_resources(known_resources))?;
        if let (Some(source), Some(index)) = (source.as_ref(), codex_active_index.as_deref())
            && let Reader::Native(config) = source.reader()
        {
            codex_projects::warm_active_index(config, index)
                .map_err(|error| RuntimeError::context("Codex 项目读取失败", error))?;
        }
        Ok(())
    }

    fn load(
        self,
        telemetry: Option<&Arc<velune_telemetry::ClientTelemetry>>,
        known_resources: &BTreeSet<String>,
    ) -> LoadedHistoryIndex {
        let Self {
            runtime,
            source,
            preparation,
            codex_active_index,
        } = self;
        let mut acp_capabilities = None;
        let (prepared_resources, index) = match preparation
            .and_then(|preparation| preparation.prepare_with_known_resources(known_resources))
        {
            Ok(prepared_resources) => (
                prepared_resources,
                read_index(
                    &runtime,
                    source.as_ref().expect("prepared source").reader(),
                    telemetry,
                    &mut acp_capabilities,
                    codex_active_index.as_deref(),
                ),
            ),
            Err(error) => (None, Err(error)),
        };
        LoadedHistoryIndex {
            runtime,
            source,
            prepared_resources,
            acp_capabilities,
            index,
        }
    }
}

impl LoadedHistoryIndex {
    fn matches(&self, core: &CoreRuntime, runtime: &RuntimeInstance) -> bool {
        match (&self.source, core.runtime_source_key(runtime)) {
            (Some(source), Ok(current)) => *source == current,
            (None, Err(_)) => self.runtime == *runtime,
            _ => false,
        }
    }

    fn install(self, core: &CoreRuntime) -> Result<(), RuntimeError> {
        let Some(source) = self.source else {
            return self.index.map(|_| ());
        };
        if !source.is_current(core) {
            return Err(RuntimeError::invalid("运行时历史来源已改变，请刷新后重试"));
        }
        if let Some(key) = self.prepared_resources {
            core.prepared_remote_resources.borrow_mut().insert(key);
        }
        if let Some(capabilities) = self.acp_capabilities {
            core.acp_capability_cache
                .borrow_mut()
                .insert(self.runtime.id.clone(), capabilities);
        }
        let entries = match self.index? {
            NativeIndex::Entries(entries) => entries,
            NativeIndex::Summaries(summaries) => index_summaries(shared_summaries(summaries)),
            NativeIndex::Codex(listing) => {
                let listing = listing.inspect_err(|_| core.forget_index(&self.runtime))?;
                index_summaries(core.store_native_projects(source.clone(), listing, false)?)
            }
        };
        core.store_history_index(source, entries)
    }
}

fn read_index(
    runtime: &RuntimeInstance,
    reader: &Reader,
    telemetry: Option<&Arc<velune_telemetry::ClientTelemetry>>,
    acp_capabilities: &mut Option<crate::AcpCapabilities>,
    codex_active_index: Option<&ActiveListIndex>,
) -> Result<NativeIndex, RuntimeError> {
    let id = &runtime.id;
    Ok(match reader {
        Reader::Acp(config) => {
            let (session, capabilities) = acp_composition::connect_acp(config.clone(), telemetry)?;
            *acp_capabilities = Some(capabilities);
            NativeIndex::Summaries(acp_composition::list_acp_sessions(session, id)?)
        }
        Reader::Pi(config) => NativeIndex::Entries(
            history::list_pi(config, &[])
                .map_err(|error| {
                    RuntimeError::context("Pi 历史无法读取；请检查实例路径与 SDK 配置后重试", error)
                })?
                .files
                .into_iter()
                .map(|file| IndexedConversation {
                    id: format!("{id}:{}", file.path),
                    identity: format!("{}:{}", file.size, file.mtime_ns),
                    hint: ConversationHint {
                        parent_conversation_id: file
                            .parent_path
                            .map(|parent| format!("{id}:{parent}")),
                        session_id: file.session_id,
                        cwd: file.cwd,
                        activity_unix_ms: file.modified_at_unix_ms,
                        created_unix_ms: file.created_at_unix_ms,
                    },
                    state: EntryState::Unread,
                })
                .collect(),
        ),
        Reader::Native(config) => match runtime.type_id {
            RuntimeKind::Codex166 => {
                let listing = match codex_active_index {
                    Some(index) => {
                        codex_projects::list_projects_with_active_index(config, id, index)
                    }
                    None => codex_projects::list_projects(config, id, false),
                };
                NativeIndex::Codex(
                    listing.map_err(|error| RuntimeError::context("Codex 项目读取失败", error)),
                )
            }
            RuntimeKind::ClaudeCode => NativeIndex::Entries(
                history::index(config)
                    .map_err(|error| history_failure(runtime, error))?
                    .into_iter()
                    .map(|entry| IndexedConversation {
                        id: format!("{id}:{}", entry.native_id),
                        identity: format!("{}:{:?}", entry.size, entry.modified_at_unix_ms),
                        hint: ConversationHint {
                            parent_conversation_id: None,
                            session_id: None,
                            cwd: entry.cwd,
                            activity_unix_ms: entry.modified_at_unix_ms,
                            created_unix_ms: entry.created_at_unix_ms,
                        },
                        state: EntryState::Unread,
                    })
                    .collect(),
            ),
            RuntimeKind::DeepSeek => NativeIndex::Summaries(
                history::list(config, id).map_err(|error| history_failure(runtime, error))?,
            ),
            RuntimeKind::Pi | RuntimeKind::Acp => unreachable!("not a native reader"),
        },
    })
}

fn codex_active_index_path(home: &Path, runtime_id: &str) -> PathBuf {
    let key = format!("{:x}", Sha256::digest(runtime_id.as_bytes()));
    home.join("codex-active-index").join(format!("{key}.json"))
}

fn index_summaries(summaries: Vec<SharedSummary>) -> Vec<IndexedConversation> {
    summaries
        .into_iter()
        .map(|summary| IndexedConversation {
            id: summary.id.clone(),
            identity: String::new(),
            hint: ConversationHint::default(),
            state: EntryState::Read(summary),
        })
        .collect()
}
