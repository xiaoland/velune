//! Provider-owned ChatGPT account observations. These facts never grant model admission.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ProviderAccountState {
    Unsupported,
    Unauthenticated,
    Refreshing,
    Expired,
    Available,
    PartialFailure,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ProviderAccountSource {
    Usage,
    Resets,
    Profile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountFailure {
    pub source: ProviderAccountSource,
    /// Sanitized summary only; upstream response bodies and credentials are never returned.
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountCredits {
    pub has_credits: Option<bool>,
    pub unlimited: Option<bool>,
    /// Provider value preserved as text; currency and decimal scale are not inferred.
    pub balance: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountResetGrant {
    pub id: String,
    pub reset_type: String,
    pub status: String,
    pub granted_at: Option<String>,
    pub expires_at: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountResets {
    /// Independently reported provider count, retained even if grant details fail.
    pub available_count: Option<i64>,
    /// None means details were not available; Some([]) means an empty list was confirmed.
    pub grants: Option<Vec<ProviderAccountResetGrant>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountDailyUsage {
    /// Provider-supplied date label; no timezone is inferred.
    pub date: String,
    pub tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountTopInvocation {
    pub kind: String,
    pub display_name: String,
    pub usage_count: i64,
    pub plugin_id: Option<String>,
    pub plugin_name: Option<String>,
    pub skill_id: Option<String>,
    pub skill_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountProfile {
    pub lifetime_tokens: Option<i64>,
    pub peak_daily_tokens: Option<i64>,
    pub longest_task_seconds: Option<i64>,
    pub current_streak_days: Option<i64>,
    pub longest_streak_days: Option<i64>,
    pub fast_mode_percent: Option<f64>,
    pub reasoning_effort: Option<String>,
    pub reasoning_effort_percent: Option<f64>,
    pub skills_explored: Option<i64>,
    pub total_skills_used: Option<i64>,
    pub total_threads: Option<i64>,
    /// None means the provider omitted buckets; Some([]) is an explicit empty result.
    pub daily: Option<Vec<ProviderAccountDailyUsage>>,
    /// None means the provider omitted the field; Some([]) is an explicit empty result.
    pub top_invocations: Option<Vec<ProviderAccountTopInvocation>>,
    /// Provider date/timestamp, distinct from local fetched_at.
    pub stats_as_of: Option<String>,
    pub generated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountStatus {
    pub gateway_id: String,
    pub provider_id: String,
    /// Opaque hash of authenticated subject + ChatGPT account scope.
    pub identity_key: Option<String>,
    /// Opaque subject hash; profile scope is undocumented and must not be merged across account scopes.
    pub profile_identity_key: Option<String>,
    pub state: ProviderAccountState,
    /// Last time any provider component returned reportable data.
    pub fetched_at: Option<String>,
    pub credits_observed_at: Option<String>,
    pub reset_count_observed_at: Option<String>,
    pub reset_grants_observed_at: Option<String>,
    pub profile_observed_at: Option<String>,
    pub refreshing: bool,
    pub stale: bool,
    /// Components whose own last successful observation is older than the cache TTL.
    pub stale_sources: Vec<ProviderAccountSource>,
    pub credits: Option<ProviderAccountCredits>,
    pub banked_resets: Option<ProviderAccountResets>,
    pub profile: Option<ProviderAccountProfile>,
    pub failures: Vec<ProviderAccountFailure>,
}
