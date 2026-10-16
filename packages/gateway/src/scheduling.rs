//! Request admission and session scheduling; quota observations remain owned by the caller.
use crate::config::{QuotaLimit, RoutingStrategy};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub struct QuotaWindow {
    pub id: String,
    /// Exact provider model IDs, or all models when absent.
    pub model_ids: Option<Vec<String>>,
    pub used_percent: f64,
    pub reset_at_ms: Option<i64>,
    pub observed_at_ms: i64,
    pub expires_at_ms: i64,
}
#[derive(Debug, Clone)]
pub enum QuotaSnapshot {
    Known {
        available: Option<bool>,
        windows: Vec<QuotaWindow>,
    },
    Unknown,
    Stale {
        available: Option<bool>,
        windows: Vec<QuotaWindow>,
    },
    Unsupported,
}
/// A nonblocking read of caller-owned observations. Implementations may enqueue
/// collection work, but must not perform network I/O while serving this call.
pub trait QuotaSource: Send + Sync {
    fn snapshot(&self, provider_id: &str) -> QuotaSnapshot;
}
pub struct UnknownQuotaSource;
impl QuotaSource for UnknownQuotaSource {
    fn snapshot(&self, _: &str) -> QuotaSnapshot {
        QuotaSnapshot::Unknown
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    pub namespace: String,
    pub id: String,
}

/// Runtime-only health, allocation counters and successful session bindings.
/// Share across runners; provider invalidation isolates authentication epochs.
#[derive(Clone, Default)]
pub struct SchedulingState(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    providers: BTreeMap<(String, String), ProviderState>,
    routes: BTreeMap<(String, String), RouteState>,
    sessions: BTreeMap<(String, String, String, String), Binding>,
}
#[derive(Default)]
struct ProviderState {
    epoch: u64,
    cooldown_generation: u64,
    cooldown: Option<Cooldown>,
}
struct Cooldown {
    until: Instant,
    probing: bool,
}
#[derive(Default)]
struct RouteState {
    strategy: Option<RoutingStrategy>,
    members: Vec<(String, u32)>,
    cursor: usize,
    credits: BTreeMap<String, i128>,
}
struct Binding {
    provider: String,
    model: String,
    epoch: u64,
}

pub(crate) struct Candidate<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub model_id: &'a str,
    pub epoch: u64,
    pub limits: &'a [QuotaLimit],
    pub weight: u32,
}
pub(crate) struct ScheduleUnavailable {
    pub retry_after: Option<Duration>,
}
struct Eligible {
    index: usize,
    margin: Option<f64>,
}

