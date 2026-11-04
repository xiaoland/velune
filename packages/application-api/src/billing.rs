//! Provider-owned account reporting. These facts never grant model admission.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BillingService {
    OpenAi,
    Anthropic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BillingScope {
    Organization,
    Project { id: String },
    Workspace { id: String },
    ApiKey { id: String },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BillingConfiguration {
    pub service: BillingService,
    /// Explicitly asserted organization identity; not inferred from a model or brand.
    pub organization_id: String,
    pub scope: BillingScope,
}
impl BillingConfiguration {
    pub fn validate(&self) -> Result<(), &'static str> {
        let id = |s: &str| {
            !s.is_empty()
                && s.len() <= 256
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        };
        if !id(&self.organization_id) {
            return Err("请填写有效的组织标识");
        }
        match (&self.scope, self.service) {
            (BillingScope::Organization, _) => Ok(()),
            (BillingScope::Project { id: value }, BillingService::OpenAi)
            | (BillingScope::Workspace { id: value }, BillingService::Anthropic)
            | (BillingScope::ApiKey { id: value }, _)
                if id(value) =>
            {
                Ok(())
            }
            _ => Err("账单统计范围与服务不匹配，或范围标识无效"),
        }
    }
}
/// Only exact official HTTPS authorities qualify. Query targets are fixed separately.
pub fn billing_service(endpoint: &str) -> Option<BillingService> {
    let authority = endpoint
        .strip_prefix("https://")?
        .split(['/', '?', '#'])
        .next()?
        .to_ascii_lowercase();
    match authority.as_str() {
        "api.openai.com" => Some(BillingService::OpenAi),
        "api.anthropic.com" => Some(BillingService::Anthropic),
        _ => None,
    }
}
#[derive(Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BillingAuthenticationEdit {
    Keep,
    SetApiKey { value: String },
    Clear,
}
impl std::fmt::Debug for BillingAuthenticationEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BillingAuthenticationEdit([redacted])")
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BillingUsage {
    /// Includes cached and cache-creation input, as reported by each API.
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub requests: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BilledCost {
    pub amount: f64,
    pub currency: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BillingSpendLimit {
    pub amount: f64,
    pub currency: String,
    pub enforced: bool,
    /// The source's configured cycle, not a derived balance or spent percentage.
    pub interval: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BillingRateLimit {
    pub group_id: String,
    pub name: String,
    pub model_ids: Vec<String>,
    pub limit_type: String,
    pub value: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BillingReport {
    pub configuration: BillingConfiguration,
    pub starting_at_ms: i64,
    pub ending_at_ms: i64,
    pub observed_at_ms: i64,
    /// Upstream data refresh time; absent stays unknown, distinct from our fetch time.
    pub data_refreshed_at_ms: Option<i64>,
    pub usage: Option<BillingUsage>,
    pub costs: Option<Vec<BilledCost>>,
    pub spend_limit: Option<BillingSpendLimit>,
    pub rate_limits: Option<Vec<BillingRateLimit>>,
    pub problems: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderBillingStatus {
    pub configuration: Option<BillingConfiguration>,
    pub authentication: crate::QuotaAuthenticationDescription,
    pub report: Option<BillingReport>,
    pub refreshing: bool,
    pub stale: bool,
    pub problem: Option<String>,
}
