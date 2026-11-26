//! A cold conversation read captures native read inputs on the owner and
//! installs a history-only session after IO. A live session always wins.
use super::*;
use crate::{SnapshotResult, config::ConversationLink};
use browsing::{HistoryRead, HistorySnapshot, LoadedHistory, history_read_changed};

pub(crate) enum ConversationRead {
    Ready(Box<SnapshotResult>),
    Present,
    Load(Box<ConversationHistoryRequest>),
}

enum HistoryParts<T> {
    Native(T),
    Linked {
        link: ConversationLink,
        parts: Vec<T>,
    },
}

pub(crate) struct ConversationHistoryRequest {
    runtime_id: String,
    id: String,
    parts: HistoryParts<Result<HistoryRead, RuntimeError>>,
}

pub(crate) struct LoadedConversationHistory {
    runtime_id: String,
    id: String,
    parts: HistoryParts<Result<LoadedHistory, RuntimeError>>,
    sources: Vec<runtime_source::RuntimeSourceKey>,
}

impl ConversationHistoryRequest {
    pub(crate) fn affected_ids(&self) -> Vec<String> {
        let mut ids = vec![self.id.clone()];
        if let HistoryParts::Linked { link, .. } = &self.parts {
            ids.extend(
                link.segments
                    .iter()
                    .map(|segment| segment.native_conversation_id.clone()),
            );
        }
        ids
    }

    pub(crate) fn load(self) -> LoadedConversationHistory {
        let Self {
            runtime_id,
            id,
            parts,
        } = self;
        let mut sources = Vec::new();
        let mut load = |part: Result<HistoryRead, RuntimeError>, segment_index| {
            let read = part?;
            sources.push(read.source().clone());
            read.load_with_context(&id, Some(segment_index))
        };
        let parts = match parts {
            HistoryParts::Native(part) => HistoryParts::Native(load(part, 0)),
            HistoryParts::Linked { link, parts } => HistoryParts::Linked {
                link,
                parts: parts
                    .into_iter()
                    .enumerate()
                    .map(|(index, part)| load(part, index))
                    .collect(),
            },
        };
        LoadedConversationHistory {
            runtime_id,
            id,
            parts,
            sources,
        }
    }
}

impl LoadedConversationHistory {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn runtime_id(&self) -> &str {
        &self.runtime_id
    }
}

impl CoreRuntime {
    pub(crate) fn begin_history_read(&mut self, id: &str) {
        *self.pending_history_reads.entry(id.to_owned()).or_default() += 1;
    }

    pub(crate) fn end_history_read(&mut self, id: &str) {
        if let Some(count) = self.pending_history_reads.get_mut(id) {
            *count -= 1;
            if *count == 0 {
                self.pending_history_reads.remove(id);
            }
        }
    }

    fn conversation_is_present(
        &self,
        runtime_id: &str,
        id: &str,
    ) -> Result<bool, RuntimeError> {
        let address = self.conversation_read_address(runtime_id, id)?;
        let identity = self
            .session
            .logical_projection
            .as_ref()
            .map(|projection| &projection.origin)
            .or_else(|| {
                self.session
                    .projected()
                    .map(|snapshot| &snapshot.conversation)
            });
        Ok(identity.is_some_and(|identity| {
            identity.id == id && identity.runtime_id == address.source_runtime_id
        }))
    }

    pub(crate) fn read_present_conversation(
        &mut self,
        runtime_id: &str,
        id: &str,
    ) -> Result<Option<SnapshotResult>, RuntimeError> {
        if !self.conversation_is_present(runtime_id, id)? {
            return Ok(None);
        }
        self.sync_projection()?;
        self.snapshot_result().map(Some)
    }

    pub(crate) fn prepare_conversation_read(
        &mut self,
        runtime_id: &str,
        id: &str,
    ) -> Result<ConversationRead, RuntimeError> {
        if let Some(snapshot) = self.read_present_conversation(runtime_id, id)? {
            return Ok(ConversationRead::Ready(Box::new(snapshot)));
        }
        self.prepare_cold_conversation_read(runtime_id, id)
    }

    /// Sync a conversation already in memory without building a `SnapshotResult`
    /// when the caller will re-query via `Command::ReadSnapshotUpdate` / `Retry`.
    pub(crate) fn prepare_conversation_present(
        &mut self,
        runtime_id: &str,
        id: &str,
    ) -> Result<ConversationRead, RuntimeError> {
        if self.conversation_is_present(runtime_id, id)? {
            self.sync_projection()?;
            return Ok(ConversationRead::Present);
        }
        self.prepare_cold_conversation_read(runtime_id, id)
    }

    /// Prepare a deliberate native refresh. A loaded, idle snapshot is not a
    /// cache hit here; active work and live-only sessions are handled by the
    /// caller before requesting the refresh.
    pub(crate) fn prepare_conversation_refresh(
        &mut self,
        runtime_id: &str,
        id: &str,
    ) -> Result<ConversationRead, RuntimeError> {
        // A refresh cannot replace an execution, approval, or live-only
        // conversation that still owns native state.
        if self.session.executing()
            || self.session.requires_live_connection()
            || self.session.pending_native_fork.is_some()
            || self.session.unsubmitted_draft.is_some()
            || self.session.side.is_some()
        {
            return Ok(ConversationRead::Ready(Box::new(self.snapshot_result()?)));
        }
        self.prepare_cold_conversation_read(runtime_id, id)
    }

