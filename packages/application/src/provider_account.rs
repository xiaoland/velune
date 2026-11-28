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
    ComposerQuotaCredits, ProviderAccountCredits, ProviderAccountDailyUsage,
    ProviderAccountFailure, ProviderAccountProfile, ProviderAccountResetGrant,
    ProviderAccountResets, ProviderAccountSource, ProviderAccountState, ProviderAccountStatus,
    ProviderAccountTopInvocation,
};
use velune_gateway::CredentialResolutionError;

mod model_usage;

const TTL: Duration = Duration::from_secs(15 * 60);
const RETRY_DELAY: Duration = Duration::from_secs(60);
const MAX_INFLIGHT: usize = 4;
type ProviderKey = (String, String);
type EntryKey = (String, CollectionScope);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CollectionScope {
    Credits,
    Full,
    Analysis,
}

impl CollectionScope {
    fn primary_source(self) -> ProviderAccountSource {
        match self {
            Self::Analysis => ProviderAccountSource::AnalyticsUsage,
            Self::Credits | Self::Full => ProviderAccountSource::Usage,
        }
    }
}

#[derive(Clone)]
struct Capture {
    credential_id: String,
    generation: u64,
    codex_client_version: String,
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
    entries: BTreeMap<EntryKey, Entry>,
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
    model_usage: model_usage::Snapshot,
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
struct CreditsResponse {
    credits: Option<UsageCredits>,
}
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct UsageCredits {
    has_credits: Option<bool>,
    unlimited: Option<bool>,
    balance: Option<UsageBalance>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum UsageBalance {
    Text(String),
    Number(serde_json::Number),
}

impl From<UsageBalance> for String {
    fn from(balance: UsageBalance) -> Self {
        match balance {
            UsageBalance::Text(text) => text,
            UsageBalance::Number(number) => number.to_string(),
        }
    }
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
                        } => {
                            let generation = provider.authentication.generation;
                            let codex_client_version = provider.effective_codex_client_version();
                            Binding::ChatGpt(Capture {
                                credential_id: credential_id.clone(),
                                generation,
                                codex_client_version: codex_client_version.to_owned(),
                                cache_key: cache_key(
                                    credential_id,
                                    generation,
                                    codex_client_version,
                                ),
                            })
                        }
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
        state.entries.retain(|(key, _), entry| {
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
        self.query_account(gateway_id, provider_id, force, false)
    }

    pub(crate) fn query_analysis(
        self: &Arc<Self>,
        gateway_id: &str,
        provider_id: &str,
        force: bool,
    ) -> Result<ProviderAccountStatus, crate::Error> {
        self.query_account(gateway_id, provider_id, force, true)
    }

    fn query_account(
        self: &Arc<Self>,
        gateway_id: &str,
        provider_id: &str,
        force: bool,
        include_analytics: bool,
    ) -> Result<ProviderAccountStatus, crate::Error> {
        let mut state = self.state.lock().expect("provider account state");
        if state.stopped {
            return Ok(empty_status(
                gateway_id,
                provider_id,
                ProviderAccountState::Failed,
                Some("账户报告服务已停止"),
            ));
        }
        let binding = account_binding(&state, gateway_id, provider_id)?;
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
        let status = |entry: &Entry, refreshing| {
            let mut status = status_from_entry(gateway_id, provider_id, entry);
            if refreshing {
                status.refreshing = true;
                status.state = ProviderAccountState::Refreshing;
            }
            status
        };
        let mut full = self.query_capture(
            &mut state,
            capture.clone(),
            CollectionScope::Full,
            force,
            status,
        );
        if include_analytics {
            let analytics = self.query_capture(
                &mut state,
                capture,
                CollectionScope::Analysis,
                force,
                status,
            );
            merge_analytics_status(&mut full, analytics);
        }
        Ok(full)
    }

    pub(crate) fn query_credits(
        self: &Arc<Self>,
        gateway_id: &str,
        provider_id: &str,
        force: bool,
    ) -> Result<Option<(ComposerQuotaCredits, Option<String>)>, crate::Error> {
        let mut state = self.state.lock().expect("provider account state");
        if state.stopped {
            return Ok(None);
        }
        let Binding::ChatGpt(capture) = account_binding(&state, gateway_id, provider_id)? else {
            return Ok(None);
        };
        Ok(Some(self.query_capture(
            &mut state,
            capture,
            CollectionScope::Credits,
            force,
            credits_from_entry,
        )))
    }

    fn query_capture<T>(
        self: &Arc<Self>,
        state: &mut State,
        capture: Capture,
        scope: CollectionScope,
        force: bool,
        read: impl FnOnce(&Entry, bool) -> T,
    ) -> T {
        let capacity = state.inflight < MAX_INFLIGHT;
        let entry_key = (capture.cache_key.clone(), scope);
        let from_full = match scope {
            CollectionScope::Credits => fresh_full_credits(state, &capture.cache_key),
            CollectionScope::Full | CollectionScope::Analysis => None,
        };
        let (result, worker) = {
            let entry = state
                .entries
                .entry(entry_key.clone())
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
            if let Some((snapshot, observed)) = from_full
                && !entry.running
                && !entry.expired
                && entry
                    .fetched
                    .is_none_or(|fetched| fetched < observed.instant)
            {
                entry.snapshot = Some(snapshot);
                entry.fetched = Some(observed.instant);
                entry.fetched_at = Some(observed.at);
                entry.failures.clear();
            }
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
                if scope == CollectionScope::Full {
                    entry.failures.clear();
                }
                entry.expired = false;
                entry.retry_at = Instant::now() + RETRY_DELAY;
            }
            // `refreshing` also covers a refresh request waiting for capacity or backoff.
            // Callers can keep polling the cache; no extra HTTP is started by the wait itself.
            let waiting_to_retry_failed_analysis = scope == CollectionScope::Analysis
                && !entry.failures.is_empty()
                && !entry.force_pending
                && Instant::now() < entry.retry_at;
            let refreshing = entry.running
                || (!entry.expired && refresh_requested && !waiting_to_retry_failed_analysis);
            (
                read(entry, refreshing),
                start.then(|| (entry.cancel.subscribe(), entry.cancel.clone())),
            )
        };
        if let Some((cancel, cancel_signal)) = worker {
            state.inflight += 1;
            let service = Arc::clone(self);
            std::thread::spawn(move || {
                let result = collect(service.home.clone(), capture, scope, cancel);
                service.finish(&entry_key, &cancel_signal, result);
            });
        }
        result
    }

    fn finish(
        &self,
        key: &EntryKey,
        cancel_signal: &tokio::sync::watch::Sender<bool>,
        result: Result<Collected, String>,
    ) {
        let mut state = self.state.lock().expect("provider account state");
        state.inflight = state.inflight.saturating_sub(1);
        if *cancel_signal.borrow() {
            return;
        }
        let Some(entry) = state.entries.get_mut(key) else {
            return;
        };
        entry.running = false;
        match result {
            Ok(mut collected) => {
                if collected.succeeded {
                    let observed = match key.1 {
                        CollectionScope::Credits => collected.snapshot.credits_observed.clone(),
                        CollectionScope::Full | CollectionScope::Analysis => None,
                    }
                    .unwrap_or_else(observation_now);
                    entry.fetched = Some(observed.instant);
                    entry.fetched_at = Some(observed.at);
                }
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
                collected
                    .snapshot
                    .model_usage
                    .retain_missing(old.model_usage);
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
                entry.snapshot = Some(collected.snapshot);
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
                    source: key.1.primary_source(),
                    message,
                }];
                entry.retry_at = Instant::now() + RETRY_DELAY;
            }
        }
    }
}

