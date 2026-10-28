//! Local application configuration use cases.
use super::*;
use crate::RuntimeExecution;
enum RuntimeChange {
    Upsert(RuntimeInstance),
    Delete(String),
}

impl CoreRuntime {
    pub(crate) fn set_conversation_browser_preferences(
        &mut self,
        preferences: crate::config::ConversationBrowserPreferences,
    ) -> Result<crate::ConfigurationSnapshot, RuntimeError> {
        let previous = std::mem::replace(&mut self.conversation_browser_preferences, preferences);
        if let Err(error) = self.persist() {
            self.conversation_browser_preferences = previous;
            return Err(error);
        }
        let current = &self.conversation_browser_preferences;
        if (
            current.grouping,
            current.sort,
            current.oldest_first,
            &current.runtime_id,
            &current.project,
        ) != (
            previous.grouping,
            previous.sort,
            previous.oldest_first,
            &previous.runtime_id,
            &previous.project,
        ) {
            self.reset_browser_pages();
        }
        self.list()
    }

    pub(crate) fn set_outline_presentation(
        &mut self,
        presentation: crate::config::OutlinePresentation,
    ) -> Result<crate::config::OutlinePresentation, RuntimeError> {
        let previous = self.outline_presentation;
        self.outline_presentation = presentation;
        if let Err(error) = self.persist() {
            self.outline_presentation = previous;
            return Err(error);
        }
        Ok(presentation)
    }

    pub(crate) fn set_linked_conversation_opening(
        &mut self,
        opening: crate::config::LinkedConversationOpening,
    ) -> Result<crate::config::LinkedConversationOpening, RuntimeError> {
        let previous = self.linked_conversation_opening;
        self.linked_conversation_opening = opening;
        if let Err(error) = self.persist() {
            self.linked_conversation_opening = previous;
            return Err(error);
        }
        Ok(opening)
    }

    pub(crate) fn set_usage_display(
        &mut self,
        display: crate::config::UsageDisplay,
    ) -> Result<crate::config::UsageDisplay, RuntimeError> {
        let previous = self.usage_display;
        self.usage_display = display;
        if let Err(error) = self.persist() {
            self.usage_display = previous;
            return Err(error);
        }
        Ok(display)
    }

    pub(crate) fn set_context_usage_display(
        &mut self,
        display: crate::config::UsageDisplay,
    ) -> Result<crate::config::UsageDisplay, RuntimeError> {
        let previous = self.context_usage_display;
        self.context_usage_display = display;
        if let Err(error) = self.persist() {
            self.context_usage_display = previous;
            return Err(error);
        }
        Ok(display)
    }

    pub(crate) fn set_status_line(
        &mut self,
        items: Vec<crate::config::StatusLineItem>,
    ) -> Result<Vec<crate::config::StatusLineItem>, RuntimeError> {
        let items = crate::config::canonical_status_line(items);
        let previous = std::mem::replace(&mut self.status_line, items);
        if let Err(error) = self.persist() {
            self.status_line = previous;
            return Err(error);
        }
        Ok(self.status_line.clone())
    }

    pub(crate) fn composer_drafts(
        &self,
    ) -> Result<Vec<crate::config::ComposerDraft>, RuntimeError> {
        Ok(self
            .composer_drafts
            .iter()
            .map(|(id, text)| crate::config::ComposerDraft {
                conversation_id: id.clone(),
                text: text.clone(),
            })
            .collect())
    }

    /// Empty text removes the conversation's draft.
    pub(crate) fn set_composer_draft(
        &mut self,
        conversation_id: String,
        text: String,
    ) -> Result<(), RuntimeError> {
        if conversation_id.is_empty() {
            return Err(RuntimeError::invalid("conversation id"));
        }
        let mut next = self.composer_drafts.clone();
        if text.is_empty() {
            next.remove(&conversation_id);
        } else {
            next.insert(conversation_id, text);
        }
        if next != self.composer_drafts {
            if self.client_baseline.is_none() {
                self.repository.store_composer_drafts(&next)?;
            }
            self.composer_drafts = next;
        }
        Ok(())
    }

    /// The conversation is already deleted; a failed write only leaves a stale
    /// entry on disk, which the next successful draft write drops.
    pub(super) fn discard_composer_draft(&mut self, conversation_id: &str) {
        if self.composer_drafts.remove(conversation_id).is_some() && self.client_baseline.is_none()
        {
            let _ = self.repository.store_composer_drafts(&self.composer_drafts);
        }
    }

