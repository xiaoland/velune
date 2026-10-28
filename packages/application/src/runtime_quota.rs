//! Native login observations have no provider routing, cap, alert or history policy.
use crate::native_quota_source::Capture;
use crate::{QuotaFreshness, QuotaWindowObservation, RuntimeQuotaStatus};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use velune_gateway::QuotaWindow;

const TTL_MS: i64 = 60_000;
const MAX_INFLIGHT: usize = 4;

struct Entry {
    capture: Capture,
    token: Arc<()>,
    cancel: tokio::sync::watch::Sender<bool>,
    windows: Vec<QuotaWindow>,
    running: bool,
    retry_at: Instant,
    problem: Option<String>,
}
impl Entry {
    fn fresh(&self) -> bool {
        let now = crate::local::unix_ms_now();
        !self.windows.is_empty()
            && self
                .windows
                .iter()
                .all(|window| window.observed_at_ms <= now && window.expires_at_ms > now)
    }
    fn status(&self, runtime_id: &str) -> RuntimeQuotaStatus {
        RuntimeQuotaStatus {
            runtime_id: runtime_id.into(),
            freshness: if self.fresh() {
                QuotaFreshness::Fresh
            } else if self.windows.is_empty() {
                QuotaFreshness::Unknown
            } else {
                QuotaFreshness::Stale
            },
            windows: self
                .windows
                .iter()
                .map(|window| QuotaWindowObservation {
                    id: window.id.clone(),
                    model_ids: window.model_ids.clone(),
                    used_percent: window.used_percent,
                    reset_at_ms: window.reset_at_ms,
                    window_ms: window.window_ms,
                    observed_at_ms: Some(window.observed_at_ms),
                    expires_at_ms: Some(window.expires_at_ms),
                    pace: None,
                })
                .collect(),
            refreshing: self.running,
            problem: self.problem.clone(),
        }
    }
}

#[derive(Default)]
struct State {
    entries: BTreeMap<String, Entry>,
    inflight: usize,
    stopped: bool,
}
#[derive(Default)]
pub(crate) struct RuntimeQuota {
    state: Mutex<State>,
}
impl RuntimeQuota {
    /// Configuration changes and deletion revoke pending observations immediately.
    pub(crate) fn retain(&self, bindings: &BTreeMap<String, Capture>) {
        self.state
            .lock()
            .expect("runtime quota")
            .entries
            .retain(|id, entry| {
                let keep = bindings
                    .get(id)
                    .is_some_and(|capture| capture == &entry.capture);
                if !keep {
                    let _ = entry.cancel.send(true);
                }
                keep
            });
    }
    pub(crate) fn stop(&self) {
        let mut state = self.state.lock().expect("runtime quota");
        state.stopped = true;
        for entry in state.entries.values() {
            let _ = entry.cancel.send(true);
        }
        state.entries.clear();
    }
    pub(crate) fn query(
        self: &Arc<Self>,
        runtime_id: &str,
        capture: Result<Capture, (QuotaFreshness, String)>,
        refresh: bool,
    ) -> Result<RuntimeQuotaStatus, crate::Error> {
        let mut state = self.state.lock().expect("runtime quota");
        if state.stopped {
            return Err(crate::Error::invalid("应用正在退出"));
        }
        let capture = match capture {
            Ok(capture) => capture,
            Err((freshness, problem)) => {
                if let Some(entry) = state.entries.remove(runtime_id) {
                    let _ = entry.cancel.send(true);
                }
                return Ok(RuntimeQuotaStatus {
                    runtime_id: runtime_id.to_owned(),
                    freshness,
                    windows: Vec::new(),
                    refreshing: false,
                    problem: Some(problem),
                });
            }
        };
        if state
            .entries
            .get(runtime_id)
            .is_some_and(|entry| entry.capture != capture)
            && let Some(entry) = state.entries.remove(runtime_id)
        {
            let _ = entry.cancel.send(true);
        }
        let now = Instant::now();
        let available = state.inflight < MAX_INFLIGHT;
        let entry = state
            .entries
            .entry(runtime_id.to_owned())
            .or_insert_with(|| Entry {
                capture: capture.clone(),
                token: Arc::new(()),
                cancel: tokio::sync::watch::channel(false).0,
                windows: Vec::new(),
                running: false,
                retry_at: now,
                problem: None,
            });
        if !entry.running && available && (refresh || (!entry.fresh() && now >= entry.retry_at)) {
            entry.running = true;
            let token = Arc::clone(&entry.token);
            let mut cancel = entry.cancel.subscribe();
            let id = runtime_id.to_owned();
            let service = Arc::clone(self);
            state.inflight += 1;
            let spawned = std::thread::Builder::new()
                .name("native-login-quota".into())
                .spawn(move || {
                    let result = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| ())
                        .and_then(|runtime| {
                            runtime.block_on(async {
                                if *cancel.borrow() {
                                    return Err(());
                                }
                                tokio::select! {
                                    value = capture.read(TTL_MS) => value,
                                    _ = cancel.changed() => Err(()),
                                }
                            })
                        });
                    let mut state = service.state.lock().expect("runtime quota");
                    state.inflight -= 1;
                    if state.stopped {
                        return;
                    }
                    let Some(entry) = state
                        .entries
                        .get_mut(&id)
                        .filter(|entry| Arc::ptr_eq(&entry.token, &token))
                    else {
                        return;
                    };
                    entry.running = false;
                    match result {
                        Ok(windows) => {
                            entry.windows = windows;
                            entry.problem = None;
                        }
                        Err(()) => {
                            entry.problem = Some("无法读取登录额度，请检查运行时的登录状态".into());
                            entry.retry_at = Instant::now() + Duration::from_secs(30);
                        }
                    }
                });
            if spawned.is_err() {
                state.inflight -= 1;
                let entry = state
                    .entries
                    .get_mut(runtime_id)
                    .expect("registered runtime quota");
                entry.running = false;
                entry.problem = Some("无法启动登录额度读取".into());
                entry.retry_at = now + Duration::from_secs(30);
            }
        }
        Ok(state.entries[runtime_id].status(runtime_id))
    }
}