fn account_binding(
    state: &State,
    gateway_id: &str,
    provider_id: &str,
) -> Result<Binding, crate::Error> {
    state
        .bindings
        .get(&(gateway_id.to_owned(), provider_id.to_owned()))
        .cloned()
        .ok_or_else(|| crate::Error::invalid("provider id"))
}

fn fresh_full_credits(state: &State, cache_key: &str) -> Option<(Snapshot, Observation)> {
    let entry = state
        .entries
        .get(&(cache_key.into(), CollectionScope::Full))?;
    if entry.running
        || entry
            .failures
            .iter()
            .any(|failure| failure.source == ProviderAccountSource::Usage)
    {
        return None;
    }
    let snapshot = entry.snapshot.as_ref()?;
    let observed = snapshot.credits_observed.as_ref()?;
    if observed.instant.elapsed() >= TTL {
        return None;
    }
    Some((
        Snapshot {
            identity_key: snapshot.identity_key.clone(),
            credits: snapshot.credits.clone(),
            credits_observed: Some(observed.clone()),
            ..Snapshot::default()
        },
        observed.clone(),
    ))
}

fn credits_from_entry(entry: &Entry, refreshing: bool) -> (ComposerQuotaCredits, Option<String>) {
    let failed = entry.expired || !entry.failures.is_empty();
    let snapshot = entry.snapshot.as_ref();
    let stale = snapshot
        .and_then(|snapshot| snapshot.credits_observed.as_ref())
        .is_some_and(|observed| {
            failed
                || observed.instant.elapsed() >= TTL
                || entry
                    .fetched
                    .is_some_and(|fetched| fetched > observed.instant)
        });
    (
        ComposerQuotaCredits {
            observation: snapshot.and_then(|snapshot| snapshot.credits.clone()),
            stale,
            refreshing,
            failed,
        },
        snapshot.and_then(|snapshot| snapshot.identity_key.clone()),
    )
}

