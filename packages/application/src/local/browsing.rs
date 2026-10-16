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
    runtime: RuntimeInstance,
    entries: Vec<IndexedConversation>,
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
    pub(super) cwd: Option<String>,
    /// The file's modification time: which sessions to read first, never shown.
    pub(super) activity_unix_ms: Option<i64>,
    pub(super) created_unix_ms: Option<i64>,
}
#[derive(Clone)]
pub(super) enum EntryState {
    Unread,
    Read(ConversationSummary),
    /// The runtime could not read it as a session; not listed until it changes.
    Unreadable,
}
/// Runtimes whose sessions are files read one by one.
fn reads_lazily(runtime: &RuntimeInstance) -> bool {
    matches!(runtime.type_id.as_str(), "pi-1.0.2" | "claude-code-2.1.286")
}
impl CoreRuntime {
    pub(super) fn runtime_instance(&self, id: &str) -> Result<&RuntimeInstance, RuntimeError> {
        self.runtime_instances
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))
    }
    pub(super) fn select_runtime_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if self.busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let id = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
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
        let context = self.prepare_execution_context(runtime)?;
        Ok(PiConfig {
            mcp_servers: Vec::new(),
            execution: context.target,
            os_home: context.os_home,
            runtime_type_id: runtime.type_id.clone(),
            binary: setting_path(runtime, "binary")?,
            node_binary: Some(setting_path(runtime, "nodeBinary")?),
            sdk_helper: Some(context.resources.join("pi_sessions.mjs")),
            agent_dir: Some(context.agent_dir),
            session_dir: setting_path_optional(runtime, "sessionDir"),
            working_dir: None,
            provider: None,
            model: None,
            session: None,
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
        let read = |summaries: Vec<ConversationSummary>| {
            summaries
                .into_iter()
                .map(|summary| IndexedConversation {
                    id: summary.id.clone(),
                    identity: String::new(),
                    hint: ConversationHint::default(),
                    state: EntryState::Read(summary),
                })
                .collect::<Vec<_>>()
        };
        let mut entries = match runtime.type_id.as_str() {
            "acp-v1" => read(self.acp_summaries(runtime)?),
            "pi-1.0.2" => history::list_pi(&self.pi_history_config(runtime)?, &[])
                .map_err(|error| {
                    RuntimeError::context("Pi 历史无法读取；请检查实例路径与 SDK 配置后重试", error)
                })?
                .files
                .into_iter()
                .map(|file| IndexedConversation {
                    id: format!("{}:{}", runtime.id, file.path),
                    identity: format!("{}:{}", file.size, file.mtime_ns),
                    hint: ConversationHint {
                        cwd: file.cwd,
                        activity_unix_ms: file.modified_at_unix_ms,
                        created_unix_ms: file.created_at_unix_ms,
                    },
                    state: EntryState::Unread,
                })
                .collect(),
            "claude-code-2.1.286" => history::index(&self.history_config(runtime)?)
                .map_err(|error| self.history_failure(runtime, error))?
                .into_iter()
                .map(|entry| IndexedConversation {
                    id: format!("{}:{}", runtime.id, entry.native_id),
                    identity: format!("{}:{:?}", entry.size, entry.modified_at_unix_ms),
                    hint: ConversationHint {
                        cwd: entry.cwd,
                        activity_unix_ms: entry.modified_at_unix_ms,
                        created_unix_ms: entry.created_at_unix_ms,
                    },
                    state: EntryState::Unread,
                })
                .collect(),
            _ => read(
                history::list(&self.history_config(runtime)?, &runtime.id)
                    .map_err(|error| self.history_failure(runtime, error))?,
            ),
        };
        let previous = self
            .history_listings
            .borrow_mut()
            .remove(&runtime.id)
            .filter(|listing| listing.runtime == *runtime)
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
            runtime.id.clone(),
            HistoryListing {
                runtime: runtime.clone(),
                entries,
            },
        );
        Ok(())
    }
    fn history_failure(
        &self,
        runtime: &RuntimeInstance,
        error: velune_agent_runtime::history::HistoryError,
    ) -> RuntimeError {
        let variant = velune_agent_runtime::version::variant(&runtime.type_id);
        let family = variant.map(|value| value.family_id).unwrap_or("unknown");
        let runtime_type = variant.map(|value| value.id).unwrap_or("unknown");
        tracing::warn!(target: "velune_application", event = "runtime_history_read_failed", phase = error.phase(), failure_kind = error.code(), runtime_family = family, runtime_type, runtime_id=%runtime.id, runtime_name=%runtime.name, runtime_type_id=%runtime.type_id, binary=?runtime.settings.get("binary"), node_binary=?runtime.settings.get("nodeBinary"), agent_dir=?runtime.settings.get("agentDir"), detail=%error);
        RuntimeError::History(error)
    }
    /// The runtime's indexed conversations as last indexed.
    pub(super) fn indexed(&self, runtime: &RuntimeInstance) -> Vec<IndexedConversation> {
        self.history_listings
            .borrow()
            .get(&runtime.id)
            .filter(|listing| listing.runtime == *runtime)
            .map(|listing| listing.entries.clone())
            .unwrap_or_default()
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
        let prefix = format!("{}:", runtime.id);
        let natives: Vec<String> = ids
            .iter()
            .filter_map(|id| id.strip_prefix(&prefix).map(str::to_owned))
            .collect();
        let (summaries, identities): (
            BTreeMap<String, ConversationSummary>,
            BTreeMap<String, String>,
        ) = if runtime.type_id == "pi-1.0.2" {
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
            .filter(|listing| listing.runtime == *runtime)
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
                    entry.state = summary.map_or(EntryState::Unreadable, EntryState::Read);
                }
                None => {
                    if let Some(summary) = summary {
                        listing.entries.push(IndexedConversation {
                            id: id.clone(),
                            identity: String::new(),
                            hint: ConversationHint::default(),
                            state: EntryState::Read(summary),
                        });
                    }
                }
            }
        }
        Ok(())
    }
    /// Drop a runtime's index, so a runtime that cannot be read lists nothing.
    pub(super) fn forget_index(&self, runtime: &RuntimeInstance) {
        self.history_listings.borrow_mut().remove(&runtime.id);
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
        self.history_listings
            .borrow()
            .get(&runtime.id)
            .filter(|listing| listing.runtime == *runtime)
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
                    .iter()
                    .find(|summary| summary.id == id && summary.runtime_id == runtime.id)
                    .cloned()
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
        let protocols = GatewayProtocol::runtime_provider_protocols(&runtime.type_id)
            .ok_or_else(|| RuntimeError::invalid("runtime type"))?;
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
        let summary = match self.listed_summary(runtime, id) {
            Some(summary) => summary,
            None => self
                .conversation_summary(runtime, id)
                .map_err(|error| {
                    tracing::warn!(target: "velune_application", event="runtime_history_read_failed", phase="history_lookup", runtime_id=%runtime.id, runtime_name=%runtime.name, runtime_type_id=%runtime.type_id, binary=?runtime.settings.get("binary"), node_binary=?runtime.settings.get("nodeBinary"), agent_dir=?runtime.settings.get("agentDir"), detail=%error);
                    if runtime.type_id == "pi-1.0.2" {
                        RuntimeError::context(&runtime_context(runtime), error)
                    } else {
                        error
                    }
                })?
                .ok_or_else(|| RuntimeError::invalid("此会话不属于所选运行时历史目录"))?,
        };
        if runtime.type_id == "acp-v1" {
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
            transcript_turns: Vec::new(),
            transcript_items: Vec::new(),
            transcript_outline: Vec::new(),
            transcript_message_identities: Vec::new(),
            message_identity_confirmations: Vec::new(),
            pending_interactions: Vec::new(),
            follow_up: Default::default(),
            actions: ConversationActions {
                can_send: false,
                can_cancel: false,
                can_switch: true,
            },
        };
        if runtime.type_id == "pi-1.0.2" {
            let saved = pi_session_helper(
                &self.pi_history_config(runtime)?,
                Path::new(native_id),
            ).map_err(|error| {
                tracing::warn!(target: "velune_application", event="runtime_history_read_failed", phase="history_read", runtime_id=%runtime.id, runtime_name=%runtime.name, runtime_type_id=%runtime.type_id, binary=?runtime.settings.get("binary"), node_binary=?runtime.settings.get("nodeBinary"), agent_dir=?runtime.settings.get("agentDir"), detail=%error);
                RuntimeError::context(&runtime_context(runtime), error)
            })?;
            let mut projection = PiProjection::new(snapshot.conversation.clone());
            projection.replace_history(&saved);
            snapshot.messages = projection.snapshot.expect("history projection").messages;
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
                let variant = velune_agent_runtime::version::variant(&runtime.type_id);
                let family = variant.map(|value| value.family_id).unwrap_or("unknown");
                let runtime_type = variant.map(|value| value.id).unwrap_or("unknown");
                tracing::warn!(target: "velune_application", event = "runtime_history_read_failed", phase = error.phase(), failure_kind = error.code(), runtime_family = family, runtime_type, runtime_id=%runtime.id, runtime_name=%runtime.name, runtime_type_id=%runtime.type_id, binary=?runtime.settings.get("binary"), node_binary=?runtime.settings.get("nodeBinary"), agent_dir=?runtime.settings.get("agentDir"), detail=%error);
                RuntimeError::History(error)
            })?;
            snapshot.conversation.cwd = read.cwd;
            snapshot.messages = read.messages;
        }
        // A history projection is send-capable once the UI supplies the next
        // turn's model. The source record may intentionally have no model
        // marker, so this flag must not encode the old model-selection state.
        // A native sub-agent's conversation is continued by its parent's agent.
        snapshot.actions.can_send = snapshot.conversation.parent_conversation_id.is_none();
        Ok(snapshot)
    }
    pub(super) fn invalidate_execution(&mut self) -> Result<(), RuntimeError> {
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
            &runtime.type_id,
            &setting_path(&runtime, "binary")?,
            setting_path_optional(&runtime, "nodeBinary").as_deref(),
            &context.target,
            context.os_home.as_deref(),
        )
        .map_err(|error| RuntimeError::Invalid(error.to_string()))?;
        if runtime.type_id == "pi-1.0.2" {
            runnable_pi_model(gateway, key)?;
        }
        gateway.to_gateway_config()?;
        let result = (|| {
            self.prepare_runtime(&runtime_id, key)?;
            if runtime.type_id != "pi-1.0.2" {
                if is_new {
                    self.native_create(cwd.as_deref(), key)?;
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
            let subscription = gateway.subscription(key);
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
                subscription,
            )?;
            if is_new {
                self.session
                    .pi
                    .client
                    .as_mut()
                    .expect("started Pi")
                    .request(json!({"type":"new_session"}))
                    .map_err(|error| RuntimeError::context("conversation create", error))?;
            }
            if !is_new {
                self.session
                    .pi
                    .projection
                    .as_mut()
                    .expect("started Pi")
                    .set_conversation(snapshot.conversation.clone());
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
