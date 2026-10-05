//! Gateway configuration and routing invariants.
//!
//! These types contain ordinary configuration only. Credential references are
//! opaque handles; resolving them and constructing an HTTP client belongs to
//! the platform composition root.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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
    pub credential_ref: Option<String>,
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
            if model.record_key.is_empty()
                || model.nickname.is_empty()
                || model.max_output_tokens == Some(0)
                || model.context_window.is_some_and(|window| {
                    window == 0
                        || model
                            .max_output_tokens
                            .is_some_and(|maximum| maximum > window)
                })
                || !models.insert(&model.record_key)
            {
                return Err("invalid or duplicate model definition");
            }
        }
        let mut providers = BTreeMap::new();
        for provider in &self.providers {
            if provider.id.is_empty()
                || provider.name.is_empty()
                || provider.endpoint.is_empty()
                || matches!(provider.protocol, GatewayProtocol::MessagesV1)
                || provider
                    .credential_ref
                    .as_deref()
                    .is_some_and(str::is_empty)
                || providers.insert(&provider.id, provider).is_some()
            {
                return Err("provider is unsupported or duplicated");
            }
            let mut bindings = BTreeSet::new();
            for binding in &provider.models {
                if !models.contains(&binding.model_record_key)
                    || binding.provider_model_id.is_empty()
                    || binding.provider_model_id.chars().any(char::is_control)
                    || !bindings.insert(&binding.model_record_key)
                {
                    return Err("provider model binding is invalid");
                }
                validate_limits(binding.context_window, binding.max_output_tokens)?;
                if let Some(reasoning) = &binding.reasoning {
                    if reasoning.protocol != provider.protocol {
                        return Err("reasoning declaration must match provider protocol");
                    }
                    let mut levels = BTreeSet::new();
                    if reasoning
                        .levels
                        .iter()
                        .any(|level| level.is_empty() || !levels.insert(level))
                    {
                        return Err("invalid reasoning declaration");
                    }
                }
            }
        }
        let mut routes = BTreeMap::new();
        for route in &self.routes {
            let provider = providers
                .get(&route.provider_id)
                .ok_or("route provider is missing")?;
            if !models.contains(&route.model_record_key)
                || routes
                    .insert(&route.model_record_key, &route.provider_id)
                    .is_some()
                || !provider
                    .models
                    .iter()
                    .any(|binding| binding.model_record_key == route.model_record_key)
            {
                return Err("route must select one configured provider model");
            }
        }
        Ok(())
    }

    pub fn model(&self, id: &str) -> Option<&ModelDefinition> {
        self.models.iter().find(|model| model.record_key == id)
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

pub(crate) fn credential_ready(provider: &ProviderDefinition) -> bool {
    provider
        .credential_ref
        .as_deref()
        .is_some_and(|reference| !reference.is_empty())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolReasoning {
    pub protocol: GatewayProtocol,
    pub levels: Vec<String>,
}

fn validate_limits(context: Option<u32>, output: Option<u32>) -> Result<(), &'static str> {
    if context == Some(0)
        || output == Some(0)
        || context
            .zip(output)
            .is_some_and(|(context, output)| output > context)
    {
        return Err("invalid provider model limits");
    }
    Ok(())
}
