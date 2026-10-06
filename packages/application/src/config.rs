//! Application-owned provider configuration and private adapter integration metadata.
#[cfg(feature = "local-runtime")]
use velune_agent_runtime::model_projection::PiModelProjection;
#[cfg(not(feature = "local-runtime"))]
type PiModelProjection = serde_json::Value;
use serde::{Deserialize, Serialize};
#[cfg(feature = "local-runtime")]
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderModel {
    pub record_key: String,
    pub provider_model_id: String,
    pub nickname: String,
    pub icon: Option<String>,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Option<Vec<String>>,
    pub pi_projection: Option<PiModelProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelTemplate {
    pub id: String,
    pub name: String,
    pub suggested_provider_model_id: String,
    pub nickname: String,
    pub icon: Option<String>,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum GatewayProtocol {
    ChatCompletionsV1,
    ResponsesV1,
    MessagesV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderDefinition {
    pub id: String,
    pub name: String,
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    pub(crate) authentication: crate::provider_authentication::ProviderAuthentication,
    pub models: Vec<ProviderModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum FailoverMode {
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailoverPolicy {
    pub mode: FailoverMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayConfig {
    pub id: String,
    pub name: String,
    pub providers: Vec<ProviderDefinition>,
    pub failover: FailoverPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeInstance {
    #[serde(default = "runtime_enabled_by_default")]
    pub enabled: bool,
    pub id: String,
    pub name: String,
    pub type_id: String,
    pub gateway_id: String,
    pub settings: BTreeMap<String, String>,
}

/// Association metadata only. Native sessions remain the authority for content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg(feature = "local-runtime")]
pub(crate) struct ConversationLink {
    pub id: String,
    pub segments: Vec<ConversationSegmentReference>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg(feature = "local-runtime")]
pub(crate) struct ConversationSegmentReference {
    pub runtime_instance_id: String,
    pub runtime_type_id: String,
    pub native_conversation_id: String,
    pub cutoff: Option<ConversationCutoff>,
    pub handoff: Option<ConversationHandoff>,
    pub deleted: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg(feature = "local-runtime")]
pub(crate) struct ConversationCutoff {
    pub message_count: u64,
    pub prefix_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg(feature = "local-runtime")]
pub(crate) struct ConversationHandoff {
    pub marker: String,
    pub payload_digest: String,
    pub user_message_ordinal: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeTypeDescriptor {
    pub id: String,
    pub family_id: String,
    pub version_regex: String,
    pub can_rename_conversations: bool,
    pub can_delete_conversations: bool,
    pub supported_protocols: Vec<GatewayProtocol>,
    pub name: String,
    pub fields: Vec<crate::conversation::SettingField>,
}

fn runtime_enabled_by_default() -> bool {
    true
}

impl RuntimeInstance {
    /// Validate execution-path settings when saving an instance. Existing
    /// configuration remains readable so users can repair incomplete settings.
    pub(crate) fn validate_execution_paths(&self) -> Result<(), &'static str> {
        if !matches!(
            self.type_id.as_str(),
            "pi-1.0.2" | "codex-0.159.3" | "dsh-acp-0.2.0-rc.2"
        ) {
            return Err("不支持的运行时版本类型");
        }
        for key in ["binary", "agentDir", "nodeBinary"] {
            let value = self
                .settings
                .get(key)
                .filter(|v| !v.is_empty())
                .ok_or("运行时入口、运行时目录与 Node 可执行文件必须配置")?;
            if !std::path::Path::new(value).is_absolute() || value.contains('\0') {
                return Err("运行时路径必须是绝对路径");
            }
        }
        if let Some(value) = self.settings.get("sessionDir").filter(|v| !v.is_empty())
            && (self.type_id != "pi-1.0.2"
                || !std::path::Path::new(value).is_absolute()
                || value.contains('\0'))
        {
            return Err("会话存储覆盖只适用于 Pi，且必须是绝对路径");
        }

        Ok(())
    }
}

impl GatewayConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        for provider in &self.providers {
            provider
                .authentication
                .validate_target(&provider.protocol, &provider.endpoint)?;
            let endpoint = url::Url::parse(&provider.endpoint).map_err(|_| "提供商服务地址无效")?;
            if !matches!(endpoint.scheme(), "http" | "https")
                || endpoint.host_str().is_none()
                || !endpoint.username().is_empty()
                || endpoint.password().is_some()
                || endpoint.query().is_some()
                || endpoint.fragment().is_some()
            {
                return Err("提供商服务地址必须是没有认证信息、查询或片段的 HTTP(S) 地址");
            }
        }
        self.to_gateway_config()
            .map_err(|_| "gateway configuration encoding")?
            .validate()
    }

    #[cfg(feature = "local-runtime")]
    pub(crate) fn authentication_revision(&self, key: &str) -> u64 {
        self.providers
            .iter()
            .find(|p| p.models.iter().any(|m| m.record_key == key))
            .map_or(0, |p| p.authentication.generation)
    }
    #[cfg(feature = "local-runtime")]
    pub(crate) fn subscription(&self, key: &str) -> bool {
        self.providers
            .iter()
            .find(|p| p.models.iter().any(|m| m.record_key == key))
            .is_some_and(|p| p.authentication.subscription())
    }
    /// Generate private record keys only for newly-created records. Native provider IDs remain unchanged.
    pub(crate) fn assign_record_keys(&mut self) {
        for model in self.providers.iter_mut().flat_map(|p| &mut p.models) {
            if model.record_key.is_empty() {
                model.record_key = crate::new_record_key();
            }
        }
    }
    pub fn model(&self, key: &str) -> Option<&ProviderModel> {
        self.providers
            .iter()
            .flat_map(|p| &p.models)
            .find(|m| m.record_key == key)
    }

    /// Return the stable physical model identity used by a Pi catalog.
    ///
    /// The logical model remains the gateway request key. This identity binds
    /// the complete provider target without including bearer material or the
    /// Node executable path, so changing a provider target or account generation cannot
    /// reuse an incompatible Pi model entry.
    #[cfg(feature = "local-runtime")]
    pub fn pi_binding_id(
        &self,
        model_record_key: &str,
        authentication_revision: u64,
    ) -> Result<String, &'static str> {
        let provider = self.validate_dispatch(model_record_key)?;
        let binding = provider
            .models
            .iter()
            .find(|binding| binding.record_key == model_record_key)
            .ok_or("provider model binding is missing")?;
        let identity = PiBindingIdentity {
            model_record_key,
            provider_id: provider.id.as_str(),
            protocol: provider.protocol.identity_name(),
            endpoint: provider.endpoint.as_str(),
            provider_model_id: binding.provider_model_id.as_str(),
            pi_projection: binding.pi_projection.as_ref(),
            authentication_generation: provider.authentication.generation,
            authentication_revision,
            context_window: binding.context_window,
            max_output_tokens: binding.max_output_tokens,
            reasoning_levels: binding.reasoning_levels.as_ref(),
        };
        let canonical = serde_json::to_vec(&identity).map_err(|_| "binding identity encoding")?;
        let digest = Sha256::digest(canonical);
        Ok(format!("vln_{digest:x}"))
    }

    /// Validate the stricter boundary used immediately before a gateway call.
    /// Model ownership determines the provider; no second routing configuration is required.
    pub fn validate_dispatch(
        &self,
        model_record_key: &str,
    ) -> Result<&ProviderDefinition, &'static str> {
        self.validate()?;
        self.providers
            .iter()
            .find(|p| p.models.iter().any(|m| m.record_key == model_record_key))
            .ok_or("provider model is not configured")
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg(feature = "local-runtime")]
struct PiBindingIdentity<'a> {
    model_record_key: &'a str,
    provider_id: &'a str,
    protocol: &'static str,
    endpoint: &'a str,
    provider_model_id: &'a str,
    pi_projection: Option<&'a PiModelProjection>,
    authentication_generation: u64,
    authentication_revision: u64,
    context_window: Option<u32>,
    max_output_tokens: Option<u32>,
    reasoning_levels: Option<&'a Vec<String>>,
}

impl GatewayProtocol {
    /// The versioned runtime registry owns support; family IDs never grant capabilities.
    #[cfg(feature = "local-runtime")]
    pub(crate) fn runtime_protocols(type_id: &str) -> Option<Vec<Self>> {
        use velune_agent_runtime::version::{RuntimeProtocol, variant};
        Some(
            variant(type_id)?
                .supported_protocols
                .iter()
                .map(|protocol| match protocol {
                    RuntimeProtocol::ChatCompletionsV1 => Self::ChatCompletionsV1,
                    RuntimeProtocol::ResponsesV1 => Self::ResponsesV1,
                    RuntimeProtocol::MessagesV1 => Self::MessagesV1,
                })
                .collect(),
        )
    }
    #[cfg(feature = "local-runtime")]
    fn identity_name(&self) -> &'static str {
        match self {
            Self::ChatCompletionsV1 => "chatCompletionsV1",
            Self::ResponsesV1 => "responsesV1",
            Self::MessagesV1 => "messagesV1",
        }
    }
}

impl GatewayConfig {
    /// Strip adapter metadata at the application-to-gateway boundary.
    pub fn to_gateway_config(&self) -> Result<velune_gateway::GatewayConfig, crate::Error> {
        Ok(velune_gateway::GatewayConfig {
            id: self.id.clone(),
            name: self.name.clone(),
            providers: self
                .providers
                .iter()
                .map(|p| {
                    Ok(velune_gateway::ProviderDefinition {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        protocol: match p.protocol {
                            GatewayProtocol::ChatCompletionsV1 => {
                                velune_gateway::GatewayProtocol::ChatCompletionsV1
                            }
                            GatewayProtocol::ResponsesV1 => {
                                velune_gateway::GatewayProtocol::ResponsesV1
                            }
                            GatewayProtocol::MessagesV1 => {
                                velune_gateway::GatewayProtocol::MessagesV1
                            }
                        },
                        endpoint: p.endpoint.clone(),
                        credential_ref: Some(p.id.clone()),
                        models: p
                            .models
                            .iter()
                            .map(|m| velune_gateway::ProviderModel {
                                record_key: m.record_key.clone(),
                                nickname: m.nickname.clone(),
                                icon: m.icon.clone(),
                                provider_model_id: m.provider_model_id.clone(),
                                context_window: m.context_window,
                                max_output_tokens: m.max_output_tokens,
                                reasoning_levels: m.reasoning_levels.clone(),
                            })
                            .collect(),
                    })
                })
                .collect::<Result<Vec<_>, crate::Error>>()?,
            failover: velune_gateway::FailoverPolicy {
                mode: velune_gateway::FailoverMode::Disabled,
            },
        })
    }
}
