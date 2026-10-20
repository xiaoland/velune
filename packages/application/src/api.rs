use crate::{Application, Error, config::*};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub use velune_application_api::records::*;
impl From<&GatewayConfig> for GatewaySummary {
    fn from(g: &GatewayConfig) -> Self {
        Self {
            id: g.id.clone(),
            name: g.name.clone(),
            providers: g
                .providers
                .iter()
                .map(|p| ProviderSummary {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    protocol: p.protocol.clone(),
                    endpoint: p.endpoint.clone(),
                    execution_backend: p.execution_backend.clone(),
                    request_profile: p.request_profile,
                    authentication_source: p.authentication.source(),
                    quota_source: p.quota_source,
                    authentication: {
                        let mut description = p.authentication.description();
                        if p.request_profile == crate::config::RequestProfile::KimiCoding {
                            description.disclosure =
                                Some(crate::SubscriptionDisclosure::kimi_coding_plan());
                        }
                        description
                    },
                    quota_limits: p.quota_limits.clone(),
                    models: p.models.iter().map(Into::into).collect(),
                })
                .collect(),
            routes: g.routes.clone(),
        }
    }
}
impl Application {
    /// List native source resources without launching the Harness or creating resource roots.
    /// Project scope requires a cwd on the execution machine. Disabled instances remain manageable.
    pub fn runtime_resource_catalog(
        &mut self,
        runtime_id: String,
        cwd: Option<String>,
        kind: crate::RuntimeResourceKind,
        scope: crate::RuntimeResourceScope,
    ) -> Result<crate::RuntimeResourceCatalog, Error> {
        self.execute(
            "runtimeResourceCatalog",
            json!({"cwd":cwd,"kind":kind,"scope":scope}),
            Some(&runtime_id),
        )
    }

    /// Read one native file or MCP server entry. The returned bytes may contain private settings.
    pub fn read_runtime_resource(
        &mut self,
        runtime_id: String,
        cwd: Option<String>,
        key: crate::RuntimeResourceKey,
    ) -> Result<crate::RuntimeResourceDocument, Error> {
        self.execute(
            "readRuntimeResource",
            json!({"cwd":cwd,"key":key}),
            Some(&runtime_id),
        )
    }

    /// Save one native resource against its last observed revision. This does not reload or close
    /// an active session; when the Harness notices the change is owned by that Harness.
    pub fn write_runtime_resource(
        &mut self,
        runtime_id: String,
        cwd: Option<String>,
        key: crate::RuntimeResourceKey,
        expected: crate::RuntimeResourceRevision,
        bytes: Vec<u8>,
        executable: Option<bool>,
    ) -> Result<crate::RuntimeResourceDocument, Error> {
        self.execute(
            "writeRuntimeResource",
            json!({"cwd":cwd,"key":key,"expected":expected,"bytes":bytes,"executable":executable}),
            Some(&runtime_id),
        )
    }

    /// Delete one native resource, or an empty directory. Never removes a complete skill tree.
    pub fn delete_runtime_resource(
        &mut self,
        runtime_id: String,
        cwd: Option<String>,
        key: crate::RuntimeResourceKey,
        expected: crate::RuntimeResourceRevision,
    ) -> Result<(), Error> {
        self.execute(
            "deleteRuntimeResource",
            json!({"cwd":cwd,"key":key,"expected":expected}),
            Some(&runtime_id),
        )
    }

    /// Resource library summaries; no content, so no secrets.
    pub fn list_resource_templates(
        &mut self,
    ) -> Result<Vec<crate::ResourceTemplateSummary>, Error> {
        self.execute("resourceTemplates", json!({"operation":"list"}), None)
    }

    /// One resource with its full content, including any secret values.
    pub fn read_resource_template(
        &mut self,
        template_id: String,
    ) -> Result<crate::ResourceTemplate, Error> {
        self.execute(
            "resourceTemplates",
            json!({"operation":"read","templateID":template_id}),
            None,
        )
    }

    /// Create (empty id) or replace a resource, then write it to the runtimes it is enabled on
    /// and in sync with. Runtimes changed outside Velune, or all of them while a turn runs, stay
    /// as they are and report their state through `resource_sync_status`.
    pub fn save_resource_template(
        &mut self,
        template: crate::ResourceTemplate,
    ) -> Result<crate::ResourceSaveResult, Error> {
        self.execute(
            "resourceTemplates",
            json!({"operation":"save","template":template}),
            None,
        )
    }

    /// Delete a resource. Runtimes keep the native entries it wrote, now unmanaged.
    pub fn delete_resource_template(&mut self, template_id: String) -> Result<(), Error> {
        self.execute(
            "resourceTemplates",
            json!({"operation":"delete","templateID":template_id}),
            None,
        )
    }

    /// Read one native entry back into an unsaved resource, with what could not be carried over.
    /// `path` is the entry's key path: skill directory, MCP server name, or agent/instruction file.
    pub fn import_runtime_resource(
        &mut self,
        runtime_id: String,
        kind: crate::RuntimeResourceKind,
        path: String,
    ) -> Result<crate::ResourceTemplateImport, Error> {
        self.execute(
            "resourceTemplates",
            json!({"operation":"import","kind":kind,"path":path}),
            Some(&runtime_id),
        )
    }

