//! The application owns subscription observations; gateways only read this shared view.
use crate::{
    ProviderQuotaHistory, ProviderQuotaStatus, QuotaAlert, QuotaAlertKind, QuotaAlerts,
    QuotaBalanceObservation, QuotaFreshness, QuotaHistoryBalance, QuotaHistoryPoint,
    QuotaHistoryWindow, QuotaObservation, QuotaPaceObservation, QuotaPaceStatus,
    QuotaWindowObservation,
};
use crate::{
    antigravity_cli::Launcher as AntigravityLauncher,
    config::{ExecutionBackend, GatewayConfig, ProviderQuotaSource},
    provider_authentication::AuthenticationMaterial,
};
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
    // The request version can invalidate a sample without changing the account's history.
    codex_client_version: Option<String>,
    binary: Option<String>,
    directory: LoginDirectory,
    kimi: Option<crate::kimi_quota::Capture>,
    ark: Option<crate::ark_quota::Capture>,
    account: Option<crate::api_account_quota::Capture>,
    antigravity: Option<AntigravityLauncher>,
}
impl Binding {
    /// History stays with the captured credential or external execution target.
    /// The Antigravity ID includes the target digest from credential_reference.
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
        crate::config::ExecutionBackend::Http
        | crate::config::ExecutionBackend::AntigravityCli { .. } => None,
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
#[derive(Clone)]
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
    demand_until: Option<Instant>,
    problem: Option<String>,
    cancel: tokio::sync::watch::Sender<bool>,
}
impl Entry {
    fn expire_observation(&mut self) {
        let expired = now_ms();
        self.expires_at_ms = self.expires_at_ms.min(expired);
        for window in &mut self.windows {
            window.expires_at_ms = window.expires_at_ms.min(expired);
        }
    }
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
    /// A fork cannot retire an account epoch still held by the old execution.
    defer_identity_retirement: bool,
    identity_changes: Arc<Mutex<BTreeSet<Key>>>,
    state: Mutex<State>,
    history: Arc<crate::quota_history::QuotaHistory>,
}
struct GatewayView {
    service: Arc<SubscriptionQuota>,
    gateway_id: String,
    bindings: BTreeMap<String, ViewBinding>,
}