impl SchedulingState {
    /// Invalidate an authentication binding, not a normal access-token refresh.
    /// Captured old runners and in-flight results cannot modify the new epoch.
    pub fn invalidate_provider(&self, gateway_id: &str, provider_id: &str) {
        let mut state = self.0.lock().expect("scheduling state");
        let provider = state
            .providers
            .entry((gateway_id.into(), provider_id.into()))
            .or_default();
        provider.epoch = provider.epoch.wrapping_add(1);
        provider.cooldown_generation = provider.cooldown_generation.wrapping_add(1);
        provider.cooldown = None;
        state.sessions.retain(|(gateway, _, _, _), binding| {
            gateway != gateway_id || binding.provider != provider_id
        });
    }
    pub(crate) fn epoch(&self, gateway: &str, provider: &str) -> u64 {
        self.0
            .lock()
            .expect("scheduling state")
            .providers
            .entry((gateway.into(), provider.into()))
            .or_default()
            .epoch
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rank(
        &self,
        gateway: &str,
        route: &str,
        strategy: RoutingStrategy,
        sticky: bool,
        session: Option<&SessionIdentity>,
        candidates: &[Candidate<'_>],
        quotas: &dyn QuotaSource,
    ) -> Result<Vec<usize>, ScheduleUnavailable> {
        let now_ms = crate::analytics::now_ms();
        let facts: Vec<_> = candidates
            .iter()
            .map(|candidate| quota_margin(candidate, quotas.snapshot(candidate.provider), now_ms))
            .collect();
        let now = Instant::now();
        let mut state = self.0.lock().expect("scheduling state");
        let mut eligible = Vec::new();
        let mut retry_after = None;
        for (index, candidate) in candidates.iter().enumerate() {
            let provider = state
                .providers
                .entry((gateway.into(), candidate.provider.into()))
                .or_default();
            if provider.epoch != candidate.epoch {
                continue;
            }
            if let Some(cooldown) = &provider.cooldown
                && (cooldown.until > now || cooldown.probing)
            {
                let delay = cooldown
                    .until
                    .saturating_duration_since(now)
                    .max(Duration::from_secs(1));
                earliest(&mut retry_after, delay);
                continue;
            }
            match facts[index] {
                Ok(margin) => eligible.push(Eligible { index, margin }),
                Err(Some(delay)) => earliest(&mut retry_after, delay),
                Err(None) => {}
            }
        }
        if eligible.is_empty() {
            return Err(ScheduleUnavailable { retry_after });
        }
        if strategy == RoutingStrategy::Smart {
            eligible.sort_by(|a, b| match (a.margin, b.margin) {
                (Some(a), Some(b)) => b.total_cmp(&a),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            });
        }
        if sticky && let Some(session) = session {
            let key = (
                gateway.into(),
                session.namespace.clone(),
                session.id.clone(),
                route.into(),
            );
            if let Some(binding) = state.sessions.get(&key)
                && let Some(position) = eligible.iter().position(|entry| {
                    let candidate = &candidates[entry.index];
                    candidate.provider == binding.provider
                        && candidate.model == binding.model
                        && candidate.epoch == binding.epoch
                })
            {
                let bound = eligible.remove(position);
                eligible.insert(0, bound);
                return Ok(eligible.into_iter().map(|entry| entry.index).collect());
            }
        }
        let route_state = state
            .routes
            .entry((gateway.into(), route.into()))
            .or_default();
        let members: Vec<_> = candidates
            .iter()
            .map(|candidate| (candidate.model.to_owned(), candidate.weight))
            .collect();
        if route_state.strategy != Some(strategy) || route_state.members != members {
            route_state.strategy = Some(strategy);
            route_state.members = members;
            route_state.cursor = 0;
            route_state.credits.clear();
        }
        match strategy {
            RoutingStrategy::Smart | RoutingStrategy::Order => {}
            RoutingStrategy::Rotate => {
                let start = eligible
                    .iter()
                    .position(|entry| entry.index >= route_state.cursor)
                    .unwrap_or(0);
                eligible.rotate_left(start);
                route_state.cursor = (eligible[0].index + 1) % candidates.len();
            }
            RoutingStrategy::Weight => {
                let mut total = 0i128;
                for entry in &eligible {
                    let candidate = &candidates[entry.index];
                    total += i128::from(candidate.weight);
                    *route_state
                        .credits
                        .entry(candidate.model.into())
                        .or_default() += i128::from(candidate.weight);
                }
                eligible.sort_by_key(|entry| {
                    std::cmp::Reverse(route_state.credits[candidates[entry.index].model])
                });
                *route_state
                    .credits
                    .get_mut(candidates[eligible[0].index].model)
                    .expect("weighted member") -= total;
                // Ineligible members cannot accumulate credit while paused.
                route_state.credits.retain(|model, _| {
                    eligible
                        .iter()
                        .any(|entry| candidates[entry.index].model == model)
                });
            }
        }
        Ok(eligible.into_iter().map(|entry| entry.index).collect())
    }
    pub(crate) fn next_retry(
        &self,
        gateway: &str,
        candidates: &[Candidate<'_>],
        quotas: &dyn QuotaSource,
    ) -> Option<Duration> {
        let now_ms = crate::analytics::now_ms();
        let facts: Vec<_> = candidates
            .iter()
            .map(|candidate| quota_margin(candidate, quotas.snapshot(candidate.provider), now_ms))
            .collect();
        let now = Instant::now();
        let state = self.0.lock().expect("scheduling state");
        let mut retry = None;
        for (candidate, quota) in candidates.iter().zip(facts) {
            let Some(provider) = state
                .providers
                .get(&(gateway.into(), candidate.provider.into()))
            else {
                continue;
            };
            if provider.epoch != candidate.epoch {
                continue;
            }
            let quota_delay = match quota {
                Ok(_) => Duration::ZERO,
                Err(Some(delay)) => delay,
                Err(None) => continue,
            };
            let health_delay = provider
                .cooldown
                .as_ref()
                .map_or(Duration::ZERO, |cooldown| {
                    cooldown.until.saturating_duration_since(now)
                });
            earliest(
                &mut retry,
                quota_delay.max(health_delay).max(Duration::from_secs(1)),
            );
        }
        retry
    }
    pub(crate) fn acquire(
        &self,
        gateway: &str,
        candidate: &Candidate<'_>,
        quotas: &dyn QuotaSource,
    ) -> Result<SchedulePermit, ScheduleUnavailable> {
        quota_margin(
            candidate,
            quotas.snapshot(candidate.provider),
            crate::analytics::now_ms(),
        )
        .map_err(|retry_after| ScheduleUnavailable { retry_after })?;
        let mut state = self.0.lock().expect("scheduling state");
        let provider = state
            .providers
            .entry((gateway.into(), candidate.provider.into()))
            .or_default();
        if provider.epoch != candidate.epoch {
            return Err(ScheduleUnavailable { retry_after: None });
        }
        let probe = if let Some(cooldown) = &mut provider.cooldown {
            if cooldown.until > Instant::now() || cooldown.probing {
                return Err(ScheduleUnavailable {
                    retry_after: Some(
                        cooldown
                            .until
                            .saturating_duration_since(Instant::now())
                            .max(Duration::from_secs(1)),
                    ),
                });
            }
            cooldown.probing = true;
            true
        } else {
            false
        };
        Ok(SchedulePermit {
            state: self.clone(),
            gateway: gateway.into(),
            provider: candidate.provider.into(),
            epoch: provider.epoch,
            generation: provider.cooldown_generation,
            probe,
        })
    }
}
fn earliest(value: &mut Option<Duration>, delay: Duration) {
    *value = Some(value.map_or(delay, |old| old.min(delay)));
}

/// A single half-open permit. Cancellation and early return release it on drop.
pub(crate) struct SchedulePermit {
    state: SchedulingState,
    gateway: String,
    provider: String,
    epoch: u64,
    generation: u64,
    probe: bool,
}
impl SchedulePermit {
    pub(crate) fn cool(&self, duration: Duration) {
        let mut state = self.state.0.lock().expect("scheduling state");
        let provider = state
            .providers
            .get_mut(&(self.gateway.clone(), self.provider.clone()))
            .expect("captured provider");
        if provider.epoch != self.epoch {
            return;
        }
        let until = Instant::now() + duration;
        let until = provider
            .cooldown
            .as_ref()
            .map_or(until, |old| old.until.max(until));
        provider.cooldown_generation = provider.cooldown_generation.wrapping_add(1);
        provider.cooldown = Some(Cooldown {
            until,
            probing: false,
        });
    }
    pub(crate) fn completed(&self, route: &str, model: &str, session: Option<&SessionIdentity>) {
        let mut state = self.state.0.lock().expect("scheduling state");
        let provider = state
            .providers
            .get_mut(&(self.gateway.clone(), self.provider.clone()))
            .expect("captured provider");
        if provider.epoch != self.epoch {
            return;
        }
        if self.probe && provider.cooldown_generation == self.generation {
            provider.cooldown = None;
        }
        if let Some(session) = session {
            state.sessions.insert(
                (
                    self.gateway.clone(),
                    session.namespace.clone(),
                    session.id.clone(),
                    route.into(),
                ),
                Binding {
                    provider: self.provider.clone(),
                    model: model.into(),
                    epoch: self.epoch,
                },
            );
        }
    }
}
impl Drop for SchedulePermit {
    fn drop(&mut self) {
        if !self.probe {
            return;
        }
        let mut state = self.state.0.lock().expect("scheduling state");
        if let Some(provider) = state
            .providers
            .get_mut(&(self.gateway.clone(), self.provider.clone()))
            && provider.epoch == self.epoch
            && provider.cooldown_generation == self.generation
            && let Some(cooldown) = &mut provider.cooldown
        {
            cooldown.probing = false;
        }
    }
}

fn quota_margin(
    candidate: &Candidate<'_>,
    snapshot: QuotaSnapshot,
    now: i64,
) -> Result<Option<f64>, Option<Duration>> {
    let QuotaSnapshot::Known { available, windows } = snapshot else {
        return if candidate.limits.is_empty() {
            Ok(None)
        } else {
            Err(None)
        };
    };
    if available == Some(false) {
        return Err(None);
    }
    let applicable: Vec<_> = windows
        .iter()
        .filter(|window| {
            window
                .model_ids
                .as_ref()
                .is_none_or(|models| models.iter().any(|id| id == candidate.model_id))
        })
        .collect();
    let fresh = |window: &QuotaWindow| {
        window.used_percent.is_finite()
            && (0.0..=100.0).contains(&window.used_percent)
            && window.observed_at_ms <= now
            && window.expires_at_ms > now
            && window.reset_at_ms.is_none_or(|reset| reset > now)
    };
    let mut margin: Option<f64> = None;
    for limit in candidate.limits {
        if windows.iter().any(|window| window.id == limit.window_id)
            && !applicable.iter().any(|window| window.id == limit.window_id)
        {
            continue;
        }
        let matching: Vec<_> = applicable
            .iter()
            .copied()
            .filter(|window| window.id == limit.window_id)
            .collect();
        if matching.is_empty() {
            return Err(None);
        }
        for window in matching {
            if !fresh(window) {
                return Err(None);
            }
            let remaining = f64::from(limit.max_used_percent) - window.used_percent;
            if remaining <= 0.0 {
                return Err(reset_delay(window, now));
            }
            let remaining = remaining / f64::from(limit.max_used_percent);
            margin = Some(margin.map_or(remaining, |old| old.min(remaining)));
        }
    }
    for window in applicable.into_iter().filter(|window| fresh(window)) {
        let remaining = (100.0 - window.used_percent) / 100.0;
        if remaining <= 0.0 {
            return Err(reset_delay(window, now));
        }
        margin = Some(margin.map_or(remaining, |old| old.min(remaining)));
    }
    Ok(margin)
}
fn reset_delay(window: &QuotaWindow, now: i64) -> Option<Duration> {
    window
        .reset_at_ms
        .filter(|reset| *reset > now)
        .map(|reset| Duration::from_millis((reset - now) as u64))
}
