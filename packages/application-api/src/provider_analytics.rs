//! Provider-owned usage reports combined without treating them as gateway requests.
use crate::{BilledCost, ProviderAccountCredits, ProviderAccountProfile, ProviderAccountResets};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ProviderAnalyticsSourceKind {
    ChatgptSubscription,
    OpenAiOrganization,
    AnthropicOrganization,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ProviderAnalyticsSourceState {
    Unsupported,
    Unconfigured,
    Unauthenticated,
    Refreshing,
    Expired,
    Available,
    PartialFailure,
    Failed,
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, uniffi::Enum,
)]
#[serde(rename_all = "camelCase")]
pub enum ProviderAnalyticsScope {
    SubscriptionAccount,
    Organization,
    Project,
    Workspace,
    ApiKey,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ProviderAnalyticsArea {
    AccountUsage,
    BankedResets,
    Profile,
    ModelUsage,
    OrganizationUsage,
    Costs,
    SpendLimit,
    RateLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAnalyticsFailure {
    pub area: ProviderAnalyticsArea,
    pub message: String,
}

/// Usage tokens reported by an API organization for one explicit report range.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAnalyticsUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub requests: u64,
    pub cached_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
}

/// A provider-reported model observation. Dates without explicit bounds
/// remain provider labels. Metrics from different units never substitute for one another.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAnalyticsModelUsage {
    pub model: String,
    pub date: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub input_tokens: Option<u64>,
    pub uncached_input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub requests: Option<u64>,
    pub turns: Option<u64>,
    pub credits: Option<f64>,
    pub relative_usage: Option<f64>,
}

/// One configured provider or a set of configurations that resolve to the same source.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAnalyticsSource {
    /// Stable only within this report; never contains authentication material.
    pub id: String,
    pub gateway_ids: Vec<String>,
    pub provider_ids: Vec<String>,
    pub provider_names: Vec<String>,
    pub kind: ProviderAnalyticsSourceKind,
    /// Opaque source identity used for duplicate detection; safe to compare, not to reverse.
    pub identity_key: Option<String>,
    /// Separate profile identity, used only to prevent repeated profile reads across workspaces.
    pub profile_identity_key: Option<String>,
    /// Configured organization identity for API reports; not inferred from credentials.
    pub organization_id: Option<String>,
    pub label: String,
    pub state: ProviderAnalyticsSourceState,
    pub scope: ProviderAnalyticsScope,
    pub scope_id: Option<String>,
    pub range_start_at_ms: Option<i64>,
    pub range_end_at_ms: Option<i64>,
    /// Local time this source result was observed; distinct from provider refresh time.
    pub observed_at_ms: Option<i64>,
    pub data_refreshed_at_ms: Option<i64>,
    pub credits_observed_at_ms: Option<i64>,
    pub reset_count_observed_at_ms: Option<i64>,
    pub reset_grants_observed_at_ms: Option<i64>,
    pub profile_observed_at_ms: Option<i64>,
    #[serde(default)]
    pub model_usage_observed_at_ms: Option<i64>,
    pub stale: bool,
    pub refreshing: bool,
    pub stale_areas: Vec<ProviderAnalyticsArea>,
    /// Present only when the source has a complete usage result; zero is a known value.
    pub usage: Option<ProviderAnalyticsUsage>,
    /// Native-grain rows for the model breakdown; None means unavailable or unreported.
    #[serde(default)]
    pub model_usage: Option<Vec<ProviderAnalyticsModelUsage>>,
    /// None means the source did not report costs; Some([]) is a confirmed empty result.
    pub costs: Option<Vec<BilledCost>>,
    pub profile: Option<ProviderAccountProfile>,
    pub credits: Option<ProviderAccountCredits>,
    pub banked_resets: Option<ProviderAccountResets>,
    pub failures: Vec<ProviderAnalyticsFailure>,
}

/// De-duplicated totals for one service, scope family, UTC month start, and selected organizations.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAnalyticsOrganizationTotal {
    pub services: Vec<crate::BillingService>,
    pub organization_ids: Vec<String>,
    pub provider_ids: Vec<String>,
    pub scope: ProviderAnalyticsScope,
    pub scope_ids: Vec<String>,
    pub range_start_at_ms: i64,
    /// Present only when every contributing source ended at the same instant.
    pub range_end_at_ms: Option<i64>,
    pub range_end_min_at_ms: i64,
    pub range_end_max_at_ms: i64,
    pub range_is_heterogeneous: bool,
    /// Known subtotal. Compare usage_source_count with source_count before treating it as complete.
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub usage_source_count: u32,
    /// Number of selected, non-overlapping source scopes contributing to this row.
    pub source_count: u32,
    /// Known same-currency subtotal; absent source costs are not treated as zero.
    pub costs: Vec<BilledCost>,
    pub cost_source_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAnalyticsDailyTotal {
    /// Provider date label; no timezone or local-day conversion is inferred.
    pub date: String,
    /// Known subtotal for sources that explicitly reported this date.
    pub tokens: u64,
    pub source_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAnalyticsChatgptProfileTotal {
    /// Unique profile identities with an account association in this report.
    pub profile_count: u32,
    /// Unique profiles that supplied profile statistics.
    pub profile_reported_count: u32,
    /// Known subtotal only; compare lifetime_source_count to profile_count.
    pub lifetime_tokens: Option<u64>,
    pub lifetime_source_count: u32,
    /// Unique profiles whose daily buckets were present, including a confirmed empty list.
    pub daily_reported_profile_count: u32,
    pub daily: Vec<ProviderAnalyticsDailyTotal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAnalyticsReport {
    pub observed_at_ms: i64,
    pub sources: Vec<ProviderAnalyticsSource>,
    pub chatgpt_profile_totals: ProviderAnalyticsChatgptProfileTotal,
    pub organization_totals: Vec<ProviderAnalyticsOrganizationTotal>,
    /// Primary source observations for the comparison table. All source
    /// configurations remain available in the provenance drill-down.
    #[serde(default)]
    pub comparison_source_ids: Option<Vec<String>>,
}