fn cache_key(credential_id: &str, generation: u64, codex_client_version: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"velune-provider-account-cache-v1\0");
    digest.update(credential_id.as_bytes());
    digest.update([0]);
    digest.update(generation.to_le_bytes());
    digest.update(codex_client_version.as_bytes());
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
        model_usage_observed_at: None,
        refreshing: false,
        stale: false,
        stale_sources: Vec::new(),
        credits: None,
        banked_resets: None,
        profile: None,
        model_usage: None,
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
    let state = account_state(
        entry.running,
        entry.expired,
        !entry.failures.is_empty(),
        snapshot.credits.is_some()
            || snapshot.resets.is_some()
            || snapshot.profile.is_some()
            || snapshot.model_usage.observed_at().is_some(),
    );
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
    if snapshot.model_usage.observed_at().is_some()
        && (snapshot.model_usage.is_stale()
            || entry
                .failures
                .iter()
                .any(|failure| failure.source == ProviderAccountSource::AnalyticsUsage))
    {
        stale_sources.push(ProviderAccountSource::AnalyticsUsage);
    }
    stale_sources.sort_by_key(|source| match source {
        ProviderAccountSource::Usage => 0,
        ProviderAccountSource::Resets => 1,
        ProviderAccountSource::Profile => 2,
        ProviderAccountSource::AnalyticsUsage => 3,
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
        model_usage_observed_at: snapshot
            .model_usage
            .observed_at()
            .map(|observed| observed.at.clone()),
        refreshing: entry.running,
        stale: !stale_sources.is_empty(),
        stale_sources,
        credits: snapshot.credits,
        banked_resets: snapshot.resets,
        profile: snapshot.profile,
        model_usage: snapshot.model_usage.into_rows(),
        failures: entry.failures.clone(),
    }
}

fn merge_analytics_status(full: &mut ProviderAccountStatus, analysis: ProviderAccountStatus) {
    let expired = full.state == ProviderAccountState::Expired
        || analysis.state == ProviderAccountState::Expired;
    full.identity_key = full.identity_key.take().or(analysis.identity_key);
    full.profile_identity_key = full
        .profile_identity_key
        .take()
        .or(analysis.profile_identity_key);
    if analysis.fetched_at > full.fetched_at {
        full.fetched_at = analysis.fetched_at;
    }
    full.model_usage = analysis.model_usage;
    full.model_usage_observed_at = analysis.model_usage_observed_at;
    full.refreshing |= analysis.refreshing;
    full.stale |= analysis.stale;
    full.stale_sources.extend(analysis.stale_sources);
    full.failures.extend(analysis.failures);
    let has_data = full.credits.is_some()
        || full.banked_resets.is_some()
        || full.profile.is_some()
        || full.model_usage.is_some();
    full.state = account_state(
        full.refreshing,
        expired,
        !full.failures.is_empty(),
        has_data,
    );
}

