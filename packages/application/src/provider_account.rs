//! Read-only ChatGPT subscription account observations, kept separate from quota admission.
use crate::{config::GatewayConfig, provider_authentication::AuthenticationMaterial};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use velune_application_api::{
    ProviderAccountCredits, ProviderAccountDailyUsage, ProviderAccountFailure,
    ProviderAccountProfile, ProviderAccountResetGrant, ProviderAccountResets,
    ProviderAccountSource, ProviderAccountState, ProviderAccountStatus,
    ProviderAccountTopInvocation,
};
use velune_gateway::CredentialResolutionError;

const TTL: Duration = Duration::from_secs(15 * 60);
const RETRY_DELAY: Duration = Duration::from_secs(60);
const MAX_INFLIGHT: usize = 4;
type ProviderKey = (String, String);

#[derive(Clone)]
struct Capture {
    credential_id: String,
    generation: u64,
    cache_key: String,
}

#[derive(Clone)]
enum Binding {
    Unsupported,
    Unauthenticated,
    ChatGpt(Capture),
}

struct Entry {
    snapshot: Option<Snapshot>,
    failures: Vec<ProviderAccountFailure>,
    expired: bool,
    running: bool,
    force_pending: bool,
    fetched: Option<Instant>,
    fetched_at: Option<String>,
    retry_at: Instant,
    cancel: tokio::sync::watch::Sender<bool>,
}

#[derive(Default)]
struct State {
    bindings: BTreeMap<ProviderKey, Binding>,
    entries: BTreeMap<String, Entry>,
    stopped: bool,
    inflight: usize,
}

#[derive(Clone, Default)]
struct Snapshot {
    identity_key: Option<String>,
    profile_identity_key: Option<String>,
    credits: Option<ProviderAccountCredits>,
    resets: Option<ProviderAccountResets>,
    profile: Option<ProviderAccountProfile>,
    credits_observed: Option<Observation>,
    reset_count_observed: Option<Observation>,
    reset_grants_observed: Option<Observation>,
    profile_observed: Option<Observation>,
}

