//! Native login observations have no provider routing, cap, alert or history policy.
use crate::{ProviderQuotaWindow, QuotaFreshness, RuntimeQuotaStatus, config::RuntimeInstance};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use velune_gateway::QuotaWindow;

const TTL_MS: i64 = 60_000;
const MAX_INFLIGHT: usize = 4;
pub(crate) const SUPPORTED_RUNTIME: &str = "claude-code-2.1";

#[derive(Clone, PartialEq, Eq)]
struct Capture {
    binary: String,
    directory: Option<PathBuf>,
}
impl Capture {
    fn from_runtime(runtime: &RuntimeInstance) -> Result<Self, (QuotaFreshness, String)> {
        if runtime.type_id != SUPPORTED_RUNTIME || !runtime.enabled {
            return Err((QuotaFreshness::Unsupported, "此运行时不提供登录额度".into()));
        }
        if runtime
            .execution()
            .map_err(|error| (QuotaFreshness::Unknown, error.to_string()))?
            != crate::RuntimeExecution::default()
        {
            return Err((
                QuotaFreshness::Unsupported,
                "目前仅支持本机、继承 HOME 的 Claude Code 登录额度".into(),
            ));
        }
        let setting = |key| {
            runtime
                .settings
                .get(key)
                .filter(|value| Path::new(value).is_absolute())
                .cloned()
                .ok_or_else(|| {
                    (
                        QuotaFreshness::Unknown,
                        "请先配置运行时入口和运行时目录".into(),
                    )
                })
        };
        let directory = setting("agentDir")?;
        Ok(Self {
            binary: setting("binary")?,
            directory: velune_agent_runtime::native::claude_login_directory(Path::new(&directory)),
        })
    }
}

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
                .map(|window| ProviderQuotaWindow {
                    id: window.id.clone(),
                    model_ids: window.model_ids.clone(),
                    used_percent: window.used_percent,
                    reset_at_ms: window.reset_at_ms,
                    window_ms: window.window_ms,
                    observed_at_ms: window.observed_at_ms,
                    expires_at_ms: window.expires_at_ms,
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
    pub(crate) fn retain(&self, runtimes: &[RuntimeInstance]) {
        self.state
            .lock()
            .expect("runtime quota")
            .entries
            .retain(|id, entry| {
                let keep = runtimes
                    .iter()
                    .find(|runtime| &runtime.id == id)
                    .and_then(|runtime| Capture::from_runtime(runtime).ok())
                    .is_some_and(|capture| capture == entry.capture);
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
        runtime: &RuntimeInstance,
        refresh: bool,
    ) -> Result<RuntimeQuotaStatus, crate::Error> {
        let mut state = self.state.lock().expect("runtime quota");
        if state.stopped {
            return Err(crate::Error::invalid("应用正在退出"));
        }
        let capture = match Capture::from_runtime(runtime) {
            Ok(capture) => capture,
            Err((freshness, problem)) => {
                if let Some(entry) = state.entries.remove(&runtime.id) {
                    let _ = entry.cancel.send(true);
                }
                return Ok(RuntimeQuotaStatus {
                    runtime_id: runtime.id.clone(),
                    freshness,
                    windows: Vec::new(),
                    refreshing: false,
                    problem: Some(problem),
                });
            }
        };
        if state
            .entries
            .get(&runtime.id)
            .is_some_and(|entry| entry.capture != capture)
            && let Some(entry) = state.entries.remove(&runtime.id)
        {
            let _ = entry.cancel.send(true);
        }
        let now = Instant::now();
        let available = state.inflight < MAX_INFLIGHT;
        let entry = state
            .entries
            .entry(runtime.id.clone())
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
            let id = runtime.id.clone();
            let service = Arc::clone(self);
            state.inflight += 1;
            let spawned = std::thread::Builder::new().name("native-login-quota".into()).spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| ())
                    .and_then(|runtime| runtime.block_on(async {
                        if *cancel.borrow() { return Err(()); }
                        tokio::select! {
                            value = crate::claude_quota::read(&capture.binary, capture.directory.as_deref(), TTL_MS) => value,
                            _ = cancel.changed() => Err(()),
                        }
                    }));
                let mut state = service.state.lock().expect("runtime quota");
                state.inflight -= 1;
                if state.stopped { return; }
                let Some(entry) = state.entries.get_mut(&id).filter(|entry| Arc::ptr_eq(&entry.token, &token)) else { return; };
                entry.running = false;
                match result {
                    Ok(windows) => { entry.windows = windows; entry.problem = None; }
                    Err(()) => { entry.problem = Some("无法读取 Claude Code 登录额度，请检查运行时的登录状态".into()); entry.retry_at = Instant::now() + Duration::from_secs(30); }
                }
            });
            if spawned.is_err() {
                state.inflight -= 1;
                let entry = state
                    .entries
                    .get_mut(&runtime.id)
                    .expect("registered runtime quota");
                entry.running = false;
                entry.problem = Some("无法启动登录额度读取".into());
                entry.retry_at = now + Duration::from_secs(30);
            }
        }
        Ok(state.entries[&runtime.id].status(&runtime.id))
    }
}