    /// Add a native entry to the library and keep it enabled on its runtime; the native entry is
    /// rewritten in the library's form.
    pub fn adopt_runtime_resource(
        &mut self,
        runtime_id: String,
        kind: crate::RuntimeResourceKind,
        path: String,
    ) -> Result<crate::ResourceTemplateSummary, Error> {
        self.execute(
            "resourceSync",
            json!({"operation":"adopt","kind":kind,"path":path}),
            Some(&runtime_id),
        )
    }

    /// Every resource on every runtime instance.
    pub fn resource_sync_status(&mut self) -> Result<Vec<crate::ResourceSyncStatus>, Error> {
        self.execute("resourceSync", json!({"operation":"status"}), None)
    }

    /// How the native entries of an enabled resource differ from the library's version. The texts
    /// may contain MCP secrets, like an explicit read.
    pub fn resource_drift(
        &mut self,
        runtime_id: String,
        template_id: String,
    ) -> Result<Vec<crate::ResourceDriftFile>, Error> {
        self.execute(
            "resourceSync",
            json!({"operation":"drift","templateID":template_id}),
            Some(&runtime_id),
        )
    }

    /// Native entries of one kind that no resource manages, on every runtime instance.
    pub fn unmanaged_runtime_resources(
        &mut self,
        kind: crate::RuntimeResourceKind,
    ) -> Result<Vec<crate::UnmanagedRuntimeResource>, Error> {
        self.execute(
            "resourceSync",
            json!({"operation":"unmanaged","kind":kind}),
            None,
        )
    }

    /// Enable a resource on, or re-sync it to, each target. A conflicting native entry stops the
    /// target unless `overwrite` is set.
    pub fn apply_resource_template(
        &mut self,
        template_id: String,
        targets: Vec<crate::TemplateApplyTarget>,
    ) -> Result<Vec<crate::TemplateApplyReport>, Error> {
        self.execute(
            "resourceSync",
            json!({"operation":"apply","templateID":template_id,"targets":targets}),
            None,
        )
    }

    /// What a resource would become if taken from its native entries on a runtime, with the
    /// native fields it has no place for; nothing is saved.
    pub fn resource_native_import(
        &mut self,
        runtime_id: String,
        template_id: String,
    ) -> Result<crate::ResourceTemplateImport, Error> {
        self.execute(
            "resourceSync",
            json!({"operation":"nativeImport","templateID":template_id}),
            Some(&runtime_id),
        )
    }

    /// Overwrite drifted entries, stop syncing (keeping native content), remove them, or take
    /// the native content into the resource.
    pub fn resolve_resource_binding(
        &mut self,
        runtime_id: String,
        template_id: String,
        resolution: crate::BindingResolution,
    ) -> Result<(), Error> {
        self.execute(
            "resourceSync",
            json!({"operation":"resolve","templateID":template_id,"resolution":resolution}),
            Some(&runtime_id),
        )
    }