enum Refresh {
    IfDue,
    Forced,
    Explicit,
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
    if entry
        .binding
        .as_ref()
        .is_some_and(|binding| binding.antigravity.is_some())
    {
        return entry
            .windows
            .iter()
            .any(|window| antigravity_window_fresh(window, now));
    }
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
            history: Arc::new(crate::quota_history::QuotaHistory::new(
                home.join("quota-history"),
            )),
            home,
            scheduling,
            defer_identity_retirement: false,
            identity_changes: Arc::default(),
            state: Mutex::new(State::default()),
        });
        service.configure(gateways);
        Self::start_refresh(&service);
        service
    }
    /// Keep unchanged observations and account claims; workers/cancellation belong
    /// to one configuration version, so changing the new one cannot cancel the old.
    pub(crate) fn fork_configuration(&self, gateways: &[GatewayConfig]) -> Arc<Self> {
        let state = self.state.lock().expect("quota state");
        let service = Arc::new(Self {
            home: self.home.clone(),
            scheduling: self.scheduling.clone(),
            defer_identity_retirement: true,
            identity_changes: self.identity_changes.clone(),
            history: self.history.clone(),
            state: Mutex::new(State {
                entries: state
                    .entries
                    .iter()
                    .map(|(key, entry)| {
                        let mut snapshot = entry.clone();
                        snapshot.cancel = tokio::sync::watch::channel(false).0;
                        snapshot.running = false;
                        snapshot.resample = false;
                        (key.clone(), snapshot)
                    })
                    .collect(),
                alerts: state.alerts.clone(),
                alert_sequence: state.alert_sequence,
                ..State::default()
            }),
        });
        drop(state);
        service.configure(gateways);
        Self::start_refresh(&service);
        service
    }

    fn start_refresh(service: &Arc<Self>) {
        let weak = Arc::downgrade(service);
        if std::thread::Builder::new()
            .name("velune-quota-refresh".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    let Some(service) = weak.upgrade() else { break };
                    let due = {
                        let state = service.state.lock().expect("quota state");
                        if state.stopped {
                            break;
                        }
                        let now = Instant::now();
                        state
                            .entries
                            .iter()
                            .filter(|(_, entry)| {
                                !entry.running
                                    && entry.demand_until.is_some_and(|until| until > now)
                                    && entry.retry_at <= now
                            })
                            .map(|(key, entry)| {
                                (
                                    key.clone(),
                                    ViewBinding {
                                        binding: entry.binding.clone(),
                                        incarnation: entry.incarnation.clone(),
                                    },
                                )
                            })
                            .collect::<Vec<_>>()
                    };
                    for ((gateway, provider), binding) in due {
                        let _ = service.query_binding_inner(
                            &gateway,
                            &provider,
                            Refresh::IfDue,
                            Some(&binding),
                            false,
                        );
                    }
                }
            })
            .is_err()
        {
            tracing::warn!(target: "velune_application", event = "quota_refresh_unavailable");
        }
    }

    pub(crate) fn take_identity_changes(&self) -> BTreeSet<Key> {
        std::mem::take(
            &mut *self
                .identity_changes
                .lock()
                .expect("quota identity changes"),
        )
    }

    pub(crate) fn configure(&self, gateways: &[GatewayConfig]) {
        let desired: BTreeMap<Key, (Option<Binding>, Vec<crate::config::QuotaLimit>)> = gateways
            .iter()
            .flat_map(|g| {
                g.providers.iter().map(|p| {
                    let source = p.effective_quota_source();
                    let binding: Option<Binding> = match (&p.authentication.material, source) {
                        (
                            AuthenticationMaterial::CodexSubscription { credential_id, .. },
                            crate::config::ProviderQuotaSource::CodexUsage,
                        ) => Some(Binding {
                            id: credential_id.clone(),
                            generation: p.authentication.generation,
                            source,
                            codex_client_version: Some(p.effective_codex_client_version().into()),
                            binary: None,
                            directory: LoginDirectory::Owned,
                            kimi: None,
                            ark: None,
                            account: None,
                            antigravity: None,
                        }),
                        (
                            AuthenticationMaterial::ClaudeCli { credential_id, .. },
                            crate::config::ProviderQuotaSource::ClaudeCliUsage,
                        ) => Some(Binding {
                            id: credential_id.clone(),
                            generation: p.authentication.generation,
                            source,
                            codex_client_version: None,
                            binary: cli_binary(p),
                            directory: LoginDirectory::Owned,
                            kimi: None,
                            ark: None,
                            account: None,
                            antigravity: None,
                        }),
                        (
                            AuthenticationMaterial::ClaudeCliLinked { config_dir, .. },
                            crate::config::ProviderQuotaSource::ClaudeCliUsage,
                        ) => Some(Binding {
                            id: p.id.clone(),
                            generation: p.authentication.generation,
                            source,
                            codex_client_version: None,
                            binary: cli_binary(p),
                            directory: LoginDirectory::Linked(config_dir.clone()),
                            kimi: None,
                            ark: None,
                            account: None,
                            antigravity: None,
                        }),
                        (
                            AuthenticationMaterial::ApiKey { value },
                            crate::config::ProviderQuotaSource::KimiUsage,
                        ) => crate::config::kimi_quota_region(&p.endpoint).map(|region| Binding {
                            id: p.id.clone(),
                            generation: p.authentication.generation,
                            source,
                            codex_client_version: None,
                            binary: None,
                            directory: LoginDirectory::Owned,
                            ark: None,
                            account: None,
                            antigravity: None,
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
                            source,
                            codex_client_version: None,
                            binary: None,
                            directory: LoginDirectory::Owned,
                            kimi: None,
                            account: None,
                            antigravity: None,
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
                                source,
                                codex_client_version: None,
                                binary: None,
                                directory: LoginDirectory::Owned,
                                kimi: None,
                                ark: None,
                                antigravity: None,
                                account: Some(crate::api_account_quota::Capture {
                                    service,
                                    key: value.clone(),
                                }),
                            },
                        ),
                        (
                            AuthenticationMaterial::AntigravityCli,
                            ProviderQuotaSource::AntigravityCliUsage,
                        ) => match &p.execution_backend {
                            ExecutionBackend::AntigravityCli {
                                binary,
                                ssh_destination,
                            } => Some(Binding {
                                id: p.credential_reference(),
                                generation: p.authentication.generation,
                                source,
                                codex_client_version: None,
                                binary: None,
                                directory: LoginDirectory::Owned,
                                kimi: None,
                                ark: None,
                                account: None,
                                antigravity: Some(AntigravityLauncher::new(
                                    binary.clone(),
                                    ssh_destination.clone(),
                                )),
                            }),
                            _ => None,
                        },
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
        let State {
            entries, alerts, ..
        } = &mut *state;
        alerts.retain(|alert| {
            let key = (alert.gateway_id.clone(), alert.provider_id.clone());
            let old = entries.get(&key).and_then(|entry| entry.binding.as_ref());
            !old.is_some_and(|binding| binding.antigravity.is_some())
                || desired
                    .get(&key)
                    .is_some_and(|(binding, _)| binding.as_ref() == old)
        });
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
        let history_bindings = desired
            .iter()
            .map(|((gateway, provider), (binding, _))| {
                (
                    crate::quota_history::QuotaHistory::name(gateway, provider),
                    binding.as_ref().map(Binding::epoch),
                )
            })
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
                demand_until: None,
                problem: None,
                cancel: tokio::sync::watch::channel(false).0,
            });
        }
        drop(state);
        self.history.configure(history_bindings);
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
        let refresh = if force {
            Refresh::Forced
        } else {
            Refresh::IfDue
        };
        self.query_binding(gateway_id, provider_id, refresh, None)
    }

    /// The provider's explicit refresh action also invalidates an Antigravity
    /// observation if the CLI login may have changed outside Velune.
    pub(crate) fn refresh_provider(
        self: &Arc<Self>,
        gateway_id: &str,
        provider_id: &str,
    ) -> Result<ProviderQuotaStatus, crate::Error> {
        self.query_binding(gateway_id, provider_id, Refresh::Explicit, None)
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
        refresh: Refresh,
        expected: Option<&ViewBinding>,
    ) -> Result<ProviderQuotaStatus, crate::Error> {
        self.query_binding_inner(gateway_id, provider_id, refresh, expected, true)
    }

    fn query_binding_inner(
        self: &Arc<Self>,
        gateway_id: &str,
        provider_id: &str,
        refresh: Refresh,
        expected: Option<&ViewBinding>,
        renew_demand: bool,
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
        // A query renews observation demand; the scheduler does not renew its
        // own lease. Idle providers stop polling after the final caller leaves.
        let antigravity = entry
            .binding
            .as_ref()
            .is_some_and(|binding| binding.antigravity.is_some());
        if antigravity && matches!(refresh, Refresh::Explicit) {
            entry.expire_observation();
            // A user may have switched the external login since the active
            // read began. Let the worker discard its result and refresh again.
            entry.resample |= entry.running;
        }
        let force = !matches!(refresh, Refresh::IfDue);
        if renew_demand {
            entry.demand_until = Some(Instant::now() + Duration::from_secs(120));
        }
        let refresh_due = !fresh(entry)
            || entry.expires_at_ms.saturating_sub(now_ms()) <= 10_000
            || (entry
                .binding
                .as_ref()
                .is_some_and(|binding| binding.kimi.is_some())
                && entry.owner.is_none());
        let start = may_start
            && entry.binding.as_ref().is_some_and(|b| {
                b.source != crate::config::ProviderQuotaSource::ArkCodingUsage || b.ark.is_some()
            })
            && !entry.running
            && (force || (refresh_due && entry.retry_at <= Instant::now()));
        if antigravity && force && !may_start && !entry.running {
            entry.problem = Some("额度读取暂时繁忙，请稍后重新点击刷新。".into());
        }
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
                            if service.defer_identity_retirement {
                                service
                                    .identity_changes
                                    .lock()
                                    .expect("quota identity changes")
                                    .insert(worker_key.clone());
                            } else {
                                service
                                    .scheduling
                                    .invalidate_provider(&worker_key.0, &worker_key.1);
                            }
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
                            if antigravity {
                                entry.expire_observation();
                                entry.problem = Some(if entry.observed_at_ms > 0 {
                                    "Antigravity 额度读取失败。上次观测仅供参考，请确认目标的 agy 登录后再试。"
                                } else {
                                    "Antigravity 额度读取失败。请确认该目标的 agy 已登录，并使用 1.5.0 或更新的稳定版本。"
                                }.into());
                            } else {
                                // An early failure leaves the previous expiry intact.
                                entry.problem =
                                    Some("额度暂不可用；保留上次观测，稍后可重新读取。".into());
                            }
                            return;
                        }
                    };
                    let (observed, expires) =
                        (observation.observed_at_ms, observation.expires_at_ms);
                    let raised = alert_conditions(&observation, &entry.limits, antigravity);
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
                        let lifetime = expires.saturating_sub(now_ms()).max(1);
                        let lead = (lifetime / 5).clamp(1, 10_000);
                        entry.retry_at = Instant::now()
                            + Duration::from_millis(
                                u64::try_from((lifetime - lead).max(1000)).expect("future expiry"),
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
                    service.history.record(&worker_key.0, &worker_key.1, &binding.epoch(), point);
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
                refreshing: entry.running || (antigravity && entry.resample),
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
                entry.expire_observation();
            }
            let _ = self.service.query_binding(
                &self.gateway_id,
                provider_id,
                Refresh::Forced,
                Some(binding),
            );
        }
    }
    fn snapshot(&self, provider_id: &str) -> QuotaSnapshot {
        let Some(binding) = self.bindings.get(provider_id) else {
            return QuotaSnapshot::Unknown;
        };
        let Ok(status) = self.service.query_binding(
            &self.gateway_id,
            provider_id,
            Refresh::IfDue,
            Some(binding),
        ) else {
            return QuotaSnapshot::Unknown;
        };
        if binding.binding.as_ref().is_some_and(|b| {
            b.source == crate::config::ProviderQuotaSource::ArkCodingUsage
                && b.ark.as_ref().is_none_or(|a| !a.confirmed)
        }) {
            return QuotaSnapshot::Unknown;
        }
        let status = status.observation;
        let windows: Vec<QuotaWindow> = status
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
        if binding
            .binding
            .as_ref()
            .is_some_and(|binding| binding.antigravity.is_some())
        {
            return antigravity_projection(windows);
        }
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
/// Keep mapped windows even if one has expired. The scheduler applies each
/// window's own freshness and scope to the candidate model's saved limits.
fn antigravity_projection(mut windows: Vec<QuotaWindow>) -> QuotaSnapshot {
    windows.retain(antigravity_window_mapped);
    if windows.is_empty() {
        return QuotaSnapshot::Unknown;
    }
    let now = now_ms();
    if windows
        .iter()
        .any(|window| antigravity_window_fresh(window, now))
    {
        QuotaSnapshot::Known {
            windows,
            available: None,
        }
    } else {
        QuotaSnapshot::Stale {
            windows,
            available: None,
        }
    }
}

fn antigravity_window_mapped(window: &QuotaWindow) -> bool {
    window.model_ids.as_ref().is_some_and(|ids| !ids.is_empty())
}

fn antigravity_window_fresh(window: &QuotaWindow, now: i64) -> bool {
    window.observed_at_ms > 0
        && window.observed_at_ms <= now
        && window.expires_at_ms > now
        && window.reset_at_ms.is_none_or(|reset| reset > now)
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
    antigravity: bool,
) -> BTreeMap<String, AlertCondition> {
    let mut raised = BTreeMap::new();
    if observation.available == Some(false) {
        raised.insert(
            "unavailable".into(),
            (QuotaAlertKind::Unavailable, None, None),
        );
    }
    for window in &observation.windows {
        if antigravity
            && !(antigravity_window_mapped(window)
                && antigravity_window_fresh(window, observation.observed_at_ms))
        {
            continue;
        }
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
                    crate::config::ProviderQuotaSource::CodexUsage => {
                        let version = binding.codex_client_version.as_deref().ok_or(())?;
                        let response = crate::subscription_auth::read_usage(
                            home, binding.id.clone(), binding.generation, version,
                        )
                        .await
                        .map_err(|_| ())?;
                        parse_windows(&response, ttl_ms)
                    },
                    crate::config::ProviderQuotaSource::ClaudeCliUsage => { let owned=crate::claude_cli::directory(&home,&binding.id); let dir=match &binding.directory { LoginDirectory::Owned=>Some(owned.as_path()), LoginDirectory::Linked(dir)=>dir.as_deref().map(std::path::Path::new) }; let windows=crate::claude_quota::read(binding.binary.as_deref().ok_or(())?,dir,ttl_ms).await?; let observed=windows.first().ok_or(())?.observed_at_ms; let expires=windows.iter().map(|window|window.expires_at_ms).min().ok_or(())?; Ok(observation(windows,None,observed,expires)) },
                    ProviderQuotaSource::AntigravityCliUsage => {
                        let windows = binding.antigravity.as_ref().ok_or(())?.quota_windows(ttl_ms).await?;
                        let observed = windows.first().ok_or(())?.observed_at_ms;
                        let expiries = windows.iter().map(|window| window.expires_at_ms);
                        let expires = expiries
                            .clone()
                            .filter(|expiry| *expiry > observed)
                            .min()
                            .or_else(|| expiries.min())
                            .ok_or(())?;
                        Ok(observation(windows, None, observed, expires))
                    },
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

#[cfg(test)]
mod configuration_tests {
    use super::*;

    fn antigravity_fixture(home: &std::path::Path, executable: &str) -> GatewayConfig {
        use crate::config::{Protocol, ProviderDefinition, RequestProfile};
        GatewayConfig {
            id: "synthetic-gateway".into(),
            name: "Synthetic gateway".into(),
            providers: vec![ProviderDefinition {
                id: "agy".into(),
                name: "Fixture".into(),
                execution_backend: ExecutionBackend::AntigravityCli {
                    binary: home.join(executable).to_string_lossy().into_owned(),
                    ssh_destination: None,
                },
                // Exercise the legacy storage shape and its effective source.
                quota_source: ProviderQuotaSource::None,
                protocol: Protocol::Messages,
                request_profile: RequestProfile::Native,
                endpoint: String::new(),
                authentication: crate::provider_authentication::ProviderAuthentication {
                    material: AuthenticationMaterial::AntigravityCli,
                    generation: 1,
                },
                quota_authentication: Default::default(),
                billing: Default::default(),
                models: Vec::new(),
                quota_limits: Vec::new(),
                codex_client_version: None,
            }],
            routes: Vec::new(),
        }
    }

    fn wait_for(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !condition() {
            assert!(Instant::now() < deadline, "quota worker did not settle");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn antigravity_quota_samples_on_demand_and_expires_refresh_failures() {
        let home = std::env::temp_dir().join(crate::new_record_key());
        let gateway = antigravity_fixture(&home, "missing-agy");
        let service = SubscriptionQuota::new(home.clone(), &[gateway], Default::default());
        let gateway_id = "synthetic-gateway";
        let provider_id = "agy";
        let view = service.source(gateway_id);
        {
            let state = service.state.lock().unwrap();
            assert_eq!(state.inflight, 0);
            assert!(
                state
                    .entries
                    .values()
                    .next()
                    .unwrap()
                    .demand_until
                    .is_none()
            );
        }
        let initial = service
            .query(gateway_id, provider_id, false)
            .unwrap()
            .observation;
        assert!(matches!(initial.freshness, QuotaFreshness::Unknown));
        assert!(initial.refreshing);
        wait_for(|| service.state.lock().unwrap().inflight == 0);
        {
            let mut state = service.state.lock().unwrap();
            let entry = state.entries.values_mut().next().unwrap();
            assert!(entry.demand_until.is_some());
            let now = now_ms();
            entry.observed_at_ms = now;
            entry.expires_at_ms = now + 60_000;
            entry.windows.push(QuotaWindow {
                id: "fixture-group".into(),
                model_ids: Some(vec!["fixture-model".into()]),
                used_percent: 50.0,
                reset_at_ms: None,
                window_ms: None,
                observed_at_ms: now,
                expires_at_ms: now + 60_000,
            });
        }
        assert!(matches!(
            view.snapshot(provider_id),
            QuotaSnapshot::Known { .. }
        ));
        let checking = service
            .refresh_provider(gateway_id, provider_id)
            .unwrap()
            .observation;
        assert!(checking.refreshing);
        assert!(matches!(checking.freshness, QuotaFreshness::Stale));
        wait_for(|| service.state.lock().unwrap().inflight == 0);
        let result = service
            .query(gateway_id, provider_id, false)
            .unwrap()
            .observation;
        assert!(matches!(result.freshness, QuotaFreshness::Stale));
        assert!(!result.refreshing);
        assert!(result.problem.unwrap().contains("上次观测仅供参考"));
        assert_eq!(result.windows.len(), 1);
        assert!(
            result.windows[0]
                .expires_at_ms
                .is_some_and(|expiry| expiry <= now_ms())
        );
        assert!(matches!(
            view.snapshot(provider_id),
            QuotaSnapshot::Stale { .. }
        ));
        assert!(!home.exists());
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_quota_keeps_late_samples_with_their_execution_binding() {
        use std::{fs, os::unix::fs::PermissionsExt};
        let home = std::env::temp_dir().join(crate::new_record_key());
        fs::create_dir(&home).unwrap();
        let binary = home.join("fixture-agy");
        fs::write(
            &binary,
            r#"#!/bin/sh
if [ "$1" = --version ]; then
    printf '1.5.9\n'
    exit 0
fi
if [ "$1" = acp ]; then
    IFS= read -r request
    printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}'
    IFS= read -r request
    printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"fixture-session","models":{"availableModels":[{"modelId":"gemini-4-hydrogen-medium","name":"Gemini 4 Hydrogen (Medium)"}]}}}'
    exit 0
fi
[ "$1" = -p ] && [ "$2" = /usage ] || exit 1
directory="${0%/*}"
report=$(/bin/cat "$directory/report.json") || exit 1
: > "$directory/started"
while [ -f "$directory/hold" ]; do /bin/sleep 0.02; done
printf '%s\n' "$report"
"#,
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let mut report: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/antigravity-quota/usage.json"
        ))
        .unwrap();
        report["command"]["data"]["groups"][0]["buckets"][0]["remaining_fraction"] = 0.2.into();
        let original_reset =
            report["command"]["data"]["groups"][0]["buckets"][1]["reset_time"].clone();
        report["command"]["data"]["groups"][0]["buckets"][1]["remaining_fraction"] = 0.0.into();
        report["command"]["data"]["groups"][0]["buckets"][1]["reset_time"] =
            "2000-01-01T00:00:00Z".into();
        fs::write(home.join("hold"), b"").unwrap();
        fs::write(
            home.join("report.json"),
            serde_json::to_vec(&report).unwrap(),
        )
        .unwrap();

        let mut gateway = antigravity_fixture(&home, "fixture-agy");
        gateway.providers[0]
            .quota_limits
            .push(crate::config::QuotaLimit {
                window_id: "gemini-weekly".into(),
                max_used_percent: 20,
            });
        let service = SubscriptionQuota::new(home.clone(), &[gateway], Default::default());
        let gateway_id = "synthetic-gateway";
        let provider_id = "agy";
        let view = service.source(gateway_id);
        assert!(
            service
                .query(gateway_id, provider_id, false)
                .unwrap()
                .observation
                .refreshing
        );
        wait_for(|| home.join("started").exists());
        // The explicit refresh supersedes the background sample that may
        // come from a login that has just been switched outside Velune.
        assert!(
            service
                .refresh_provider(gateway_id, provider_id)
                .unwrap()
                .observation
                .refreshing
        );
        report["command"]["data"]["groups"][0]["buckets"][0]["remaining_fraction"] = 0.8.into();
        fs::write(
            home.join("report.json"),
            serde_json::to_vec(&report).unwrap(),
        )
        .unwrap();
        fs::remove_file(home.join("hold")).unwrap();
        wait_for(|| {
            !service
                .history(gateway_id, provider_id, 0)
                .unwrap()
                .points
                .is_empty()
        });
        let history = service.history(gateway_id, provider_id, 0).unwrap();
        assert_eq!(history.points.len(), 1);
        assert_eq!(history.points[0].windows.len(), 5);
        let weekly = history.points[0]
            .windows
            .iter()
            .find(|window| window.id == "gemini-weekly")
            .unwrap();
        assert_eq!(weekly.used_percent, 20.0);
        assert!(matches!(
            service
                .query(gateway_id, provider_id, false)
                .unwrap()
                .observation
                .freshness,
            QuotaFreshness::Fresh
        ));
        assert!(
            service
                .state
                .lock()
                .unwrap()
                .entries
                .values()
                .next()
                .unwrap()
                .expires_at_ms
                > now_ms()
        );
        let alerts = service.alerts(0);
        assert_eq!(alerts.alerts.len(), 1);
        assert!(matches!(alerts.alerts[0].kind, QuotaAlertKind::CapReached));
        assert_eq!(alerts.alerts[0].window_id.as_deref(), Some("gemini-weekly"));
        assert!(matches!(
            view.snapshot(provider_id),
            QuotaSnapshot::Known { .. }
        ));

        fs::remove_file(home.join("started")).unwrap();
        fs::write(home.join("hold"), b"").unwrap();
        report["command"]["data"]["groups"][0]["buckets"][0]["remaining_fraction"] = 0.0.into();
        report["command"]["data"]["groups"][0]["buckets"][1]["reset_time"] = original_reset;
        fs::write(
            home.join("report.json"),
            serde_json::to_vec(&report).unwrap(),
        )
        .unwrap();
        assert!(
            service
                .refresh_provider(gateway_id, provider_id)
                .unwrap()
                .observation
                .refreshing
        );
        wait_for(|| home.join("started").exists());
        assert_eq!(service.state.lock().unwrap().inflight, 1);

        // An in-flight execution keeps its original binding when the saved
        // executable is changed. The late observation cannot cross that boundary.
        let changed = antigravity_fixture(&home, "other-agy");
        let next = service.fork_configuration(&[changed]);
        let unread = next.alerts(0);
        assert!(unread.alerts.is_empty());
        assert_eq!(unread.cursor, alerts.cursor);
        next.state
            .lock()
            .unwrap()
            .entries
            .values_mut()
            .next()
            .unwrap()
            .retry_at = Instant::now() + Duration::from_secs(3600);
        let new = next
            .query(gateway_id, provider_id, false)
            .unwrap()
            .observation;
        assert!(matches!(new.freshness, QuotaFreshness::Unknown));
        assert!(!new.refreshing);
        fs::remove_file(home.join("hold")).unwrap();
        wait_for(|| service.state.lock().unwrap().inflight == 0);

        let old = service
            .query(gateway_id, provider_id, false)
            .unwrap()
            .observation;
        assert!(
            matches!(old.freshness, QuotaFreshness::Fresh),
            "{:?}",
            old.problem
        );
        assert_eq!(old.windows.len(), 5);
        assert!(
            old.windows
                .iter()
                .any(|window| window.used_percent == 100.0)
        );
        assert!(matches!(
            view.snapshot(provider_id),
            QuotaSnapshot::Known { .. }
        ));
        assert!(next.alerts(alerts.cursor).alerts.is_empty());
        assert!(
            next.query(gateway_id, provider_id, false)
                .unwrap()
                .observation
                .windows
                .is_empty()
        );
        assert!(
            next.history(gateway_id, provider_id, 0)
                .unwrap()
                .points
                .is_empty()
        );
        assert_eq!(
            service
                .history(gateway_id, provider_id, 0)
                .unwrap()
                .points
                .len(),
            1
        );
        assert_eq!(next.state.lock().unwrap().inflight, 0);
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn antigravity_windows_drive_smart_and_protect_scoped_caps_at_gateway_ingress() {
        use crate::config::{ModelRoute, ProviderModel, QuotaLimit, RoutingStrategy};
        use reqwest::{Client, StatusCode};
        use velune_gateway::{
            CredentialResolutionError, CredentialResolver, CredentialTarget, ResolvedCredential,
            Runner,
        };

        #[derive(Default)]
        struct Attempts(Mutex<Vec<String>>);
        impl CredentialResolver for Attempts {
            fn resolve(
                &self,
                reference: String,
                _: CredentialTarget,
            ) -> velune_ai::OperationFuture<Result<ResolvedCredential, CredentialResolutionError>>
            {
                self.0.lock().unwrap().push(reference);
                Box::pin(async { Err(CredentialResolutionError::Unavailable) })
            }
        }

        let home = std::env::temp_dir().join(crate::new_record_key());
        let mut gateway = antigravity_fixture(&home, "fixture-agy");
        let model = |record_key: &str, provider_model_id: &str| ProviderModel {
            record_key: record_key.into(),
            provider_model_id: provider_model_id.into(),
            nickname: record_key.into(),
            icon: None,
            context_window: None,
            max_output_tokens: None,
            reasoning_levels: None,
            request_parameters: None,
            pi_projection: None,
        };
        let mut backup = gateway.providers[0].clone();
        backup.id = "second-agy".into();
        backup.execution_backend = ExecutionBackend::AntigravityCli {
            binary: "/fixture/agy".into(),
            ssh_destination: Some("second-fixture-target".into()),
        };
        backup.models = vec![model("backup-model", "hydrogen")];
        gateway.providers[0].models = vec![
            model("primary-model", "hydrogen"),
            model("other-model", "argon"),
        ];
        gateway.providers[0].quota_limits.push(QuotaLimit {
            window_id: "gemini-weekly".into(),
            max_used_percent: 70,
        });
        gateway.providers.push(backup);
        gateway.routes.push(ModelRoute {
            record_key: "smart-fixture".into(),
            name: "Smart fixture".into(),
            members: vec!["primary-model".into(), "backup-model".into()],
            strategy: RoutingStrategy::Smart,
            sticky: false,
            weights: BTreeMap::new(),
        });
        gateway.validate().unwrap();
        let service =
            SubscriptionQuota::new(home, std::slice::from_ref(&gateway), Default::default());
        let now = now_ms();
        let sample = |id: &str, scope: &[&str], used_percent, expiry| QuotaWindow {
            id: id.into(),
            model_ids: Some(scope.iter().map(|id| (*id).to_owned()).collect()),
            used_percent,
            reset_at_ms: None,
            window_ms: None,
            observed_at_ms: now,
            expires_at_ms: expiry,
        };
        for (provider, windows) in [
            (
                "agy",
                vec![
                    sample("gemini-weekly", &["hydrogen"], 65.0, now + 60_000),
                    sample("argon-weekly", &["argon"], 95.0, now + 60_000),
                    sample("unknown-group", &[], 100.0, now),
                ],
            ),
            (
                "second-agy",
                vec![sample("gemini-weekly", &["hydrogen"], 20.0, now + 60_000)],
            ),
        ] {
            let mut state = service.state.lock().unwrap();
            let entry = state
                .entries
                .get_mut(&(gateway.id.clone(), provider.into()))
                .unwrap();
            entry.observed_at_ms = now;
            entry.expires_at_ms = windows
                .iter()
                .map(|window| window.expires_at_ms)
                .min()
                .unwrap();
            entry.windows = windows;
            // Neither fixture target is contacted. These are recorded windows
            // supplied to the production quota projection and gateway.
            entry.retry_at = Instant::now() + Duration::from_secs(3600);
        }
        let primary_ref = gateway.providers[0].credential_reference();
        let backup_ref = gateway.providers[1].credential_reference();
        let attempts = Arc::new(Attempts::default());
        let runner = Runner::start_with_scheduling(
            gateway.to_gateway_config().unwrap(),
            attempts.clone(),
            BTreeMap::new(),
            None,
            Default::default(),
            service.source(&gateway.id),
            None,
            Default::default(),
        )
        .unwrap();
        let request = |model| {
            Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap()
                .post(format!("{}/messages", runner.endpoint()))
                .bearer_auth(runner.token())
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .body(
                    serde_json::json!({
                        "model": format!("velune/model/{model}"),
                        "max_tokens": 64,
                        "messages": [{"role":"user","content":"fixture"}],
                    })
                    .to_string(),
                )
                .send()
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            request("smart-fixture").await.unwrap();
            let first = std::mem::take(&mut *attempts.0.lock().unwrap());
            assert_eq!(first.first(), Some(&backup_ref));

            let key = (gateway.id.clone(), "agy".into());
            service
                .state
                .lock()
                .unwrap()
                .entries
                .get_mut(&key)
                .unwrap()
                .windows[0]
                .used_percent = 80.0;
            assert_eq!(
                request("primary-model").await.unwrap().status(),
                StatusCode::TOO_MANY_REQUESTS
            );
            assert!(attempts.0.lock().unwrap().is_empty());

            request("other-model").await.unwrap();
            let other = std::mem::take(&mut *attempts.0.lock().unwrap());
            assert_eq!(other, [primary_ref]);

            service
                .state
                .lock()
                .unwrap()
                .entries
                .get_mut(&key)
                .unwrap()
                .windows[0]
                .model_ids = Some(Vec::new());
            let missing = request("primary-model").await.unwrap();
            assert_eq!(missing.status(), StatusCode::SERVICE_UNAVAILABLE);
            let error: Value = serde_json::from_slice(&missing.bytes().await.unwrap()).unwrap();
            assert_eq!(error["error"]["code"], "quota_window_missing");
            assert!(attempts.0.lock().unwrap().is_empty());
        });
    }

    #[test]
    fn configuration_fork_preserves_quota_and_does_not_cancel_the_running_version() {
        let home =
            std::env::temp_dir().join(format!("velune-quota-fork-{}", crate::new_record_key()));
        let provider = crate::config::ProviderDefinition {
            id: "fixture".into(),
            name: "Fixture".into(),
            protocol: crate::config::Protocol::Responses,
            endpoint: crate::subscription_auth::ENDPOINT.into(),
            execution_backend: crate::config::ExecutionBackend::Http,
            request_profile: crate::config::RequestProfile::CodexResponses,
            codex_client_version: Some("0.166.7".into()),
            quota_source: crate::config::ProviderQuotaSource::CodexUsage,
            authentication: crate::provider_authentication::ProviderAuthentication {
                material: AuthenticationMaterial::CodexSubscription {
                    credential_id: "synthetic".into(),
                    configured: true,
                },
                generation: 1,
            },
            quota_authentication: Default::default(),
            billing: Default::default(),
            models: Vec::new(),
            quota_limits: Vec::new(),
        };
        let mut gateways = vec![GatewayConfig {
            id: "fixture".into(),
            name: "Fixture".into(),
            providers: vec![provider],
            routes: Vec::new(),
        }];
        let service = SubscriptionQuota::new(home.clone(), &gateways, Default::default());
        let now = now_ms();
        let old_cancellation = {
            let mut state = service.state.lock().unwrap();
            let entry = state
                .entries
                .get_mut(&("fixture".into(), "fixture".into()))
                .unwrap();
            entry.observed_at_ms = now;
            entry.expires_at_ms = now + 60_000;
            entry.windows = vec![QuotaWindow {
                id: "fixture-window".into(),
                model_ids: None,
                used_percent: 98.0,
                reset_at_ms: Some(now + 60_000),
                window_ms: Some(300_000),
                observed_at_ms: now,
                expires_at_ms: now + 60_000,
            }];
            entry.cancel.subscribe()
        };
        gateways[0].providers[0].name = "Saved label".into();
        let next = service.fork_configuration(&gateways);
        let old_epoch = service.state.lock().unwrap().entries
            [&("fixture".into(), "fixture".into())]
            .binding
            .as_ref()
            .unwrap()
            .epoch();
        let point = crate::QuotaHistoryPoint {
            observed_at_ms: now,
            available: Some(true),
            windows: Vec::new(),
            balances: Vec::new(),
        };
        service
            .history
            .record("fixture", "fixture", &old_epoch, point.clone());
        let observation = next.query("fixture", "fixture", false).unwrap().observation;
        assert!(matches!(observation.freshness, QuotaFreshness::Fresh));
        assert_eq!(observation.windows[0].used_percent, 98.0);
        assert!(!observation.refreshing);
        // A version edit refreshes the sample while keeping the account history.
        gateways[0].providers[0].codex_client_version = Some("0.166.8".into());
        next.configure(&gateways);
        {
            let state = next.state.lock().unwrap();
            let entry = &state.entries[&("fixture".into(), "fixture".into())];
            assert!(!fresh(entry));
            let binding = entry.binding.as_ref().unwrap();
            assert_eq!(binding.codex_client_version.as_deref(), Some("0.166.8"));
            assert_eq!(binding.epoch(), old_epoch);
        }
        assert_eq!(
            next.history("fixture", "fixture", 0).unwrap().points,
            vec![point.clone()]
        );
        // Replacing the next version's binding must not revoke the running one.
        gateways[0].providers[0].authentication.generation = 2;
        next.configure(&gateways);
        let new_epoch = next.state.lock().unwrap().entries[&("fixture".into(), "fixture".into())]
            .binding
            .as_ref()
            .unwrap()
            .epoch();
        next.history
            .record("fixture", "fixture", &new_epoch, point.clone());
        // A late old worker cannot replace the current binding's retained history.
        service
            .history
            .record("fixture", "fixture", &old_epoch, point.clone());
        assert_eq!(
            next.history("fixture", "fixture", 0).unwrap().points,
            vec![point]
        );
        assert!(!*old_cancellation.borrow());
        let old = service
            .query("fixture", "fixture", false)
            .unwrap()
            .observation;
        assert!(matches!(old.freshness, QuotaFreshness::Fresh));
        assert_eq!(old.windows[0].used_percent, 98.0);
        next.configure(&[]);
        service.history.record(
            "fixture",
            "fixture",
            &old_epoch,
            crate::QuotaHistoryPoint {
                observed_at_ms: now,
                available: Some(true),
                windows: Vec::new(),
                balances: Vec::new(),
            },
        );
        assert!(
            std::fs::read_dir(home.join("quota-history"))
                .unwrap()
                .next()
                .is_none()
        );
        service.stop();
        next.stop();
        let _ = std::fs::remove_dir_all(home);
    }
}