fn account_state(
    refreshing: bool,
    expired: bool,
    failed: bool,
    has_data: bool,
) -> ProviderAccountState {
    match (refreshing, expired, failed, has_data) {
        (true, _, _, _) => ProviderAccountState::Refreshing,
        (_, true, _, _) => ProviderAccountState::Expired,
        (_, _, true, true) => ProviderAccountState::PartialFailure,
        (_, _, true, false) => ProviderAccountState::Failed,
        (_, _, false, true) => ProviderAccountState::Available,
        _ => ProviderAccountState::Refreshing,
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
                || snapshot.model_usage.is_stale()
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
    scope: CollectionScope,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<Collected, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "账户报告采集无法启动".to_owned())?;
    runtime.block_on(async move {
        tokio::select! {
            result = tokio::time::timeout(Duration::from_secs(60), collect_async(home, capture, scope)) => {
                match result {
                    Ok(result) => result,
                    Err(_) => Err("账户报告采集超时".to_owned()),
                }
            }
            _ = cancel.changed() => Err("账户报告采集已取消".to_owned()),
        }
    })
}

fn collect_usage_payload(payload: Value, scope: CollectionScope, collected: &mut Collected) {
    let usage = match scope {
        CollectionScope::Credits => {
            serde_json::from_value::<CreditsResponse>(payload).map(|response| UsageResponse {
                credits: response.credits,
                rate_limit_reset_credits: None,
            })
        }
        _ => serde_json::from_value(payload),
    };
    match usage {
        Ok(usage) => {
            let credits = usage.credits.map(|credits| ProviderAccountCredits {
                has_credits: credits.has_credits,
                unlimited: credits.unlimited,
                balance: credits.balance.map(String::from),
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
            if collected.snapshot.credits.is_some()
                || count.is_some()
                || scope == CollectionScope::Credits
            {
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

fn collect_reset_payload(payload: Value, collected: &mut Collected) {
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

fn collect_profile_payload(payload: Value, collected: &mut Collected) {
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
                    reasoning_effort_percent: profile.stats.most_used_reasoning_effort_percentage,
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

async fn collect_async(
    home: PathBuf,
    capture: Capture,
    scope: CollectionScope,
) -> Result<Collected, String> {
    use crate::subscription_auth::{AccountDataEndpoint as Endpoint, account_identity};
    let mut collected = Collected::default();
    let primary_source = scope.primary_source();
    let identity = match account_identity(&home, &capture.credential_id, capture.generation) {
        Ok(identity) => identity,
        Err(CredentialResolutionError::AuthenticationExpired) => {
            collected.expired = true;
            collected
                .failures
                .push(failure(primary_source, "ChatGPT 订阅授权已过期"));
            return Ok(collected);
        }
        Err(CredentialResolutionError::StaleBinding) => {
            collected.failures.push(failure(
                primary_source,
                "提供商认证已改变，请重新登录后刷新",
            ));
            return Ok(collected);
        }
        Err(_) => {
            collected
                .failures
                .push(failure(primary_source, "订阅登录信息不可用，请重新登录"));
            return Ok(collected);
        }
    };
    collected.snapshot.identity_key = Some(identity.account_scope_key);
    collected.snapshot.profile_identity_key = Some(identity.profile_subject_key);
    if scope == CollectionScope::Analysis {
        model_usage::collect(&home, &capture, identity.analytics_kind, &mut collected).await;
        return Ok(collected);
    }

    let usage = fetch_json(
        &home,
        &capture,
        Endpoint::Usage,
        ProviderAccountSource::Usage,
        &mut collected,
    )
    .await;
    if let Some(payload) = usage {
        collect_usage_payload(payload, scope, &mut collected);
    }

    if scope == CollectionScope::Credits {
        return Ok(collected);
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
        collect_reset_payload(payload, &mut collected);
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
        collect_profile_payload(payload, &mut collected);
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
        &capture.codex_client_version,
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
        ProviderAccountSource::AnalyticsUsage => "暂时无法读取订阅模型用量，请稍后刷新",
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
    use std::time::{SystemTime, UNIX_EPOCH};

    const TEST_CLIENT_VERSION: &str = "0.166.7";

    fn full_entry_key(name: &str) -> EntryKey {
        (name.into(), CollectionScope::Full)
    }

    #[test]
    fn analysis_demand_is_independent_from_settings_and_reports_failure_after_backoff() {
        let service = Arc::new(ProviderAccountService {
            home: PathBuf::from("/synthetic"),
            state: Mutex::new(State::default()),
        });
        let analysis_key = ("scope".into(), CollectionScope::Analysis);
        {
            let mut state = service.state.lock().unwrap();
            state.bindings.insert(
                ("g".into(), "p".into()),
                Binding::ChatGpt(Capture {
                    credential_id: "unused".into(),
                    generation: 1,
                    codex_client_version: TEST_CLIENT_VERSION.into(),
                    cache_key: "scope".into(),
                }),
            );
            state.inflight = MAX_INFLIGHT;
        }
        assert!(
            service
                .query("g", "p", false)
                .unwrap()
                .model_usage
                .is_none()
        );
        let cancel = {
            let state = service.state.lock().unwrap();
            assert!(!state.entries.contains_key(&analysis_key));
            state.entries[&full_entry_key("scope")].cancel.clone()
        };
        service.finish(
            &full_entry_key("scope"),
            &cancel,
            Ok(Collected {
                snapshot: Snapshot {
                    identity_key: Some("opaque-account".into()),
                    credits: Some(ProviderAccountCredits {
                        has_credits: Some(true),
                        unlimited: Some(false),
                        balance: Some("2".into()),
                    }),
                    credits_observed: Some(observation_now()),
                    ..Snapshot::default()
                },
                succeeded: true,
                ..Collected::default()
            }),
        );
        service.state.lock().unwrap().inflight = MAX_INFLIGHT;
        assert!(service.query_analysis("g", "p", false).unwrap().refreshing);
        let cancel = service.state.lock().unwrap().entries[&analysis_key]
            .cancel
            .clone();
        service.finish(&analysis_key, &cancel, Err("synthetic model error".into()));
        let analysis = service.query_analysis("g", "p", false).unwrap();
        assert_eq!(analysis.state, ProviderAccountState::PartialFailure);
        assert_eq!(
            analysis.failures[0].source,
            ProviderAccountSource::AnalyticsUsage
        );
        assert_eq!(analysis.credits.unwrap().balance.as_deref(), Some("2"));
        assert!(service.query("g", "p", false).unwrap().failures.is_empty());
    }

    #[test]
    fn credits_query_preserves_scope_and_adopts_newer_full_usage() {
        let service = Arc::new(ProviderAccountService {
            home: PathBuf::from("/synthetic"),
            state: Mutex::new(State::default()),
        });
        let credits_key = ("scope".into(), CollectionScope::Credits);
        let full_key = full_entry_key("scope");
        {
            let mut state = service.state.lock().unwrap();
            state.bindings.insert(
                ("g".into(), "p".into()),
                Binding::ChatGpt(Capture {
                    credential_id: "unused".into(),
                    generation: 1,
                    codex_client_version: TEST_CLIENT_VERSION.into(),
                    cache_key: "scope".into(),
                }),
            );
            // Keep the worker paused so the test can inject results without any HTTP access.
            state.inflight = MAX_INFLIGHT;
        }

        let (initial, _) = service.query_credits("g", "p", false).unwrap().unwrap();
        assert!(initial.refreshing);
        assert!(initial.observation.is_none());
        let cancel = {
            let state = service.state.lock().unwrap();
            assert!(!state.entries.contains_key(&full_key));
            state.entries[&credits_key].cancel.clone()
        };
        let observation = |balance: &str, observed: Observation| Collected {
            snapshot: Snapshot {
                identity_key: Some("opaque-account".into()),
                credits: Some(ProviderAccountCredits {
                    has_credits: Some(true),
                    unlimited: Some(false),
                    balance: Some(balance.into()),
                }),
                credits_observed: Some(observed),
                ..Snapshot::default()
            },
            succeeded: true,
            ..Collected::default()
        };
        let zero = Observation {
            at: "first".into(),
            instant: Instant::now() - Duration::from_secs(1),
        };
        service.finish(&credits_key, &cancel, Ok(observation("0", zero)));
        service.state.lock().unwrap().inflight = MAX_INFLIGHT;
        let (cached, identity) = service.query_credits("g", "p", false).unwrap().unwrap();
        assert_eq!(cached.observation.unwrap().balance.as_deref(), Some("0"));
        assert_eq!(identity.as_deref(), Some("opaque-account"));
        assert!(!cached.refreshing);
        assert!(service.query("g", "p", false).unwrap().refreshing);

        let full_cancel = service.state.lock().unwrap().entries[&full_key]
            .cancel
            .clone();
        let mut newer = observation("4.5", observation_now());
        newer
            .failures
            .push(failure(ProviderAccountSource::Profile, "合成失败"));
        service.finish(&full_key, &full_cancel, Ok(newer));
        let (new, _) = service.query_credits("g", "p", false).unwrap().unwrap();
        assert_eq!(new.observation.unwrap().balance.as_deref(), Some("4.5"));
        assert!(!new.stale);
        assert!(!new.failed);
        assert!(!new.refreshing);
        let full = service.query("g", "p", false).unwrap();
        assert_eq!(full.state, ProviderAccountState::PartialFailure);
        assert_eq!(full.failures[0].source, ProviderAccountSource::Profile);
    }

    #[test]
    fn credits_payload_keeps_numeric_zero_and_ignores_reset_fields() {
        let response = serde_json::from_value::<CreditsResponse>(serde_json::json!({
            "credits": {"balance": 0},
            "rate_limit_reset_credits": "unrecognized schema",
        }))
        .unwrap();
        let balance = response.credits.unwrap().balance.map(String::from);
        assert_eq!(balance.as_deref(), Some("0"));
    }

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
        let mut collected = Collected::default();
        collect_profile_payload(payload, &mut collected);
        assert!(collected.succeeded && collected.failures.is_empty());
        assert!(collected.snapshot.profile_observed.is_some());
        let profile = collected.snapshot.profile.unwrap();
        assert_eq!(profile.lifetime_tokens, Some(123456));
        assert_eq!(profile.peak_daily_tokens, Some(45678));
        assert_eq!(profile.longest_task_seconds, Some(7200));
        assert_eq!(profile.current_streak_days, Some(3));
        assert_eq!(profile.longest_streak_days, Some(9));
        assert_eq!(profile.fast_mode_percent, Some(12.5));
        assert_eq!(profile.reasoning_effort.as_deref(), Some("medium"));
        assert_eq!(profile.reasoning_effort_percent, Some(55.0));
        assert_eq!(profile.skills_explored, Some(2));
        assert_eq!(profile.total_skills_used, Some(6));
        assert_eq!(profile.total_threads, Some(8));
        assert_eq!(
            profile.daily.unwrap(),
            [ProviderAccountDailyUsage {
                date: "2026-11-15".into(),
                tokens: 321
            }]
        );
        assert_eq!(profile.stats_as_of.as_deref(), Some("2026-11-15"));
        assert_eq!(
            profile.generated_at.as_deref(),
            Some("2026-11-16T10:00:00Z")
        );
        let invocations = profile.top_invocations.unwrap();
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].kind, "skill");
        assert_eq!(invocations[0].display_name, "sample");
        assert_eq!(invocations[0].usage_count, 4);
    }

    #[test]
    fn missing_or_partial_daily_bucket_invalidates_profile_payload() {
        for bucket in [
            serde_json::json!({"tokens":4}),
            serde_json::json!({"start_date":"2026-11-15"}),
            serde_json::json!({"start_date":"","tokens":4}),
            serde_json::json!({"start_date":"2026-11-15","tokens":-1}),
        ] {
            let mut collected = Collected::default();
            collect_profile_payload(
                serde_json::json!({
                    "stats":{"lifetime_tokens":10,"daily_usage_buckets":[
                        {"start_date":"2026-11-14","tokens":5}, bucket
                    ]}, "metadata":{"generated_at":"2026-11-16T10:00:00Z"}
                }),
                &mut collected,
            );
            assert!(!collected.succeeded);
            assert!(collected.snapshot.profile.is_none());
            assert!(collected.snapshot.profile_observed.is_none());
            assert_eq!(collected.failures.len(), 1);
            assert_eq!(collected.failures[0].source, ProviderAccountSource::Profile);
            assert!(collected.failures[0].message.contains("响应格式无效"));
        }
    }

    #[test]
    fn profile_admission_distinguishes_missing_fields_from_explicit_empty_reports() {
        for (payload, accepted, diagnostic) in [
            (
                serde_json::json!({"stats":{},"metadata":{}}),
                false,
                "未报告可用统计",
            ),
            (
                serde_json::json!({"stats":{"lifetime_tokens":0},"metadata":{}}),
                true,
                "",
            ),
            (
                serde_json::json!({"stats":{"daily_usage_buckets":[]},"metadata":{}}),
                true,
                "",
            ),
            (
                serde_json::json!({"stats":{},"metadata":{"generated_at":"2026-11-16"}}),
                true,
                "",
            ),
            (
                serde_json::json!({"stats":{"lifetime_tokens":10},"metadata":{"stats_error":"synthetic unavailable"}}),
                false,
                "暂时无法提供账户用量统计",
            ),
        ] {
            let mut collected = Collected::default();
            collect_profile_payload(payload, &mut collected);
            assert_eq!(collected.succeeded, accepted);
            assert_eq!(collected.snapshot.profile.is_some(), accepted);
            assert_eq!(collected.snapshot.profile_observed.is_some(), accepted);
            if accepted {
                assert!(collected.failures.is_empty());
            } else {
                assert!(
                    collected
                        .failures
                        .iter()
                        .any(|failure| failure.source == ProviderAccountSource::Profile
                            && failure.message.contains(diagnostic))
                );
            }
        }
    }

    #[test]
    fn reset_payload_merge_preserves_reported_counts_rows_and_observation_owners() {
        let rows = serde_json::json!([
            {"id":"expired","reset_type":"codex_rate_limits","status":"available", "granted_at":"2025-01-01", "expires_at":"2026-01-01T00:00:00Z", "title":"Earlier grant", "description":"Keep original detail"},
            {"id":"used","reset_type":"codex_rate_limits","status":"redeemed"},
            {"id":"new","reset_type":"other","status":"available"}
        ]);
        for (payload, count, row_count, refresh_count, failure_expected) in [
            (serde_json::json!({"credits":null}), 3, None, false, true),
            (
                serde_json::json!({"available_count":2,"credits":rows.clone()}),
                2,
                Some(3),
                true,
                false,
            ),
            (
                serde_json::json!({"available_count":0,"credits":[]}),
                0,
                Some(0),
                true,
                false,
            ),
            (
                serde_json::json!({"credits":rows}),
                3,
                Some(3),
                false,
                false,
            ),
        ] {
            let mut collected = Collected::default();
            collect_usage_payload(
                serde_json::json!({
                    "rate_limit_reset_credits":{"available_count":3}
                }),
                CollectionScope::Full,
                &mut collected,
            );
            assert_eq!(
                collected.snapshot.resets.as_ref().unwrap().available_count,
                Some(3)
            );
            assert!(collected.snapshot.reset_count_observed.is_some());
            assert!(collected.snapshot.reset_grants_observed.is_none());
            collected.snapshot.reset_count_observed.as_mut().unwrap().at = "earlier-count".into();
            collect_reset_payload(payload, &mut collected);
            let result = collected.snapshot.resets.unwrap();
            assert_eq!(result.available_count, Some(count));
            assert_eq!(result.grants.as_ref().map(Vec::len), row_count);
            assert_eq!(
                collected.snapshot.reset_count_observed.unwrap().at != "earlier-count",
                refresh_count
            );
            assert_eq!(
                collected.snapshot.reset_grants_observed.is_some(),
                row_count.is_some()
            );
            assert!(collected.succeeded);
            assert_eq!(collected.failures.len(), usize::from(failure_expected));
            if failure_expected {
                assert_eq!(collected.failures[0].source, ProviderAccountSource::Resets);
                assert!(collected.failures[0].message.contains("未报告明细"));
            }
            if row_count == Some(3) {
                let grants = result.grants.unwrap();
                assert_eq!(
                    grants
                        .iter()
                        .map(|grant| (
                            grant.id.as_str(),
                            grant.reset_type.as_str(),
                            grant.status.as_str()
                        ))
                        .collect::<Vec<_>>(),
                    [
                        ("expired", "codex_rate_limits", "available"),
                        ("used", "codex_rate_limits", "redeemed"),
                        ("new", "other", "available"),
                    ]
                );
                assert_eq!(grants[0].granted_at.as_deref(), Some("2025-01-01"));
                assert_eq!(
                    grants[0].expires_at.as_deref(),
                    Some("2026-01-01T00:00:00Z")
                );
                assert_eq!(grants[0].title.as_deref(), Some("Earlier grant"));
                assert_eq!(
                    grants[0].description.as_deref(),
                    Some("Keep original detail")
                );
                assert!(grants[1].expires_at.is_none());
            }
        }
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
            full_entry_key("scope"),
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
            &full_entry_key("scope"),
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
        let status = status_from_entry("g", "p", &state.entries[&full_entry_key("scope")]);
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
        let key = full_entry_key("scope");
        service.state.lock().unwrap().entries.insert(
            key.clone(),
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
        service.finish(&key, &cancel, Err("late response".into()));
        let state = service.state.lock().unwrap();
        assert!(state.entries[&key].running);
        assert!(state.entries[&key].failures.is_empty());
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
        let key = full_entry_key("scope");
        {
            let mut state = service.state.lock().unwrap();
            state.bindings.insert(
                ("g".into(), "p".into()),
                Binding::ChatGpt(Capture {
                    credential_id: "missing-session".into(),
                    generation: 1,
                    codex_client_version: TEST_CLIENT_VERSION.into(),
                    cache_key: "scope".into(),
                }),
            );
            state.inflight = MAX_INFLIGHT;
            for index in 0..MAX_INFLIGHT {
                state.entries.insert(
                    full_entry_key(&format!("occupied-{index}")),
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
        assert!(!service.state.lock().unwrap().entries[&key].running);
        assert!(service.state.lock().unwrap().entries[&key].force_pending);

        service.state.lock().unwrap().inflight = MAX_INFLIGHT - 1;
        let started = service.query("g", "p", false).unwrap();
        assert!(started.refreshing);
        assert!(service.state.lock().unwrap().entries[&key].running);
        assert!(!service.state.lock().unwrap().entries[&key].force_pending);

        for _ in 0..100 {
            let state = service.state.lock().unwrap();
            let entry = &state.entries[&key];
            if !entry.running && !entry.failures.is_empty() {
                assert_eq!(entry.failures[0].source, ProviderAccountSource::Usage);
                let _ = std::fs::remove_dir_all(&home);
                return;
            }
            drop(state);
            std::thread::sleep(Duration::from_millis(10));
        }
        let state = service.state.lock().unwrap();
        let entry = &state.entries[&key];
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

    #[test]
    fn cache_key_is_opaque_and_scoped_to_credentials_and_client_version() {
        let a = cache_key("session-id", 1, TEST_CLIENT_VERSION);
        assert!(a.starts_with("sha256:"));
        assert!(!a.contains("session-id"));
        assert_ne!(a, cache_key("session-id", 2, TEST_CLIENT_VERSION));
        assert_ne!(a, cache_key("another", 1, TEST_CLIENT_VERSION));
        assert_ne!(a, cache_key("session-id", 1, "0.166.8"));
    }
}
