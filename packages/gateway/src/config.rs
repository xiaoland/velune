//! Gateway configuration and routing invariants.
//!
//! These types contain ordinary configuration only. Credential references are
//! opaque handles; resolving them and constructing an HTTP client belongs to
//! the platform composition root.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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

impl GatewayConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.is_empty() || self.name.is_empty() {
            return Err("gateway identity is required");
        }
        if !matches!(self.failover.mode, FailoverMode::Disabled) {
            return Err("gateway failover mode is unsupported");
        }
        let mut models = BTreeSet::new();
        let mut providers = BTreeMap::new();
        for provider in &self.providers {
            if provider.id.is_empty()
                || provider.name.is_empty()
                || provider.endpoint.is_empty()
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
                if binding.record_key.is_empty()
                    || !models.insert(&binding.record_key)
                    || binding.provider_model_id.is_empty()
                    || binding.provider_model_id.chars().any(char::is_control)
                    || !bindings.insert(&binding.record_key)
                {
                    return Err("provider model binding is invalid");
                }
                validate_limits(binding.context_window, binding.max_output_tokens)?;
                if let Some(levels) = &binding.reasoning_levels {
                    let mut unique = BTreeSet::new();
                    if levels
                        .iter()
                        .any(|level| level.is_empty() || !unique.insert(level))
                    {
                        return Err("invalid reasoning declaration");
                    }
                }
            }
        }
        Ok(())
    }
    pub fn model(&self, key: &str) -> Option<&ProviderModel> {
        self.providers
            .iter()
            .flat_map(|p| &p.models)
            .find(|m| m.record_key == key)
    }
    pub fn validate_dispatch(&self, key: &str) -> Result<&ProviderDefinition, &'static str> {
        self.validate()?;
        self.providers
            .iter()
            .find(|p| p.models.iter().any(|m| m.record_key == key))
            .ok_or("provider model is not configured")
    }
}

pub(crate) fn credential_ready(provider: &ProviderDefinition) -> bool {
    provider
        .credential_ref
        .as_deref()
        .is_some_and(|reference| !reference.is_empty())
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
