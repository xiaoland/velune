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

/// An ordered set of provider models selected under one record key. The
/// gateway tries members in order and fails over before anything reaches the
/// caller; a provider model selected directly never fails over.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelRoute {
    pub record_key: String,
    pub name: String,
    /// Provider model record keys, in priority order.
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayConfig {
    pub id: String,
    pub name: String,
    pub providers: Vec<ProviderDefinition>,
    pub routes: Vec<ModelRoute>,
    pub failover: FailoverPolicy,
}

impl GatewayConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.is_empty() || self.name.is_empty() {
            return Err("gateway identity is required");
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
            for binding in &provider.models {
                if binding.record_key.is_empty()
                    || !models.insert(&binding.record_key)
                    || binding.provider_model_id.is_empty()
                    || binding.provider_model_id.chars().any(char::is_control)
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
        for route in &self.routes {
            if route.record_key.is_empty() || !models.insert(&route.record_key) {
                return Err("model route identity is invalid");
            }
            if route.name.trim().is_empty() || route.members.is_empty() {
                return Err("model route needs a name and members");
            }
            let mut members = BTreeSet::new();
            let mut protocol = None;
            for key in &route.members {
                let provider = self
                    .providers
                    .iter()
                    .find(|p| p.models.iter().any(|m| &m.record_key == key))
                    .ok_or("model route member is not a provider model")?;
                if !members.insert(key) {
                    return Err("model route member is duplicated");
                }
                // Same protocol is necessary for the request to stay as sent.
                if protocol
                    .replace(&provider.protocol)
                    .is_some_and(|p| p != &provider.protocol)
                {
                    return Err("model route members must share a protocol");
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
}

fn validate_limits(context: Option<u32>, output: Option<u32>) -> Result<(), &'static str> {
    if context == Some(0) || output == Some(0) {
        return Err("invalid provider model limits");
    }
    Ok(())
}
