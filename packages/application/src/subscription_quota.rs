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
#[derive(Clone, PartialEq, Eq)]
struct Binding {
    id: String,
    generation: u64,
    source: crate::config::ProviderQuotaSource,
    binary: Option<String>,
}
struct Entry {
    binding: Option<Binding>,
    windows: Vec<QuotaWindow>,
    available: Option<bool>,
    observed_at_ms: i64,
    expires_at_ms: i64,
    running: bool,
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
    state: Mutex<State>,
}
struct GatewayView {
    service: Arc<SubscriptionQuota>,
    gateway_id: String,
    bindings: BTreeMap<String, Option<Binding>>,
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
    pub(crate) fn new(home: PathBuf, gateways: &[GatewayConfig]) -> Arc<Self> {
        let service = Arc::new(Self {
            home,
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
                        }),
                        (
                            AuthenticationMaterial::ClaudeCli { credential_id, .. },
                            crate::config::ProviderQuotaSource::ClaudeCliUsage,
                        ) => {
                            let binary = match &p.execution_backend {
                                crate::config::ExecutionBackend::ClaudeCli { binary } => {
                                    Some(binary.clone())
                                }
                                _ => None,
                            };
                            Some(Binding {
                                id: credential_id.clone(),
                                generation: p.authentication.generation,
                                source: p.quota_source,
                                binary,
                            })
                        }
                        _ => None,
                    };
                    ((g.id.clone(), p.id.clone()), binding)
                })
            })
            .collect();
        let mut state = self.state.lock().expect("quota state");
        state.entries.retain(|key, entry| {
            let keep = desired.get(key) == Some(&entry.binding);
            if !keep {
                let _ = entry.cancel.send(true);
            }
            keep
        });
        for (key, binding) in desired {
            state.entries.entry(key).or_insert_with(|| Entry {
                binding,
                windows: Vec::new(),
                available: None,
                observed_at_ms: 0,
                expires_at_ms: 0,
                running: false,
                retry_at: Instant::now(),
                problem: None,
                cancel: tokio::sync::watch::channel(false).0,
            });
        }
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
                .map(|(key, entry)| (key.1.clone(), entry.binding.clone()))
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
        expected: Option<&Option<Binding>>,
    ) -> Result<ProviderQuotaStatus, crate::Error> {
        let key = (gateway_id.to_owned(), provider_id.to_owned());
        let mut state = self.state.lock().expect("quota state");
        let may_start = !state.stopped && state.inflight < 4;
        let entry = state
            .entries
            .get_mut(&key)
            .ok_or_else(|| crate::Error::invalid("provider id"))?;
        if expected.is_some_and(|binding| binding != &entry.binding) {
            return Err(crate::Error::invalid("quota binding changed"));
        }
        let start = may_start
            && entry.binding.is_some()
            && !entry.running
            && (force || (!fresh(entry) && entry.retry_at <= Instant::now()));
        if start {
            entry.running = true;
            entry.problem = None;
            let binding = entry.binding.clone().expect("native subscription");
            let cancel = entry.cancel.subscribe();
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
                    let result = collect(home, &binding, cancel, refresh_ms);
                    let Some(service) = weak.upgrade() else {
                        return;
                    };
                    let mut state = service.state.lock().expect("quota state");
                    state.inflight -= 1;
                    if state.stopped {
                        return;
                    }
                    let Some(entry) = state
                        .entries
                        .get_mut(&worker_key)
                        .filter(|entry| entry.binding.as_ref() == Some(&binding))
                    else {
                        return;
                    };
                    entry.running = false;
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
            problem: entry.problem.clone(),
        })
    }
}
impl Drop for SubscriptionQuota {
    fn drop(&mut self) {
        self.stop();
    }
}
impl QuotaSource for GatewayView {
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
) -> Result<(Vec<QuotaWindow>, Option<bool>, i64, i64), ()> {
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
                    crate::config::ProviderQuotaSource::ClaudeCliUsage => { let text=crate::claude_cli::usage(binding.binary.as_deref().ok_or(())?,&crate::claude_cli::directory(&home,&binding.id)).await?; parse_claude_windows(&text,ttl_ms) },
                    crate::config::ProviderQuotaSource::None => Err(()),
                }
            } => result,
            _ = cancel.changed() => Err(()),
        }
    })
}

fn parse_windows(
    value: &Value,
    ttl_ms: i64,
) -> Result<(Vec<QuotaWindow>, Option<bool>, i64, i64), ()> {
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

fn parse_claude_windows(
    text: &str,
    ttl_ms: i64,
) -> Result<(Vec<QuotaWindow>, Option<bool>, i64, i64), ()> {
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
