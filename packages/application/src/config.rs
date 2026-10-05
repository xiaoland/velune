//! Application-owned schema 4 configuration and adapter integration metadata.
//!
//! These types contain ordinary configuration only. Credential references are
//! opaque handles; resolving them and constructing an HTTP client belongs to
//! the platform composition root.
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
pub struct ModelDefinition {
    pub record_key: String,
    pub nickname: String,
    pub icon: Option<String>,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderModelBinding {
    pub model_record_key: String,
    pub provider_model_id: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning: Option<ProtocolReasoning>,
    pub pi_projection: Option<PiModelProjection>,
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
    pub authentication_id: Option<String>,
    pub models: Vec<ProviderModelBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Route {
    pub model_record_key: String,
    pub provider_id: String,
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
    pub models: Vec<ModelDefinition>,
    pub providers: Vec<ProviderDefinition>,
    pub routes: Vec<Route>,
    pub failover: FailoverPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeInstance {
    pub id: String,
    pub name: String,
    pub type_id: String,
    pub gateway_id: String,
    pub settings: BTreeMap<String, String>,
    pub model_record_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeTypeDescriptor {
    pub id: String,
    pub name: String,
    pub fields: Vec<crate::conversation::SettingField>,
    pub actions: Vec<crate::conversation::SettingAction>,
}

impl RuntimeInstance {
    /// Validate execution-path settings when saving an instance. Existing
    /// configuration remains readable so users can repair incomplete settings.
    pub(crate) fn validate_execution_paths(&self) -> Result<(), &'static str> {
        if self.type_id == "pi" {
            let directory = self
                .settings
                .get("agentDir")
                .filter(|value| !value.is_empty())
                .ok_or("Pi Agent 运行时必须配置运行时目录")?;
            if !std::path::Path::new(directory).is_absolute() || directory.contains('\0') {
                return Err("Pi Agent 运行时目录必须是绝对路径");
            }
            let node = self
                .settings
                .get("nodeBinary")
                .filter(|value| !value.is_empty())
                .ok_or("Pi Agent 运行时必须配置 Node 可执行文件")?;
            if !std::path::Path::new(node).is_absolute() || node.contains('\0') {
                return Err("Pi Agent 的 Node 可执行文件必须是绝对路径");
            }
        }
        Ok(())
    }
}

impl GatewayConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.to_gateway_config()
            .map_err(|_| "gateway configuration encoding")?
            .validate()
    }

    /// Generate private record keys only for newly-created records. Native provider IDs remain unchanged.
    pub(crate) fn assign_record_keys(&mut self) {
        for model in &mut self.models {
            if model.record_key.is_empty() {
                model.record_key = crate::new_record_key();
            }
        }
    }

    pub fn model(&self, id: &str) -> Option<&ModelDefinition> {
        self.models.iter().find(|model| model.record_key == id)
    }

    /// Return the stable physical model identity used by a Pi catalog.
    ///
    /// The logical model remains the gateway request key. This identity binds
    /// the complete provider target without including bearer material or the
    /// Node executable path, so changing a route or account generation cannot
    /// reuse an incompatible Pi model entry.
    #[cfg(feature = "local-runtime")]
    pub fn pi_binding_id(
        &self,
        model_record_key: &str,
        authentication_revision: u64,
    ) -> Result<String, &'static str> {
        let provider = self.validate_dispatch(model_record_key)?;
        let route = self
            .routes
            .iter()
            .find(|route| route.model_record_key == model_record_key)
            .ok_or("model needs exactly one route")?;
        let binding = provider
            .models
            .iter()
            .find(|binding| binding.model_record_key == model_record_key)
            .ok_or("provider model binding is missing")?;
        let identity = PiBindingIdentity {
            model_record_key,
            provider_id: route.provider_id.as_str(),
            protocol: provider.protocol.identity_name(),
            endpoint: provider.endpoint.as_str(),
            provider_model_id: binding.provider_model_id.as_str(),
            pi_projection: binding.pi_projection.as_ref(),
            authentication_id: provider.authentication_id.as_deref(),
            authentication_revision,
            context_window: binding.context_window,
            max_output_tokens: binding.max_output_tokens,
            reasoning: binding.reasoning.as_ref(),
        };
        let canonical = serde_json::to_vec(&identity).map_err(|_| "binding identity encoding")?;
        let digest = Sha256::digest(canonical);
        Ok(format!("vln_{digest:x}"))
    }

    /// Validate the stricter boundary used immediately before a gateway call.
    /// Configuration editing may save incomplete models/providers/routes, but a
    /// dispatch must have exactly one explicit provider mapping.
    pub fn validate_dispatch(
        &self,
        model_record_key: &str,
    ) -> Result<&ProviderDefinition, &'static str> {
        self.validate()?;
        if self.model(model_record_key).is_none() {
            return Err("model is not configured");
        }
        let routes: Vec<_> = self
            .routes
            .iter()
            .filter(|route| route.model_record_key == model_record_key)
            .collect();
        if routes.len() != 1 {
            return Err("model needs exactly one route");
        }
        let provider = self
            .providers
            .iter()
            .find(|provider| provider.id == routes[0].provider_id)
            .ok_or("route provider is missing")?;
        if matches!(provider.protocol, GatewayProtocol::MessagesV1) {
            return Err("provider protocol is unsupported");
        }
        if !provider
            .models
            .iter()
            .any(|binding| binding.model_record_key == model_record_key)
        {
            return Err("provider model binding is missing");
        }
        Ok(provider)
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
    authentication_id: Option<&'a str>,
    authentication_revision: u64,
    context_window: Option<u32>,
    max_output_tokens: Option<u32>,
    reasoning: Option<&'a ProtocolReasoning>,
}

impl GatewayProtocol {
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
            models: self
                .models
                .iter()
                .map(|m| velune_gateway::ModelDefinition {
                    record_key: m.record_key.clone(),
                    nickname: m.nickname.clone(),
                    icon: m.icon.clone(),
                    max_output_tokens: m.max_output_tokens,
                    context_window: m.context_window,
                })
                .collect(),
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
                        credential_ref: p.authentication_id.clone(),
                        models: p
                            .models
                            .iter()
                            .map(|m| velune_gateway::ProviderModelBinding {
                                model_record_key: m.model_record_key.clone(),
                                provider_model_id: m.provider_model_id.clone(),
                                context_window: m.context_window,
                                max_output_tokens: m.max_output_tokens,
                                reasoning: m.reasoning.as_ref().map(|r| {
                                    velune_gateway::ProtocolReasoning {
                                        protocol: match r.protocol {
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
                                        levels: r.levels.clone(),
                                    }
                                }),
                            })
                            .collect(),
                    })
                })
                .collect::<Result<Vec<_>, crate::Error>>()?,
            routes: self
                .routes
                .iter()
                .map(|r| velune_gateway::Route {
                    model_record_key: r.model_record_key.clone(),
                    provider_id: r.provider_id.clone(),
                })
                .collect(),
            failover: velune_gateway::FailoverPolicy {
                mode: velune_gateway::FailoverMode::Disabled,
            },
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolReasoning {
    pub protocol: GatewayProtocol,
    pub levels: Vec<String>,
}
