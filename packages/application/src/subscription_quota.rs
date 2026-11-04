//! The application owns subscription observations; gateways only read this shared view.
use crate::{
    ProviderQuotaHistory, ProviderQuotaStatus, QuotaAlert, QuotaAlertKind, QuotaAlerts,
    QuotaBalanceObservation, QuotaFreshness, QuotaHistoryBalance, QuotaHistoryPoint,
    QuotaHistoryWindow, QuotaObservation, QuotaPaceObservation, QuotaPaceStatus,
    QuotaWindowObservation,
};
use crate::{config::GatewayConfig, provider_authentication::AuthenticationMaterial};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use velune_gateway::{QuotaSnapshot, QuotaSource, QuotaWindow};

type Key = (String, String);
struct Observation {
    windows: Vec<QuotaWindow>,
    balances: Vec<QuotaBalanceObservation>,
    available: Option<bool>,
    observed_at_ms: i64,
    expires_at_ms: i64,
}
type ObservationResult = Result<Observation, ()>;
/// Alerts kept for platforms that poll late; older ones are dropped.
const RETAINED_ALERTS: usize = 64;
#[derive(Clone, PartialEq, Eq)]
struct Binding {
    id: String,
    generation: u64,
    source: crate::config::ProviderQuotaSource,
    binary: Option<String>,
    directory: LoginDirectory,
    kimi: Option<crate::kimi_quota::Capture>,
    ark: Option<crate::ark_quota::Capture>,
    account: Option<crate::api_account_quota::Capture>,
}
impl Binding {
    /// Durable identity of the observed account binding. A new credential
    /// or source is a separate history; it does not reveal the previous one.
    fn epoch(&self) -> String {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(
            format!("{:?}\0{}\0{}", self.source, self.id, self.generation).as_bytes(),
        );
        format!("{digest:x}")[..16].into()
    }
}
fn cli_binary(provider: &crate::config::ProviderDefinition) -> Option<String> {
    match &provider.execution_backend {
        crate::config::ExecutionBackend::ClaudeCli { binary } => Some(binary.clone()),
        crate::config::ExecutionBackend::Http => None,
    }
}
/// Where a Claude Code CLI reads usage from.
#[derive(Clone, PartialEq, Eq)]
enum LoginDirectory {
    /// The application-owned directory of the binding's credential.
    Owned,
    /// A referenced login's `CLAUDE_CONFIG_DIR`; `None` is the user's default.
    Linked(Option<String>),
}
#[derive(Clone)]
struct ViewBinding {
    binding: Option<Binding>,
    incarnation: String,
}
struct Entry {
    // Recreating a deleted provider can reuse its ID, key and credential generation.
    incarnation: String,
    binding: Option<Binding>,
    owner: Option<crate::kimi_quota::Owner>,
    reservation: Option<crate::kimi_quota::Owner>,
    identity_rejected: bool,
    windows: Vec<QuotaWindow>,
    balances: Vec<QuotaBalanceObservation>,
    available: Option<bool>,
    observed_at_ms: i64,
    expires_at_ms: i64,
    /// Usage caps of the provider, for alerts; they do not change the binding.
    limits: Vec<crate::config::QuotaLimit>,
    /// Alert conditions that held at the last observation.
    alerting: BTreeSet<String>,
    running: bool,
    resample: bool,
    retry_at: Instant,
    problem: Option<String>,
    cancel: tokio::sync::watch::Sender<bool>,
}
#[derive(Default)]
struct State {
    entries: BTreeMap<Key, Entry>,
    inflight: usize,
    stopped: bool,
    alerts: VecDeque<QuotaAlert>,
    alert_sequence: u64,
}
pub(crate) struct SubscriptionQuota {
    home: PathBuf,
    scheduling: velune_gateway::SchedulingState,
    state: Mutex<State>,
    history: crate::quota_history::QuotaHistory,
}
struct GatewayView {
    service: Arc<SubscriptionQuota>,
    gateway_id: String,
    bindings: BTreeMap<String, ViewBinding>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|v| i64::try_from(v.as_millis()).ok())
        .unwrap_or(0)
}
fn fresh(entry: &Entry) -> bool {
    let now = now_ms();
    entry.observed_at_ms > 0
        && entry.observed_at_ms <= now
        && entry.expires_at_ms > now
        && entry
            .windows
            .iter()
            .all(|w| w.reset_at_ms.is_none_or(|reset| reset > now))
}
impl SubscriptionQuota {
    pub(crate) fn new(
        home: PathBuf,
        gateways: &[GatewayConfig],
        scheduling: velune_gateway::SchedulingState,
    ) -> Arc<Self> {
        let service = Arc::new(Self {
            history: crate::quota_history::QuotaHistory::new(home.join("quota-history")),
            home,
            scheduling,
            state: Mutex::new(State::default()),
        });
        service.configure(gateways);
        service
    }
    pub(crate) fn configure(&self, gateways: &[GatewayConfig]) {
        let desired: BTreeMap<Key, (Option<Binding>, Vec<crate::config::QuotaLimit>)> = gateways
            .iter()
            .flat_map(|g| {
                g.providers.iter().map(|p| {
                    let binding: Option<Binding> =
                        match (&p.authentication.material, p.quota_source) {
                            (
                                AuthenticationMaterial::CodexSubscription { credential_id, .. },
                                crate::config::ProviderQuotaSource::CodexUsage,
                            ) => Some(Binding {
                                id: credential_id.clone(),
                                generation: p.authentication.generation,
                                source: p.quota_source,
                                binary: None,
                                directory: LoginDirectory::Owned,
                                kimi: None,
                                ark: None,
                                account: None,
                            }),
                            (
                                AuthenticationMaterial::ClaudeCli { credential_id, .. },
                                crate::config::ProviderQuotaSource::ClaudeCliUsage,
                            ) => Some(Binding {
                                id: credential_id.clone(),
                                generation: p.authentication.generation,
                                source: p.quota_source,
                                binary: cli_binary(p),
                                directory: LoginDirectory::Owned,
                                kimi: None,
                                ark: None,
                                account: None,
                            }),
                            (
                                AuthenticationMaterial::ClaudeCliLinked { config_dir, .. },
                                crate::config::ProviderQuotaSource::ClaudeCliUsage,
                            ) => Some(Binding {
                                id: p.id.clone(),
                                generation: p.authentication.generation,
                                source: p.quota_source,
                                binary: cli_binary(p),
                                directory: LoginDirectory::Linked(config_dir.clone()),
                                kimi: None,
                                ark: None,
                                account: None,
                            }),
                            (
                                AuthenticationMaterial::ApiKey { value },
                                crate::config::ProviderQuotaSource::KimiUsage,
                            ) => crate::config::kimi_region(&p.endpoint).map(|region| Binding {
                                id: p.id.clone(),
                                generation: p.authentication.generation,
                                source: p.quota_source,
                                binary: None,
                                directory: LoginDirectory::Owned,
                                ark: None,
                                account: None,
                                kimi: Some(crate::kimi_quota::Capture {
                                    region,
                                    key: value.clone(),
                                }),
                            }),
                            (
                                AuthenticationMaterial::ApiKey { .. },
                                crate::config::ProviderQuotaSource::ArkCodingUsage,
                            ) => Some(Binding {
                                id: p.id.clone(),
                                generation: p.authentication.generation,
                                source: p.quota_source,
                                binary: None,
                                directory: LoginDirectory::Owned,
                                kimi: None,
                                account: None,
                                ark: p.quota_authentication.access_key().map(|key| {
                                    crate::ark_quota::Capture {
                                        key: key.clone(),
                                        generation: p.quota_authentication.generation,
                                        confirmed: p
                                            .quota_authentication
                                            .description(p.authentication.generation)
                                            .binding_confirmed,
                                    }
                                }),
                            }),
                            (
                                AuthenticationMaterial::ApiKey { value },
                                crate::config::ProviderQuotaSource::ApiAccount,
                            ) => velune_application_api::api_account_service(&p.endpoint).map(
                                |service| Binding {
                                    id: p.id.clone(),
                                    generation: p.authentication.generation,
                                    source: p.quota_source,
                                    binary: None,
                                    directory: LoginDirectory::Owned,
                                    kimi: None,
                                    ark: None,
                                    account: Some(crate::api_account_quota::Capture {
                                        service,
                                        key: value.clone(),
                                    }),
                                },
                            ),
                            _ => None,
                        };
                    (
                        (g.id.clone(), p.id.clone()),
                        (binding, p.quota_limits.clone()),
                    )
                })
            })
            .collect();
        let mut state = self.state.lock().expect("quota state");
        state.entries.retain(|key, entry| {
            if let Some((_, limits)) = desired.get(key) {
                entry.limits = limits.clone();
            }
            if desired.get(key).map(|(binding, _)| binding) == Some(&entry.binding) {
                return true;
            }
            let _ = entry.cancel.send(true);
            if let Some((binding, _)) = desired.get(key)
                && binding.as_ref().is_some_and(|b| b.kimi.is_some())
                && entry.binding.as_ref().is_some_and(|b| b.kimi.is_some())
            {
                entry.reservation = entry.owner.take().or(entry.reservation.take());
                entry.binding = binding.clone();
                entry.running = false;
                entry.identity_rejected = false;
                entry.cancel = tokio::sync::watch::channel(false).0;
                entry.retry_at = Instant::now();
                entry.problem = None;
                return true;
            }
            false
        });
        let names: BTreeSet<String> = desired
            .keys()
            .map(|(g, p)| crate::quota_history::QuotaHistory::name(g, p))
            .collect();
        for (key, (binding, limits)) in desired {
            state.entries.entry(key).or_insert_with(|| Entry {
                incarnation: crate::new_record_key(),
                binding,
                owner: None,
                reservation: None,
                identity_rejected: false,
                windows: Vec::new(),
                balances: Vec::new(),
                available: None,
                observed_at_ms: 0,
                expires_at_ms: 0,
                limits,
                alerting: BTreeSet::new(),
                running: false,
                resample: false,
                retry_at: Instant::now(),
                problem: None,
                cancel: tokio::sync::watch::channel(false).0,
            });
        }
        drop(state);
        self.history.retain(&names);
    }
    pub(crate) async fn authorize(
        self: &Arc<Self>,
        gateway: &str,
        provider: &str,
        generation: u64,
    ) -> Result<(), velune_gateway::CredentialResolutionError> {
        use velune_gateway::CredentialResolutionError as Error;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let _ = self
                .query(gateway, provider, false)
                .map_err(|_| Error::Unavailable)?;
            {
                let state = self.state.lock().expect("quota state");
                let entry = state
                    .entries
                    .get(&(gateway.to_owned(), provider.to_owned()))
                    .ok_or(Error::StaleBinding)?;
                if state.stopped
                    || entry.binding.as_ref().is_none_or(|binding| {
                        binding.generation != generation || binding.kimi.is_none()
                    })
                {
                    return Err(Error::StaleBinding);
                }
                if entry.owner.is_some() {
                    return Ok(());
                }
                if entry.identity_rejected || !entry.running {
                    return Err(Error::Unavailable);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    pub(crate) fn identity_status(&self, gateway: &str, provider: &str) -> (bool, Option<String>) {
        let state = self.state.lock().expect("quota state");
        state
            .entries
            .get(&(gateway.to_owned(), provider.to_owned()))
            .map_or((false, None), |entry| {
                (
                    entry.owner.is_some(),
                    entry
                        .owner
                        .as_ref()
                        .or(entry.reservation.as_ref())
                        .map(crate::kimi_quota::Owner::label),
                )
            })
    }
    pub(crate) fn source(self: &Arc<Self>, gateway_id: &str) -> Arc<dyn QuotaSource> {
        Arc::new(GatewayView {
            service: self.clone(),
            gateway_id: gateway_id.into(),
            bindings: self
                .state
                .lock()
                .expect("quota state")
                .entries
                .iter()
                .filter(|(key, _)| key.0 == gateway_id)
                .map(|(key, entry)| {
                    (
                        key.1.clone(),
                        ViewBinding {
                            binding: entry.binding.clone(),
                            incarnation: entry.incarnation.clone(),
                        },
                    )
                })
                .collect(),
        })
    }
    pub(crate) fn stop(&self) {
        let mut state = self.state.lock().expect("quota state");
        state.stopped = true;
        for entry in state.entries.values_mut() {
            let _ = entry.cancel.send(true);
        }
    }
    pub(crate) fn query(
        self: &Arc<Self>,
        gateway_id: &str,
        provider_id: &str,
        force: bool,
    ) -> Result<ProviderQuotaStatus, crate::Error> {
        self.query_binding(gateway_id, provider_id, force, None)
    }
    /// The current binding's retained observations since `since_ms`.
    pub(crate) fn history(
        &self,
        gateway_id: &str,
        provider_id: &str,
        since_ms: i64,
    ) -> Result<ProviderQuotaHistory, crate::Error> {
        let epoch = self
            .state
            .lock()
            .expect("quota state")
            .entries
            .get(&(gateway_id.to_owned(), provider_id.to_owned()))
            .ok_or_else(|| crate::Error::invalid("provider id"))?
            .binding
            .as_ref()
            .map(Binding::epoch);
        let points = match epoch {
            Some(epoch) => self
                .history
                .read(gateway_id, provider_id, &epoch, since_ms, now_ms())
                .map_err(|_| crate::Error::invalid("额度历史无法读取"))?,
            None => Vec::new(),
        };
        Ok(ProviderQuotaHistory {
            gateway_id: gateway_id.into(),
            provider_id: provider_id.into(),
            points,
        })
    }
    /// Remove the provider's retained observations; current quota is kept.
    pub(crate) fn clear_history(
        &self,
        gateway_id: &str,
        provider_id: &str,
    ) -> Result<ProviderQuotaHistory, crate::Error> {
        if !self
            .state
            .lock()
            .expect("quota state")
            .entries
            .contains_key(&(gateway_id.to_owned(), provider_id.to_owned()))
        {
            return Err(crate::Error::invalid("provider id"));
        }
        self.history
            .clear(gateway_id, provider_id)
            .map_err(|_| crate::Error::invalid("额度历史无法清除"))?;
        Ok(ProviderQuotaHistory {
            gateway_id: gateway_id.into(),
            provider_id: provider_id.into(),
            points: Vec::new(),
        })
    }
    /// Alerts raised after `since`, within the retained few.
    pub(crate) fn alerts(&self, since: u64) -> QuotaAlerts {
        let state = self.state.lock().expect("quota state");
        QuotaAlerts {
            alerts: state
                .alerts
                .iter()
                .filter(|alert| alert.sequence > since)
                .cloned()
                .collect(),
            cursor: state.alert_sequence,
        }
    }
    fn query_binding(
        self: &Arc<Self>,
        gateway_id: &str,
        provider_id: &str,
        force: bool,
        expected: Option<&ViewBinding>,
    ) -> Result<ProviderQuotaStatus, crate::Error> {
        let key = (gateway_id.to_owned(), provider_id.to_owned());
        let mut state = self.state.lock().expect("quota state");
        let may_start = !state.stopped && state.inflight < 4;
        let entry = state
            .entries
            .get_mut(&key)
            .ok_or_else(|| crate::Error::invalid("provider id"))?;
        if expected.is_some_and(|view| {
            view.binding != entry.binding || view.incarnation != entry.incarnation
        }) {
            return Err(crate::Error::invalid("quota binding changed"));
        }
        let start = may_start
            && entry.binding.as_ref().is_some_and(|b| {
                b.source != crate::config::ProviderQuotaSource::ArkCodingUsage || b.ark.is_some()
            })
            && !entry.running
            && (force
                || ((!fresh(entry)
                    || (entry.binding.as_ref().is_some_and(|b| b.kimi.is_some())
                        && entry.owner.is_none()))
                    && entry.retry_at <= Instant::now()));
        if start {
            entry.running = true;
            entry.resample = false;
            entry.problem = None;
            let binding = entry.binding.clone().expect("native subscription");
            let incarnation = entry.incarnation.clone();
            let cancel = entry.cancel.subscribe();
            let verified_owner = entry.owner.clone();
            entry.identity_rejected = false;
            let weak = Arc::downgrade(self);
            let home = self.home.clone();
            let worker_key = key.clone();
            state.inflight += 1;
            let jitter = worker_key
                .1
                .bytes()
                .fold(0u64, |n, b| n.wrapping_add(u64::from(b)))
                % 21;
            let refresh_ms = i64::try_from((50 + jitter) * 1000).expect("bounded interval");
            let spawned = std::thread::Builder::new()
                .name("velune-subscription-quota".into())
                .spawn(move || {
                    let (candidate_owner, result) = if let Some(kimi) = &binding.kimi {
                        collect_kimi(kimi, verified_owner, cancel, refresh_ms)
                    } else {
                        (None, collect(home, &binding, cancel, refresh_ms))
                    };
                    let Some(service) = weak.upgrade() else {
                        return;
                    };
                    let mut state = service.state.lock().expect("quota state");
                    state.inflight -= 1;
                    if state.stopped {
                        return;
                    }
                    if state.entries.get(&worker_key).is_none_or(|entry| {
                        entry.binding.as_ref() != Some(&binding) || entry.incarnation != incarnation
                    }) {
                        return;
                    }
                    let duplicate = candidate_owner.as_ref().is_some_and(|owner| {
                        state.entries.iter().any(|(key, entry)| {
                            key != &worker_key
                                && entry.owner.as_ref().or(entry.reservation.as_ref())
                                    == Some(owner)
                        })
                    });
                    let Some(entry) = state.entries.get_mut(&worker_key).filter(|entry| {
                        entry.binding.as_ref() == Some(&binding) && entry.incarnation == incarnation
                    }) else {
                        return;
                    };
                    entry.running = false;
                    if entry.resample {
                        entry.retry_at = Instant::now();
                        return;
                    }
                    if binding.kimi.is_some() {
                        if duplicate || candidate_owner.is_none() {
                            entry.identity_rejected = true;
                            entry.problem = Some(
                                if duplicate {
                                    "此 Kimi 账户已绑定另一个提供商。"
                                } else {
                                    "Kimi 账户归属未验证；不会派发模型请求。"
                                }
                                .into(),
                            );
                            entry.retry_at = Instant::now() + Duration::from_secs(50 + jitter);
                            return;
                        }
                        let owner = candidate_owner.expect("verified Kimi owner");
                        if entry
                            .reservation
                            .as_ref()
                            .is_some_and(|previous| previous != &owner)
                        {
                            service
                                .scheduling
                                .invalidate_provider(&worker_key.0, &worker_key.1);
                            entry.windows.clear();
                            entry.balances.clear();
                            entry.available = None;
                            entry.observed_at_ms = 0;
                            entry.expires_at_ms = 0;
                        }
                        entry.owner = Some(owner);
                        entry.reservation = None;
                    }
                    entry.retry_at = Instant::now() + Duration::from_secs(50 + jitter);
                    let observation = match result {
                        Ok(observation) => observation,
                        Err(()) => {
                            let expired = now_ms();
                            entry.expires_at_ms = entry.expires_at_ms.min(expired);
                            for window in &mut entry.windows {
                                window.expires_at_ms = window.expires_at_ms.min(expired);
                            }
                            entry.problem =
                                Some("额度暂不可用；保留上次观测，稍后可重新读取。".into());
                            return;
                        }
                    };
                    let (observed, expires) =
                        (observation.observed_at_ms, observation.expires_at_ms);
                    let raised = alert_conditions(&observation, &entry.limits);
                    let point = history_point(&observation);
                    let fresh_alerts: Vec<_> = raised
                        .iter()
                        .filter(|(key, _)| !entry.alerting.contains(*key))
                        .map(|(_, alert)| alert.clone())
                        .collect();
                    entry.alerting = raised.into_keys().collect();
                    entry.windows = observation.windows;
                    entry.balances = observation.balances;
                    entry.available = observation.available;
                    entry.observed_at_ms = observed;
                    entry.expires_at_ms = expires;
                    entry.problem = None;
                    if expires > observed {
                        entry.retry_at = Instant::now()
                            + Duration::from_millis(
                                u64::try_from(expires - observed).expect("future expiry"),
                            );
                    }
                    for (kind, window_id, max_used_percent) in fresh_alerts {
                        state.alert_sequence += 1;
                        let alert = QuotaAlert {
                            sequence: state.alert_sequence,
                            gateway_id: worker_key.0.clone(),
                            provider_id: worker_key.1.clone(),
                            kind,
                            window_id,
                            max_used_percent,
                            observed_at_ms: observed,
                        };
                        state.alerts.push_back(alert);
                        if state.alerts.len() > RETAINED_ALERTS {
                            state.alerts.pop_front();
                        }
                    }
                    drop(state);
                    service
                        .history
                        .record(&worker_key.0, &worker_key.1, &binding.epoch(), point);
                });
            if spawned.is_err() {
                state.inflight -= 1;
                if let Some(entry) = state.entries.get_mut(&key) {
                    entry.running = false;
                    entry.problem = Some("无法启动额度读取。".into());
                    entry.retry_at = Instant::now() + Duration::from_secs(60);
                }
            }
        }
        let entry = &state.entries[&key];
        Ok(ProviderQuotaStatus {
            gateway_id: gateway_id.into(),
            provider_id: provider_id.into(),
            observation: QuotaObservation {
                freshness: if entry.binding.is_none() {
                    QuotaFreshness::Unsupported
                } else if entry.binding.as_ref().is_some_and(|b| b.kimi.is_some())
                    && entry.owner.is_none()
                {
                    QuotaFreshness::Unknown
                } else if fresh(entry) {
                    QuotaFreshness::Fresh
                } else if entry.observed_at_ms == 0 {
                    QuotaFreshness::Unknown
                } else {
                    QuotaFreshness::Stale
                },
                windows: entry
                    .windows
                    .iter()
                    .map(|w| QuotaWindowObservation {
                        id: w.id.clone(),
                        model_ids: w.model_ids.clone(),
                        used_percent: w.used_percent,
                        reset_at_ms: w.reset_at_ms,
                        window_ms: w.window_ms,
                        observed_at_ms: Some(w.observed_at_ms),
                        expires_at_ms: Some(w.expires_at_ms),
                        pace: pace(w),
                    })
                    .collect(),
                balances: entry.balances.clone(),
                available: entry.available,
                refreshing: entry.running,
                problem: entry.problem.clone().or_else(|| {
                    entry
                        .binding
                        .as_ref()
                        .filter(|b| {
                            b.source == crate::config::ProviderQuotaSource::ArkCodingUsage
                                && b.ark.is_none()
                        })
                        .map(|_| "尚未配置额度查询认证。".into())
                }),
            },
        })
    }
}
impl Drop for SubscriptionQuota {
    fn drop(&mut self) {
        self.stop();
    }
}
impl QuotaSource for GatewayView {
    fn quota_exhausted(&self, provider_id: &str) {
        if let Some(binding) = self.bindings.get(provider_id) {
            {
                let mut state = self.service.state.lock().expect("quota state");
                let Some(entry) = state
                    .entries
                    .get_mut(&(self.gateway_id.clone(), provider_id.to_owned()))
                    .filter(|entry| {
                        entry.binding == binding.binding && entry.incarnation == binding.incarnation
                    })
                else {
                    return;
                };
                entry.resample = entry.running;
                let expired = now_ms();
                entry.expires_at_ms = entry.expires_at_ms.min(expired);
                for window in &mut entry.windows {
                    window.expires_at_ms = window.expires_at_ms.min(expired);
                }
            }
            let _ = self
                .service
                .query_binding(&self.gateway_id, provider_id, true, Some(binding));
        }
    }
    fn snapshot(&self, provider_id: &str) -> QuotaSnapshot {
        let Some(binding) = self.bindings.get(provider_id) else {
            return QuotaSnapshot::Unknown;
        };
        let Ok(status) =
            self.service
                .query_binding(&self.gateway_id, provider_id, false, Some(binding))
        else {
            return QuotaSnapshot::Unknown;
        };
        if binding.binding.as_ref().is_some_and(|b| {
            b.source == crate::config::ProviderQuotaSource::ArkCodingUsage
                && b.ark.as_ref().is_none_or(|a| !a.confirmed)
        }) {
            return QuotaSnapshot::Unknown;
        }
        let status = status.observation;
        let windows = status
            .windows
            .into_iter()
            .map(|w| QuotaWindow {
                id: w.id,
                model_ids: w.model_ids,
                used_percent: w.used_percent,
                reset_at_ms: w.reset_at_ms,
                window_ms: w.window_ms,
                observed_at_ms: w.observed_at_ms.expect("provider observation timestamp"),
                expires_at_ms: w.expires_at_ms.expect("provider observation expiry"),
            })
            .collect();
        match status.freshness {
            QuotaFreshness::Fresh => QuotaSnapshot::Known {
                windows,
                available: status.available,
            },
            QuotaFreshness::Stale => QuotaSnapshot::Stale {
                windows,
                available: status.available,
            },
            QuotaFreshness::Unknown => QuotaSnapshot::Unknown,
            QuotaFreshness::Unsupported => QuotaSnapshot::Unsupported,
        }
    }
}
/// The observed use against an even spend over the window's period, at the
/// time of the observation. The rate is the average since the period began.
fn pace(window: &QuotaWindow) -> Option<QuotaPaceObservation> {
    let observed = window.observed_at_ms;
    let ahead = window.remaining_period(observed)?;
    let (reset, period) = (window.reset_at_ms?, window.window_ms? as f64);
    let used = window.used_percent.clamp(0.0, 100.0);
    let expected = 100.0 * (1.0 - ahead);
    if used >= 100.0 {
        return Some(QuotaPaceObservation {
            expected_used_percent: expected,
            runs_out_at_ms: None,
            status: QuotaPaceStatus::Exhausted,
        });
    }
    let elapsed = period * (1.0 - ahead);
    // Too early in the period for an average to mean anything.
    let runs_out_at_ms = (elapsed >= period * 0.02 && used > 0.0)
        .then(|| observed + ((100.0 - used) * elapsed / used) as i64)
        .filter(|at| *at < reset);
    // A few points over an even spend still read as on pace.
    let status = if runs_out_at_ms.is_some() && used > expected + 3.0 {
        QuotaPaceStatus::RunsOutBeforeReset
    } else {
        QuotaPaceStatus::OnPace
    };
    Some(QuotaPaceObservation {
        expected_used_percent: expected,
        runs_out_at_ms,
        status,
    })
}
type AlertCondition = (QuotaAlertKind, Option<String>, Option<u8>);
/// Conditions holding at one observation, keyed so that a condition lasting
/// across observations is raised once.
fn alert_conditions(
    observation: &Observation,
    limits: &[crate::config::QuotaLimit],
) -> BTreeMap<String, AlertCondition> {
    let mut raised = BTreeMap::new();
    if observation.available == Some(false) {
        raised.insert(
            "unavailable".into(),
            (QuotaAlertKind::Unavailable, None, None),
        );
    }
    for window in &observation.windows {
        let scope = window
            .model_ids
            .as_ref()
            .map_or(String::new(), |ids| ids.join(","));
        if window.used_percent >= 100.0 {
            raised.insert(
                format!("exhausted:{}:{scope}", window.id),
                (QuotaAlertKind::Exhausted, Some(window.id.clone()), None),
            );
        } else if let Some(limit) = limits.iter().find(|limit| {
            limit.window_id == window.id && window.used_percent >= f64::from(limit.max_used_percent)
        }) {
            raised.insert(
                format!("cap:{}:{scope}", window.id),
                (
                    QuotaAlertKind::CapReached,
                    Some(window.id.clone()),
                    Some(limit.max_used_percent),
                ),
            );
        }
    }
    raised
}
fn history_point(observation: &Observation) -> QuotaHistoryPoint {
    QuotaHistoryPoint {
        observed_at_ms: observation.observed_at_ms,
        available: observation.available,
        windows: observation
            .windows
            .iter()
            .map(|w| QuotaHistoryWindow {
                id: w.id.clone(),
                model_ids: w.model_ids.clone(),
                used_percent: w.used_percent,
                reset_at_ms: w.reset_at_ms,
                window_ms: w.window_ms,
            })
            .collect(),
        balances: observation
            .balances
            .iter()
            .map(|b| QuotaHistoryBalance {
                id: b.id.clone(),
                amount: b.amount,
                unit: b.unit.clone(),
            })
            .collect(),
    }
}
fn observation(
    windows: Vec<QuotaWindow>,
    available: Option<bool>,
    observed: i64,
    expires: i64,
) -> Observation {
    Observation {
        windows,
        balances: Vec::new(),
        available,
        observed_at_ms: observed,
        expires_at_ms: expires,
    }
}
fn collect(
    home: PathBuf,
    binding: &Binding,
    mut cancel: tokio::sync::watch::Receiver<bool>,
    ttl_ms: i64,
) -> ObservationResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| ())?;
    runtime.block_on(async {
        if *cancel.borrow() { return Err(()); }
        tokio::select! {
            result = async {
                match binding.source {
                    crate::config::ProviderQuotaSource::CodexUsage => crate::subscription_auth::read_usage(home, binding.id.clone(), binding.generation).await.map_err(|_| ()).and_then(|value| parse_windows(&value,ttl_ms)),
                    crate::config::ProviderQuotaSource::ClaudeCliUsage => { let owned=crate::claude_cli::directory(&home,&binding.id); let dir=match &binding.directory { LoginDirectory::Owned=>Some(owned.as_path()), LoginDirectory::Linked(dir)=>dir.as_deref().map(std::path::Path::new) }; let windows=crate::claude_quota::read(binding.binary.as_deref().ok_or(())?,dir,ttl_ms).await?; let observed=windows.first().ok_or(())?.observed_at_ms; let expires=windows.iter().map(|window|window.expires_at_ms).min().ok_or(())?; Ok(observation(windows,None,observed,expires)) },
                    crate::config::ProviderQuotaSource::ArkCodingUsage => { let observed=now_ms(); let windows=binding.ark.as_ref().ok_or(())?.windows(observed,ttl_ms).await?; let expires=windows.iter().map(|w|w.expires_at_ms).min().ok_or(())?; Ok(observation(windows,None,observed,expires)) },
                    crate::config::ProviderQuotaSource::ApiAccount => { let observed=now_ms(); let found=binding.account.as_ref().ok_or(())?.observe(observed,ttl_ms).await?; let expires=found.windows.iter().map(|w|w.expires_at_ms).min().unwrap_or(observed+ttl_ms); Ok(Observation { windows: found.windows, balances: found.balances, available: found.available, observed_at_ms: observed, expires_at_ms: expires }) },
                    crate::config::ProviderQuotaSource::None | crate::config::ProviderQuotaSource::KimiUsage => Err(()),
                }
            } => result,
            _ = cancel.changed() => Err(()),
        }
    })
}

fn parse_windows(value: &Value, ttl_ms: i64) -> ObservationResult {
    let rate = value
        .get("rate_limit")
        .filter(|v| v.is_object())
        .ok_or(())?;
    let available = match rate.get("allowed") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(value)) => Some(*value),
        _ => return Err(()),
    };
    let observed = now_ms();
    if observed <= 0 {
        return Err(());
    }
    let mut windows = Vec::new();
    for field in ["primary_window", "secondary_window"] {
        let Some(window) = rate.get(field).filter(|v| !v.is_null()) else {
            continue;
        };
        let used = window["used_percent"]
            .as_f64()
            .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
            .ok_or(())?;
        let span = window["limit_window_seconds"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or(())?;
        let reset_at_ms = match window.get("reset_at") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_i64()
                    .filter(|n| *n > 0)
                    .and_then(|n| n.checked_mul(1000))
                    .ok_or(())?,
            ),
        };
        let model_ids = match window.get("model_ids") {
            None | Some(Value::Null) => None,
            Some(Value::Array(ids)) if !ids.is_empty() => Some(
                ids.iter()
                    .map(|id| {
                        id.as_str()
                            .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
                            .map(str::to_owned)
                            .ok_or(())
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            _ => return Err(()),
        };
        let id = format!("window:{span}");
        if windows
            .iter()
            .any(|w: &QuotaWindow| w.id == id && w.model_ids == model_ids)
        {
            return Err(());
        }
        windows.push(QuotaWindow {
            id,
            model_ids,
            used_percent: used,
            reset_at_ms,
            window_ms: i64::try_from(span).ok().and_then(|s| s.checked_mul(1000)),
            observed_at_ms: observed,
            expires_at_ms: reset_at_ms
                .map_or(observed + ttl_ms, |reset| reset.min(observed + ttl_ms)),
        });
    }
    if windows.is_empty() && available != Some(false) {
        return Err(());
    }
    let expires = windows
        .iter()
        .map(|w| w.expires_at_ms)
        .min()
        .unwrap_or(observed + ttl_ms);
    Ok(observation(windows, available, observed, expires))
}

fn collect_kimi(
    capture: &crate::kimi_quota::Capture,
    verified: Option<crate::kimi_quota::Owner>,
    mut cancel: tokio::sync::watch::Receiver<bool>,
    ttl: i64,
) -> (Option<crate::kimi_quota::Owner>, ObservationResult) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return (None, Err(()));
    };
    runtime.block_on(async {
        if *cancel.borrow() { return (None,Err(())); }
        let owner=tokio::select! { result=async { match verified { Some(owner)=>Ok(owner),None=>capture.owner().await } }=>result.ok(), _=cancel.changed()=>None };
        let Some(owner)=owner else { return (None,Err(())); };
        let observed=now_ms();
        let result=tokio::select! { result=capture.windows(observed,ttl)=>result.map(|windows| { let expires=windows.iter().map(|w|w.expires_at_ms).min().expect("nonempty windows"); observation(windows,None,observed,expires) }), _=cancel.changed()=>Err(()) };
        (Some(owner),result)
    })
}
