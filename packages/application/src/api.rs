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
    pub fn snapshot(&mut self, runtime_id: String) -> Result<SnapshotResult, Error> {
        self.execute("getSnapshot", json!({}), Some(&runtime_id))
    }
    pub fn send(&mut self, runtime_id: String, text: String) -> Result<SnapshotResult, Error> {
        self.execute("send", json!({"text":text}), Some(&runtime_id))
    }
    pub fn cancel(&mut self, runtime_id: String) -> Result<SnapshotResult, Error> {
        self.execute("cancel", json!({}), Some(&runtime_id))
    }
    pub fn select_model(
        &mut self,
        runtime_id: String,
        model_record_key: String,
    ) -> Result<SnapshotResult, Error> {
        self.execute(
            "selectModel",
            json!({"modelRecordKey":model_record_key}),
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
