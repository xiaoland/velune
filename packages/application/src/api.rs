//! Typed application operations and non-secret result records.
use crate::{Application, Error, config::*};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use velune_conversation::{
    ConversationSnapshot, ConversationSummary, SettingAction, SettingOption,
};

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
    pub transcript_presentation: TranscriptPresentation,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotResult {
    pub snapshot: Option<ConversationSnapshot>,
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
    pub fn set_transcript_presentation(
        &mut self,
        presentation: TranscriptPresentation,
    ) -> Result<TranscriptPresentation, Error> {
        self.execute(
            "transcriptPresentation",
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
    pub fn create_conversation(
        &mut self,
        runtime_id: String,
        cwd: String,
        model_record_key: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "create",
            json!({"cwd":cwd,"modelRecordKey":model_record_key}),
            Some(&runtime_id),
        )
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
    pub fn snapshot(&mut self, runtime_id: String) -> Result<SnapshotResult, Error> {
        self.execute("getSnapshot", json!({}), Some(&runtime_id))
    }
    /// Accept this turn's runtime and model. A different instance creates a new
    /// native segment and carries quoted text context; it does not replay tools
    /// or resume a foreign native session. Association commits precede dispatch,
    /// and an ambiguous dispatch is never automatically retried.
    pub fn send_turn(
        &mut self,
        runtime_id: String,
        model_record_key: String,
        text: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "sendTurn",
            json!({"modelRecordKey":model_record_key,"text":text}),
            Some(&runtime_id),
        )
    }
    pub fn cancel(&mut self, runtime_id: String) -> Result<SnapshotResult, Error> {
        self.execute("cancel", json!({}), Some(&runtime_id))
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
        interaction_id: String,
        reply: velune_conversation::RuntimeInteractionReply,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "replyRuntimeInteraction",
            json!({"interactionID":interaction_id,"reply":reply}),
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
