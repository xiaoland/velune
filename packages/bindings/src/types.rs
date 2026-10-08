//! UniFFI records mirror domain types without adding FFI dependencies to them.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationSummary {
    pub id: String,
    #[serde(deserialize_with = "display_conversation_title")]
    pub title: String,
    /// Unix epoch milliseconds; None means the source did not provide a timestamp.
    pub updated_at_unix_ms: Option<i64>,
    pub created_at_unix_ms: Option<i64>,
    pub runtime_id: String,
    pub cwd: Option<String>,
    pub can_rename: bool,
    pub can_delete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum BindingRunState {
    Idle,
    Running,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationActions {
    pub can_send: bool,
    pub can_cancel: bool,
    pub can_switch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct BindingMessage {
    pub id: String,
    pub role: BindingMessageRole,
    /// Native message time in Unix epoch milliseconds, when available.
    pub timestamp_unix_ms: Option<i64>,
    pub blocks: Vec<BindingMessageBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum BindingMessageRole {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum BindingToolState {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BindingMessageBlock {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    Tool {
        #[serde(rename = "toolID")]
        tool_id: Option<String>,
        title: String,
        state: BindingToolState,
        output: Option<String>,
    },
    Notice {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationSnapshot {
    /// Local projection revision; may reset between history and execution preparation.
    /// Do not use it to order snapshots across those lifecycle boundaries.
    pub revision: u64,
    pub conversation: BindingConversationSummary,
    pub context_runtime_id: String,
    pub resource_id: Option<String>,
    pub model_record_key: Option<String>,
    pub run_state: BindingRunState,
    pub messages: Vec<BindingMessage>,
    pub transcript_turns: Vec<BindingTranscriptTurn>,
    pub message_identity_confirmations: Vec<BindingMessageIdentityConfirmation>,
    #[serde(default)]
    pub pending_interactions: Vec<BindingRuntimeInteraction>,
    pub actions: BindingConversationActions,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingSettingOption {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingExecutableDiscovery {
    pub command: String,
    pub minimum_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingSettingField {
    pub key: String,
    pub label: String,
    pub kind: BindingSettingKind,
    pub required: bool,
    pub value: String,
    pub options: Vec<BindingSettingOption>,
    pub help: Option<String>,
    pub executable_discovery: Option<BindingExecutableDiscovery>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingSettingAction {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingProviderModel {
    pub record_key: String,
    pub provider_model_id: String,
    pub nickname: String,
    pub icon: Option<String>,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Option<Vec<String>>,
    #[serde(rename = "piProjection", with = "adapter_metadata")]
    pub adapter_metadata_json: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingModelTemplate {
    pub id: String,
    pub name: String,
    pub suggested_provider_model_id: String,
    pub nickname: String,
    pub icon: Option<String>,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Option<Vec<String>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum BindingGatewayProtocol {
    ChatCompletionsV1,
    ResponsesV1,
    MessagesV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingProviderDefinition {
    pub id: String,
    pub name: String,
    pub protocol: BindingGatewayProtocol,
    pub endpoint: String,
    pub authentication: BindingProviderAuthentication,
    pub models: Vec<BindingProviderModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingProviderDraft {
    pub id: String,
    pub name: String,
    pub protocol: BindingGatewayProtocol,
    pub endpoint: String,
    pub models: Vec<BindingProviderModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum BindingFailoverMode {
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingFailoverPolicy {
    pub mode: BindingFailoverMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingGatewayConfig {
    pub id: String,
    pub name: String,
    pub providers: Vec<BindingProviderDefinition>,
    pub failover: BindingFailoverPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingRuntimeInstance {
    pub enabled: bool,
    pub id: String,
    pub name: String,
    pub type_id: String,
    pub gateway_id: String,
    pub settings: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeDiscoveryHint {
    pub family_id: String,
    pub command: String,
    pub agent_directory: String,
    pub directory_exists: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeDiscoveryProbe {
    pub family_id: String,
    pub binary: String,
    pub node_binary: String,
    pub agent_directory: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeDiscoveryCandidate {
    pub runtime: BindingRuntimeInstance,
    pub version: Option<String>,
    pub supported: bool,
    pub already_configured: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingRuntimeTypeDescriptor {
    pub id: String,
    pub family_id: String,
    pub version_regex: String,
    pub can_rename_conversations: bool,
    pub can_delete_conversations: bool,
    pub supported_protocols: Vec<BindingGatewayProtocol>,
    pub supported_provider_protocols: Vec<BindingGatewayProtocol>,
    pub name: String,
    pub fields: Vec<BindingSettingField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConfigurationSnapshot {
    pub conversations: Vec<BindingConversationSummary>,
    pub history_failures: Vec<BindingHistoryFailure>,
    pub gateways: Vec<BindingGatewayConfig>,
    pub runtime_instances: Vec<BindingRuntimeInstance>,
    pub runtime_types: Vec<BindingRuntimeTypeDescriptor>,
    pub model_templates: Vec<BindingModelTemplate>,
    pub conversation_browser_group_limit: u32,
    pub transcript_presentation: BindingTranscriptPresentation,
    pub conversation_browser_preferences: BindingConversationBrowserPreferences,
    pub provider_import_types: Vec<BindingRuntimeTypeDescriptor>,
    pub protocols: Vec<BindingProtocolDescriptor>,
    #[serde(rename = "selectedRuntimeInstanceID")]
    pub selected_runtime_instance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingProtocolDescriptor {
    pub id: BindingGatewayProtocol,
    pub name: String,
    pub supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingGatewayUpdate {
    pub gateways: Vec<BindingGatewayConfig>,
    pub execution_invalidated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingRuntimeUpdate {
    pub runtime_instances: Vec<BindingRuntimeInstance>,
    pub runtime_types: Vec<BindingRuntimeTypeDescriptor>,
    pub execution_invalidated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingSnapshotResult {
    pub snapshot: Option<BindingConversationSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAnalyticsQuery {
    pub from_ms: i64,
    pub to_ms: i64,
    pub bucket_boundaries_ms: Vec<i64>,
    pub provider_id: Option<String>,
    pub model_record_key: Option<String>,
    pub request_limit: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingAnalyticsProtocol {
    ChatCompletions,
    Responses,
    Messages,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingAnalyticsOutcome {
    Completed,
    Incomplete,
    Failed,
    Cancelled,
    Rejected,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAnalyticsTotals {
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
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAnalyticsBucket {
    pub from_ms: i64,
    pub to_ms: i64,
    pub totals: BindingAnalyticsTotals,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAnalyticsBreakdown {
    pub key: String,
    pub name: String,
    pub provider_id: Option<String>,
    pub model_record_key: Option<String>,
    pub totals: BindingAnalyticsTotals,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAnalyticsRequest {
    pub request_id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model_record_key: String,
    pub provider_model_id: String,
    pub protocol: BindingAnalyticsProtocol,
    pub started_at_ms: i64,
    pub terminal_at_ms: i64,
    pub elapsed_ms: u64,
    pub first_output_ms: Option<u64>,
    pub terminal_elapsed_ms: Option<u64>,
    pub output_tokens_per_second: Option<f64>,
    pub status: Option<u16>,
    pub outcome: BindingAnalyticsOutcome,
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
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAnalyticsReport {
    pub overview: BindingAnalyticsTotals,
    pub trend: Vec<BindingAnalyticsBucket>,
    pub providers: Vec<BindingAnalyticsBreakdown>,
    pub models: Vec<BindingAnalyticsBreakdown>,
    pub requests: Vec<BindingAnalyticsRequest>,
    pub coverage: String,
    pub storage_warning: Option<String>,
    pub dropped_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingProviderImportSource {
    pub kind: String,
    pub harness_type_id: String,
    pub source_instance_id: Option<String>,
    pub provider_id: Option<String>,
    pub settings: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingImportSelection {
    pub provider_id: String,
    pub candidate_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingProviderImportPreview {
    pub contract_version: u64,
    pub source_fingerprint: String,
    pub target_fingerprint: String,
    pub sdk_version: String,
    pub source: BindingProviderImportSource,
    pub providers: Vec<BindingImportProviderCandidate>,
    pub warnings: Vec<String>,
    pub source_label: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingImportProviderCandidate {
    pub id: String,
    pub source_provider_id: String,
    pub name: String,
    pub protocol: String,
    pub endpoint: Option<String>,
    pub can_import: bool,
    pub already_imported: bool,
    pub credential_status: String,
    pub issues: Vec<String>,
    pub models: Vec<BindingImportModelCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingImportModelCandidate {
    pub candidate_key: String,
    pub provider_model_id: String,
    pub name: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Vec<String>,
    pub can_import: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingImportResult {
    pub imported_provider_ids: Vec<String>,
    pub skipped_provider_ids: Vec<String>,
    pub gateways: Vec<BindingGatewayConfig>,
    pub execution_invalidated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingAuthenticationMetadata {
    pub configured: bool,
    pub capabilities: BindingAuthenticationCapabilities,
    pub actions: Option<Vec<BindingSettingAction>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingAuthenticationCapabilities {
    pub protocol: BindingGatewayProtocol,
    pub endpoint: String,
    pub explicit_output_cap: bool,
    pub temperature: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingAuthenticationProgress {
    pub running: bool,
    pub events: Vec<BindingAuthenticationEvent>,
    pub gateways: Option<Vec<BindingGatewayConfig>>,
    pub execution_invalidated: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingAuthenticationEvent {
    #[serde(rename = "type")]
    pub type_id: String,
    pub id: Option<String>,
    pub prompt: Option<BindingAuthenticationPrompt>,
    pub notification: Option<BindingAuthenticationNotification>,
    pub ok: Option<bool>,
    pub error: Option<String>,
    pub cancelled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingAuthenticationPrompt {
    pub kind: String,
    pub text: String,
    pub options: Option<Vec<BindingSettingOption>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingAuthenticationNotification {
    pub kind: String,
    pub id: Option<String>,
    pub text: Option<String>,
    pub url: Option<String>,
    pub instructions: Option<String>,
    pub user_code: Option<String>,
    pub verification_uri: Option<String>,
}

mod adapter_metadata {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(
        value: &Option<String>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let parsed = value
            .as_ref()
            .map(|text| {
                serde_json::from_str::<serde_json::Value>(text).map_err(serde::ser::Error::custom)
            })
            .transpose()?;
        parsed.serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<String>, D::Error> {
        Option::<serde_json::Value>::deserialize(deserializer)?
            .map(|value| serde_json::to_string(&value).map_err(serde::de::Error::custom))
            .transpose()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum BindingSettingKind {
    Text,
    FilePath,
    DirectoryPath,
    Choice,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum BindingAuthenticationMethod {
    ApiKey,
    #[serde(rename = "oauth")]
    OAuth,
    Unconfigured,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingAuthenticationProvenance {
    pub display_name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingProviderAuthentication {
    pub method: BindingAuthenticationMethod,
    pub configured: bool,
    pub provenance: Option<BindingAuthenticationProvenance>,
    pub actions: Vec<BindingSettingAction>,
}
#[derive(Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum BindingAuthenticationEdit {
    Keep,
    SetApiKey { value: String },
    Clear,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeInteraction {
    pub id: String,
    pub kind: BindingInteractionKind,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BindingInteractionKind {
    Approval {
        title: String,
        detail: String,
        options: Vec<BindingInteractionOption>,
    },
    UserInput {
        questions: Vec<BindingInteractionQuestion>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingInteractionOption {
    pub id: String,
    pub label: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingInteractionQuestion {
    pub id: String,
    pub text: String,
    pub options: Vec<BindingInteractionOption>,
    pub secret: bool,
}
#[derive(Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BindingRuntimeInteractionReply {
    Decision {
        #[serde(rename = "optionId")]
        option_id: String,
    },
    Answers {
        answers: Vec<BindingInteractionAnswer>,
    },
    Cancel,
}
#[derive(Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingInteractionAnswer {
    pub question_id: String,
    pub values: Vec<String>,
}

/// A provider-scoped public catalog entry, used only for editable template copies.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingCatalogModel {
    pub source_provider_id: String,
    pub source_provider_name: String,
    pub model_id: String,
    pub name: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingHistoryFailure {
    pub runtime_id: String,
    pub detail: String,
}

fn display_conversation_title<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let title = velune_application::conversation::ConversationTitle::deserialize(deserializer)?;
    Ok(title.display_text().into())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingTranscriptPresentation {
    Conversation,
    UserOutline,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingTranscriptTurn {
    pub id: String,
    pub user_message_id: String,
    pub work_message_ids: Vec<String>,
    pub last_message_id: Option<String>,
    pub duration_ms: Option<u64>,
    pub is_running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingMessageIdentityConfirmation {
    pub previous_id: String,
    pub current_id: String,
}

/// Application-owned sidebar choices; these never select an execution runtime.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationBrowserPreferences {
    pub grouping: BindingConversationBrowserGrouping,
    pub sort: BindingConversationBrowserSort,
    pub oldest_first: bool,
    pub runtime_id: Option<String>,
    pub project: BindingConversationBrowserProject,
}
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingConversationBrowserGrouping {
    #[default]
    None,
    Runtime,
    Project,
}
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingConversationBrowserSort {
    #[default]
    Updated,
    Created,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingConversationBrowserProject {
    #[default]
    All,
    Unspecified,
    Path {
        path: String,
    },
}
