//! Read-only browsing and deferred execution preparation share one active view.
use super::*;
use crate::conversation::{ConversationActions, ConversationSnapshot, Message, MessageBlock};
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

/// Fallback projections retain the same provenance as ordinary indexes, including across client views.
#[derive(Clone)]
pub(super) struct CachedSource {
    pub(super) source: runtime_source::RuntimeSourceKey,
    pub(super) value: organization::ListedSource,
}

#[derive(Clone)]
pub(super) struct HistoryArchive {
    pub(super) source: runtime_source::RuntimeSourceKey,
    pub(super) summaries: Vec<ConversationSummary>,
}
#[derive(Clone)]
pub(super) struct IndexedConversation {
    pub(super) id: String,
    /// The native record's state; a read summary holds while it is unchanged.
    identity: String,
    /// What the files tell before the session is read.
    pub(super) hint: ConversationHint,
    pub(super) state: EntryState,
}
#[derive(Clone, Default)]
pub(super) struct ConversationHint {
    pub(super) session_id: Option<String>,
    pub(super) cwd: Option<String>,
    /// The file's modification time: which sessions to read first, never shown.
    pub(super) activity_unix_ms: Option<i64>,
    pub(super) created_unix_ms: Option<i64>,
}
#[derive(Clone)]
pub(super) enum EntryState {
    Unread,
    Read(Box<ConversationSummary>),
    /// The runtime could not read it as a session; not listed until it changes.
    Unreadable,
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
    /// Index a runtime's native history. Read summaries are kept for records
    /// that did not change; the index is also the source-scoped membership
    /// used to open a conversation without listing again.
    pub(super) fn index_history(&self, runtime: &RuntimeInstance) -> Result<(), RuntimeError> {
        let source = self.runtime_source_key(runtime)?;
        let read = |summaries: Vec<ConversationSummary>| {
            summaries
                .into_iter()
                .map(|summary| IndexedConversation {
                    id: summary.id.clone(),
                    identity: String::new(),
                    hint: ConversationHint::default(),
                    state: EntryState::Read(Box::new(summary)),
                })
                .collect::<Vec<_>>()
        };
        let entries = match runtime.type_id {
            RuntimeKind::Acp => read(self.acp_summaries(runtime)?),
            RuntimeKind::Codex166 => read(self.read_codex_projects(runtime, false)?),
            RuntimeKind::Pi => history::list_pi(&self.pi_history_config(runtime)?, &[])
                .map_err(|error| {
                    RuntimeError::context("Pi 历史无法读取；请检查实例路径与 SDK 配置后重试", error)
                })?
                .files
                .into_iter()
                .map(|file| IndexedConversation {
                    id: format!("{}:{}", runtime.id, file.path),
                    identity: format!("{}:{}", file.size, file.mtime_ns),
                    hint: ConversationHint {
                        session_id: file.session_id,
                        cwd: file.cwd,
                        activity_unix_ms: file.modified_at_unix_ms,
                        created_unix_ms: file.created_at_unix_ms,
                    },
                    state: EntryState::Unread,
                })
                .collect(),
            RuntimeKind::ClaudeCode => history::index(&self.history_config(runtime)?)
                .map_err(|error| self.history_failure(runtime, error))?
                .into_iter()
                .map(|entry| IndexedConversation {
                    id: format!("{}:{}", runtime.id, entry.native_id),
                    identity: format!("{}:{:?}", entry.size, entry.modified_at_unix_ms),
                    hint: ConversationHint {
                        session_id: None,
                        cwd: entry.cwd,
                        activity_unix_ms: entry.modified_at_unix_ms,
                        created_unix_ms: entry.created_at_unix_ms,
                    },
                    state: EntryState::Unread,
                })
                .collect(),
            RuntimeKind::DeepSeek => read(
                history::list(&self.history_config(runtime)?, &runtime.id)
                    .map_err(|error| self.history_failure(runtime, error))?,
            ),
        };
        self.store_history_index(source, entries)
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
        Ok(())
    }
    fn history_failure(
        &self,
        runtime: &RuntimeInstance,
        error: velune_agent_runtime::history::HistoryError,
    ) -> RuntimeError {
        let family = velune_agent_runtime::version::variant(runtime.type_id).family_id;
        tracing::warn!(target: "velune_application", event = "runtime_history_read_failed", phase = error.phase(), failure_kind = error.code(), runtime_family = family, runtime_id=%runtime.id, runtime_name=%runtime.name, runtime_type_id=%runtime.type_id, binary=?runtime.settings.get("binary"), node_binary=?runtime.settings.get("nodeBinary"), agent_dir=?runtime.settings.get("agentDir"), detail=%error);
        RuntimeError::History(error)
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
                    let id = format!("{prefix}{}", session.path);
                    (
                        id.clone(),
                        ConversationSummary {
                            native_read_state: None,
                            can_rename: false,
                            can_delete: false,
                            id,
                            title: velune_conversation::conversation_title(
                                session.name.as_deref(),
                                session.first_message.as_deref(),
                            ),
                            updated_at_unix_ms: session.modified_unix_ms,
                            created_at_unix_ms: session.created_unix_ms,
                            runtime_id: runtime.id.clone(),
                            cwd: session.cwd,
                            parent_conversation_id: None,
                        },
                    )
                })
                .collect();
            (summaries, identities)
        } else {
            let summaries =
                history::summaries(&self.history_config(runtime)?, &runtime.id, &natives)
                    .map_err(|error| self.history_failure(runtime, error))?
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
                        EntryState::Read(Box::new(summary))
                    });
                }
                None => {
                    if let Some(summary) = summary {
                        listing.entries.push(IndexedConversation {
                            id: id.clone(),
                            identity: String::new(),
                            hint: ConversationHint::default(),
                            state: EntryState::Read(Box::new(summary)),
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
        *self.last_listed.borrow_mut() = organization::ListedProjection::default();
        self.history_listings.borrow_mut().remove(runtime_id);
        self.native_projects.borrow_mut().remove(runtime_id);
        self.reached
            .borrow_mut()
            .retain(|_, cached| cached.source.runtime_id() != runtime_id);
        self.archived_listing.borrow_mut().remove(runtime_id);
    }
    /// Read the native conversations continued ones are made of, so a
    /// continued conversation lists with its own title and activity.
    pub(super) fn read_link_segments(&self) {
        for runtime in self.runtime_instances.iter().filter(|r| r.enabled) {
            let unread: Vec<String> = self
                .indexed(runtime)
                .into_iter()
                .filter(|entry| {
                    matches!(entry.state, EntryState::Unread)
                        && self.conversation_links.iter().any(|link| {
                            link.segments
                                .iter()
                                .any(|segment| segment.native_conversation_id == entry.id)
                        })
                })
                .map(|entry| entry.id)
                .collect();
            if let Err(error) = self.read_indexed(runtime, &unread) {
                tracing::warn!(target:"velune_application",event="runtime_history_read_failed",phase="link_segments",runtime_id=%runtime.id,runtime_type_id=%runtime.type_id,detail=%error);
            }
        }
    }
    /// Read what the runtime's index has not read yet.
    pub(super) fn read_unread(&self, runtime: &RuntimeInstance) -> Result<(), RuntimeError> {
        let unread: Vec<String> = self
            .indexed(runtime)
            .into_iter()
            .filter(|entry| matches!(entry.state, EntryState::Unread))
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
    ) -> Result<Option<ConversationSummary>, RuntimeError> {
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
    ) -> Option<ConversationSummary> {
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
                    EntryState::Read(summary) if entry.id == id => Some(summary.as_ref().clone()),
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
    ) -> Result<ConversationSnapshot, RuntimeError> {
        let runtime = self.runtime_instance(runtime_id)?;
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用，请先启用"));
        }
        let native_id = id
            .strip_prefix(&format!("{runtime_id}:"))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        // Source-scoped lookup prevents arbitrary caller paths from being opened.
        // Conversations are opened from a listing, so its membership is reused;
        // an identifier the latest listing does not contain lists again.
        // Codex can omit a fork with inherited history and no own user turn;
        // its native read-by-identity API still owns that conversation.
        let summary = match self.listed_summary(runtime, id) {
            Some(summary) => summary,
            None => self
                .conversation_summary(runtime, id)
                .map_err(|error| {
                    tracing::warn!(target: "velune_application", event="runtime_history_read_failed", phase="history_lookup", runtime_id=%runtime.id, runtime_name=%runtime.name, runtime_type_id=%runtime.type_id, binary=?runtime.settings.get("binary"), node_binary=?runtime.settings.get("nodeBinary"), agent_dir=?runtime.settings.get("agentDir"), detail=%error);
                    if runtime.type_id == RuntimeKind::Pi {
                        RuntimeError::context(&runtime_context(runtime), error)
                    } else {
                        error
                    }
                })?
                .or_else(|| {
                    matches!(runtime.type_id, RuntimeKind::Codex166)
                        .then(|| self.reach(runtime, id))
                        .flatten()
                        .map(|source| source.summary)
                })
                .ok_or_else(|| RuntimeError::invalid("此会话不属于所选运行时历史目录"))?,
        };
        if runtime.type_id == RuntimeKind::Acp {
            let cwd = summary
                .cwd
                .as_deref()
                .map(Path::new)
                .ok_or_else(|| RuntimeError::invalid("ACP 会话列表缺少工作目录"))?;
            return self.read_acp_history(runtime, native_id, cwd);
        }
        let mut snapshot = ConversationSnapshot {
            revision: 1,
            context_runtime_id: runtime_id.into(),
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
        if runtime.type_id == RuntimeKind::Pi {
            let saved = pi_session_helper(
                &self.pi_history_config(runtime)?,
                Path::new(native_id),
            ).map_err(|error| {
                tracing::warn!(target: "velune_application", event="runtime_history_read_failed", phase="history_read", runtime_id=%runtime.id, runtime_name=%runtime.name, runtime_type_id=%runtime.type_id, binary=?runtime.settings.get("binary"), node_binary=?runtime.settings.get("nodeBinary"), agent_dir=?runtime.settings.get("agentDir"), detail=%error);
                RuntimeError::context(&runtime_context(runtime), error)
            })?;
            let mut projection = PiProjection::new(snapshot.conversation.clone());
            projection.replace_history(&saved);
            let projected = projection.snapshot.expect("history projection");
            snapshot.messages = projected.messages;
            snapshot.executions = projected.executions;
            snapshot.conversation.cwd = saved["cwd"].as_str().map(str::to_owned);
            let marker = &saved["virtualState"];
            if marker["provider"] == "velune" || marker["provider"] == "velune-gateway" {
                snapshot.model_record_key = marker["state"]["modelRecordKey"]
                    .as_str()
                    .filter(|key| self.validate_session_model(runtime, key).is_ok())
                    .map(str::to_owned);
            }
        } else {
            let read = history::read(&self.history_config(runtime)?, runtime_id, native_id).map_err(|error| {
                let family = velune_agent_runtime::version::variant(runtime.type_id).family_id;
                tracing::warn!(target: "velune_application", event = "runtime_history_read_failed", phase = error.phase(), failure_kind = error.code(), runtime_family = family, runtime_id=%runtime.id, runtime_name=%runtime.name, runtime_type_id=%runtime.type_id, binary=?runtime.settings.get("binary"), node_binary=?runtime.settings.get("nodeBinary"), agent_dir=?runtime.settings.get("agentDir"), detail=%error);
                RuntimeError::History(error)
            })?;
            snapshot.conversation.native_read_state = read.native_read_state;
            snapshot.conversation.cwd = read.cwd;
            snapshot.messages = read.messages;
            snapshot.executions = read.executions;
            snapshot.follow_up.can_resume = read.can_resume;
        }
        // A history projection is send-capable once the UI supplies the next
        // turn's model. The source record may intentionally have no model
        // marker, so this flag must not encode the old model-selection state.
        // A native sub-agent's conversation is continued by its parent's agent.
        snapshot.actions.can_send = snapshot.conversation.parent_conversation_id.is_none();
        Ok(snapshot)
    }
    pub(super) fn invalidate_execution(&mut self) -> Result<(), RuntimeError> {
        self.ensure_work_ended()?;
        let can_restore = match &self.session.active_state {
            ActiveState::Acp(session) => session.capabilities().load_session,
            _ => true,
        };
        let mut snapshot = self.native_snapshot();
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
                view.pending_interactions.clear();
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
                self.session.active_state = ActiveState::History(Box::new(snapshot));
            }
        }
        shutdown_result
    }
    pub(super) fn prepare_snapshot(
        &mut self,
        snapshot: ConversationSnapshot,
        key: &str,
        is_new: bool,
        project: Option<&velune_agent_runtime::native::codex_projects::NativeProject>,
    ) -> Result<(), RuntimeError> {
        let previous = self.native_snapshot();
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
                if is_new {
                    self.native_create(cwd.as_deref(), key, project)?;
                } else {
                    self.native_resume(&snapshot, key)?;
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
            let session = if is_new {
                None
            } else {
                Some(
                    snapshot
                        .conversation
                        .id
                        .strip_prefix(&format!("{runtime_id}:"))
                        .ok_or_else(|| RuntimeError::invalid("conversation id"))?,
                )
            };
            self.start_pi_for_session(
                cwd.as_deref(),
                session.map(Path::new),
                Some(key),
                Some(&physical),
            )?;
            let persistent = self.session.side.is_none();
            let session = self.session.pi()?;
            if is_new && persistent {
                session
                    .new_session()
                    .map_err(|error| RuntimeError::context("conversation create", error))?;
            } else {
                session.restore_context(&snapshot);
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
                view.messages.push(Message {
                    turn_settings: None,
                    turn_usage: None,
                    branch_point: None,
                    completed: true,
                    id: format!("prepare-failed:{}", view.revision),
                    role: velune_conversation::MessageRole::System,
                    timestamp_unix_ms: None,
                    blocks: vec![MessageBlock::Notice {
                        text: "运行时准备失败；会话内容已保留，可修正配置后重试。".into(),
                    }],
                });
                view.revision = view.revision.saturating_add(1);
                self.session.active_state = ActiveState::History(Box::new(view));
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
            state: EntryState::Read(Box::new(summary("synthetic:thread"))),
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
    fn native_caches_keep_non_source_edits_and_reject_old_results_and_fallbacks() {
        let (mut app, root) = open();
        let core = &mut app.runtime;
        let old = runtime(RuntimeKind::Codex166);
        core.upsert_runtime(old.clone()).unwrap();
        let source = core.runtime_source_key(&old).unwrap();
        core.store_history_index(source.clone(), entries()).unwrap();
        core.store_native_projects(source.clone(), projects("original"), false)
            .unwrap();
        let stale_archive = HistoryArchive {
            source: source.clone(),
            summaries: vec![summary("synthetic:archived")],
        };
        let stale_reached = CachedSource {
            source: source.clone(),
            value: organization::ListedSource {
                summary: summary("synthetic:reached"),
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
        core.store_native_projects(fresh.clone(), projects("fresh"), false)
            .unwrap();
        assert!(core.store_history_index(source.clone(), entries()).is_err());
        assert!(
            core.store_native_projects(source, projects("stale"), true)
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
                    summary: summary("synthetic:thread"),
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
}

impl CoreRuntime {
    pub(super) fn keep_native_read_state(
        &mut self,
        receipt: &velune_conversation::NativeReadState,
    ) -> Result<(), RuntimeError> {
        let update = |summary: &mut ConversationSummary| {
            if summary.id == receipt.conversation_id
                || summary
                    .native_read_state
                    .as_ref()
                    .is_some_and(|s| s.conversation_id == receipt.conversation_id)
            {
                summary.native_read_state = Some(receipt.clone());
            }
        };
        for source in self.last_listed.borrow_mut().entries.values_mut() {
            update(&mut source.summary);
        }
        for source in self.reached.borrow_mut().values_mut() {
            update(&mut source.value.summary);
        }
        for archive in self.archived_listing.borrow_mut().values_mut() {
            for summary in &mut archive.summaries {
                update(summary);
            }
        }
        for listing in self.history_listings.borrow_mut().values_mut() {
            for entry in &mut listing.entries {
                if let EntryState::Read(summary) = &mut entry.state {
                    update(summary);
                }
            }
        }
        self.for_each_session(|core| {
            match &mut core.session.active_state {
                ActiveState::History(snapshot) => update(&mut snapshot.conversation),
                ActiveState::Native(session) => session.set_read_state(receipt),
                _ => {}
            }
            Ok(())
        })
    }
}
