//! The application owns subscription observations; gateways only read this shared view.
use crate::{ProviderQuotaStatus, ProviderQuotaWindow, QuotaFreshness};
use crate::{config::GatewayConfig, provider_authentication::AuthenticationMaterial};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use velune_gateway::{QuotaSnapshot, QuotaSource, QuotaWindow};

type Key = (String, String);
type Observation = (Vec<QuotaWindow>, Option<bool>, i64, i64);
type ObservationResult = Result<Observation, ()>;
#[derive(Clone, PartialEq, Eq)]
struct Binding {
    id: String,
    generation: u64,
    source: crate::config::ProviderQuotaSource,
    binary: Option<String>,
    directory: LoginDirectory,
    kimi: Option<crate::kimi_quota::Capture>,
    ark: Option<crate::ark_quota::Capture>,
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
    available: Option<bool>,
    observed_at_ms: i64,
    expires_at_ms: i64,
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
}
pub(crate) struct SubscriptionQuota {
    home: PathBuf,
    scheduling: velune_gateway::SchedulingState,
    state: Mutex<State>,
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
            home,
            scheduling,
            state: Mutex::new(State::default()),
        });
        service.configure(gateways);
        service
    }
    pub(crate) fn configure(&self, gateways: &[GatewayConfig]) {
        let desired: BTreeMap<Key, Option<Binding>> = gateways
            .iter()
            .flat_map(|g| {
                g.providers.iter().map(|p| {
                    let binding = match (&p.authentication.material, p.quota_source) {
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
                            ark: p.quota_authentication.access_key.as_ref().map(|key| {
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
                        _ => None,
                    };
                    ((g.id.clone(), p.id.clone()), binding)
                })
            })
            .collect();
        let mut state = self.state.lock().expect("quota state");
        state.entries.retain(|key, entry| {
            if desired.get(key) == Some(&entry.binding) {
                return true;
            }
            let _ = entry.cancel.send(true);
            if let Some(binding) = desired.get(key)
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
        for (key, binding) in desired {
            state.entries.entry(key).or_insert_with(|| Entry {
                incarnation: crate::new_record_key(),
                binding,
                owner: None,
                reservation: None,
                identity_rejected: false,
                windows: Vec::new(),
                available: None,
                observed_at_ms: 0,
                expires_at_ms: 0,
                running: false,
                resample: false,
                retry_at: Instant::now(),
                problem: None,
                cancel: tokio::sync::watch::channel(false).0,
            });
        }
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
                            entry.available = None;
                            entry.observed_at_ms = 0;
                            entry.expires_at_ms = 0;
                        }
                        entry.owner = Some(owner);
                        entry.reservation = None;
                    }
                    entry.retry_at = Instant::now() + Duration::from_secs(50 + jitter);
                    match result {
                        Ok((windows, available, observed, expires)) => {
                            entry.windows = windows;
                            entry.available = available;
                            entry.observed_at_ms = observed;
                            entry.expires_at_ms = expires;
                            entry.problem = None;
                            if expires > observed {
                                entry.retry_at = Instant::now()
                                    + Duration::from_millis(
                                        u64::try_from(expires - observed).expect("future expiry"),
                                    );
                            }
                        }
                        Err(()) => {
                            let expired = now_ms();
                            entry.expires_at_ms = entry.expires_at_ms.min(expired);
                            for window in &mut entry.windows {
                                window.expires_at_ms = window.expires_at_ms.min(expired);
                            }
                            entry.problem =
                                Some("额度暂不可用；保留上次观测，稍后可重新读取。".into())
                        }
                    }
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
                .map(|w| ProviderQuotaWindow {
                    id: w.id.clone(),
                    model_ids: w.model_ids.clone(),
                    used_percent: w.used_percent,
                    reset_at_ms: w.reset_at_ms,
                    observed_at_ms: w.observed_at_ms,
                    expires_at_ms: w.expires_at_ms,
                })
                .collect(),
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
        let windows = status
            .windows
            .into_iter()
            .map(|w| QuotaWindow {
                id: w.id,
                model_ids: w.model_ids,
                used_percent: w.used_percent,
                reset_at_ms: w.reset_at_ms,
                observed_at_ms: w.observed_at_ms,
                expires_at_ms: w.expires_at_ms,
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
                    crate::config::ProviderQuotaSource::ClaudeCliUsage => { let owned=crate::claude_cli::directory(&home,&binding.id); let dir=match &binding.directory { LoginDirectory::Owned=>Some(owned.as_path()), LoginDirectory::Linked(dir)=>dir.as_deref().map(std::path::Path::new) }; let text=crate::claude_cli::usage(binding.binary.as_deref().ok_or(())?,dir).await?; parse_claude_windows(&text,ttl_ms) },
                    crate::config::ProviderQuotaSource::ArkCodingUsage => { let observed=now_ms(); let windows=binding.ark.as_ref().ok_or(())?.windows(observed,ttl_ms).await?; let expires=windows.iter().map(|w|w.expires_at_ms).min().ok_or(())?; Ok((windows,None,observed,expires)) },
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
    Ok((windows, available, observed, expires))
}

fn parse_claude_windows(text: &str, ttl_ms: i64) -> ObservationResult {
    let observed = now_ms();
    if observed <= 0 {
        return Err(());
    }
    let mut windows = Vec::new();
    for line in text.lines().map(str::trim) {
        let (id, body) = if let Some(body) = line.strip_prefix("Current session:") {
            ("window:18000", body)
        } else if let Some(body) = line
            .strip_prefix("Current week (all models):")
            .or_else(|| line.strip_prefix("Current week:"))
        {
            ("window:604800", body)
        } else {
            continue;
        };
        let (used, tail) = body.trim().split_once("% used").ok_or(())?;
        if !tail.is_empty() && !tail.trim_start().starts_with('·') {
            return Err(());
        }
        let used = used.parse::<f64>().map_err(|_| ())?;
        if !used.is_finite()
            || !(0.0..=100.0).contains(&used)
            || windows.iter().any(|w: &QuotaWindow| w.id == id)
        {
            return Err(());
        }
        // Human reset strings are not a stable timestamp contract. TTL requires re-observation; never infer replenishment.
        windows.push(QuotaWindow {
            id: id.into(),
            model_ids: None,
            used_percent: used,
            reset_at_ms: None,
            observed_at_ms: observed,
            expires_at_ms: observed + ttl_ms,
        });
    }
    if windows.is_empty() {
        return Err(());
    }
    Ok((windows, None, observed, observed + ttl_ms))
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
        let result=tokio::select! { result=capture.windows(observed,ttl)=>result.map(|windows| { let expires=windows.iter().map(|w|w.expires_at_ms).min().expect("nonempty windows"); (windows,None,observed,expires) }), _=cancel.changed()=>Err(()) };
        (Some(owner),result)
    })
}
