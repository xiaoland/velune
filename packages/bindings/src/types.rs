//! UniFFI records mirror domain types without adding FFI dependencies to them.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingConversationSummary {
    pub id: String,
    pub title: String,
    pub updated_at: Option<String>,
    pub runtime_id: String,
    #[serde(default)]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct BindingConversationSnapshot {
    pub revision: u64,
    pub conversation: BindingConversationSummary,
    pub resource_id: Option<String>,
    pub model_id: Option<String>,
    pub run_state: BindingRunState,
    pub messages: Vec<BindingMessage>,
    pub actions: BindingConversationActions,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingConnection {
    pub id: String,
    pub name: String,
    pub state: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingSettingOption {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingExecutableDiscovery {
    pub command: String,
    pub minimum_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct BindingSettingAction {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingModelDefinition {
    pub id: String,
    pub nickname: String,
    pub icon: Option<String>,
    pub max_output_tokens: u32,
    #[serde(default)]
    pub context_window: Option<u32>,
    pub reasoning_levels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingProviderModelBinding {
    pub model_id: String,
    pub external_model_id: String,
    #[serde(default, rename = "piProjection", with = "adapter_metadata")]
    pub adapter_metadata_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingGatewayProtocol {
    ChatCompletionsV1,
    ResponsesV1,
    MessagesV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingProviderDefinition {
    pub id: String,
    pub name: String,
    pub protocol: BindingGatewayProtocol,
    pub endpoint: String,
    #[serde(default)]
    pub authentication_id: Option<String>,
    pub models: Vec<BindingProviderModelBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRoute {
    pub model_id: String,
    pub provider_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingFailoverMode {
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingFailoverPolicy {
    pub mode: BindingFailoverMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingGatewayConfig {
    pub id: String,
    pub name: String,
    pub models: Vec<BindingModelDefinition>,
    pub providers: Vec<BindingProviderDefinition>,
    pub routes: Vec<BindingRoute>,
    pub failover: BindingFailoverPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeInstance {
    pub id: String,
    pub name: String,
    pub type_id: String,
    pub gateway_id: String,
    pub settings: HashMap<String, String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeTypeDescriptor {
    pub id: String,
    pub name: String,
    pub fields: Vec<BindingSettingField>,
    pub actions: Vec<BindingSettingAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingConfigurationSnapshot {
    pub conversations: Vec<BindingConversationSummary>,
    pub connections: Vec<BindingConnection>,
    pub models: Vec<BindingModelDefinition>,
    pub gateways: Vec<BindingGatewayConfig>,
    pub runtime_instances: Vec<BindingRuntimeInstance>,
    pub runtime_types: Vec<BindingRuntimeTypeDescriptor>,
    pub authentication_bindings: Vec<BindingAuthenticationBinding>,
    pub provider_import_types: Vec<BindingRuntimeTypeDescriptor>,
    pub protocols: Vec<BindingProtocolDescriptor>,
    #[serde(rename = "activeRuntimeInstanceID")]
    pub active_runtime_instance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingProtocolDescriptor {
    pub id: BindingGatewayProtocol,
    pub name: String,
    pub supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingGatewayUpdate {
    pub gateways: Vec<BindingGatewayConfig>,
    pub models: Vec<BindingModelDefinition>,
    pub requires_reconnect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeUpdate {
    pub runtime_instances: Vec<BindingRuntimeInstance>,
    pub runtime_types: Vec<BindingRuntimeTypeDescriptor>,
    pub requires_reconnect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingConnectionResult {
    #[serde(rename = "runtimeInstanceID")]
    pub runtime_instance_id: Option<String>,
    pub connections: Vec<BindingConnection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingSnapshotResult {
    pub snapshot: Option<BindingConversationSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingProviderImportSource {
    pub kind: String,
    pub harness_type_id: String,
    pub source_instance_id: Option<String>,
    pub provider_id: Option<String>,
    pub settings: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingImportSelection {
    pub provider_id: String,
    pub model_ids: Vec<String>,
    pub model_mappings: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct BindingImportModelCandidate {
    pub id: String,
    pub external_model_id: String,
    pub name: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Vec<String>,
    pub can_import: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingImportResult {
    pub imported_provider_ids: Vec<String>,
    pub skipped_provider_ids: Vec<String>,
    pub gateways: Vec<BindingGatewayConfig>,
    pub requires_reconnect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAuthenticationMetadata {
    pub configured: bool,
    pub capabilities: BindingAuthenticationCapabilities,
    pub actions: Option<Vec<BindingSettingAction>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAuthenticationCapabilities {
    pub protocol: BindingGatewayProtocol,
    pub endpoint: String,
    pub explicit_output_cap: bool,
    pub temperature: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAuthenticationProgress {
    pub running: bool,
    pub events: Vec<BindingAuthenticationEvent>,
    pub gateways: Option<Vec<BindingGatewayConfig>>,
    pub requires_reconnect: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct BindingAuthenticationPrompt {
    pub kind: String,
    pub text: String,
    pub options: Option<Vec<BindingSettingOption>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub enum BindingSettingKind {
    Text,
    FilePath,
    DirectoryPath,
    Choice,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingAuthenticationMethod {
    ApiKey,
    #[serde(rename = "oauth")]
    OAuth,
    Unconfigured,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAuthenticationProvenance {
    pub display_name: String,
    pub runtime_instance_id: Option<String>,
    pub runtime_type_id: String,
    pub source_provider_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAuthenticationBinding {
    pub id: String,
    pub name: String,
    pub method: BindingAuthenticationMethod,
    pub configured: bool,
    pub protocol: BindingGatewayProtocol,
    pub endpoint: String,
    pub provenance: Option<BindingAuthenticationProvenance>,
    pub actions: Vec<BindingSettingAction>,
    pub owns_secret: bool,
    pub generation: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAuthenticationMutation {
    pub binding: BindingAuthenticationBinding,
    pub obsolete_owned_keychain_refs: Vec<String>,
    pub requires_reconnect: bool,
    pub warnings: Vec<String>,
}
