//! One observation lifetime per physical context, shared by every view and caller.
use super::*;
use crate::conversation::{ConversationSnapshot, InputAccess};
use std::{
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use velune_agent_runtime::control::{Observation, Target};

/// How long a pending observation may run before the watch gives up and
/// reports `ControlUnavailable`. This covers filesystem stalls on external
/// volumes where `File::open` can block even though `flock(LOCK_NB)` is
/// non-blocking; it also gives subprocess-based observations (DeepSeek /
/// SSH) headroom before `bounded_process`'s own 15-second timeout fires.
const OBSERVATION_DEADLINE: Duration = Duration::from_secs(10);

/// Interval after a deadline expiry before retrying.
const DEADLINE_RETRY: Duration = Duration::from_secs(5);

/// Interval after the pool queue is full before retrying.
const POOL_FULL_RETRY: Duration = Duration::from_millis(500);

/// Number of observation worker threads.
const POOL_WORKERS: usize = 4;

/// Bounded queue depth. Observations are fast (non-blocking flock or a
/// subprocess with its own timeout), so a small queue is sufficient.
const POOL_QUEUE: usize = 32;

struct PoolRequest {
    target: Target,
    result_tx: mpsc::Sender<Observation>,
}

/// Shared pool of worker threads that execute `Target::observe()`.
/// Replaces per-watch `std::thread::spawn` calls, bounding total thread count.
pub(super) struct ControlObservationPool {
    request_tx: mpsc::SyncSender<PoolRequest>,
}

impl ControlObservationPool {
    pub(super) fn new() -> Self {
        let (request_tx, request_rx) = mpsc::sync_channel::<PoolRequest>(POOL_QUEUE);
        let request_rx = Arc::new(Mutex::new(request_rx));
        for index in 0..POOL_WORKERS {
            let rx = request_rx.clone();
            std::thread::Builder::new()
                .name(format!("velune-ctrl-{index}"))
                .spawn(move || {
                    while let Ok(req) = rx.lock().expect("control pool").recv() {
                        let _ = req.result_tx.send(req.target.observe());
                    }
                })
                .expect("control worker thread");
        }
        Self { request_tx }
    }

    /// Submit an observation for `target`. Returns `None` when the pool queue
    /// is full; the caller should back off and retry.
    fn observe(&self, target: Target) -> Option<mpsc::Receiver<Observation>> {
        let (result_tx, result_rx) = mpsc::channel();
        self.request_tx
            .try_send(PoolRequest { target, result_tx })
            .ok()
            .map(|()| result_rx)
    }
}

pub(super) struct ControlWatch {
    target: Target,
    observation: Option<Observation>,
    pending: Option<mpsc::Receiver<Observation>>,
    /// Absolute time after which a pending observation is abandoned.
    deadline: Option<Instant>,
    next: Instant,
}
impl ControlWatch {
    pub(super) fn new(target: Target) -> Self {
        Self {
            target,
            observation: None,
            pending: None,
            deadline: None,
            next: Instant::now(),
        }
    }
    pub(super) fn poll(&mut self, pool: &ControlObservationPool) -> InputAccess {
        if self.pending.is_some() {
            let now = Instant::now();
            let expired = self.deadline.is_some_and(|d| now >= d);
            if expired {
                // Observation worker is stuck (filesystem stall or subprocess
                // hang). Abandon the in-flight result and report unavailable.
                self.observation = Some(Observation::Unknown);
                self.pending = None;
                self.deadline = None;
                self.next = now + DEADLINE_RETRY;
            } else if let Some(receiver) = &self.pending {
                match receiver.try_recv() {
                    Ok(observation) => {
                        self.observation = Some(observation);
                        self.pending = None;
                        self.deadline = None;
                        self.next = Instant::now() + Duration::from_secs(2);
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.observation = Some(Observation::Unknown);
                        self.pending = None;
                        self.deadline = None;
                        self.next = Instant::now() + Duration::from_secs(2);
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
        }
        if self.pending.is_none() && Instant::now() >= self.next {
            match pool.observe(self.target.clone()) {
                Some(rx) => {
                    self.pending = Some(rx);
                    self.deadline = Some(Instant::now() + OBSERVATION_DEADLINE);
                }
                None => {
                    // Pool saturated; retry after a short backoff without
                    // resetting the last known observation.
                    self.next = Instant::now() + POOL_FULL_RETRY;
                }
            }
        }
        match self.observation {
            None => InputAccess::CheckingControl,
            Some(Observation::HeldElsewhere) => InputAccess::ExternalController,
            Some(Observation::Unknown) => InputAccess::ControlUnavailable,
            Some(Observation::Available | Observation::NotReported) => InputAccess::Editable,
        }
    }
}

/// Keep pending control observations when the associated full history view is
/// evicted. The cache is small and independent of transcript memory.
const RETAINED_CONTROL_WATCHES: usize = 256;

pub(super) fn retain_in_cache(cache: &mut Vec<ControlWatch>, watch: ControlWatch) {
    if let Some(index) = cache.iter().position(|known| known.target == watch.target) {
        cache.remove(index);
    }
    cache.push(watch);
    if cache.len() > RETAINED_CONTROL_WATCHES {
        cache.remove(0);
    }
}

pub(super) fn take_from_cache(
    cache: &mut Vec<ControlWatch>,
    target: &Target,
) -> Option<ControlWatch> {
    let index = cache.iter().position(|watch| watch.target == *target)?;
    Some(cache.remove(index))
}

pub(super) fn remove_from_cache(cache: &mut Vec<ControlWatch>, target: &Target) {
    cache.retain(|watch| watch.target != *target);
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn target() -> Target {
        Target {
            execution: velune_agent_runtime::execution::ExecutionTarget::Local,
            os_home: None,
            kind: velune_agent_runtime::version::RuntimeKind::Codex166,
            agent_dir: PathBuf::from("/synthetic/codex-home"),
            native_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(),
            node_binary: PathBuf::from("/usr/bin/node"),
            resources: PathBuf::from("/synthetic/resources"),
        }
    }

    #[test]
    fn explicit_refresh_discards_retired_and_late_control_observations() {
        let target = target();
        let (sender, receiver) = mpsc::channel();
        let mut retired = ControlWatch::new(target.clone());
        retired.observation = Some(Observation::HeldElsewhere);
        retired.pending = Some(receiver);
        let mut cache = Vec::new();
        retain_in_cache(&mut cache, retired);

        remove_from_cache(&mut cache, &target);
        assert!(take_from_cache(&mut cache, &target).is_none());
        assert!(sender.send(Observation::Available).is_err());

        let pool = ControlObservationPool::new();
        let mut refreshed = ControlWatch::new(target);
        refreshed.next = Instant::now() + Duration::from_secs(60);
        assert_eq!(refreshed.poll(&pool), InputAccess::CheckingControl);
        assert_eq!(refreshed.observation, None);
    }
}

impl CoreRuntime {
    pub(super) fn control_target(&self) -> Result<Option<Target>, RuntimeError> {
        if self.session.side.is_some() || self.session.unsubmitted_draft.is_some() {
            return Ok(None);
        }
        let Some(view) = self.session.projected() else {
            return Ok(None);
        };
        self.read_control_target(view, self.session.logical_projection.as_ref())
    }
    pub(super) fn read_control_target(
        &self,
        view: &ConversationSnapshot,
        projection: Option<&continuation::LogicalProjection>,
    ) -> Result<Option<Target>, RuntimeError> {
        let (runtime_id, id) = projection
            .and_then(|p| p.link.segments.last())
            .map(|s| {
                (
                    s.runtime_instance_id.as_str(),
                    s.native_conversation_id.as_str(),
                )
            })
            .unwrap_or((&view.context_runtime_id, &view.conversation.id));
        let runtime = self
            .session
            .execution_configuration
            .as_ref()
            .filter(|configuration| configuration.runtime.id == runtime_id)
            .map(|configuration| &configuration.runtime)
            .map_or_else(|| self.runtime_instance(runtime_id), Ok)?;
        if !velune_agent_runtime::control::reports_control(runtime.type_id) {
            return Ok(None);
        }
        let context = self.execution_context(runtime)?;
        let native_id = id
            .strip_prefix(&format!("{runtime_id}:"))
            .unwrap_or(id)
            .to_owned();
        Ok(Some(Target {
            execution: context.target,
            os_home: context.os_home,
            kind: runtime.type_id,
            agent_dir: context.agent_dir,
            native_id,
            node_binary: setting_path_optional(runtime, "nodeBinary").unwrap_or_default(),
            resources: context.resources,
        }))
    }
    fn observed_input_access(&mut self) -> InputAccess {
        if self.current_handoff_is_released() {
            self.session.control = None;
            return InputAccess::Editable;
        }
        self.observe_control_target(self.control_target())
    }
    pub(super) fn observe_control_target(
        &mut self,
        target: Result<Option<Target>, RuntimeError>,
    ) -> InputAccess {
        let target = match target {
            Ok(None) => {
                if let Some(watch) = self.session.control.take() {
                    retain_in_cache(&mut self.control_watches, watch);
                }
                return InputAccess::Editable;
            }
            Err(_) => {
                if let Some(watch) = self.session.control.take() {
                    retain_in_cache(&mut self.control_watches, watch);
                }
                return InputAccess::ControlUnavailable;
            }
            Ok(Some(target)) => target,
        };
        // Views of this session share its native handle. A different runtime
        // configuration parked here is another native client: do not borrow
        // its model, account, MCP services or approval context implicitly.
        if matches!(&self.session.active_state, ActiveState::Native(native) if native.owns_control(&target))
        {
            self.session.control = None;
            remove_from_cache(&mut self.control_watches, &target);
            return InputAccess::Editable;
        }
        if self
            .session
            .control
            .as_ref()
            .is_none_or(|watch| watch.target != target)
        {
            if let Some(watch) = self.session.control.take() {
                retain_in_cache(&mut self.control_watches, watch);
            }
            self.session.control = Some(
                take_from_cache(&mut self.control_watches, &target)
                    .unwrap_or_else(|| ControlWatch::new(target)),
            );
        }
        let pool = &self.control_pool;
        self.session.control.as_mut().expect("control watch").poll(pool)
    }
    pub(super) fn apply_input_access(&mut self, snapshot: &mut ConversationSnapshot) {
        let access = self.observed_input_access();
        Self::decorate_input_access(
            snapshot,
            access,
            matches!(self.session.active_state, ActiveState::History(_)),
        );
    }
    fn decorate_input_access(
        snapshot: &mut ConversationSnapshot,
        mut access: InputAccess,
        history: bool,
    ) {
        if snapshot.conversation.parent_conversation_id.is_some() {
            access = InputAccess::ReadOnly;
        } else if snapshot.run_state == RunState::Elsewhere && access == InputAccess::Editable {
            access = InputAccess::ExternalExecution;
        } else if history && !snapshot.actions.can_send && access == InputAccess::Editable {
            access = InputAccess::ReadOnly;
        }
        snapshot.actions.input_access = access;
    }
    pub(super) fn control_held_elsewhere(&mut self) {
        if let Ok(Some(target)) = self.control_target() {
            remove_from_cache(&mut self.control_watches, &target);
            let mut watch = ControlWatch::new(target);
            watch.observation = Some(Observation::HeldElsewhere);
            watch.next = Instant::now() + Duration::from_secs(2);
            if let Some(previous) = self.session.control.take() {
                retain_in_cache(&mut self.control_watches, previous);
            }
            self.session.control = Some(watch);
        }
    }
    /// All writers use the same admission, including queued and scheduled turns.
    pub(super) fn require_input_access(&mut self) -> Result<(), RuntimeError> {
        let Some(snapshot) = self.current_snapshot() else {
            return Ok(());
        };
        self.check_handoff_input(&snapshot)?;
        Self::check_input_access(snapshot.actions.input_access)
    }
    /// Validate the observation already captured for a command's projection.
    pub(super) fn check_input_access(access: InputAccess) -> Result<(), RuntimeError> {
        let message = match access {
            InputAccess::Editable => return Ok(()),
            InputAccess::CheckingControl => "正在检查会话控制权，请稍候",
            InputAccess::ExternalController => {
                "另一个客户端正在控制此会话；草稿已保留，释放后可继续"
            }
            InputAccess::ControlUnavailable => "暂时无法确认会话控制权，请检查运行时连接",
            InputAccess::ExternalExecution => "此会话正在其他客户端执行，请等待执行结束",
            InputAccess::ReadOnly => "此会话当前只可阅读",
        };
        Err(RuntimeError::invalid(message))
    }
}
