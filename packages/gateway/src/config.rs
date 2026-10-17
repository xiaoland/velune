//! Gateway configuration and routing invariants.
//!
//! These types contain ordinary configuration only. Credential references are
//! opaque handles; resolving them and constructing an HTTP client belongs to
//! the platform composition root.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub use velune_ai_provider::config::RequestProfile;

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
pub enum ExecutionBackend {
    Http,
    ClaudeCli { binary: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderDefinition {
    pub id: String,
    pub name: String,
    pub protocol: GatewayProtocol,
    pub execution_backend: ExecutionBackend,
    pub request_profile: RequestProfile,
    pub endpoint: String,
    pub credential_ref: Option<String>,
    pub models: Vec<ProviderModel>,
    pub quota_limits: Vec<QuotaLimit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuotaLimit {
    pub window_id: String,
    pub max_used_percent: u8,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RoutingStrategy {
    Smart,
    Order,
    Rotate,
    Weight,
}

/// Provider models selected by one routing strategy and optional session binding.
/// Fail-over happens before delivery; directly selected models never fail over.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelRoute {
    pub record_key: String,
    pub name: String,
    /// Provider model record keys, in priority order.
    pub members: Vec<String>,
    pub strategy: RoutingStrategy,
    /// Keep an eligible successful member for the caller's explicit session.
    pub sticky: bool,
    /// Positive weights keyed by member record key; empty for other strategies.
    pub weights: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayConfig {
    pub id: String,
    pub name: String,
    pub providers: Vec<ProviderDefinition>,
    pub routes: Vec<ModelRoute>,
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
                || provider
                    .credential_ref
                    .as_deref()
                    .is_some_and(str::is_empty)
                || providers.insert(&provider.id, provider).is_some()
            {
                return Err("provider is unsupported or duplicated");
            }
            match &provider.execution_backend {
                ExecutionBackend::Http if provider.endpoint.is_empty() => {
                    return Err("HTTP backend requires an endpoint");
                }
                ExecutionBackend::ClaudeCli { binary }
                    if binary.trim().is_empty()
                        || binary.chars().any(char::is_control)
                        || !provider.endpoint.is_empty()
                        || provider.protocol != GatewayProtocol::MessagesV1
                        || provider.request_profile != RequestProfile::Native =>
                {
                    return Err(
                        "Claude CLI requires a binary, Messages, native profile and no HTTP endpoint",
                    );
                }
                _ => {}
            }
            if provider.request_profile == RequestProfile::CodexResponses {
                let endpoint = reqwest::Url::parse(&provider.endpoint)
                    .map_err(|_| "invalid profile endpoint")?;
                if provider.protocol != GatewayProtocol::ResponsesV1
                    || endpoint.scheme() != "https"
                    || endpoint.host_str() != Some("chatgpt.com")
                    || endpoint.port_or_known_default() != Some(443)
                    || endpoint.path().trim_end_matches('/') != "/backend-api/codex"
                    || endpoint.query().is_some()
                    || endpoint.fragment().is_some()
                    || !endpoint.username().is_empty()
                    || endpoint.password().is_some()
                {
                    return Err("Codex request profile requires its bound Responses endpoint");
                }
            }
            let mut windows = BTreeSet::new();
            if provider.quota_limits.iter().any(|limit| {
                limit.window_id.is_empty()
                    || limit.max_used_percent > 100
                    || !windows.insert(&limit.window_id)
            }) {
                return Err("invalid provider quota limits");
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
            if route.strategy == RoutingStrategy::Weight {
                if route.weights.len() != route.members.len()
                    || route
                        .members
                        .iter()
                        .any(|key| route.weights.get(key).is_none_or(|value| *value == 0))
                {
                    return Err("weighted route requires a positive weight for every member");
                }
            } else if !route.weights.is_empty() {
                return Err("weights only apply to weighted routes");
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