#[derive(Clone)]
struct Observation {
    at: String,
    instant: Instant,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct UsageResponse {
    credits: Option<UsageCredits>,
    rate_limit_reset_credits: Option<UsageResetCount>,
}
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct UsageCredits {
    has_credits: Option<bool>,
    unlimited: Option<bool>,
    balance: Option<String>,
}
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct UsageResetCount {
    available_count: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ResetResponse {
    available_count: Option<i64>,
    credits: Option<Vec<ResetGrant>>,
}
#[derive(Debug, Deserialize)]
struct ResetGrant {
    id: String,
    reset_type: String,
    status: String,
    #[serde(default)]
    granted_at: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ProfileResponse {
    stats: ProfileStats,
    metadata: ProfileMetadata,
}
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ProfileStats {
    lifetime_tokens: Option<i64>,
    peak_daily_tokens: Option<i64>,
    longest_running_turn_sec: Option<i64>,
    current_streak_days: Option<i64>,
    longest_streak_days: Option<i64>,
    daily_usage_buckets: Option<Vec<DailyBucket>>,
    top_invocations: Option<Vec<TopInvocation>>,
    fast_mode_usage_percentage: Option<f64>,
    most_used_reasoning_effort: Option<String>,
    most_used_reasoning_effort_percentage: Option<f64>,
    unique_skills_used: Option<i64>,
    total_skills_used: Option<i64>,
    total_threads: Option<i64>,
}
#[derive(Debug, Deserialize)]
#[serde(try_from = "RawDailyBucket")]
struct DailyBucket {
    start_date: String,
    tokens: i64,
}
#[derive(Debug, Deserialize)]
struct RawDailyBucket {
    start_date: String,
    tokens: i64,
}
impl TryFrom<RawDailyBucket> for DailyBucket {
    type Error = &'static str;

    fn try_from(bucket: RawDailyBucket) -> Result<Self, Self::Error> {
        if bucket.start_date.trim().is_empty() || bucket.tokens < 0 {
            return Err("invalid daily usage bucket");
        }
        Ok(Self {
            start_date: bucket.start_date,
            tokens: bucket.tokens,
        })
    }
}
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TopInvocation {
    #[serde(rename = "type")]
    kind: Option<String>,
    plugin_id: Option<String>,
    plugin_name: Option<String>,
    skill_id: Option<String>,
    skill_name: Option<String>,
    usage_count: Option<i64>,
}
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ProfileMetadata {
    stats_as_of: Option<String>,
    generated_at: Option<String>,
    stats_error: Option<String>,
}

#[derive(Default)]
struct Collected {
    snapshot: Snapshot,
    failures: Vec<ProviderAccountFailure>,
    expired: bool,
    succeeded: bool,
}

pub(crate) struct ProviderAccountService {
    home: PathBuf,
    state: Mutex<State>,
}
impl ProviderAccountService {
    pub(crate) fn new(home: PathBuf, gateways: &[GatewayConfig]) -> Arc<Self> {
        let service = Arc::new(Self {
            home,
            state: Mutex::new(State::default()),
        });
        service.configure(gateways);
        service
    }

    pub(crate) fn configure(&self, gateways: &[GatewayConfig]) {
        let bindings: BTreeMap<_, _> = gateways
            .iter()
            .flat_map(|gateway| {
                gateway.providers.iter().map(move |provider| {
                    let binding = match &provider.authentication.material {
                        AuthenticationMaterial::CodexSubscription {
                            credential_id,
                            configured: true,
                        } => Binding::ChatGpt(Capture {
                            credential_id: credential_id.clone(),
                            generation: provider.authentication.generation,
                            cache_key: cache_key(credential_id, provider.authentication.generation),
                        }),
                        AuthenticationMaterial::CodexSubscription { .. } => {
                            Binding::Unauthenticated
                        }
                        _ => Binding::Unsupported,
                    };
                    ((gateway.id.clone(), provider.id.clone()), binding)
                })
            })
            .collect();
        let live: BTreeSet<_> = bindings
            .values()
            .filter_map(|binding| match binding {
                Binding::ChatGpt(capture) => Some(capture.cache_key.clone()),
                _ => None,
            })
            .collect();
        let mut state = self.state.lock().expect("provider account state");
        state.bindings = bindings;
        state.entries.retain(|key, entry| {
            if live.contains(key) {
                true
            } else {
                entry.cancel.send_replace(true);
                false
            }
        });
    }

    pub(crate) fn stop(&self) {
        let mut state = self.state.lock().expect("provider account state");
        state.stopped = true;
        for entry in state.entries.values() {
            entry.cancel.send_replace(true);
        }
        state.entries.clear();
        state.bindings.clear();
    }

    pub(crate) fn query(
        self: &Arc<Self>,
        gateway_id: &str,
        provider_id: &str,
        force: bool,
    ) -> Result<ProviderAccountStatus, crate::Error> {
        let key = (gateway_id.to_owned(), provider_id.to_owned());
        let mut state = self.state.lock().expect("provider account state");
        if state.stopped {
            return Ok(empty_status(
                gateway_id,
                provider_id,
                ProviderAccountState::Failed,
                Some("账户报告服务已停止"),
            ));
        }
        let binding = state
            .bindings
            .get(&key)
            .cloned()
            .ok_or_else(|| crate::Error::invalid("provider id"))?;
        let capture = match binding {
            Binding::Unsupported => {
                return Ok(empty_status(
                    gateway_id,
                    provider_id,
                    ProviderAccountState::Unsupported,
                    None,
                ));
            }
            Binding::Unauthenticated => {
                return Ok(empty_status(
                    gateway_id,
                    provider_id,
                    ProviderAccountState::Unauthenticated,
                    None,
                ));
            }
            Binding::ChatGpt(capture) => capture,
        };
        let capacity = state.inflight < MAX_INFLIGHT;
        let (status, start, cancel, cancel_signal) = {
            let entry = state
                .entries
                .entry(capture.cache_key.clone())
                .or_insert_with(|| Entry {
                    snapshot: None,
                    failures: Vec::new(),
                    expired: false,
                    running: false,
                    force_pending: false,
                    fetched: None,
                    fetched_at: None,
                    retry_at: Instant::now(),
                    cancel: tokio::sync::watch::channel(false).0,
                });
            if force && !entry.expired {
                entry.force_pending = true;
            }
            let stale = entry_is_stale(entry);
            let refresh_requested = stale || entry.force_pending;
            let start = !entry.running
                && !entry.expired
                && capacity
                && Instant::now() >= entry.retry_at
                && refresh_requested;
            if start {
                entry.running = true;
                entry.force_pending = false;
                entry.failures.clear();
                entry.expired = false;
                entry.retry_at = Instant::now() + RETRY_DELAY;
            }
            let mut status = status_from_entry(gateway_id, provider_id, entry);
            // `refreshing` also covers a refresh request waiting for capacity or backoff.
            // Callers can keep polling the cache; no extra HTTP is started by the wait itself.
            if entry.running || (!entry.expired && refresh_requested && !start) {
                status.refreshing = true;
                status.state = ProviderAccountState::Refreshing;
            }
            (
                status,
                start,
                entry.cancel.subscribe(),
                entry.cancel.clone(),
            )
        };
        if !start {
            return Ok(status);
        }
        state.inflight += 1;
        let service = Arc::clone(self);
        let cache_identity = capture.cache_key.clone();
        drop(state);
        std::thread::spawn(move || {
            let result = collect(service.home.clone(), capture, cancel);
            service.finish(&cache_identity, &cancel_signal, result);
        });
        Ok(status)
    }

    fn finish(
        &self,
        identity: &str,
        cancel_signal: &tokio::sync::watch::Sender<bool>,
        result: Result<Collected, String>,
    ) {
        let mut state = self.state.lock().expect("provider account state");
        state.inflight = state.inflight.saturating_sub(1);
        if *cancel_signal.borrow() {
            return;
        }
        let Some(entry) = state.entries.get_mut(identity) else {
            return;
        };
        entry.running = false;
        match result {
            Ok(mut collected) => {
                let mut old = entry.snapshot.take().unwrap_or_default();
                if collected.snapshot.identity_key.is_none() {
                    collected.snapshot.identity_key = old.identity_key;
                }
                if collected.snapshot.profile_identity_key.is_none() {
                    collected.snapshot.profile_identity_key = old.profile_identity_key;
                }
                if collected.snapshot.credits.is_none() {
                    collected.snapshot.credits = old.credits;
                }
                if let Some(new_resets) = &mut collected.snapshot.resets {
                    if new_resets.available_count.is_none() {
                        new_resets.available_count = old
                            .resets
                            .as_ref()
                            .and_then(|resets| resets.available_count);
                    }
                    if new_resets.grants.is_none() {
                        new_resets.grants =
                            old.resets.as_mut().and_then(|resets| resets.grants.take());
                    }
                } else {
                    collected.snapshot.resets = old.resets;
                }
                if collected.snapshot.profile.is_none() {
                    collected.snapshot.profile = old.profile;
                }
                if collected.snapshot.credits_observed.is_none() {
                    collected.snapshot.credits_observed = old.credits_observed;
                }
                if collected.snapshot.reset_count_observed.is_none() {
                    collected.snapshot.reset_count_observed = old.reset_count_observed;
                }
                if collected.snapshot.reset_grants_observed.is_none() {
                    collected.snapshot.reset_grants_observed = old.reset_grants_observed;
                }
                if collected.snapshot.profile_observed.is_none() {
                    collected.snapshot.profile_observed = old.profile_observed;
                }
                if collected.succeeded {
                    let observed = observation_now();
                    entry.fetched = Some(observed.instant);
                    entry.fetched_at = Some(observed.at);
                    entry.snapshot = Some(collected.snapshot);
                } else {
                    entry.snapshot = Some(collected.snapshot);
                }
                entry.failures = collected.failures;
                entry.expired = collected.expired;
                if collected.expired {
                    entry.retry_at = Instant::now() + TTL;
                } else if !entry.failures.is_empty() {
                    entry.retry_at = Instant::now() + RETRY_DELAY;
                }
            }
            Err(message) => {
                entry.expired = false;
                entry.failures = vec![ProviderAccountFailure {
                    source: ProviderAccountSource::Usage,
                    message,
                }];
                entry.retry_at = Instant::now() + RETRY_DELAY;
            }
        }
    }
}

fn cache_key(credential_id: &str, generation: u64) -> String {
    let mut digest = Sha256::new();
    digest.update(b"velune-provider-account-cache-v1\0");
    digest.update(credential_id.as_bytes());
    digest.update([0]);
    digest.update(generation.to_le_bytes());
    format!("sha256:{:x}", digest.finalize())
}

fn empty_status(
    gateway_id: &str,
    provider_id: &str,
    state: ProviderAccountState,
    message: Option<&str>,
) -> ProviderAccountStatus {
    ProviderAccountStatus {
        gateway_id: gateway_id.into(),
        provider_id: provider_id.into(),
        identity_key: None,
        profile_identity_key: None,
        state,
        fetched_at: None,
        credits_observed_at: None,
        reset_count_observed_at: None,
        reset_grants_observed_at: None,
        profile_observed_at: None,
        refreshing: false,
        stale: false,
        stale_sources: Vec::new(),
        credits: None,
        banked_resets: None,
        profile: None,
        failures: message
            .map(|message| {
                vec![ProviderAccountFailure {
                    source: ProviderAccountSource::Usage,
                    message: message.into(),
                }]
            })
            .unwrap_or_default(),
    }
}

fn status_from_entry(gateway_id: &str, provider_id: &str, entry: &Entry) -> ProviderAccountStatus {
    let snapshot = entry.snapshot.clone().unwrap_or_default();
    let has_data =
        snapshot.credits.is_some() || snapshot.resets.is_some() || snapshot.profile.is_some();
    let expired = entry.expired;
    let state = if entry.running {
        ProviderAccountState::Refreshing
    } else if expired {
        ProviderAccountState::Expired
    } else if !entry.failures.is_empty() && has_data {
        ProviderAccountState::PartialFailure
    } else if !entry.failures.is_empty() {
        ProviderAccountState::Failed
    } else if has_data {
        ProviderAccountState::Available
    } else {
        ProviderAccountState::Refreshing
    };
    let mut stale_sources = Vec::new();
    if snapshot
        .credits_observed
        .as_ref()
        .is_some_and(|observed| observed.instant.elapsed() >= TTL)
    {
        stale_sources.push(ProviderAccountSource::Usage);
    }
    if snapshot
        .reset_count_observed
        .as_ref()
        .is_some_and(|observed| observed.instant.elapsed() >= TTL)
    {
        stale_sources.push(ProviderAccountSource::Usage);
    }
    if snapshot
        .reset_grants_observed
        .as_ref()
        .is_some_and(|observed| observed.instant.elapsed() >= TTL)
    {
        stale_sources.push(ProviderAccountSource::Resets);
    }
    if snapshot
        .profile_observed
        .as_ref()
        .is_some_and(|observed| observed.instant.elapsed() >= TTL)
    {
        stale_sources.push(ProviderAccountSource::Profile);
    }
    stale_sources.sort_by_key(|source| match source {
        ProviderAccountSource::Usage => 0,
        ProviderAccountSource::Resets => 1,
        ProviderAccountSource::Profile => 2,
    });
    stale_sources.dedup();
    ProviderAccountStatus {
        gateway_id: gateway_id.into(),
        provider_id: provider_id.into(),
        identity_key: snapshot.identity_key,
        profile_identity_key: snapshot.profile_identity_key,
        state,
        fetched_at: entry.fetched_at.clone(),
        credits_observed_at: snapshot
            .credits_observed
            .as_ref()
            .map(|observed| observed.at.clone()),
        reset_count_observed_at: snapshot
            .reset_count_observed
            .as_ref()
            .map(|observed| observed.at.clone()),
        reset_grants_observed_at: snapshot
            .reset_grants_observed
            .as_ref()
            .map(|observed| observed.at.clone()),
        profile_observed_at: snapshot
            .profile_observed
            .as_ref()
            .map(|observed| observed.at.clone()),
        refreshing: entry.running,
        stale: !stale_sources.is_empty(),
        stale_sources,
        credits: snapshot.credits,
        banked_resets: snapshot.resets,
        profile: snapshot.profile,
        failures: entry.failures.clone(),
    }
}

fn entry_is_stale(entry: &Entry) -> bool {
    entry.fetched.is_none_or(|time| time.elapsed() >= TTL)
        || entry.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .credits_observed
                .as_ref()
                .is_some_and(|observed| observed.instant.elapsed() >= TTL)
                || snapshot
                    .reset_count_observed
                    .as_ref()
                    .is_some_and(|observed| observed.instant.elapsed() >= TTL)
                || snapshot
                    .reset_grants_observed
                    .as_ref()
                    .is_some_and(|observed| observed.instant.elapsed() >= TTL)
                || snapshot
                    .profile_observed
                    .as_ref()
                    .is_some_and(|observed| observed.instant.elapsed() >= TTL)
        })
}

fn now_iso() -> String {
    use chrono::SecondsFormat;
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
fn observation_now() -> Observation {
    Observation {
        at: now_iso(),
        instant: Instant::now(),
    }
}

fn collect(
    home: PathBuf,
    capture: Capture,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<Collected, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "账户报告采集无法启动".to_owned())?;
    runtime.block_on(async move {
        tokio::select! {
            result = tokio::time::timeout(Duration::from_secs(60), collect_async(home, capture)) => {
                match result {
                    Ok(result) => result,
                    Err(_) => Err("账户报告采集超时".to_owned()),
                }
            }
            _ = cancel.changed() => Err("账户报告采集已取消".to_owned()),
        }
    })
}

async fn collect_async(home: PathBuf, capture: Capture) -> Result<Collected, String> {
    use crate::subscription_auth::{AccountDataEndpoint as Endpoint, account_identity};
    let mut collected = Collected::default();
    let identity = match account_identity(&home, &capture.credential_id, capture.generation) {
        Ok(identity) => identity,
        Err(CredentialResolutionError::AuthenticationExpired) => {
            collected.expired = true;
            collected.failures.push(failure(
                ProviderAccountSource::Usage,
                "ChatGPT 订阅授权已过期",
            ));
            return Ok(collected);
        }
        Err(CredentialResolutionError::StaleBinding) => {
            collected.failures.push(failure(
                ProviderAccountSource::Usage,
                "提供商认证已改变，请重新登录后刷新",
            ));
            return Ok(collected);
        }
        Err(_) => {
            collected.failures.push(failure(
                ProviderAccountSource::Usage,
                "订阅登录信息不可用，请重新登录",
            ));
            return Ok(collected);
        }
    };
    collected.snapshot.identity_key = Some(identity.account_scope_key);
    collected.snapshot.profile_identity_key = Some(identity.profile_subject_key);

    let usage = fetch_json(
        &home,
        &capture,
        Endpoint::Usage,
        ProviderAccountSource::Usage,
        &mut collected,
    )
    .await;
    if let Some(payload) = usage {
        match serde_json::from_value::<UsageResponse>(payload) {
            Ok(usage) => {
                let credits = usage.credits.map(|credits| ProviderAccountCredits {
                    has_credits: credits.has_credits,
                    unlimited: credits.unlimited,
                    balance: credits.balance,
                });
                let count = usage
                    .rate_limit_reset_credits
                    .and_then(|summary| summary.available_count);
                collected.snapshot.credits = credits;
                if count.is_some() {
                    collected.snapshot.resets = Some(ProviderAccountResets {
                        available_count: count,
                        grants: None,
                    });
                    collected.snapshot.reset_count_observed = Some(observation_now());
                }
                if collected.snapshot.credits.is_some() || count.is_some() {
                    if collected.snapshot.credits.is_some() {
                        collected.snapshot.credits_observed = Some(observation_now());
                    }
                    collected.succeeded = true;
                } else {
                    collected.failures.push(failure(
                        ProviderAccountSource::Usage,
                        "额度响应未报告 credits 余额或重置权益次数",
                    ));
                }
            }
            Err(_) => collected.failures.push(failure(
                ProviderAccountSource::Usage,
                "额度响应格式无效，请稍后刷新",
            )),
        }
    }

    let resets = fetch_json(
        &home,
        &capture,
        Endpoint::Resets,
        ProviderAccountSource::Resets,
        &mut collected,
    )
    .await;
    if let Some(payload) = resets {
        match serde_json::from_value::<ResetResponse>(payload) {
            Ok(resets) => {
                let grants = resets.credits.map(|credits| {
                    credits
                        .into_iter()
                        .map(|grant| ProviderAccountResetGrant {
                            id: grant.id,
                            reset_type: grant.reset_type,
                            status: grant.status,
                            granted_at: grant.granted_at,
                            expires_at: grant.expires_at,
                            title: grant.title,
                            description: grant.description,
                        })
                        .collect()
                });
                let old_count = collected
                    .snapshot
                    .resets
                    .as_ref()
                    .and_then(|resets| resets.available_count);
                collected.snapshot.resets = Some(ProviderAccountResets {
                    available_count: resets.available_count.or(old_count),
                    grants: grants.clone(),
                });
                if resets.available_count.is_some() {
                    collected.snapshot.reset_count_observed = Some(observation_now());
                }
                if grants.is_some() {
                    collected.snapshot.reset_grants_observed = Some(observation_now());
                }
                collected.succeeded |= resets.available_count.is_some() || grants.is_some();
                if grants.is_none() {
                    collected.failures.push(failure(
                        ProviderAccountSource::Resets,
                        "重置权益响应未报告明细",
                    ));
                }
            }
            Err(_) => collected.failures.push(failure(
                ProviderAccountSource::Resets,
                "重置权益响应格式无效，请稍后刷新",
            )),
        }
    }

    let profile = fetch_json(
        &home,
        &capture,
        Endpoint::Profile,
        ProviderAccountSource::Profile,
        &mut collected,
    )
    .await;
    if let Some(payload) = profile {
        match serde_json::from_value::<ProfileResponse>(payload) {
            Ok(profile) => {
                let has_stats_error = profile
                    .metadata
                    .stats_error
                    .as_ref()
                    .is_some_and(|message| !message.is_empty());
                if has_stats_error {
                    collected.failures.push(failure(
                        ProviderAccountSource::Profile,
                        "提供商暂时无法提供账户用量统计",
                    ));
                }
                let has_profile_data = profile.stats.lifetime_tokens.is_some()
                    || profile.stats.peak_daily_tokens.is_some()
                    || profile.stats.longest_running_turn_sec.is_some()
                    || profile.stats.current_streak_days.is_some()
                    || profile.stats.longest_streak_days.is_some()
                    || profile.stats.daily_usage_buckets.is_some()
                    || profile.stats.top_invocations.is_some()
                    || profile.stats.fast_mode_usage_percentage.is_some()
                    || profile.stats.most_used_reasoning_effort.is_some()
                    || profile
                        .stats
                        .most_used_reasoning_effort_percentage
                        .is_some()
                    || profile.stats.unique_skills_used.is_some()
                    || profile.stats.total_skills_used.is_some()
                    || profile.stats.total_threads.is_some()
                    || profile.metadata.stats_as_of.is_some()
                    || profile.metadata.generated_at.is_some();
                let daily = profile.stats.daily_usage_buckets.map(|buckets| {
                    buckets
                        .into_iter()
                        .map(|bucket| ProviderAccountDailyUsage {
                            date: bucket.start_date,
                            tokens: bucket.tokens,
                        })
                        .collect()
                });
                let top_invocations = profile.stats.top_invocations.map(|items| {
                    items
                        .into_iter()
                        .map(|item| {
                            let kind = item.kind.unwrap_or_else(|| "unknown".into());
                            let display_name = item
                                .skill_name
                                .as_ref()
                                .or(item.plugin_name.as_ref())
                                .cloned()
                                .unwrap_or_else(|| "Unknown".into());
                            ProviderAccountTopInvocation {
                                kind,
                                display_name,
                                usage_count: item.usage_count.unwrap_or(0),
                                plugin_id: item.plugin_id,
                                plugin_name: item.plugin_name,
                                skill_id: item.skill_id,
                                skill_name: item.skill_name,
                            }
                        })
                        .collect()
                });
                if has_profile_data && !has_stats_error {
                    collected.snapshot.profile = Some(ProviderAccountProfile {
                        lifetime_tokens: profile.stats.lifetime_tokens,
                        peak_daily_tokens: profile.stats.peak_daily_tokens,
                        longest_task_seconds: profile.stats.longest_running_turn_sec,
                        current_streak_days: profile.stats.current_streak_days,
                        longest_streak_days: profile.stats.longest_streak_days,
                        fast_mode_percent: profile.stats.fast_mode_usage_percentage,
                        reasoning_effort: profile.stats.most_used_reasoning_effort,
                        reasoning_effort_percent: profile
                            .stats
                            .most_used_reasoning_effort_percentage,
                        skills_explored: profile.stats.unique_skills_used,
                        total_skills_used: profile.stats.total_skills_used,
                        total_threads: profile.stats.total_threads,
                        daily,
                        top_invocations,
                        stats_as_of: profile.metadata.stats_as_of,
                        generated_at: profile.metadata.generated_at,
                    });
                    collected.snapshot.profile_observed = Some(observation_now());
                    collected.succeeded = true;
                } else {
                    collected.failures.push(failure(
                        ProviderAccountSource::Profile,
                        "账户响应未报告可用统计",
                    ));
                }
            }
            Err(_) => collected.failures.push(failure(
                ProviderAccountSource::Profile,
                "账户用量响应格式无效，请稍后刷新",
            )),
        }
    }
    Ok(collected)
}

async fn fetch_json(
    home: &Path,
    capture: &Capture,
    endpoint: crate::subscription_auth::AccountDataEndpoint,
    source: ProviderAccountSource,
    collected: &mut Collected,
) -> Option<Value> {
    match crate::subscription_auth::read_provider_account_data(
        home.to_path_buf(),
        capture.credential_id.clone(),
        capture.generation,
        endpoint,
    )
    .await
    {
        Ok(value) => Some(value),
        Err(CredentialResolutionError::AuthenticationExpired) => {
            collected.expired = true;
            collected
                .failures
                .push(failure(source, "ChatGPT 订阅授权已过期"));
            None
        }
        Err(CredentialResolutionError::StaleBinding) => {
            collected
                .failures
                .push(failure(source, "提供商认证已改变，请重新登录后刷新"));
            None
        }
        Err(_) => {
            collected
                .failures
                .push(failure(source, source_unavailable(source)));
            None
        }
    }
}

fn source_unavailable(source: ProviderAccountSource) -> &'static str {
    match source {
        ProviderAccountSource::Usage => "暂时无法读取额度数据，请稍后刷新",
        ProviderAccountSource::Resets => "暂时无法读取重置权益明细，请稍后刷新",
        ProviderAccountSource::Profile => "暂时无法读取账户用量，请稍后刷新",
    }
}

