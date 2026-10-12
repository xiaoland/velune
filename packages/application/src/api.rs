//! Typed application operations and non-secret result records.
use crate::{Application, Error, config::*};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use velune_conversation::{ConversationSummary, SettingAction, SettingOption};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDiscoveryHint {
    pub family_id: String,
    pub command: String,
    pub agent_directory: String,
    pub directory_exists: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDiscoveryProbe {
    pub family_id: String,
    pub binary: String,
    pub node_binary: String,
    pub agent_directory: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDiscoveryCandidate {
    pub runtime: RuntimeInstance,
    pub version: Option<String>,
    pub supported: bool,
    pub already_configured: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSummary {
    pub id: String,
    pub name: String,
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    pub authentication: crate::ProviderAuthenticationDescription,
    pub models: Vec<ProviderModel>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewaySummary {
    pub id: String,
    pub name: String,
    pub providers: Vec<ProviderSummary>,
    pub failover: FailoverPolicy,
}
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
                    authentication: p.authentication.description(),
                    models: p.models.clone(),
                })
                .collect(),
            failover: g.failover.clone(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderDraft {
    pub id: String,
    pub name: String,
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    pub models: Vec<ProviderModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationSnapshot {
    pub conversations: Vec<ConversationSummary>,
    pub history_failures: Vec<HistoryFailure>,
    pub gateways: Vec<GatewaySummary>,
    pub runtime_instances: Vec<RuntimeInstance>,
    pub runtime_types: Vec<RuntimeTypeDescriptor>,
    pub model_templates: Vec<ModelTemplate>,
    pub conversation_browser_group_limit: u32,
    pub outline_presentation: OutlinePresentation,
    pub conversation_browser_preferences: ConversationBrowserPreferences,
    pub provider_import_types: Vec<RuntimeTypeDescriptor>,
    pub protocols: Vec<ProtocolDescriptor>,
    #[serde(rename = "selectedRuntimeInstanceID")]
    pub selected_runtime_instance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolDescriptor {
    pub id: String,
    pub name: String,
    pub supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayUpdate {
    pub gateways: Vec<GatewaySummary>,
    pub execution_invalidated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeUpdate {
    pub runtime_instances: Vec<RuntimeInstance>,
    pub runtime_types: Vec<RuntimeTypeDescriptor>,
    pub execution_invalidated: bool,
}

/// The selected model belongs either to Velune's gateway or to the target runtime.
/// Native identifiers are never resolved through the gateway catalog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ModelSelection {
    /// Keep the ACP Agent's own session default; no model identifier is invented.
    RuntimeDefault,
    Managed {
        model_record_key: String,
    },
    Native {
        provider_id: Option<String>,
        model_id: String,
    },
}

/// What the user composed for one turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposedTurn {
    pub text: String,
    /// One of `reasoning_efforts` for the turn's runtime and model; `None`
    /// leaves the level to the runtime's default.
    pub effort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeModel {
    pub selection: ModelSelection,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotResult {
    pub snapshot: Option<crate::DeliveredSnapshot>,
}

/// A conversation that executes or waits for the user, in any open session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveConversation {
    pub context_runtime_id: String,
    pub conversation_id: String,
    pub run_state: crate::conversation::RunState,
    pub pending_interactions: u32,
}

/// Uncommitted changes in a conversation's working directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingChanges {
    pub directory: String,
    /// The git work tree containing the directory; None outside one.
    pub repository: Option<String>,
    /// None when HEAD is detached.
    pub branch: Option<String>,
    pub files: Vec<ChangedFile>,
}

/// A file that differs from HEAD or is not tracked, by repository path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    pub kind: ChangeKind,
    pub staged: bool,
    pub unstaged: bool,
    /// Lines against HEAD; None for binary or untracked files.
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed { from: String },
    Untracked,
    Conflicted,
}

/// One file's diff against HEAD (or as a new file when untracked).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    pub lines: Vec<DiffLine>,
    /// The diff was longer than shown.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiffLineKind {
    Hunk,
    Context,
    Added,
    Removed,
    /// Binary content, mode changes, missing final newline.
    Notice,
}

/// How a Velune turn ended.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TurnOutcome {
    Completed,
    Failed,
    /// The user stopped it.
    Cancelled,
}

/// A Velune turn that ended, numbered in the order turns ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishedTurn {
    pub sequence: u64,
    pub context_runtime_id: String,
    pub conversation_id: String,
    pub outcome: TurnOutcome,
}

/// Conversations executing or waiting for the user, and turns that ended
/// after a cursor. `cursor` is the sequence to pass next time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationActivity {
    pub active: Vec<ActiveConversation>,
    pub finished: Vec<FinishedTurn>,
    pub cursor: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsQuery {
    pub from_ms: i64,
    pub to_ms: i64,
    pub bucket_boundaries_ms: Vec<i64>,
    pub provider_id: Option<String>,
    pub model_record_key: Option<String>,
    pub request_limit: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AnalyticsProtocol {
    ChatCompletions,
    Responses,
    Messages,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AnalyticsOutcome {
    Completed,
    Incomplete,
    Failed,
    Cancelled,
    Rejected,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsTotals {
    pub request_count: u64,
    pub completed_count: u64,
    pub failed_count: u64,
    pub incomplete_count: u64,
    pub rejected_count: u64,
    pub cancelled_count: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub usage_reported_count: u64,
    pub usage_complete_count: u64,
    pub eligible_speed_count: u64,
    pub output_tokens_per_second: Option<f64>,
    pub total_tokens: Option<u64>,
    pub total_reported_count: u64,
    pub input_reported_count: u64,
    pub output_reported_count: u64,
    pub reasoning_reported_count: u64,
    pub cache_reported_count: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsBucket {
    pub from_ms: i64,
    pub to_ms: i64,
    pub totals: AnalyticsTotals,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsBreakdown {
    pub key: String,
    pub name: String,
    pub provider_id: Option<String>,
    pub model_record_key: Option<String>,
    pub totals: AnalyticsTotals,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsRequest {
    pub request_id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model_record_key: String,
    pub provider_model_id: String,
    pub protocol: AnalyticsProtocol,
    pub started_at_ms: i64,
    pub terminal_at_ms: i64,
    pub elapsed_ms: u64,
    pub first_output_ms: Option<u64>,
    pub terminal_elapsed_ms: Option<u64>,
    pub output_tokens_per_second: Option<f64>,
    pub status: Option<u16>,
    pub outcome: AnalyticsOutcome,
    pub input_tokens: Option<u64>,
    pub uncached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub usage_reported: bool,
    pub usage_complete: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsReport {
    pub overview: AnalyticsTotals,
    pub trend: Vec<AnalyticsBucket>,
    pub providers: Vec<AnalyticsBreakdown>,
    pub models: Vec<AnalyticsBreakdown>,
    pub requests: Vec<AnalyticsRequest>,
    pub coverage: String,
    pub storage_warning: Option<String>,
    pub dropped_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderImportSource {
    pub kind: String,
    pub harness_type_id: String,
    pub source_instance_id: Option<String>,
    pub provider_id: Option<String>,
    pub settings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSelection {
    pub provider_id: String,
    pub candidate_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderImportPreview {
    pub contract_version: u64,
    pub source_fingerprint: String,
    pub target_fingerprint: String,
    pub sdk_version: String,
    pub source: ProviderImportSource,
    pub providers: Vec<ImportProviderCandidate>,
    pub warnings: Vec<String>,
    pub source_label: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProviderCandidate {
    pub id: String,
    pub source_provider_id: String,
    pub name: String,
    pub protocol: String,
    pub endpoint: Option<String>,
    pub can_import: bool,
    pub already_imported: bool,
    pub credential_status: String,
    pub issues: Vec<String>,
    pub models: Vec<ImportModelCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportModelCandidate {
    pub candidate_key: String,
    pub provider_model_id: String,
    pub name: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Vec<String>,
    pub can_import: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported_provider_ids: Vec<String>,
    pub skipped_provider_ids: Vec<String>,
    pub gateways: Vec<GatewaySummary>,
    pub execution_invalidated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationMetadata {
    pub configured: bool,
    pub capabilities: AuthenticationCapabilities,
    pub actions: Option<Vec<SettingAction>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationCapabilities {
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    pub explicit_output_cap: bool,
    pub temperature: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationProgress {
    pub running: bool,
    pub events: Vec<AuthenticationEvent>,
    pub gateways: Option<Vec<GatewaySummary>>,
    pub execution_invalidated: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationEvent {
    #[serde(rename = "type")]
    pub type_id: String,
    pub id: Option<String>,
    pub prompt: Option<AuthenticationPrompt>,
    pub notification: Option<AuthenticationNotification>,
    pub ok: Option<bool>,
    pub error: Option<String>,
    pub cancelled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationPrompt {
    pub kind: String,
    pub text: String,
    pub options: Option<Vec<SettingOption>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationNotification {
    pub kind: String,
    pub id: Option<String>,
    pub text: Option<String>,
    pub url: Option<String>,
    pub instructions: Option<String>,
    pub user_code: Option<String>,
    pub verification_uri: Option<String>,
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

    /// Overwrite drifted entries, stop syncing (keeping native content), or remove them.
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

    /// Create a root session with a fixed, host-authorized set of child targets.
    pub fn create_collaborative_conversation(
        &mut self,
        runtime_id: String,
        cwd: String,
        selection: ModelSelection,
        targets: Vec<crate::CollaborationTarget>,
        limits: crate::CollaborationLimits,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "createCollaborative",
            json!({"cwd":cwd,"modelSelection":selection,"targets":targets,"limits":limits}),
            Some(&runtime_id),
        )
    }
    pub fn collaboration_children(&mut self) -> Result<Vec<crate::ChildStatus>, Error> {
        self.execute("collaborationChildren", json!({}), None)
    }
    pub fn child_snapshot(&mut self, child_id: String) -> Result<crate::ChildSnapshot, Error> {
        self.execute("childSnapshot", json!({"childID":child_id}), None)
    }
    pub fn reply_child_interaction(
        &mut self,
        child_id: String,
        turn: u64,
        interaction_id: String,
        reply: crate::conversation::RuntimeInteractionReply,
    ) -> Result<crate::ChildStatus, Error> {
        self.execute(
            "replyChildInteraction",
            json!({"childID":child_id,"turn":turn,"interactionID":interaction_id,"reply":reply}),
            None,
        )
    }
    /// Explicitly revoke the root's collaboration tools and close all its children.
    pub fn close_collaboration(&mut self) -> Result<(), Error> {
        self.execute("closeCollaboration", json!({}), None)
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
        let value = self.runtime.request_inner(&request)?;
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
    pub fn set_conversation_browser_preferences(
        &mut self,
        preferences: ConversationBrowserPreferences,
    ) -> Result<ConversationBrowserPreferences, Error> {
        self.execute(
            "conversationBrowserPreferences",
            json!({"preferences":preferences}),
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
    pub fn set_conversation_browser_group_limit(&mut self, limit: u32) -> Result<u32, Error> {
        self.execute("conversationBrowserSettings", json!({"limit": limit}), None)
    }
    pub fn upsert_runtime(&mut self, runtime: RuntimeInstance) -> Result<RuntimeUpdate, Error> {
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
            ModelSelection::Managed { model_record_key },
        )
    }
    pub fn create_conversation_with_selection(
        &mut self,
        runtime_id: String,
        cwd: Option<String>,
        selection: ModelSelection,
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
            ModelSelection::Managed { model_record_key },
            ComposedTurn { text, effort: None },
        )
    }
    pub fn send_turn_with_selection(
        &mut self,
        source_runtime_id: String,
        conversation_id: String,
        target_runtime_id: String,
        selection: ModelSelection,
        turn: ComposedTurn,
    ) -> Result<SnapshotResult, Error> {
        self.execute("sendTurn", json!({"sourceRuntimeID":source_runtime_id,"conversationID":conversation_id,"modelSelection":selection,"text":turn.text,"effort":turn.effort}), Some(&target_runtime_id))
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryFailure {
    pub runtime_id: String,
    pub detail: String,
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
