//! Core-owned gateway configuration and routing invariants.
//!
//! These types contain ordinary configuration only. Credential references are
//! opaque handles; resolving them and constructing an HTTP client belongs to
//! the platform composition root.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

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

impl GatewayConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
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

    pub fn model(&self, id: &str) -> Option<&ModelDefinition> {
        self.models.iter().find(|model| model.id == id)
    }

    /// Return the stable physical model identity used by a Pi catalog.
    ///
    /// The logical model remains the gateway request key. This identity binds
    /// the complete provider target without including bearer material or the
    /// Node executable path, so changing a route or account generation cannot
    /// reuse an incompatible Pi model entry.
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
            PiCredentialIdentity {
                auth_path,
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
struct PiBindingIdentity<'a> {
    logical_model_id: &'a str,
    provider_id: &'a str,
    protocol: &'static str,
    endpoint: &'a str,
    external_model_id: &'a str,
    credential_ref: Option<&'a str>,
    credential_source: Option<PiCredentialIdentity<'a>>,
    credential_generation: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PiCredentialIdentity<'a> {
    #[serde(rename = "authPath")]
    auth_path: Option<&'a str>,
    harness: &'a str,
    provider: &'a str,
    #[serde(rename = "sourceInstance")]
    source_instance: Option<&'a str>,
}

impl GatewayProtocol {
    fn identity_name(&self) -> &'static str {
        match self {
            Self::ChatCompletionsV1 => "chatCompletionsV1",
            Self::ResponsesV1 => "responsesV1",
            Self::MessagesV1 => "messagesV1",
        }
    }
}

pub(crate) fn credential_ready(provider: &ProviderDefinition) -> bool {
    matches!((&provider.credential_ref, &provider.credential_source), (Some(reference), None) if !reference.is_empty())
        || matches!((&provider.credential_ref, &provider.credential_source), (None, Some(source)) if valid_credential_source(provider) && source.provider_id == "openai")
}

fn valid_credential_source(provider: &ProviderDefinition) -> bool {
    match (&provider.credential_ref, &provider.credential_source) {
        (Some(reference), None) => !reference.is_empty(),
        (None, Some(source)) => {
            matches!(source.kind, CredentialSourceKind::Harness)
                && source.harness_type_id == "pi"
                && source.provider_id == "openai"
                && matches!(provider.protocol, GatewayProtocol::ResponsesV1)
                && provider.endpoint == "https://api.openai.com/v1"
                && source
                    .settings
                    .get("authPath")
                    .is_some_and(|value| Path::new(value).is_absolute())
                && source
                    .settings
                    .get("nodeBinary")
                    .is_some_and(|value| Path::new(value).is_absolute())
                && source
                    .source_instance_id
                    .as_deref()
                    .is_none_or(|value| !value.is_empty())
        }
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> GatewayConfig {
        GatewayConfig {
            id: "gateway".into(),
            name: "Fixture gateway".into(),
            models: vec![ModelDefinition {
                id: "model".into(),
                nickname: "Fixture".into(),
                icon: None,
                max_output_tokens: 128,
                context_window: Some(8192),
                reasoning_levels: vec!["standard".into()],
            }],
            providers: vec![ProviderDefinition {
                id: "provider".into(),
                name: "Fixture provider".into(),
                protocol: GatewayProtocol::ChatCompletionsV1,
                endpoint: "http://127.0.0.1:1/v1".into(),
                credential_ref: Some("fixture".into()),
                credential_source: None,
                credential_generation: 0,
                models: vec![ProviderModelBinding {
                    model_id: "model".into(),
                    external_model_id: "upstream-model".into(),
                }],
            }],
            routes: vec![Route {
                model_id: "model".into(),
                provider_id: "provider".into(),
            }],
            failover: FailoverPolicy {
                mode: FailoverMode::Disabled,
            },
        }
    }

    #[test]
    fn context_window_allows_drafts_but_rejects_invalid_limits() {
        let mut value = config();
        value.models[0].context_window = None;
        assert!(value.validate().is_ok());
        value.models[0].context_window = Some(0);
        assert!(value.validate().is_err());
        value.models[0].context_window = Some(64);
        assert!(value.validate().is_err());
        value.models[0].context_window = Some(128);
        assert!(value.validate().is_ok());
    }

    #[test]
    fn route_must_be_explicit_and_unique() {
        let mut value = config();
        assert!(value.validate().is_ok());
        value.routes.clear();
        assert!(value.validate().is_ok());
        assert_eq!(
            value.validate_dispatch("model").map(|_| ()),
            Err("model needs exactly one route")
        );
    }

    #[test]
    fn unsupported_protocol_is_rejected() {
        let mut value = config();
        value.providers[0].protocol = GatewayProtocol::MessagesV1;
        assert_eq!(
            value.validate(),
            Err("provider is unsupported or duplicated")
        );
    }
}