    pub fn analytics_query(&self, query: AnalyticsQuery) -> Result<AnalyticsReport, Error> {
        if query.from_ms >= query.to_ms
            || query.bucket_boundaries_ms.len() < 2
            || query.bucket_boundaries_ms.len() > 370
            || query.bucket_boundaries_ms.first() != Some(&query.from_ms)
            || query.bucket_boundaries_ms.last() != Some(&query.to_ms)
            || query
                .bucket_boundaries_ms
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || query.request_limit == 0
        {
            return Err(Error::invalid("分析查询范围无效"));
        }
        let mut overview = AnalyticsAccumulator::default();
        let mut trend = query
            .bucket_boundaries_ms
            .windows(2)
            .map(|bounds| (bounds[0], bounds[1], AnalyticsAccumulator::default()))
            .collect::<Vec<_>>();
        let mut providers = BTreeMap::<String, AnalyticsGroup>::new();
        let mut models = BTreeMap::<(String, String), AnalyticsGroup>::new();
        let mut requests = Vec::new();
        self.analytics.visit(
            query.from_ms,
            query.to_ms,
            query.provider_id.as_deref(),
            query.model_record_key.as_deref(),
            |row| {
                overview.add(&row)?;
                let index = query
                    .bucket_boundaries_ms
                    .partition_point(|boundary| *boundary <= row.terminal_at_ms)
                    .saturating_sub(1);
                trend[index].2.add(&row)?;
                providers
                    .entry(row.provider_id.clone())
                    .or_insert_with(|| AnalyticsGroup::provider(&row))
                    .totals
                    .add(&row)?;
                models
                    .entry((row.provider_id.clone(), row.model_record_key.clone()))
                    .or_insert_with(|| AnalyticsGroup::model(&row))
                    .totals
                    .add(&row)?;
                if requests.len() < query.request_limit.min(500) as usize {
                    requests.push(analytics_request(row)?);
                }
                Ok(())
            },
        )?;
        Ok(AnalyticsReport {
            overview: overview.finish(),
            trend: trend
                .into_iter()
                .map(|(from_ms, to_ms, total)| AnalyticsBucket {
                    from_ms,
                    to_ms,
                    totals: total.finish(),
                })
                .collect(),
            providers: providers
                .into_values()
                .map(AnalyticsGroup::finish)
                .collect(),
            models: models.into_values().map(AnalyticsGroup::finish).collect(),
            requests,
            coverage: "gateway_upstream_observation".into(),
            storage_warning: self.analytics.warning(),
            dropped_count: self.analytics.dropped(),
        })
    }
    pub fn runtime_discovery_hints(
        &self,
        user_home: String,
        overrides: BTreeMap<String, String>,
    ) -> Result<Vec<RuntimeDiscoveryHint>, Error> {
        #[cfg(feature = "local-runtime")]
        {
            crate::runtime_discovery::hints(&user_home, overrides)
        }
        #[cfg(not(feature = "local-runtime"))]
        {
            let _ = (user_home, overrides);
            Err(Error::Unsupported("此平台不提供本地运行时发现".into()))
        }
    }
    pub fn discover_runtimes(
        &mut self,
        probes: Vec<RuntimeDiscoveryProbe>,
    ) -> Result<Vec<RuntimeDiscoveryCandidate>, Error> {
        self.execute("discoverRuntimes", json!({"probes":probes}), None)
    }
    fn execute<T: DeserializeOwned>(
        &mut self,
        action: &str,
        mut payload: Value,
        runtime_id: Option<&str>,
    ) -> Result<T, Error> {
        if let Some(runtime_id) = runtime_id {
            payload["runtimeInstanceID"] = json!(runtime_id);
        }
        let request =
            json!({"version":3,"action":action,"runtimeInstanceID":runtime_id,"payload":payload});
        let mut value = self.runtime.request_inner(&request)?;
        // ACP launch arguments and environment are read only through the explicit editor API.
        if let Some(runtimes) = value
            .get_mut("runtimeInstances")
            .and_then(Value::as_array_mut)
        {
            for runtime in runtimes {
                if let Some(settings) = runtime.get_mut("settings").and_then(Value::as_object_mut) {
                    settings.remove("acpLaunch");
                }
            }
        }
        if action == "discoverRuntimes"
            && let Some(candidates) = value.as_array_mut()
        {
            for candidate in candidates {
                if let Some(settings) = candidate
                    .get_mut("runtime")
                    .and_then(|runtime| runtime.get_mut("settings"))
                    .and_then(Value::as_object_mut)
                {
                    settings.remove("acpLaunch");
                }
            }
        }

        serde_json::from_value(value).map_err(Error::from)
    }
    pub fn list(&mut self) -> Result<ConfigurationSnapshot, Error> {
        self.execute("list", json!({}), None)
    }
    pub fn set_outline_presentation(
        &mut self,
        presentation: OutlinePresentation,
    ) -> Result<OutlinePresentation, Error> {
        self.execute(
            "outlinePresentation",
            json!({"presentation": presentation}),
            None,
        )
    }
    pub fn set_linked_conversation_opening(
        &mut self,
        opening: LinkedConversationOpening,
    ) -> Result<LinkedConversationOpening, Error> {
        self.execute(
            "linkedConversationOpening",
            json!({"opening": opening}),
            None,
        )
    }
    pub fn set_usage_display(&mut self, display: UsageDisplay) -> Result<UsageDisplay, Error> {
        self.execute("usageDisplay", json!({"display": display}), None)
    }
    /// Stores what the composer's status line shows, returning it in display order.
    pub fn set_status_line(
        &mut self,
        items: Vec<StatusLineItem>,
    ) -> Result<Vec<StatusLineItem>, Error> {
        self.execute("statusLine", json!({"items": items}), None)
    }
    pub fn set_conversation_browser_preferences(
        &mut self,
        preferences: ConversationBrowserPreferences,
    ) -> Result<ConfigurationSnapshot, Error> {
        self.execute(
            "conversationBrowserPreferences",
            json!({"preferences":preferences}),
            None,
        )
    }
    pub fn composer_choices(&mut self) -> Result<crate::config::ComposerChoices, Error> {
        self.execute("composerChoices", json!({"operation":"list"}), None)
    }
    /// An intent without choices or runtime removes the conversation's.
    pub fn set_next_turn_intent(
        &mut self,
        intent: crate::config::NextTurnIntent,
    ) -> Result<(), Error> {
        self.execute(
            "composerChoices",
            json!({"operation":"setIntent","intent":intent}),
            None,
        )
    }
    pub fn set_runtime_composer_choices(
        &mut self,
        choices: crate::config::RuntimeComposerChoices,
    ) -> Result<(), Error> {
        self.execute(
            "composerChoices",
            json!({"operation":"setRuntime","choices":choices}),
            None,
        )
    }
    pub fn composer_drafts(&mut self) -> Result<Vec<crate::config::ComposerDraft>, Error> {
        self.execute("composerDrafts", json!({"operation":"list"}), None)
    }
    /// Empty text removes the conversation's draft.
    pub fn set_composer_draft(
        &mut self,
        conversation_id: String,
        text: String,
    ) -> Result<(), Error> {
        self.execute(
            "composerDrafts",
            json!({"operation":"set","conversationID":conversation_id,"text":text}),
            None,
        )
    }
    pub fn set_conversation_browser_limits(
        &mut self,
        limits: ConversationBrowserLimits,
    ) -> Result<ConversationBrowserLimits, Error> {
        self.execute(
            "conversationBrowserSettings",
            json!({"limits": limits}),
            None,
        )
    }
    /// Changes Velune's organization of the conversations; the runtime's own
    /// archive is used where it has one.
    pub fn organize_conversations(
        &mut self,
        conversations: Vec<ConversationReference>,
        change: ConversationOrganizationChange,
    ) -> Result<ConfigurationSnapshot, Error> {
        self.execute(
            "conversationOrganization",
            json!({"conversations":conversations,"change":change}),
            None,
        )
    }
    /// Shapes the project whose primary folder is `path`: its name (none
    /// shows the folder's), primary folder, and other folders. Moves, the
    /// project filter, and folded groups follow a new primary folder.
    pub fn set_project(
        &mut self,
        path: String,
        primary_folder: String,
        name: Option<String>,
        folders: Vec<String>,
    ) -> Result<ConfigurationSnapshot, Error> {
        self.execute(
            "setProject",
            json!({"path":path,"primaryFolder":primary_folder,"name":name,"folders":folders}),
            None,
        )
    }
    /// Loads the next page of one group of the list.
    pub fn load_more_conversations(
        &mut self,
        group: ConversationGroupKey,
    ) -> Result<ConfigurationSnapshot, Error> {
        self.execute(
            "conversationBrowserWindow",
            json!({"operation":"loadMore","group":group}),
            None,
        )
    }
    /// Shows archived conversations instead of the others, or back.
    pub fn show_archived_conversations(
        &mut self,
        archived: bool,
    ) -> Result<ConfigurationSnapshot, Error> {
        self.execute(
            "conversationBrowserWindow",
            json!({"operation":"showArchived","archived":archived}),
            None,
        )
    }
    /// Conversations whose title, project, or runtime contain `text`, within
    /// the list's filters and archive view.
    pub fn search_conversations(&mut self, text: String) -> Result<ConversationSearch, Error> {
        self.execute("searchConversations", json!({"text":text}), None)
    }
    /// One conversation by identity, whether or not the list shows it.
    pub fn find_conversation(
        &mut self,
        conversation_id: String,
    ) -> Result<Option<ListedConversation>, Error> {
        self.execute(
            "findConversation",
            json!({"conversationID":conversation_id}),
            None,
        )
    }
    pub fn upsert_runtime(&mut self, mut runtime: RuntimeInstance) -> Result<RuntimeUpdate, Error> {
        self.runtime.retain_private_runtime_settings(&mut runtime);
        self.execute(
            "runtimeInstances",
            json!({"operation":"upsert","runtimeInstance":serde_json::to_string(&runtime)?}),
            None,
        )
    }
    pub fn upsert_runtime_with_execution(
        &mut self,
        mut runtime: RuntimeInstance,
        execution: crate::RuntimeExecution,
    ) -> Result<RuntimeUpdate, Error> {
        runtime.set_execution(execution)?;
        self.upsert_runtime(runtime)
    }
    pub fn upsert_acp_runtime(
        &mut self,
        mut runtime: RuntimeInstance,
        execution: crate::RuntimeExecution,
        launch: crate::AcpLaunch,
    ) -> Result<RuntimeUpdate, Error> {
        runtime.set_acp_launch(launch)?;
        self.upsert_runtime_with_execution(runtime, execution)
    }
    pub fn acp_launch(&mut self, runtime_id: String) -> Result<crate::AcpLaunch, Error> {
        self.execute("acpLaunch", json!({}), Some(&runtime_id))
    }
    pub fn acp_runtime_info(&mut self, runtime_id: String) -> Result<crate::AcpRuntimeInfo, Error> {
        self.execute("acpRuntimeInfo", json!({}), Some(&runtime_id))
    }
    /// Load a known native ACP reference even when the Agent cannot list sessions.
    pub fn open_acp_conversation(
        &mut self,
        runtime_id: String,
        native_session_id: String,
        cwd: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "openAcpConversation",
            json!({"nativeSessionID":native_session_id,"cwd":cwd}),
            Some(&runtime_id),
        )
    }
    pub fn acp_session_config(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<Vec<crate::AcpConfigOption>, Error> {
        self.execute(
            "acpSessionConfig",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    pub fn set_acp_session_config(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        config_id: String,
        value: crate::AcpConfigValue,
    ) -> Result<Vec<crate::AcpConfigOption>, Error> {
        self.execute(
            "setAcpSessionConfig",
            json!({"conversationID":conversation_id,"configID":config_id,"value":value}),
            Some(&runtime_id),
        )
    }
    pub fn delete_runtime(&mut self, id: String) -> Result<RuntimeUpdate, Error> {
        self.execute(
            "runtimeInstances",
            json!({"operation":"delete","runtimeInstanceID":id}),
            None,
        )
    }
    pub fn select_runtime(&mut self, id: String) -> Result<ConfigurationSnapshot, Error> {
        self.execute("selectRuntime", json!({"runtimeInstanceID":id}), None)
    }
    pub fn runtime_execution(
        &mut self,
        runtime_id: String,
    ) -> Result<crate::RuntimeExecution, Error> {
        self.execute("runtimeExecution", json!({}), Some(&runtime_id))
    }
    /// The HOME a runtime is launched with on its execution machine; `None`
    /// when it inherits the user's. A managed HOME is not created by asking.
    pub fn runtime_home_directory(&mut self, runtime_id: String) -> Result<Option<String>, Error> {
        self.execute("runtimeHomeDirectory", json!({}), Some(&runtime_id))
    }
    pub fn configure_runtime_execution(
        &mut self,
        runtime_id: String,
        execution: crate::RuntimeExecution,
    ) -> Result<RuntimeUpdate, Error> {
        self.execute(
            "configureRuntimeExecution",
            json!({"execution":execution}),
            Some(&runtime_id),
        )
    }
    pub fn create_conversation(
        &mut self,
        runtime_id: String,
        cwd: Option<String>,
        model_record_key: String,
    ) -> Result<SnapshotResult, Error> {
        self.create_conversation_with_selection(
            runtime_id,
            cwd,
            Some(ModelSelection::Managed { model_record_key }),
        )
    }
    pub fn create_conversation_with_selection(
        &mut self,
        runtime_id: String,
        cwd: Option<String>,
        selection: Option<ModelSelection>,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "create",
            json!({"cwd":cwd,"modelSelection":selection}),
            Some(&runtime_id),
        )
    }
    pub fn runtime_models(&mut self, runtime_id: String) -> Result<Vec<RuntimeModel>, Error> {
        self.execute("runtimeModels", json!({}), Some(&runtime_id))
    }
    /// The next turn's explicit intent, retained after a managed turn releases its provider override.
    pub fn next_turn_model_selection(&mut self) -> Result<Option<ModelSelection>, Error> {
        self.execute("nextTurnModelSelection", json!({}), None)
    }
    pub fn open_conversation(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "open",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    /// Read one native conversation without changing the active execution or
    /// transcript projection.
    pub fn read_conversation(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "readConversation",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    pub fn rename_conversation(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        title: String,
    ) -> Result<ConfigurationSnapshot, Error> {
        self.execute(
            "renameConversation",
            json!({"conversationID":conversation_id,"title":title}),
            Some(&runtime_id),
        )
    }
    pub fn delete_conversation(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<ConfigurationSnapshot, Error> {
        self.execute(
            "deleteConversation",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    /// Query the current projection in its runtime context. An enabled known
    /// instance returns no snapshot when no conversation is loaded; a different
    /// instance cannot query another instance's loaded conversation. Use the
    /// snapshot's context_runtime_id, not its logical conversation's origin.
    pub fn snapshot(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "getSnapshot",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    /// Like `snapshot`, relative to a cursor from an earlier delivery of the
    /// same view: unchanged content is not transferred again.
    pub fn snapshot_update(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        since: crate::TranscriptCursor,
    ) -> Result<crate::SnapshotUpdate, Error> {
        self.execute(
            "getSnapshotUpdate",
            json!({"conversationID":conversation_id,"since":since}),
            Some(&runtime_id),
        )
    }
    /// Complete payloads of messages in a delivered view, by message identifier.
    pub fn message_payloads(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        message_ids: Vec<String>,
    ) -> Result<Vec<crate::DeliveredMessage>, Error> {
        self.execute(
            "messagePayloads",
            json!({"conversationID":conversation_id,"messageIDs":message_ids}),
            Some(&runtime_id),
        )
    }
    /// Accept this turn's source conversation and target runtime/model. A different instance creates a new
    /// native segment and carries quoted text context; it does not replay tools
    /// or resume a foreign native session. Association commits precede dispatch,
    /// and an ambiguous dispatch is never automatically retried.
    pub fn send_turn(
        &mut self,
        source_runtime_id: String,
        conversation_id: String,
        target_runtime_id: String,
        model_record_key: String,
        text: String,
    ) -> Result<SnapshotResult, Error> {
        self.send_turn_with_selection(
            source_runtime_id,
            conversation_id,
            target_runtime_id,
            Some(ModelSelection::Managed { model_record_key }),
            ComposedTurn {
                text,
                effort: None,
                permission_mode: None,
                attachments: Vec::new(),
            },
        )
    }
    pub fn send_turn_with_selection(
        &mut self,
        source_runtime_id: String,
        conversation_id: String,
        target_runtime_id: String,
        selection: Option<ModelSelection>,
        turn: ComposedTurn,
    ) -> Result<SnapshotResult, Error> {
        self.execute("sendTurn", json!({"sourceRuntimeID":source_runtime_id,"conversationID":conversation_id,"modelSelection":selection,"text":turn.text,"effort":turn.effort,"permissionMode":turn.permission_mode,"attachments":turn.attachments}), Some(&target_runtime_id))
    }
    /// Reasoning levels a turn may request for a managed model on a runtime,
    /// in the runtime's own vocabulary; empty when it offers no choice.
    pub fn reasoning_efforts(
        &mut self,
        runtime_id: String,
        model_record_key: String,
    ) -> Result<Vec<String>, Error> {
        self.execute(
            "reasoningEfforts",
            json!({"modelRecordKey":model_record_key}),
            Some(&runtime_id),
        )
    }
    /// Change the conversation's send queue. A queue that is added to or
    /// resumed while no turn runs sends its first message at once.
    pub fn send_queue(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        operation: crate::SendQueueOperation,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "sendQueue",
            json!({"conversationID":conversation_id,"operation":operation}),
            Some(&runtime_id),
        )
    }
    /// Deliver a message into the conversation's running turn through the
    /// runtime's own channel (`FollowUp::can_steer`); it does not start a
    /// turn, and the next-turn choices do not apply to it.
    pub fn steer_turn(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        turn: ComposedTurn,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "steerTurn",
            json!({"conversationID":conversation_id,"text":turn.text,"attachments":turn.attachments}),
            Some(&runtime_id),
        )
    }
    /// Send `turn` in place of the user message `message_id`
    /// (`FollowUp::editable`), continuing as `placement` says. A fork's
    /// result is the new conversation's snapshot.
    pub fn edit_message(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        message_id: String,
        placement: crate::conversation::EditPlacement,
        selection: Option<ModelSelection>,
        turn: ComposedTurn,
    ) -> Result<SnapshotResult, Error> {
        self.execute("editMessage", json!({"conversationID":conversation_id,"messageID":message_id,"placement":placement,"modelSelection":selection,"text":turn.text,"effort":turn.effort,"permissionMode":turn.permission_mode,"attachments":turn.attachments}), Some(&runtime_id))
    }
    pub fn cancel(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "cancel",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    /// Stop the native sub-agents the conversation started (`FollowUp::can_stop_sub_agents`);
    /// they may run after its turn has ended. A queue waiting for them goes on once they have.
    pub fn stop_sub_agents(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "stopSubAgents",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    /// Conversations executing or waiting for the user across open sessions,
    /// and turns that ended after `since` (0 for all retained); cheap enough
    /// to poll.
    pub fn conversation_activity(&mut self, since: u64) -> Result<ConversationActivity, Error> {
        self.execute("conversationActivity", json!({"since":since}), None)
    }
    /// Uncommitted changes in the conversation's working directory.
    pub fn working_changes(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<WorkingChanges, Error> {
        self.execute(
            "workingChanges",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    /// The branch checked out in the conversation's working directory.
    pub fn working_branch(
        &mut self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<Option<String>, Error> {
        self.execute(
            "workingBranch",
            json!({"conversationID":conversation_id}),
            Some(&runtime_id),
        )
    }
    /// The diff of one file listed by `working_changes`.
    pub fn file_diff(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        path: String,
    ) -> Result<FileDiff, Error> {
        self.execute(
            "fileDiff",
            json!({"conversationID":conversation_id,"path":path}),
            Some(&runtime_id),
        )
    }
    pub fn preview_provider_import(
        &mut self,
        gateway_id: String,
        source: ProviderImportSource,
    ) -> Result<ProviderImportPreview, Error> {
        #[derive(Deserialize)]
        struct Preview {
            preview: ProviderImportPreview,
        }
        let result: Preview = self.execute("providerImport", json!({"operation":"preview","gatewayID":gateway_id,"source":serde_json::to_string(&source)?}), None)?;
        Ok(result.preview)
    }
    pub fn apply_provider_import(
        &mut self,
        gateway_id: String,
        source: ProviderImportSource,
        preview_token: String,
        selections: Vec<ImportSelection>,
        replace_existing: bool,
    ) -> Result<ImportResult, Error> {
        self.execute("providerImport", json!({"operation":"apply","gatewayID":gateway_id,"source":serde_json::to_string(&source)?,"previewToken":preview_token,"selections":serde_json::to_string(&selections)?,"replaceExisting":replace_existing}), None)
    }
    /// Read the application-owned observation and request a sample when due; never waits for HTTP.
    pub fn provider_quota(
        &mut self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<ProviderQuotaStatus, Error> {
        self.execute(
            "providers",
            json!({"operation":"quota","gatewayID":gateway_id,"providerID":provider_id}),
            None,
        )
    }
    /// Request one bounded sample; concurrent requests are coalesced. Poll provider_quota for completion.
    pub fn refresh_provider_quota(
        &mut self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<ProviderQuotaStatus, Error> {
        self.execute(
            "providers",
            json!({"operation":"refreshQuota","gatewayID":gateway_id,"providerID":provider_id}),
            None,
        )
    }
    pub fn save_provider(
        &mut self,
        gateway_id: String,
        provider: ProviderDraft,
        authentication_edit: crate::AuthenticationEdit,
    ) -> Result<GatewayUpdate, Error> {
        self.execute("providers",json!({"operation":"save","gatewayID":gateway_id,"provider":provider,"authenticationEdit":authentication_edit}),None)
    }
    pub fn delete_provider(
        &mut self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<GatewayUpdate, Error> {
        self.execute(
            "providers",
            json!({"operation":"delete","gatewayID":gateway_id,"providerID":provider_id}),
            None,
        )
    }
    /// Save a model route; an empty record key creates one.
    pub fn save_model_route(
        &mut self,
        gateway_id: String,
        route: crate::config::ModelRoute,
    ) -> Result<GatewayUpdate, Error> {
        self.execute(
            "providers",
            json!({"operation":"saveRoute","gatewayID":gateway_id,"route":route}),
            None,
        )
    }
    pub fn delete_model_route(
        &mut self,
        gateway_id: String,
        route_record_key: String,
    ) -> Result<GatewayUpdate, Error> {
        self.execute(
            "providers",
            json!({"operation":"deleteRoute","gatewayID":gateway_id,"routeRecordKey":route_record_key}),
            None,
        )
    }
    pub fn read_provider_api_key(
        &mut self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<String, Error> {
        self.execute(
            "providers",
            json!({"operation":"readApiKey","gatewayID":gateway_id,"providerID":provider_id}),
            None,
        )
    }
    pub fn save_model_template(
        &mut self,
        template: ModelTemplate,
    ) -> Result<Vec<ModelTemplate>, Error> {
        self.execute(
            "modelTemplates",
            json!({"operation":"save","template":template}),
            None,
        )
    }
    pub fn delete_model_template(&mut self, id: String) -> Result<Vec<ModelTemplate>, Error> {
        self.execute(
            "modelTemplates",
            json!({"operation":"delete","templateID":id}),
            None,
        )
    }
    pub fn reply_runtime_interaction(
        &mut self,
        runtime_id: String,
        conversation_id: String,
        interaction_id: String,
        reply: velune_conversation::RuntimeInteractionReply,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "replyRuntimeInteraction",
            json!({"conversationID":conversation_id,"interactionID":interaction_id,"reply":reply}),
            Some(&runtime_id),
        )
    }
    pub fn authentication_inspect(
        &mut self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<AuthenticationMetadata, Error> {
        #[derive(Deserialize)]
        struct Inspection {
            metadata: AuthenticationMetadata,
        }
        let result: Inspection = self.execute(
            "authentication",
            json!({"operation":"inspect","gatewayID":gateway_id,"providerID":provider_id}),
            None,
        )?;
        Ok(result.metadata)
    }
    pub fn authentication_start(
        &mut self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<AuthenticationProgress, Error> {
        self.execute(
            "authentication",
            json!({"operation":"start","gatewayID":gateway_id,"providerID":provider_id}),
            None,
        )
    }
    pub fn authentication_logout(
        &mut self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<GatewayUpdate, Error> {
        self.execute(
            "authentication",
            json!({"operation":"logout","gatewayID":gateway_id,"providerID":provider_id}),
            None,
        )
    }
    pub fn authentication_poll(&mut self) -> Result<AuthenticationProgress, Error> {
        self.execute("authentication", json!({"operation":"poll"}), None)
    }
    pub fn authentication_reply(
        &mut self,
        prompt_id: String,
        value: String,
    ) -> Result<AuthenticationProgress, Error> {
        self.execute(
            "authentication",
            json!({"operation":"reply","id":prompt_id,"value":value}),
            None,
        )
    }
    pub fn authentication_cancel(&mut self) -> Result<AuthenticationProgress, Error> {
        self.execute("authentication", json!({"operation":"cancel"}), None)
    }
}

#[derive(Default)]
struct AnalyticsAccumulator {
    totals: AnalyticsTotals,
    eligible_output: u64,
    eligible_elapsed_ms: u64,
}
impl AnalyticsAccumulator {
    fn add(&mut self, row: &crate::analytics::AnalyticsRow) -> Result<(), Error> {
        let total = &mut self.totals;
        total.request_count += 1;
        match row.outcome.as_str() {
            "completed" => total.completed_count += 1,
            "cancelled" => total.cancelled_count += 1,
            "failed" => total.failed_count += 1,
            "incomplete" => total.incomplete_count += 1,
            "rejected" => total.rejected_count += 1,
            _ => return Err(Error::invalid("分析记录包含未知结果")),
        }
        total.usage_reported_count += u64::from(row.usage_reported);
        total.usage_complete_count += u64::from(row.usage_complete);
        sum_optional(&mut total.input_tokens, row.input_tokens)?;
        sum_optional(&mut total.output_tokens, row.output_tokens)?;
        sum_optional(
            &mut total.reasoning_output_tokens,
            row.reasoning_output_tokens,
        )?;
        if let (Some(input), Some(output)) = (row.input_tokens, row.output_tokens) {
            sum_optional(&mut total.total_tokens, Some(sum(input, output)?))?;
            total.total_reported_count += 1;
        }
        total.input_reported_count += u64::from(row.input_tokens.is_some());
        total.output_reported_count += u64::from(row.output_tokens.is_some());
        total.reasoning_reported_count += u64::from(row.reasoning_output_tokens.is_some());
        total.cache_reported_count += u64::from(
            row.cached_input_tokens.is_some()
                || row.cache_read_input_tokens.is_some()
                || row.cache_creation_input_tokens.is_some(),
        );
        if matches!(row.outcome.as_str(), "completed" | "incomplete")
            && let (Some(output), Some(elapsed)) = (
                row.output_tokens,
                row.terminal_elapsed_ms.filter(|value| *value > 0),
            )
        {
            total.eligible_speed_count += 1;
            self.eligible_output = sum(self.eligible_output, output)?;
            self.eligible_elapsed_ms = sum(self.eligible_elapsed_ms, elapsed)?;
        }
        Ok(())
    }
    fn finish(mut self) -> AnalyticsTotals {
        if self.eligible_elapsed_ms > 0 {
            self.totals.output_tokens_per_second =
                Some(self.eligible_output as f64 * 1000.0 / self.eligible_elapsed_ms as f64);
        }
        self.totals
    }
}
fn sum(a: u64, b: u64) -> Result<u64, Error> {
    a.checked_add(b)
        .ok_or_else(|| Error::invalid("分析统计超出数量范围"))
}
fn sum_optional(target: &mut Option<u64>, value: Option<u64>) -> Result<(), Error> {
    if let Some(value) = value {
        *target = Some(sum(target.unwrap_or(0), value)?);
    }
    Ok(())
}
struct AnalyticsGroup {
    key: String,
    name: String,
    provider_id: String,
    model_record_key: Option<String>,
    totals: AnalyticsAccumulator,
}
impl AnalyticsGroup {
    fn provider(row: &crate::analytics::AnalyticsRow) -> Self {
        Self {
            key: row.provider_id.clone(),
            name: row.provider_name.clone(),
            provider_id: row.provider_id.clone(),
            model_record_key: None,
            totals: Default::default(),
        }
    }
    fn model(row: &crate::analytics::AnalyticsRow) -> Self {
        Self {
            key: serde_json::json!([row.provider_id, row.model_record_key]).to_string(),
            name: row.provider_model_id.clone(),
            provider_id: row.provider_id.clone(),
            model_record_key: Some(row.model_record_key.clone()),
            totals: Default::default(),
        }
    }
    fn finish(self) -> AnalyticsBreakdown {
        AnalyticsBreakdown {
            key: self.key,
            name: self.name,
            provider_id: Some(self.provider_id),
            model_record_key: self.model_record_key,
            totals: self.totals.finish(),
        }
    }
}
fn analytics_request(row: crate::analytics::AnalyticsRow) -> Result<AnalyticsRequest, Error> {
    let protocol = match row.protocol.as_str() {
        "chatCompletions" => AnalyticsProtocol::ChatCompletions,
        "responses" => AnalyticsProtocol::Responses,
        "messages" => AnalyticsProtocol::Messages,
        _ => return Err(Error::invalid("分析记录包含未知协议")),
    };
    let outcome = match row.outcome.as_str() {
        "completed" => AnalyticsOutcome::Completed,
        "incomplete" => AnalyticsOutcome::Incomplete,
        "cancelled" => AnalyticsOutcome::Cancelled,
        "rejected" => AnalyticsOutcome::Rejected,
        "failed" => AnalyticsOutcome::Failed,
        _ => return Err(Error::invalid("分析记录包含未知结果")),
    };
    let output_tokens_per_second = if matches!(
        outcome,
        AnalyticsOutcome::Completed | AnalyticsOutcome::Incomplete
    ) {
        row.terminal_elapsed_ms
            .filter(|v| *v > 0)
            .zip(row.output_tokens)
            .map(|(elapsed, tokens)| tokens as f64 * 1000.0 / elapsed as f64)
    } else {
        None
    };
    Ok(AnalyticsRequest {
        request_id: row.request_id,
        provider_id: row.provider_id,
        provider_name: row.provider_name,
        model_record_key: row.model_record_key,
        provider_model_id: row.provider_model_id,
        protocol,
        started_at_ms: row.started_at_ms,
        terminal_at_ms: row.terminal_at_ms,
        elapsed_ms: row.elapsed_ms,
        first_output_ms: row.first_output_ms,
        terminal_elapsed_ms: row.terminal_elapsed_ms,
        output_tokens_per_second,
        status: row.status,
        outcome,
        input_tokens: row.input_tokens,
        uncached_input_tokens: row.uncached_input_tokens,
        output_tokens: row.output_tokens,
        reasoning_output_tokens: row.reasoning_output_tokens,
        cached_input_tokens: row.cached_input_tokens,
        cache_read_input_tokens: row.cache_read_input_tokens,
        cache_creation_input_tokens: row.cache_creation_input_tokens,
        usage_reported: row.usage_reported,
        usage_complete: row.usage_complete,
    })
}