    fn prepare_cold_conversation_read(
        &self,
        runtime_id: &str,
        id: &str,
    ) -> Result<ConversationRead, RuntimeError> {
        let started = std::time::Instant::now();
        let address = self.conversation_read_address(runtime_id, id)?;
        let runtime_id = address.source_runtime_id.as_str();
        let parts = if let Some(link) = self.conversation_links.iter().find(|link| link.id == id) {
            let parts =
                link.segments
                    .iter()
                    .map(|segment| {
                        let runtime = self.runtime_instance(&segment.runtime_instance_id)?;
                        if runtime.type_id == RuntimeKind::Acp {
                            let native_id = segment
                                .native_conversation_id
                                .strip_prefix(&format!("{}:", runtime.id))
                                .ok_or_else(|| RuntimeError::invalid("ACP 会话引用"))?;
                            let cwd =
                                segment.cwd.as_deref().map(Path::new).ok_or_else(|| {
                                    RuntimeError::invalid("ACP 会话关联缺少工作目录")
                                })?;
                            self.prepare_acp_history(runtime, native_id, cwd)
                        } else {
                            self.prepare_history_read(
                                &segment.runtime_instance_id,
                                &segment.native_conversation_id,
                            )
                        }
                    })
                    .collect();
            HistoryParts::Linked {
                link: link.clone(),
                parts,
            }
        } else {
            HistoryParts::Native(Ok(self.prepare_history_read(runtime_id, id)?))
        };
        tracing::info!(target: "velune_application", event = "conversation_history_read", phase = "prepare", elapsed_ms = started.elapsed().as_millis() as u64);
        Ok(ConversationRead::Load(Box::new(
            ConversationHistoryRequest {
                runtime_id: runtime_id.to_owned(),
                id: id.to_owned(),
                parts,
            },
        )))
    }

    /// Install a loaded explicit refresh in the currently addressed session.
    /// A turn that began while the read was in flight remains authoritative.
    pub(crate) fn finish_conversation_refresh(
        &mut self,
        loaded: LoadedConversationHistory,
    ) -> Result<SnapshotResult, RuntimeError> {
        if self.session.executing() || self.session.requires_live_connection() {
            return self.snapshot_result();
        }
        let (snapshot, projection, _) = self.finish_loaded_history(loaded)?;
        // An explicit refresh invalidates the shared physical observation;
        // a pending result from its previous generation cannot restore it.
        let control_target = self
            .read_control_target(&snapshot, projection.as_ref())
            .ok()
            .flatten();
        self.invalidate_execution()?;
        self.present_history(snapshot, projection);
        if let Some(target) = control_target {
            self.control_observations.invalidate(&target);
        }
        self.snapshot_result()
    }

    pub(crate) fn finish_conversation_read(
        &mut self,
        loaded: LoadedConversationHistory,
    ) -> Result<SnapshotResult, RuntimeError> {
        let started = std::time::Instant::now();
        let LoadedConversationHistory {
            runtime_id,
            id,
            parts,
            sources,
        } = loaded;
        if let Some(snapshot) = self.read_present_conversation(&runtime_id, &id)? {
            return Ok(snapshot);
        }
        let (snapshot, projection, reusable) =
            self.finish_loaded_history_parts(id, parts, sources)?;

        // The browsing call leaves the caller's addressed session in place.
        // Subsequent snapshots or composer preparation find this history by id.
        let previous = self.session.key;
        self.focus_new();
        self.present_history(snapshot, projection);
        let response = self.snapshot_result();
        if !reusable {
            // An unavailable tail is presentation, not a reusable history seed.
            self.session.active_state = ActiveState::Empty;
            self.session.logical_projection = None;
        }
        self.focus_key(previous);
        tracing::info!(target: "velune_application", event = "conversation_history_read", phase = "projection", elapsed_ms = started.elapsed().as_millis() as u64);
        response
    }

    pub(super) fn finish_loaded_history(
        &self,
        loaded: LoadedConversationHistory,
    ) -> Result<
        (
            HistorySnapshot,
            Option<continuation::LogicalProjection>,
            bool,
        ),
        RuntimeError,
    > {
        let LoadedConversationHistory {
            id, parts, sources, ..
        } = loaded;
        self.finish_loaded_history_parts(id, parts, sources)
    }

    fn finish_loaded_history_parts(
        &self,
        id: String,
        parts: HistoryParts<Result<LoadedHistory, RuntimeError>>,
        sources: Vec<runtime_source::RuntimeSourceKey>,
    ) -> Result<
        (
            HistorySnapshot,
            Option<continuation::LogicalProjection>,
            bool,
        ),
        RuntimeError,
    > {
        if sources.iter().any(|source| !source.is_current(self)) {
            return Err(history_read_changed());
        }
        let current_link = self.conversation_links.iter().find(|link| link.id == id);
        let (snapshot, projection, reusable) = match parts {
            HistoryParts::Native(part) if current_link.is_none() => {
                (self.finish_history_read(part?)?, None, true)
            }
            HistoryParts::Linked { link, parts } if current_link == Some(&link) => {
                let reusable = parts.iter().all(Result::is_ok);
                let mut parts = parts.into_iter();
                let (snapshot, projection) = self.read_link_with(&link, |_| {
                    self.finish_history_read(parts.next().expect("each segment has a read result")?)
                })?;
                (snapshot, Some(projection), reusable)
            }
            _ => return Err(history_read_changed()),
        };
        Ok((snapshot, projection, reusable))
    }
}
