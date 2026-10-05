//! Application-owned schema 2 configuration and adapter integration metadata.
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
#[cfg(not(feature = "local-runtime"))]
use std::collections::BTreeSet;
#[cfg(feature = "local-runtime")]
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelDefinition {
    pub id: String,
    pub nickname: String,
    pub icon: Option<String>,
    pub max_output_tokens: u32,
    #[serde(default)]
    pub context_window: Option<u32>,
    pub reasoning_levels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelBinding {
    pub model_id: String,
    pub external_model_id: String,
    #[serde(default)]
    pub pi_projection: Option<PiModelProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GatewayProtocol {
    ChatCompletionsV1,
    ResponsesV1,
    MessagesV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CredentialSourceKind {
    #[serde(rename = "harness")]
    Harness,
}

/// Platform-owned authentication lookup metadata. It contains no bearer or key material.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CredentialSource {
    pub kind: CredentialSourceKind,
    pub harness_type_id: String,
    #[serde(default)]
    pub source_instance_id: Option<String>,
    pub provider_id: String,
    pub settings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDefinition {
    pub id: String,
    pub name: String,
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    #[serde(default)]
    pub credential_ref: Option<String>,
    #[serde(default)]
    pub credential_source: Option<CredentialSource>,
    #[serde(default)]
    pub credential_generation: u64,
    pub models: Vec<ProviderModelBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub model_id: String,
    pub provider_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FailoverMode {
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailoverPolicy {
    pub mode: FailoverMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConfig {
    pub id: String,
    pub name: String,
    pub models: Vec<ModelDefinition>,
    pub providers: Vec<ProviderDefinition>,
    pub routes: Vec<Route>,
    pub failover: FailoverPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInstance {
    pub id: String,
    pub name: String,
    pub type_id: String,
    pub gateway_id: String,
    pub settings: BTreeMap<String, String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
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
        #[cfg(feature = "local-runtime")]
        {
            self.to_gateway_config()
                .map_err(|_| "gateway configuration encoding")?
                .validate()?;
            for provider in &self.providers {
                if !valid_credential_source(provider) {
                    return Err("provider authentication source is invalid");
                }
                for binding in &provider.models {
                    if let Some(projection) = &binding.pi_projection {
                        if projection.thinking_level_map.keys().any(|level| {
                            !["off", "minimal", "low", "medium", "high", "xhigh", "max"]
                                .contains(&level.as_str())
                        }) {
                            return Err("provider model execution level is invalid");
                        }
                        if projection
                            .thinking_level_map
                            .values()
                            .flatten()
                            .any(|value| {
                                value.len() > 32
                                    || ![
                                        "none", "off", "minimal", "low", "medium", "high", "xhigh",
                                        "max",
                                    ]
                                    .contains(&value.as_str())
                            })
                        {
                            return Err("provider model execution wire level is invalid");
                        }
                        if projection.completions_max_tokens_field.is_some()
                            && !matches!(provider.protocol, GatewayProtocol::ChatCompletionsV1)
                        {
                            return Err(
                                "completions output field requires Chat Completions protocol",
                            );
                        }
                        if projection.responses_compat.is_some()
                            && !matches!(provider.protocol, GatewayProtocol::ResponsesV1)
                        {
                            return Err("responses execution requires Responses protocol");
                        }
                    }
                }
            }
            Ok(())
        }
        #[cfg(not(feature = "local-runtime"))]
        {
            if self.id.is_empty() || self.name.is_empty() {
                return Err("gateway identity is required");
            }
            if !matches!(self.failover.mode, FailoverMode::Disabled) {
                return Err("gateway failover mode is unsupported");
            }
            let mut models = BTreeSet::new();
            for model in &self.models {
                if model.id.is_empty()
                    || model.nickname.is_empty()
                    || model.max_output_tokens == 0
                    || model
                        .context_window
                        .is_some_and(|window| window == 0 || model.max_output_tokens > window)
                    || !models.insert(&model.id)
                {
                    return Err("invalid or duplicate model definition");
                }
                let mut levels = BTreeSet::new();
                if model
                    .reasoning_levels
                    .iter()
                    .any(|level| level.is_empty() || !levels.insert(level))
                {
                    return Err("invalid or duplicate reasoning level");
                }
            }
            let mut providers = BTreeMap::new();
            for provider in &self.providers {
                if provider.id.is_empty()
                    || provider.name.is_empty()
                    || provider.endpoint.is_empty()
                    || matches!(provider.protocol, GatewayProtocol::MessagesV1)
                    || !valid_credential_source(provider)
                    || providers.insert(&provider.id, provider).is_some()
                {
                    return Err("provider is unsupported or duplicated");
                }
                let mut bindings = BTreeSet::new();
                for binding in &provider.models {
                    if !models.contains(&binding.model_id)
                        || binding.external_model_id.is_empty()
                        || !bindings.insert(&binding.model_id)
                    {
                        return Err("provider model binding is invalid");
                    }
                    #[cfg(feature = "local-runtime")]
                    if let Some(projection) = &binding.pi_projection {
                        if projection.thinking_level_map.keys().any(|level| {
                            !["off", "minimal", "low", "medium", "high", "xhigh", "max"]
                                .contains(&level.as_str())
                        }) {
                            return Err("provider model execution level is invalid");
                        }
                        if projection
                            .thinking_level_map
                            .values()
                            .flatten()
                            .any(|value| {
                                value.len() > 32
                                    || ![
                                        "none", "off", "minimal", "low", "medium", "high", "xhigh",
                                        "max",
                                    ]
                                    .contains(&value.as_str())
                            })
                        {
                            return Err("provider model execution wire level is invalid");
                        }
                        if projection.completions_max_tokens_field.is_some()
                            && !matches!(provider.protocol, GatewayProtocol::ChatCompletionsV1)
                        {
                            return Err(
                                "completions output field requires Chat Completions protocol",
                            );
                        }
                        if projection.responses_compat.is_some()
                            && !matches!(provider.protocol, GatewayProtocol::ResponsesV1)
                        {
                            return Err("responses execution requires Responses protocol");
                        }
                    }
                }
            }
            let mut routes = BTreeMap::new();
            for route in &self.routes {
                let provider = providers
                    .get(&route.provider_id)
                    .ok_or("route provider is missing")?;
                if !models.contains(&route.model_id)
                    || routes.insert(&route.model_id, &route.provider_id).is_some()
                    || !provider
                        .models
                        .iter()
                        .any(|binding| binding.model_id == route.model_id)
                {
                    return Err("route must select one configured provider model");
                }
            }
            Ok(())
        }
    }

    pub fn model(&self, id: &str) -> Option<&ModelDefinition> {
        self.models.iter().find(|model| model.id == id)
    }

    /// Return the stable physical model identity used by a Pi catalog.
    ///
    /// The logical model remains the gateway request key. This identity binds
    /// the complete provider target without including bearer material or the
    /// Node executable path, so changing a route or account generation cannot
    /// reuse an incompatible Pi model entry.
    #[cfg(feature = "local-runtime")]
    pub fn pi_binding_id(&self, logical_model_id: &str) -> Result<String, &'static str> {
        let provider = self.validate_dispatch(logical_model_id)?;
        let route = self
            .routes
            .iter()
            .find(|route| route.model_id == logical_model_id)
            .ok_or("model needs exactly one route")?;
        let binding = provider
            .models
            .iter()
            .find(|binding| binding.model_id == logical_model_id)
            .ok_or("provider model binding is missing")?;
        let credential_source = provider.credential_source.as_ref().map(|source| {
            let auth_path = source.settings.get("authPath").map(String::as_str);
            let models_path = source.settings.get("modelsPath").map(String::as_str);
            let credential_location = source
                .settings
                .get("credentialLocation")
                .map(String::as_str);
            PiCredentialIdentity {
                auth_path,
                models_path,
                credential_location,
                harness: source.harness_type_id.as_str(),
                provider: source.provider_id.as_str(),
                source_instance: source.source_instance_id.as_deref(),
            }
        });
        let identity = PiBindingIdentity {
            logical_model_id,
            provider_id: route.provider_id.as_str(),
            protocol: provider.protocol.identity_name(),
            endpoint: provider.endpoint.as_str(),
            external_model_id: binding.external_model_id.as_str(),
            pi_projection: binding.pi_projection.as_ref(),
            credential_ref: provider.credential_ref.as_deref(),
            credential_source,
            credential_generation: provider.credential_generation,
        };
        let canonical = serde_json::to_vec(&identity).map_err(|_| "binding identity encoding")?;
        let digest = Sha256::digest(canonical);
        Ok(format!("vln_{digest:x}"))
    }

    /// Validate the stricter boundary used immediately before a gateway call.
    /// Configuration editing may save incomplete models/providers/routes, but a
    /// dispatch must have exactly one explicit provider mapping.
    pub fn validate_dispatch(&self, model_id: &str) -> Result<&ProviderDefinition, &'static str> {
        self.validate()?;
        if self.model(model_id).is_none() {
            return Err("model is not configured");
        }
        let routes: Vec<_> = self
            .routes
            .iter()
            .filter(|route| route.model_id == model_id)
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
            .any(|binding| binding.model_id == model_id)
        {
            return Err("provider model binding is missing");
        }
        Ok(provider)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg(feature = "local-runtime")]
struct PiBindingIdentity<'a> {
    logical_model_id: &'a str,
    provider_id: &'a str,
    protocol: &'static str,
    endpoint: &'a str,
    external_model_id: &'a str,
    pi_projection: Option<&'a PiModelProjection>,
    credential_ref: Option<&'a str>,
    credential_source: Option<PiCredentialIdentity<'a>>,
    credential_generation: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg(feature = "local-runtime")]
struct PiCredentialIdentity<'a> {
    #[serde(rename = "authPath")]
    auth_path: Option<&'a str>,
    #[serde(rename = "modelsPath")]
    models_path: Option<&'a str>,
    #[serde(rename = "credentialLocation")]
    credential_location: Option<&'a str>,
    harness: &'a str,
    provider: &'a str,
    #[serde(rename = "sourceInstance")]
    source_instance: Option<&'a str>,
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

#[cfg(feature = "local-runtime")]
fn valid_credential_source(provider: &ProviderDefinition) -> bool {
    match (&provider.credential_ref, &provider.credential_source) {
        (Some(reference), None) => !reference.is_empty(),
        (None, Some(source)) => {
            matches!(source.kind, CredentialSourceKind::Harness)
                && source.harness_type_id == "pi"
                && !source.provider_id.is_empty()
                && !matches!(provider.protocol, GatewayProtocol::MessagesV1)
                && source
                    .settings
                    .get("nodeBinary")
                    .is_some_and(|value| Path::new(value).is_absolute())
                && source.settings.get("bindingProjection").is_none_or(|raw| {
                    let Ok(expected) =
                        serde_json::from_str::<BTreeMap<String, PiModelProjection>>(raw)
                    else {
                        return false;
                    };
                    provider.models.iter().all(|binding| {
                        expected.get(&binding.external_model_id) == binding.pi_projection.as_ref()
                    })
                })
                && (source
                    .settings
                    .get("modelsPath")
                    .is_some_and(|value| Path::new(value).is_absolute())
                    || (source.provider_id == "openai"
                        && matches!(provider.protocol, GatewayProtocol::ResponsesV1)
                        && provider.endpoint == "https://api.openai.com/v1"
                        && source
                            .settings
                            .get("authPath")
                            .is_some_and(|value| Path::new(value).is_absolute())))
                && source
                    .source_instance_id
                    .as_deref()
                    .is_none_or(|value| !value.is_empty())
        }
        (None, None) => true,
        _ => false,
    }
}

#[cfg(not(feature = "local-runtime"))]
fn valid_credential_source(provider: &ProviderDefinition) -> bool {
    match (&provider.credential_ref, &provider.credential_source) {
        (Some(reference), None) => !reference.is_empty(),
        (None, Some(source)) => {
            !source.harness_type_id.is_empty() && !source.provider_id.is_empty()
        }
        (None, None) => true,
        _ => false,
    }
}
#[cfg(feature = "local-runtime")]
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
                    id: m.id.clone(),
                    nickname: m.nickname.clone(),
                    icon: m.icon.clone(),
                    max_output_tokens: m.max_output_tokens,
                    context_window: m.context_window,
                    reasoning_levels: m.reasoning_levels.clone(),
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
                        credential_ref: p.credential_ref.clone(),
                        credential_source: p
                            .credential_source
                            .as_ref()
                            .map(serde_json::to_value)
                            .transpose()?,
                        credential_generation: p.credential_generation,
                        models: p
                            .models
                            .iter()
                            .map(|m| velune_gateway::ProviderModelBinding {
                                model_id: m.model_id.clone(),
                                external_model_id: m.external_model_id.clone(),
                                chat_completions_output_limit_field: match m.pi_projection.as_ref().and_then(|projection| projection.completions_max_tokens_field) {
                                    Some(velune_agent_runtime::model_projection::CompletionsMaxTokensField::MaxTokens) => velune_gateway::ChatCompletionsOutputLimitField::MaxTokens,
                                    _ => velune_gateway::ChatCompletionsOutputLimitField::MaxCompletionTokens,
                                },
                            })
                            .collect(),
                    })
                })
                .collect::<Result<Vec<_>, crate::Error>>()?,
            routes: self
                .routes
                .iter()
                .map(|r| velune_gateway::Route {
                    model_id: r.model_id.clone(),
                    provider_id: r.provider_id.clone(),
                })
                .collect(),
            failover: velune_gateway::FailoverPolicy {
                mode: velune_gateway::FailoverMode::Disabled,
            },
        })
    }
}