fn failure(source: ProviderAccountSource, message: &str) -> ProviderAccountFailure {
    ProviderAccountFailure {
        source,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription_auth::{AccountDataEndpoint, read_provider_account_data_at_for_test};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn profile_fixture_maps_root_stats_metadata_and_raw_daily_dates() {
        let payload = serde_json::json!({
            "stats": {
                "lifetime_tokens": 123456,
                "peak_daily_tokens": 45678,
                "longest_running_turn_sec": 7200,
                "current_streak_days": 3,
                "longest_streak_days": 9,
                "daily_usage_buckets": [{"start_date":"2026-11-15","tokens":321}],
                "fast_mode_usage_percentage": 12.5,
                "most_used_reasoning_effort": "medium",
                "most_used_reasoning_effort_percentage": 55.0,
                "unique_skills_used": 2,
                "total_skills_used": 6,
                "total_threads": 8,
                "top_invocations": [{"type":"skill","skill_name":"sample","usage_count":4}]
            },
            "metadata": {"stats_as_of":"2026-11-15","generated_at":"2026-11-16T10:00:00Z","stats_error":null}
        });
        let profile: ProfileResponse = serde_json::from_value(payload).unwrap();
        assert_eq!(profile.stats.lifetime_tokens, Some(123456));
        assert_eq!(
            profile.stats.daily_usage_buckets.as_ref().unwrap()[0].start_date,
            "2026-11-15"
        );
        assert_eq!(profile.metadata.stats_as_of.as_deref(), Some("2026-11-15"));
        assert_eq!(
            profile.metadata.generated_at.as_deref(),
            Some("2026-11-16T10:00:00Z")
        );
        assert_eq!(profile.stats.daily_usage_buckets.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn missing_or_partial_daily_bucket_invalidates_profile_payload() {
        let missing_date = serde_json::from_value::<ProfileResponse>(serde_json::json!({
            "stats": {"lifetime_tokens": 10, "daily_usage_buckets": [{"tokens": 4}]},
            "metadata": {"generated_at":"2026-11-16T10:00:00Z"}
        }));
        let missing_tokens = serde_json::from_value::<ProfileResponse>(serde_json::json!({
            "stats": {"lifetime_tokens": 10, "daily_usage_buckets": [{"start_date":"2026-11-15"}]},
            "metadata": {"generated_at":"2026-11-16T10:00:00Z"}
        }));
        let empty_date = serde_json::from_value::<ProfileResponse>(serde_json::json!({
            "stats": {"lifetime_tokens": 10, "daily_usage_buckets": [{"start_date":"","tokens":4}]},
            "metadata": {"generated_at":"2026-11-16T10:00:00Z"}
        }));
        let negative_tokens = serde_json::from_value::<ProfileResponse>(serde_json::json!({
            "stats": {"lifetime_tokens": 10, "daily_usage_buckets": [{"start_date":"2026-11-15","tokens":-1}]},
            "metadata": {"generated_at":"2026-11-16T10:00:00Z"}
        }));
        let mixed = serde_json::from_value::<ProfileResponse>(serde_json::json!({
            "stats": {"daily_usage_buckets": [
                {"start_date":"2026-11-15","tokens":4}, {"tokens":5}
            ]}, "metadata":{}
        }));
        assert!(missing_date.is_err());
        assert!(missing_tokens.is_err());
        assert!(empty_date.is_err());
        assert!(negative_tokens.is_err());
        assert!(mixed.is_err());
    }

    #[test]
    fn empty_stats_and_metadata_are_not_a_report() {
        let profile: ProfileResponse = serde_json::from_value(serde_json::json!({
            "stats": {}, "metadata": {}
        }))
        .unwrap();
        assert!(profile.stats.lifetime_tokens.is_none());
        assert!(profile.stats.daily_usage_buckets.is_none());
        assert!(profile.metadata.stats_as_of.is_none());
    }

    #[test]
    fn reset_count_survives_missing_detail_rows_and_source_count_is_not_recomputed() {
        let usage: UsageResponse = serde_json::from_value(serde_json::json!({
            "rate_limit_reset_credits": {"available_count": 3}
        }))
        .unwrap();
        let details: Result<ResetResponse, _> =
            serde_json::from_value(serde_json::json!({"credits": null}));
        let count = usage.rate_limit_reset_credits.unwrap().available_count;
        assert_eq!(count, Some(3));
        assert!(details.is_ok());
        let details = details.unwrap();
        assert!(details.credits.is_none());
        let resets = ProviderAccountResets {
            available_count: count,
            grants: None,
        };
        assert_eq!(resets.available_count, Some(3));
        assert_eq!(resets.grants, None);
    }

    #[test]
    fn switcher_reset_fixture_keeps_expired_and_redeemed_rows_without_recounting() {
        let response: ResetResponse = serde_json::from_value(serde_json::json!({
            "available_count": 2,
            "credits": [
                {"id":"expired","reset_type":"codex_rate_limits","status":"available","expires_at":"2026-01-01T00:00:00Z"},
                {"id":"used","reset_type":"codex_rate_limits","status":"redeemed"}
            ]
        })).unwrap();
        assert_eq!(response.available_count, Some(2));
        assert_eq!(response.credits.unwrap().len(), 2);
    }

    #[test]
    fn successful_usage_does_not_refresh_retained_profile_timestamp() {
        let service = ProviderAccountService {
            home: PathBuf::from("/synthetic"),
            state: Mutex::new(State::default()),
        };
        let cancel = tokio::sync::watch::channel(false).0;
        let (sender, _) = tokio::sync::watch::channel(false);
        let old_profile_observation = Observation {
            at: "2026-01-01T00:00:00Z".into(),
            instant: Instant::now() - TTL - Duration::from_secs(1),
        };
        service.state.lock().unwrap().entries.insert(
            "scope".into(),
            Entry {
                snapshot: Some(Snapshot {
                    profile: Some(ProviderAccountProfile {
                        lifetime_tokens: Some(10),
                        peak_daily_tokens: None,
                        longest_task_seconds: None,
                        current_streak_days: None,
                        longest_streak_days: None,
                        fast_mode_percent: None,
                        reasoning_effort: None,
                        reasoning_effort_percent: None,
                        skills_explored: None,
                        total_skills_used: None,
                        total_threads: None,
                        daily: None,
                        top_invocations: None,
                        stats_as_of: None,
                        generated_at: None,
                    }),
                    profile_observed: Some(old_profile_observation),
                    ..Snapshot::default()
                }),
                failures: Vec::new(),
                expired: false,
                running: true,
                force_pending: false,
                fetched: None,
                fetched_at: None,
                retry_at: Instant::now(),
                cancel,
            },
        );
        service.state.lock().unwrap().inflight = 1;
        let now = observation_now();
        service.finish(
            "scope",
            &sender,
            Ok(Collected {
                snapshot: Snapshot {
                    credits: Some(ProviderAccountCredits {
                        has_credits: Some(true),
                        unlimited: Some(false),
                        balance: Some("12.5".into()),
                    }),
                    credits_observed: Some(now),
                    ..Snapshot::default()
                },
                failures: vec![failure(
                    ProviderAccountSource::Profile,
                    "暂时无法读取账户用量，请稍后刷新",
                )],
                expired: false,
                succeeded: true,
            }),
        );
        let state = service.state.lock().unwrap();
        let status = status_from_entry("g", "p", &state.entries["scope"]);
        assert_eq!(status.credits.unwrap().balance.as_deref(), Some("12.5"));
        assert_eq!(status.profile.as_ref().unwrap().lifetime_tokens, Some(10));
        assert_eq!(
            status.profile_observed_at.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
        assert!(status.stale);
        assert_eq!(status.stale_sources, vec![ProviderAccountSource::Profile]);
    }

    #[test]
    fn canceled_collector_cannot_write_after_binding_retirement() {
        let service = ProviderAccountService {
            home: PathBuf::from("/synthetic"),
            state: Mutex::new(State::default()),
        };
        let (cancel, _) = tokio::sync::watch::channel(false);
        cancel.send_replace(true);
        let (job, _) = tokio::sync::watch::channel(false);
        service.state.lock().unwrap().entries.insert(
            "scope".into(),
            Entry {
                snapshot: None,
                failures: Vec::new(),
                expired: false,
                running: true,
                force_pending: false,
                fetched: None,
                fetched_at: None,
                retry_at: Instant::now(),
                cancel: job,
            },
        );
        service.state.lock().unwrap().inflight = 1;
        service.finish("scope", &cancel, Err("late response".into()));
        let state = service.state.lock().unwrap();
        assert!(state.entries["scope"].running);
        assert!(state.entries["scope"].failures.is_empty());
        assert_eq!(state.inflight, 0);
    }

    #[test]
    fn capacity_wait_is_pollable_and_retries_when_a_slot_opens() {
        let home = std::env::temp_dir().join(format!(
            "velune-provider-account-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let service = Arc::new(ProviderAccountService {
            home: home.clone(),
            state: Mutex::new(State::default()),
        });
        {
            let mut state = service.state.lock().unwrap();
            state.bindings.insert(
                ("g".into(), "p".into()),
                Binding::ChatGpt(Capture {
                    credential_id: "missing-session".into(),
                    generation: 1,
                    cache_key: "scope".into(),
                }),
            );
            state.inflight = MAX_INFLIGHT;
            for index in 0..MAX_INFLIGHT {
                state.entries.insert(
                    format!("occupied-{index}"),
                    Entry {
                        snapshot: None,
                        failures: Vec::new(),
                        expired: false,
                        running: true,
                        force_pending: false,
                        fetched: None,
                        fetched_at: None,
                        retry_at: Instant::now(),
                        cancel: tokio::sync::watch::channel(false).0,
                    },
                );
            }
        }
        let waiting = service.query("g", "p", true).unwrap();
        assert!(waiting.refreshing);
        assert_eq!(waiting.state, ProviderAccountState::Refreshing);
        assert!(!service.state.lock().unwrap().entries["scope"].running);
        assert!(service.state.lock().unwrap().entries["scope"].force_pending);

        service.state.lock().unwrap().inflight = MAX_INFLIGHT - 1;
        let started = service.query("g", "p", false).unwrap();
        assert!(started.refreshing);
        assert!(service.state.lock().unwrap().entries["scope"].running);
        assert!(!service.state.lock().unwrap().entries["scope"].force_pending);

        for _ in 0..100 {
            let state = service.state.lock().unwrap();
            let entry = &state.entries["scope"];
            if !entry.running && !entry.failures.is_empty() {
                assert_eq!(entry.failures[0].source, ProviderAccountSource::Usage);
                let _ = std::fs::remove_dir_all(&home);
                return;
            }
            drop(state);
            std::thread::sleep(Duration::from_millis(10));
        }
        let state = service.state.lock().unwrap();
        let entry = &state.entries["scope"];
        let detail = format!(
            "running={}, expired={}, inflight={}, failures={}, force_pending={}",
            entry.running,
            entry.expired,
            state.inflight,
            entry.failures.len(),
            entry.force_pending,
        );
        drop(state);
        let _ = std::fs::remove_dir_all(&home);
        panic!("the pending refresh did not complete after capacity opened: {detail}");
    }

    #[tokio::test]
    async fn synthetic_http_is_get_scoped_and_does_not_follow_redirects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut request = vec![0; 4096];
            let count = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..count]).to_ascii_lowercase();
            assert!(
                request.starts_with("get /synthetic http/1.1"),
                "{request:?}"
            );
            assert!(request.contains("authorization: bearer synthetic-token"));
            assert!(request.contains("chatgpt-account-id: synthetic-account"));
            assert!(request.contains("accept: application/json"));
            assert!(request.contains("openai-beta: codex-1"));
            assert!(request.contains("originator: codex desktop"));
            assert!(request.contains("user-agent: codex-cli/1.0.0"));
            observed.fetch_add(1, Ordering::SeqCst);
            let body = r#"{"available_count":3,"credits":[]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let payload = read_provider_account_data_at_for_test(
            &format!("http://{addr}/synthetic"),
            AccountDataEndpoint::Resets,
            "synthetic-token",
            "synthetic-account",
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(payload["available_count"], 3);
    }

    #[test]
    fn cache_key_is_opaque_and_generation_scoped() {
        let a = cache_key("session-id", 1);
        assert!(a.starts_with("sha256:"));
        assert!(!a.contains("session-id"));
        assert_ne!(a, cache_key("session-id", 2));
        assert_ne!(a, cache_key("another", 1));
    }
}
