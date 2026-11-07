//! Local application configuration use cases.
use super::*;
use crate::RuntimeExecution;
fn empty_default_gateway() -> GatewayConfig {
    GatewayConfig {
        id: "default".into(),
        name: "默认网关".into(),
        providers: Vec::new(),
        routes: Vec::new(),
    }
}

#[cfg(test)]
mod runtime_start_tests {
    use super::*;
    use crate::config::{NewConversationChoice, RuntimeComposerChoices, TurnChoice};

    #[test]
    fn runtime_policy_is_one_configuration_commit_and_keeps_old_type_choices() {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-runtime-policy-{:x}",
            u64::from_le_bytes(nonce)
        ));
        fs::create_dir(&root).unwrap();
        let mut application = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        let core = &mut application.runtime;
        let mut runtime = RuntimeInstance {
            enabled: false,
            id: "synthetic".into(),
            name: "Synthetic".into(),
            type_id: RuntimeKind::Acp,
            gateway_id: "default".into(),
            settings: BTreeMap::new(),
        };
        core.upsert_acp_runtime(
            runtime.clone(),
            RuntimeExecution::default(),
            crate::AcpLaunch::default(),
        )
        .unwrap();
        core.set_runtime_composer_choices(RuntimeComposerChoices {
            runtime_id: runtime.id.clone(),
            runtime_type: RuntimeKind::Acp,
            last_chosen: TurnChoice {
                model: Some(crate::ModelSelection::Native {
                    provider_id: None,
                    model_id: "synthetic-model".into(),
                }),
                ..TurnChoice::default()
            },
        })
        .unwrap();
        let composer = fs::read(root.join("composer-choices.json")).unwrap();
        fs::create_dir(root.join("composer-choices.json.tmp")).unwrap();
        runtime.type_id = RuntimeKind::ClaudeCode;
        let policy = NewConversationChoice::Chosen {
            choice: TurnChoice {
                permission_mode: Some("manual".into()),
                ..TurnChoice::default()
            },
        };
        runtime.set_new_conversation_choice(policy.clone()).unwrap();
        core.upsert_runtime_with_execution(runtime.clone(), RuntimeExecution::default())
            .unwrap();
        assert_eq!(
            fs::read(root.join("composer-choices.json")).unwrap(),
            composer
        );
        assert_eq!(
            core.runtime_start_choice(runtime.id.clone())
                .unwrap()
                .permission_mode
                .as_deref(),
            Some("manual")
        );
        let committed = fs::read(root.join("generic-config.json")).unwrap();
        fs::create_dir(root.join("generic-config.json.tmp")).unwrap();
        runtime.type_id = RuntimeKind::Codex166;
        runtime
            .set_new_conversation_choice(NewConversationChoice::LastChosen)
            .unwrap();
        assert!(
            core.upsert_runtime_with_execution(runtime.clone(), RuntimeExecution::default())
                .is_err()
        );
        assert_eq!(
            core.runtime_instance(&runtime.id).unwrap().type_id,
            RuntimeKind::ClaudeCode
        );
        assert_eq!(
            core.runtime_instance(&runtime.id)
                .unwrap()
                .new_conversation_choice()
                .unwrap(),
            policy
        );
        assert_eq!(
            fs::read(root.join("generic-config.json")).unwrap(),
            committed
        );
        fs::remove_dir(root.join("generic-config.json.tmp")).unwrap();
        let mut invalid = core.runtime_instance(&runtime.id).unwrap().clone();
        invalid
            .set_new_conversation_choice(NewConversationChoice::Chosen {
                choice: TurnChoice {
                    permission_mode: Some("wrong-type".into()),
                    ..TurnChoice::default()
                },
            })
            .unwrap();
        assert!(core.upsert_runtime(invalid).is_err());
        assert_eq!(
            fs::read(root.join("generic-config.json")).unwrap(),
            committed
        );
        drop(application);
        let reopened = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        let saved = reopened.runtime.runtime_instance(&runtime.id).unwrap();
        assert_eq!(saved.type_id, RuntimeKind::ClaudeCode);
        assert_eq!(saved.new_conversation_choice().unwrap(), policy);
        assert_eq!(
            reopened.runtime.composer_choices.runtimes[0].runtime_type,
            RuntimeKind::Acp
        );
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
}

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

    pub(crate) fn set_conversation_browser_limits(
        &mut self,
        limits: crate::ConversationBrowserLimits,
    ) -> Result<crate::ConversationBrowserLimits, RuntimeError> {
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
    ) -> Result<(Vec<organization::ListedSource>, Vec<crate::HistoryFailure>), RuntimeError> {
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
                history_failures.push(crate::HistoryFailure {
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
            BTreeMap::new()
        };
        *self.history_failures.borrow_mut() = history_failures.clone();
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
                    browsing::EntryState::Read(summary) => conversations.push(*summary),
                    browsing::EntryState::Unread => {
                        provisional.insert(entry.id.clone());
                        conversations.push(ConversationSummary {
                            native_read_state: None,
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
            if (session.active_state.live().is_some() || session.pending_native_fork.is_some())
                && session.side.is_none()
                && let Some(view) = session.projected()
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
        sources.extend(
            self.archived_listing
                .borrow()
                .values()
                .filter(|archive| archive.source.is_current(self))
                .flat_map(|archive| {
                    archive
                        .summaries
                        .iter()
                        .cloned()
                        .map(|summary| organization::ListedSource {
                            summary,
                            archived_natively: true,
                            provisional: false,
                        })
                }),
        );
        sources
    }

    pub(crate) fn list(&self) -> Result<crate::ConfigurationSnapshot, RuntimeError> {
        let (sources, history_failures) = self.list_sources(false)?;
        Ok(self.configuration_from_sources(sources, history_failures))
    }

    /// Reapply list organization to the indexed history. A read receipt or
    /// pin changes no native files and must not wait for unrelated runtimes.
    pub(super) fn cached_list(&self) -> crate::ConfigurationSnapshot {
        self.configuration_from_sources(
            self.cached_sources(),
            self.history_failures.borrow().clone(),
        )
    }

    fn configuration_from_sources(
        &self,
        sources: Vec<organization::ListedSource>,
        history_failures: Vec<crate::HistoryFailure>,
    ) -> crate::ConfigurationSnapshot {
        let page = self.browse_read(sources, None);
        let runtime_types = runtime_types();
        let provider_import_types = std::iter::once(provider_import::descriptor())
            .chain(
                runtime_types
                    .iter()
                    .filter(|kind| crate::native_provider_import::TYPES.contains(&kind.id))
                    .cloned(),
            )
            .collect();
        let protocol = |id: crate::config::Protocol, name: &str| crate::ProtocolDescriptor {
            id,
            name: name.into(),
            supported: true,
        };
        crate::ConfigurationSnapshot {
            conversations: page.conversations,
            conversation_groups: page.groups,
            conversation_projects: page.projects,
            working_directory_projects: page.working_directory_projects,
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
                protocol(
                    crate::config::Protocol::ChatCompletions,
                    "OpenAI Chat Completions v1",
                ),
                protocol(crate::config::Protocol::Responses, "OpenAI Responses v1"),
                protocol(crate::config::Protocol::Messages, "Anthropic Messages"),
            ],
            selected_runtime_instance_id: self.next_turn_runtime_id.clone(),
        }
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

    pub(crate) fn preview_provider_import(
        &self,
        gateway_id: String,
        runtime_id: String,
    ) -> Result<crate::ProviderImportPreview, RuntimeError> {
        let empty = empty_default_gateway();
        let gateway = match self.gateways.iter().find(|item| item.id == gateway_id) {
            Some(gateway) => gateway,
            None if gateway_id == "default" => &empty,
            None => return Err(RuntimeError::invalid("gateway id")),
        };
        provider_import::preview(&runtime_id, gateway, &self.options, &self.runtime_instances)
    }

    pub(crate) fn apply_provider_import(
        &mut self,
        gateway_id: String,
        runtime_id: String,
        preview_token: String,
        selections: Vec<crate::ImportSelection>,
        replace_existing: bool,
    ) -> Result<crate::ImportResult, RuntimeError> {
        let gateway_id = gateway_id.as_str();
        let existing = self.gateways.iter().position(|item| item.id == gateway_id);
        if existing.is_none() && gateway_id != "default" {
            return Err(RuntimeError::invalid("gateway id"));
        }
        let mut imported_gateway = existing
            .map(|index| self.gateways[index].clone())
            .unwrap_or_else(empty_default_gateway);
        let previous = imported_gateway.clone();
        let result = provider_import::apply(
            &runtime_id,
            &preview_token,
            &selections,
            replace_existing,
            &mut imported_gateway,
            &self.options,
            &self.runtime_instances,
        )?;
        if imported_gateway.providers.is_empty() {
            return Err(RuntimeError::invalid(
                "provider import selected no providers",
            ));
        }
        let gateway_changed = imported_gateway != previous;
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
        let execution_invalidated =
            gateway_changed && self.invalidate_gateway_execution(gateway_id)?;
        Ok(crate::ImportResult {
            imported_provider_ids: result.imported,
            skipped_provider_ids: result.skipped,
            gateways: self.public_gateways(),
            execution_invalidated,
        })
    }

    pub(crate) fn upsert_runtime(
        &mut self,
        mut runtime: RuntimeInstance,
    ) -> Result<crate::RuntimeUpdate, RuntimeError> {
        self.retain_private_runtime_settings(&mut runtime);
        // Plain upsert callers do not edit the start policy. A changed type has its own default.
        if !runtime
            .settings
            .contains_key(velune_application_api::NEW_CONVERSATION_SETTING)
            && let Some(choice) = self
                .runtime_instances
                .iter()
                .find(|stored| stored.id == runtime.id && stored.type_id == runtime.type_id)
                .and_then(|stored| {
                    stored
                        .settings
                        .get(velune_application_api::NEW_CONVERSATION_SETTING)
                })
        {
            runtime.settings.insert(
                velune_application_api::NEW_CONVERSATION_SETTING.into(),
                choice.clone(),
            );
        }
        if let crate::config::NewConversationChoice::Chosen { choice } =
            runtime.new_conversation_choice()?
        {
            self.validate_model_selection(&runtime, choice.model.as_ref())?;
            if let Some(effort) = choice.effort.as_deref() {
                self.validate_effort(&runtime, choice.model.as_ref(), effort)?;
            }
            if let Some(mode) = choice.permission_mode.as_deref() {
                self.validate_permission_mode(&runtime, mode)?;
            }
        }
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
        match &change {
            RuntimeChange::Delete(id) if self.runtime_in_use(id) => {
                return Err(RuntimeError::invalid(
                    "此运行时仍有会话或后台任务正在运行，请停止后再删除",
                ));
            }
            _ => {}
        }
        self.acp_capability_cache.borrow_mut().clear();
        let previous = self.runtime_instances.clone();
        let previous_gateways = self.gateways.clone();
        match change {
            RuntimeChange::Upsert(runtime) => {
                if runtime.id.is_empty()
                    || runtime.name.is_empty()
                    || (runtime.gateway_id != "default"
                        && !self
                            .gateways
                            .iter()
                            .any(|item| item.id == runtime.gateway_id))
                {
                    return Err(RuntimeError::invalid("runtime instance"));
                }
                if runtime
                    .settings
                    .get("automaticNaming")
                    .is_some_and(|v| !matches!(v.as_str(), "true" | "false"))
                {
                    return Err(RuntimeError::invalid("自动命名开关无效"));
                }
                if runtime.enabled
                    && runtime
                        .settings
                        .get("automaticNaming")
                        .is_some_and(|v| v == "true")
                {
                    let key = runtime
                        .settings
                        .get("automaticNamingModel")
                        .filter(|v| !v.is_empty())
                        .ok_or_else(|| RuntimeError::invalid("请选择自动命名使用的模型"))?;
                    self.gateways
                        .iter()
                        .find(|g| g.id == runtime.gateway_id)
                        .ok_or_else(|| RuntimeError::invalid("命名模型的网关不存在"))?
                        .resolve_dispatch(key)
                        .map_err(RuntimeError::invalid)?;
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
        // Native projections are source-scoped, including fallbacks kept outside the main index.
        for old in &previous {
            let same = self
                .runtime_instances
                .iter()
                .find(|runtime| runtime.id == old.id)
                .is_some_and(|runtime| runtime.enabled && self.same_runtime_source(old, runtime));
            if !same {
                self.discard_runtime_source_caches(&old.id);
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
            let changed = |id: &String, execution: bool| match (
                previous.iter().find(|runtime| &runtime.id == id),
                core.runtime_instances
                    .iter()
                    .find(|runtime| &runtime.id == id),
            ) {
                (Some(before), Some(after)) if execution => {
                    before.execution_configuration() != after.execution_configuration()
                }
                (Some(before), Some(after)) => {
                    before.enabled != after.enabled || !core.same_runtime_source(before, after)
                }
                (None, None) => false,
                _ => true,
            };
            let source_changed = source_ids.iter().any(|id| changed(id, false));
            let invalidated = source_changed
                || core
                    .session
                    .execution_runtime_id
                    .as_ref()
                    .is_some_and(|id| changed(id, true));
            if invalidated && core.session.executing() {
                core.session.configuration_changed = true;
                return Ok(());
            }
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
