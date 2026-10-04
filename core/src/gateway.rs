//! Host-owned gateway configuration and routing invariants.
//!
//! These types contain ordinary configuration only. Credential references are
//! opaque handles; resolving them and constructing an HTTP client belongs to
//! the platform composition root.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelDefinition {
    pub id: String,
    pub nickname: String,
    pub icon: Option<String>,
    pub max_output_tokens: u32,
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
pub struct ProviderDefinition {
    pub id: String,
    pub name: String,
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    pub credential_ref: Option<String>,
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
                || !matches!(provider.protocol, GatewayProtocol::ChatCompletionsV1)
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
        if !matches!(provider.protocol, GatewayProtocol::ChatCompletionsV1) {
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
                reasoning_levels: vec!["standard".into()],
            }],
            providers: vec![ProviderDefinition {
                id: "provider".into(),
                name: "Fixture provider".into(),
                protocol: GatewayProtocol::ChatCompletionsV1,
                endpoint: "http://127.0.0.1:1/v1".into(),
                credential_ref: None,
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
        value.providers[0].protocol = GatewayProtocol::ResponsesV1;
        assert_eq!(
            value.validate(),
            Err("provider is unsupported or duplicated")
        );
    }
}