    pub(crate) fn set_conversation_browser_limits(
        &mut self,
        limits: crate::api::ConversationBrowserLimits,
    ) -> Result<crate::api::ConversationBrowserLimits, RuntimeError> {
        if limits.page == 0 || limits.total == 0 {
            return Err(RuntimeError::invalid("会话加载数量必须大于零"));
        }
        let previous = self.conversation_browser_limits;
        self.conversation_browser_limits = limits;
        if let Err(error) = self.persist() {
            self.conversation_browser_limits = previous;
            return Err(error);
        }
        Ok(limits)
    }

    /// Every enabled runtime's conversations, continued ones by their link,
    /// with the runtimes' own archives in the archive view. Indexes each
    /// runtime's history again and reads all of it only when `complete`.
    pub(super) fn list_sources(
        &self,
        complete: bool,
    ) -> Result<
        (
            Vec<organization::ListedSource>,
            Vec<crate::api::HistoryFailure>,
        ),
        RuntimeError,
    > {
        let mut history_failures = Vec::new();
        for (slot, runtime) in self.runtime_instances.iter().enumerate() {
            if !runtime.enabled {
                continue;
            }
            let indexed = self.index_history(runtime).and_then(|()| {
                if complete {
                    self.read_unread(runtime)
                } else {
                    Ok(())
                }
            });
            if let Err(error) = indexed {
                // A runtime that cannot be read lists nothing, not its last index.
                self.forget_index(runtime);
                tracing::warn!(target:"velune_application",event="runtime_history_read_failed",phase="history_list",failure_kind="history_unavailable",runtime_slot=slot+1,runtime_id=%runtime.id,runtime_name=%runtime.name,runtime_type_id=%runtime.type_id,binary=?runtime.settings.get("binary"),node_binary=?runtime.settings.get("nodeBinary"),agent_dir=?runtime.settings.get("agentDir"),detail=%error);
                history_failures.push(crate::api::HistoryFailure {
                    runtime_id: runtime.id.clone(),
                    detail: format!(
                        "{}；会话历史无法读取：{error}",
                        super::browsing::runtime_context(runtime)
                    ),
                });
            }
        }
        self.read_link_segments();
        *self.archived_listing.borrow_mut() = if self.shows_archived() {
            self.natively_archived()
        } else {
            Vec::new()
        };
        Ok((self.cached_sources(), history_failures))
    }

    /// The listing from the indexes as they stand. A conversation not read
    /// yet is provisional: identity and what its file tells, untitled.
    pub(super) fn cached_sources(&self) -> Vec<organization::ListedSource> {
        let mut conversations = Vec::new();
        let mut provisional = std::collections::BTreeSet::new();
        for runtime in self.runtime_instances.iter().filter(|r| r.enabled) {
            for entry in self.indexed(runtime) {
                match entry.state {
                    browsing::EntryState::Read(summary) => conversations.push(summary),
                    browsing::EntryState::Unread => {
                        provisional.insert(entry.id.clone());
                        conversations.push(ConversationSummary {
                            id: entry.id,
                            title: velune_conversation::ConversationTitle::Untitled,
                            updated_at_unix_ms: entry.hint.activity_unix_ms,
                            created_at_unix_ms: entry.hint.created_unix_ms,
                            runtime_id: runtime.id.clone(),
                            cwd: entry.hint.cwd,
                            can_rename: false,
                            can_delete: false,
                            parent_conversation_id: None,
                        });
                    }
                    browsing::EntryState::Unreadable => {}
                }
            }
        }
        // Active native drafts, of any open session, are legitimate current
        // conversations even before their runtime writes history; an agent may
        // have started one. They disappear when that native session closes.
        for session in std::iter::once(&self.session).chain(&self.parked) {
            if matches!(
                session.active_state,
                ActiveState::Pi | ActiveState::Native(_) | ActiveState::Acp(_)
            ) && let Some(view) = session.projected()
                && !conversations
                    .iter()
                    .any(|item| item.id == view.conversation.id)
            {
                conversations.push(view.conversation.clone());
            }
        }
        let mut sources: Vec<_> = self
            .linked_summaries(conversations)
            .into_iter()
            .map(|summary| organization::ListedSource {
                provisional: provisional.contains(&summary.id),
                summary,
                archived_natively: false,
            })
            .collect();
        sources.extend(self.archived_listing.borrow().iter().map(|summary| {
            organization::ListedSource {
                summary: summary.clone(),
                archived_natively: true,
                provisional: false,
            }
        }));
        sources
    }

