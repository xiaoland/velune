//! Local application configuration use cases.
use super::*;
impl CoreRuntime {
    pub(super) fn set_conversation_browser_preferences(
        &mut self,
        request: &Value,
    ) -> Result<Value, RuntimeError> {
        let preferences: crate::config::ConversationBrowserPreferences =
            serde_json::from_value(request["payload"]["preferences"].clone())?;
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

    pub(super) fn set_outline_presentation(
        &mut self,
        request: &Value,
    ) -> Result<Value, RuntimeError> {
        let presentation = serde_json::from_value(request["payload"]["presentation"].clone())?;
        let previous = self.outline_presentation;
        self.outline_presentation = presentation;
        if let Err(error) = self.persist() {
            self.outline_presentation = previous;
            return Err(error);
        }
        Ok(json!(presentation))
    }

    pub(super) fn set_status_line(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        let items = crate::config::canonical_status_line(serde_json::from_value(
            request["payload"]["items"].clone(),
        )?);
        let previous = std::mem::replace(&mut self.status_line, items);
        if let Err(error) = self.persist() {
            self.status_line = previous;
            return Err(error);
        }
        Ok(json!(self.status_line))
    }

    pub(super) fn composer_drafts_action(
        &mut self,
        request: &Value,
    ) -> Result<Value, RuntimeError> {
        let payload = &request["payload"];
        match payload["operation"].as_str() {
            Some("list") => Ok(json!(
                self.composer_drafts
                    .iter()
                    .map(|(id, text)| crate::config::ComposerDraft {
                        conversation_id: id.clone(),
                        text: text.clone(),
                    })
                    .collect::<Vec<_>>()
            )),
            Some("set") => {
                let id = payload["conversationID"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
                let text = payload["text"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("composer draft text"))?;
                let mut next = self.composer_drafts.clone();
                if text.is_empty() {
                    next.remove(id);
                } else {
                    next.insert(id.to_owned(), text.to_owned());
                }
                if next != self.composer_drafts {
                    self.repository.store_composer_drafts(&next)?;
                    self.composer_drafts = next;
                }
                Ok(Value::Null)
            }
            _ => Err(RuntimeError::invalid("composer draft operation")),
        }
    }

    /// The conversation is already deleted; a failed write only leaves a stale
    /// entry on disk, which the next successful draft write drops.
    pub(super) fn discard_composer_draft(&mut self, conversation_id: &str) {
        if self.composer_drafts.remove(conversation_id).is_some() {
            let _ = self.repository.store_composer_drafts(&self.composer_drafts);
        }
    }

    pub(super) fn set_conversation_browser_limits(
        &mut self,
        request: &Value,
    ) -> Result<Value, RuntimeError> {
        let limits: crate::api::ConversationBrowserLimits =
            serde_json::from_value(request["payload"]["limits"].clone())?;
        if limits.page == 0 || limits.total == 0 {
            return Err(RuntimeError::invalid("会话加载数量必须大于零"));
        }
        let previous = self.conversation_browser_limits;
        self.conversation_browser_limits = limits;
        if let Err(error) = self.persist() {
            self.conversation_browser_limits = previous;
            return Err(error);
        }
        Ok(json!(limits))
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

    pub(super) fn list(&self) -> Result<Value, RuntimeError> {
        let (sources, history_failures) = self.list_sources(false)?;
        let page = self.browse_read(sources, None);
        Ok(json!({
            "conversations": page.conversations,
            "conversationGroups": page.groups,
            "conversationProjects": page.projects,
            "showsArchived": self.shows_archived(),
            "historyFailures": history_failures,
            "gateways": self.public_gateways(),
            "runtimeInstances": self.runtime_instances,
            "runtimeTypes": runtime_types(),
            "modelTemplates":self.model_templates,
            "conversationBrowserLimits":self.conversation_browser_limits,
            "outlinePresentation":self.outline_presentation,
            "statusLine":self.status_line,
            "conversationBrowserPreferences":self.conversation_browser_preferences,
            "providerImportTypes": [provider_import::descriptor()],
            "protocols": [
                {"id":"chatCompletionsV1","name":"OpenAI Chat Completions v1","supported":true},
                {"id":"responsesV1","name":"OpenAI Responses v1","supported":true},
                {"id":"messagesV1","name":"Anthropic Messages","supported":true}
            ],
            "selectedRuntimeInstanceID": self.next_turn_runtime_id,
        }))
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
            failover: crate::config::FailoverPolicy {
                mode: crate::config::FailoverMode::Disabled,
            },
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

    pub(super) fn runtime_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if self.execution_active() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        self.acp_capability_cache.borrow_mut().clear();
        let previous = self.runtime_instances.clone();
        let previous_gateways = self.gateways.clone();
        match request["payload"]["operation"].as_str().unwrap_or("list") {
            "upsert" => {
                let raw = request["payload"]["runtimeInstance"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("runtime instance"))?;
                let runtime: RuntimeInstance = serde_json::from_str(raw)?;
                if runtime.id.is_empty()
                    || runtime.name.is_empty()
                    || !matches!(
                        runtime.type_id.as_str(),
                        "pi-1.0.2"
                            | "codex-0.159.3"
                            | "dsh-acp-0.2.0-rc.2"
                            | "claude-code-2.1.286"
                            | "acp-v1"
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
                        failover: crate::config::FailoverPolicy {
                            mode: crate::config::FailoverMode::Disabled,
                        },
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
            "delete" => {
                let id = request["payload"]["runtimeInstanceID"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
                self.runtime_instances.retain(|item| item.id != id);
                if let Err(error) = self.persist() {
                    self.runtime_instances = previous.clone();
                    self.gateways = previous_gateways.clone();
                    return Err(error);
                }
                self.discard_runtime_bindings(id);
            }
            "list" => {}
            _ => return Err(RuntimeError::invalid("runtime operation")),
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
        Ok(
            json!({"runtimeInstances":self.runtime_instances,"runtimeTypes":runtime_types(),"executionInvalidated":execution_invalidated}),
        )
    }

    pub(super) fn persist(&self) -> Result<(), RuntimeError> {
        self.repository.store(&crate::repository::PersistedConfig {
            schema_version: 8,
            model_templates: self.model_templates.clone(),
            gateways: self.gateways.clone(),
            runtime_instances: self.runtime_instances.clone(),
            conversation_browser_group_limit: self.conversation_browser_limits.page,
            conversation_browser_total_limit: self.conversation_browser_limits.total,
            outline_presentation: self.outline_presentation,
            conversation_browser_preferences: self.conversation_browser_preferences.clone(),
            status_line: self.status_line.clone(),
        })
    }
}
