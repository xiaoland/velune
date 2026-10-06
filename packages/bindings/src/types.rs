//! UniFFI records mirror domain types without adding FFI dependencies to them.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationSummary {
    pub id: String,
    pub title: String,
    pub updated_at: Option<String>,
    pub runtime_id: String,
    pub cwd: Option<String>,
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
    pub role: String,
    pub blocks: Vec<BindingMessageBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BindingMessageBlock {
    Text {
        text: String,
    },
    Tool {
        #[serde(rename = "toolID")]
        tool_id: Option<String>,
        title: String,
        state: Option<String>,
    },
    Notice {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationSnapshot {
    pub revision: u64,
    pub conversation: BindingConversationSummary,
    pub resource_id: Option<String>,
    pub model_record_key: Option<String>,
    pub run_state: BindingRunState,
    pub messages: Vec<BindingMessage>,
    #[serde(default)]
    pub pending_interactions: Vec<BindingRuntimeInteraction>,
    pub actions: BindingConversationActions,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConnection {
    pub id: String,
    pub name: String,
    pub state: String,
    pub capabilities: Vec<String>,
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
    pub id: String,
    pub name: String,
    pub type_id: String,
    pub gateway_id: String,
    pub settings: HashMap<String, String>,
    pub model_record_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingRuntimeTypeDescriptor {
    pub id: String,
    pub family_id: String,
    pub version_regex: String,
    pub supported_protocols: Vec<BindingGatewayProtocol>,
    pub name: String,
    pub fields: Vec<BindingSettingField>,
    pub actions: Vec<BindingSettingAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConfigurationSnapshot {
    pub conversations: Vec<BindingConversationSummary>,
    pub connections: Vec<BindingConnection>,
    pub gateways: Vec<BindingGatewayConfig>,
    pub runtime_instances: Vec<BindingRuntimeInstance>,
    pub runtime_types: Vec<BindingRuntimeTypeDescriptor>,
    pub model_templates: Vec<BindingModelTemplate>,
    pub provider_import_types: Vec<BindingRuntimeTypeDescriptor>,
    pub protocols: Vec<BindingProtocolDescriptor>,
    #[serde(rename = "activeRuntimeInstanceID")]
    pub active_runtime_instance_id: Option<String>,
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
    pub requires_reconnect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingRuntimeUpdate {
    pub runtime_instances: Vec<BindingRuntimeInstance>,
    pub runtime_types: Vec<BindingRuntimeTypeDescriptor>,
    pub requires_reconnect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConnectionResult {
    #[serde(rename = "runtimeInstanceID")]
    pub runtime_instance_id: Option<String>,
    pub connections: Vec<BindingConnection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingSnapshotResult {
    pub snapshot: Option<BindingConversationSnapshot>,
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
    pub requires_reconnect: bool,
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
    pub requires_reconnect: Option<bool>,
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