    pub(crate) fn list(&self) -> Result<crate::ConfigurationSnapshot, RuntimeError> {
        let (sources, history_failures) = self.list_sources(false)?;
        let page = self.browse_read(sources, None);
        let runtime_types = runtime_types();
        let provider_import_types = std::iter::once(provider_import::descriptor())
            .chain(
                runtime_types
                    .iter()
                    .filter(|kind| crate::native_provider_import::TYPES.contains(&kind.id.as_str()))
                    .cloned(),
            )
            .collect();
        let protocol = |id: &str, name: &str| crate::ProtocolDescriptor {
            id: id.into(),
            name: name.into(),
            supported: true,
        };
        Ok(crate::ConfigurationSnapshot {
            conversations: page.conversations,
            conversation_groups: page.groups,
            conversation_projects: page.projects,
            shows_archived: self.shows_archived(),
            history_failures,
            gateways: self.public_gateways(),
            runtime_instances: self.public_runtime_instances(),
            runtime_types,
            model_templates: self.model_templates.clone(),
            conversation_browser_limits: self.conversation_browser_limits,
            outline_presentation: self.outline_presentation,
            linked_conversation_opening: self.linked_conversation_opening,
            usage_display: self.usage_display,
            context_usage_display: self.context_usage_display,
            conversation_browser_preferences: self.conversation_browser_preferences.clone(),
            status_line: self.status_line.clone(),
            provider_import_types,
            protocols: vec![
                protocol("chatCompletionsV1", "OpenAI Chat Completions v1"),
                protocol("responsesV1", "OpenAI Responses v1"),
                protocol("messagesV1", "Anthropic Messages"),
            ],
            selected_runtime_instance_id: self.next_turn_runtime_id.clone(),
        })
    }

    /// Transitional: `list` for use cases not yet typed.
    pub(super) fn list_value(&self) -> Result<Value, RuntimeError> {
        Ok(serde_json::to_value(self.list()?)?)
    }

    /// Runtime instances as clients see them: ACP launch arguments and
    /// environment are read only through the explicit editor use case.
    pub(super) fn public_runtime_instances(&self) -> Vec<RuntimeInstance> {
        self.runtime_instances
            .iter()
            .cloned()
            .map(|mut runtime| {
                runtime.settings.remove(ACP_LAUNCH_SETTING);
                runtime
            })
            .collect()
    }

    pub(super) fn provider_import_action(
        &mut self,
        request: &Value,
    ) -> Result<Value, RuntimeError> {
        if self.execution_active() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let operation = request["payload"]["operation"]
            .as_str()
            .unwrap_or("preview");
        let gateway_id = request["payload"]["gatewayID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("gateway id"))?;
        let empty_default = GatewayConfig {
            id: "default".into(),
            name: "默认网关".into(),
            providers: Vec::new(),
            routes: Vec::new(),
        };
        let existing = self.gateways.iter().position(|item| item.id == gateway_id);
        if existing.is_none() && gateway_id != "default" {
            return Err(RuntimeError::invalid("gateway id"));
        }
        match operation {
            "preview" => {
                let gateway = existing
                    .map(|index| &self.gateways[index])
                    .unwrap_or(&empty_default);
                provider_import::preview(
                    &request["payload"],
                    gateway,
                    &self.options,
                    &self.runtime_instances,
                )
                .map(|preview| json!({"preview":preview,"gateway":crate::api::GatewaySummary::from(gateway)}))
            }
            "apply" => {
                let mut imported_gateway = existing
                    .map(|index| self.gateways[index].clone())
                    .unwrap_or(empty_default);
                let previous = imported_gateway.clone();
                let result = provider_import::apply(
                    &request["payload"],
                    &mut imported_gateway,
                    &self.options,
                    &self.runtime_instances,
                )?;
                if imported_gateway.providers.is_empty() {
                    return Err(RuntimeError::invalid(
                        "provider import selected no providers",
                    ));
                }
                let execution_invalidated = (imported_gateway != previous)
                    && self
                        .next_turn_runtime_id
                        .as_ref()
                        .and_then(|id| {
                            self.runtime_instances
                                .iter()
                                .find(|runtime| &runtime.id == id)
                        })
                        .is_some_and(|runtime| runtime.gateway_id == gateway_id);
                let previous_runtimes = self.runtime_instances.clone();
                if let Some(index) = existing {
                    self.gateways[index] = imported_gateway;
                } else {
                    self.gateways.push(imported_gateway);
                }
                if let Err(error) = self.persist() {
                    self.runtime_instances = previous_runtimes;
                    if let Some(index) = existing {
                        self.gateways[index] = previous;
                    } else {
                        self.gateways.pop();
                    }
                    return Err(error);
                }
                self.sync_scheduling(std::slice::from_ref(&previous));
                if execution_invalidated {
                    self.for_each_session(Self::invalidate_execution)?;
                }
                Ok(
                    json!({"importedProviderIds":result["importedProviderIds"],"skippedProviderIds":result["skippedProviderIds"],"gateways":self.public_gateways(),"executionInvalidated":execution_invalidated}),
                )
            }
            _ => Err(RuntimeError::invalid("provider import operation")),
        }
    }

    pub(crate) fn upsert_runtime(
        &mut self,
        mut runtime: RuntimeInstance,
    ) -> Result<crate::RuntimeUpdate, RuntimeError> {
        self.retain_private_runtime_settings(&mut runtime);
        self.change_runtimes(RuntimeChange::Upsert(runtime))
    }

    pub(crate) fn upsert_runtime_with_execution(
        &mut self,
        mut runtime: RuntimeInstance,
        execution: RuntimeExecution,
    ) -> Result<crate::RuntimeUpdate, RuntimeError> {
        runtime.set_execution(execution)?;
        self.upsert_runtime(runtime)
    }

    pub(crate) fn upsert_acp_runtime(
        &mut self,
        mut runtime: RuntimeInstance,
        execution: RuntimeExecution,
        launch: crate::AcpLaunch,
    ) -> Result<crate::RuntimeUpdate, RuntimeError> {
        runtime.set_acp_launch(launch)?;
        self.upsert_runtime_with_execution(runtime, execution)
    }

    pub(crate) fn delete_runtime(
        &mut self,
        id: String,
    ) -> Result<crate::RuntimeUpdate, RuntimeError> {
        self.change_runtimes(RuntimeChange::Delete(id))
    }

    fn change_runtimes(
        &mut self,
        change: RuntimeChange,
    ) -> Result<crate::RuntimeUpdate, RuntimeError> {
        if self.execution_active() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        self.acp_capability_cache.borrow_mut().clear();
        let previous = self.runtime_instances.clone();
        let previous_gateways = self.gateways.clone();
        match change {
            RuntimeChange::Upsert(runtime) => {
                if runtime.id.is_empty()
                    || runtime.name.is_empty()
                    || !matches!(
                        runtime.type_id.as_str(),
                        "pi-1.0" | "codex-0.159" | "dsh-acp-0.2" | "claude-code-2.1" | "acp-v1"
                    )
                    || (runtime.gateway_id != "default"
                        && !self
                            .gateways
                            .iter()
                            .any(|item| item.id == runtime.gateway_id))
                {
                    return Err(RuntimeError::invalid("runtime instance"));
                }
                if runtime.enabled {
                    runtime
                        .validate_execution_paths()
                        .map_err(RuntimeError::invalid)?;
                }
                if runtime.gateway_id == "default"
                    && !self.gateways.iter().any(|g| g.id == "default")
                {
                    self.gateways.push(GatewayConfig {
                        id: "default".into(),
                        name: "默认网关".into(),
                        providers: Vec::new(),
                        routes: Vec::new(),
                    });
                }
                self.runtime_instances.retain(|item| item.id != runtime.id);
                self.runtime_instances.push(runtime);
                if let Err(error) = self.persist() {
                    self.runtime_instances = previous.clone();
                    self.gateways = previous_gateways.clone();
                    return Err(error);
                }
            }
            RuntimeChange::Delete(id) => {
                self.runtime_instances.retain(|item| item.id != id);
                if let Err(error) = self.persist() {
                    self.runtime_instances = previous.clone();
                    self.gateways = previous_gateways.clone();
                    return Err(error);
                }
                self.discard_runtime_bindings(&id);
                self.discard_runtime_choices(&id);
            }
        }
        // Each open session checks whether the edit reaches its source or execution.
        let mut execution_invalidated = false;
        self.for_each_session(|core| {
            let source_ids: Vec<String> = if let Some(projection) = &core.session.logical_projection
            {
                projection
                    .link
                    .segments
                    .iter()
                    .map(|s| s.runtime_instance_id.clone())
                    .collect()
            } else {
                core.native_snapshot()
                    .map(|view| vec![view.conversation.runtime_id])
                    .unwrap_or_default()
            };
            let changed = |id: &String| {
                previous.iter().find(|runtime| &runtime.id == id)
                    != core
                        .runtime_instances
                        .iter()
                        .find(|runtime| &runtime.id == id)
            };
            let source_changed = source_ids.iter().any(|id| {
                let origin = |runtime: &RuntimeInstance| {
                    (
                        runtime.type_id.clone(),
                        runtime.enabled,
                        runtime.settings.clone(),
                    )
                };
                previous
                    .iter()
                    .find(|runtime| &runtime.id == id)
                    .map(origin)
                    != core
                        .runtime_instances
                        .iter()
                        .find(|runtime| &runtime.id == id)
                        .map(origin)
            });
            let invalidated = source_changed
                || core
                    .session
                    .execution_runtime_id
                    .as_ref()
                    .is_some_and(changed);
            execution_invalidated |= invalidated;
            if invalidated {
                core.invalidate_execution()?;
                if source_changed {
                    // A native ID is scoped to the source configuration used to
                    // load it. Editing that source requires an explicit reopen.
                    core.session.active_state = ActiveState::Empty;
                    core.session.logical_projection = None;
                    core.session.model_record_key = None;
                }
            }
            Ok(())
        })?;
        if self.next_turn_runtime_id.as_ref().is_some_and(|id| {
            !self
                .runtime_instances
                .iter()
                .any(|runtime| &runtime.id == id && runtime.enabled)
        }) {
            self.next_turn_runtime_id = None;
        }
        self.runtime_quota.retain(
            &self
                .runtime_instances
                .iter()
                .filter_map(|runtime| {
                    crate::native_quota_source::Capture::from_runtime(runtime)
                        .ok()
                        .map(|capture| (runtime.id.clone(), capture))
                })
                .collect(),
        );
        Ok(crate::RuntimeUpdate {
            runtime_instances: self.public_runtime_instances(),
            runtime_types: runtime_types(),
            execution_invalidated,
        })
    }

    pub(super) fn persist(&self) -> Result<(), RuntimeError> {
        let config = crate::repository::PersistedConfig {
            schema_version: crate::repository::CURRENT_SCHEMA_VERSION,
            model_templates: self.model_templates.clone(),
            gateways: self.gateways.clone(),
            runtime_instances: self.runtime_instances.clone(),
            conversation_browser_group_limit: self
                .client_baseline
                .as_ref()
                .map_or(self.conversation_browser_limits.page, |view| {
                    view.limits.page
                }),
            conversation_browser_total_limit: self
                .client_baseline
                .as_ref()
                .map_or(self.conversation_browser_limits.total, |view| {
                    view.limits.total
                }),
            outline_presentation: self
                .client_baseline
                .as_ref()
                .map_or(self.outline_presentation, |view| view.outline),
            linked_conversation_opening: self
                .client_baseline
                .as_ref()
                .map_or(self.linked_conversation_opening, |view| view.linked_opening),
            usage_display: self
                .client_baseline
                .as_ref()
                .map_or(self.usage_display, |view| view.usage_display),
            context_usage_display: self
                .client_baseline
                .as_ref()
                .map_or(self.context_usage_display, |view| {
                    view.context_usage_display
                }),
            conversation_browser_preferences: self.client_baseline.as_ref().map_or_else(
                || self.conversation_browser_preferences.clone(),
                |view| view.preferences.clone(),
            ),
            status_line: self
                .client_baseline
                .as_ref()
                .map_or_else(|| self.status_line.clone(), |view| view.status.clone()),
        };
        self.repository.store(&config)?;
        if self.client_baseline.is_none() {
            *self.local_presentation_commit.borrow_mut() = Some(config.presentation_defaults());
        }
        Ok(())
    }
}
