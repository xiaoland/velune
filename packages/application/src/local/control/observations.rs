//! Shared native-control facts. Cache reads never perform IO; a bounded scheduler
//! owns refresh, single-flight admission and cancellation independently of views.
use super::{InputAccess, Target};
use std::{
    collections::HashMap,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use velune_agent_runtime::control::Observation;
use velune_agent_runtime::probe::ProbeBudget;

const WORKERS: usize = 4;
const QUEUE: usize = 32;
const CACHE: usize = 256;
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const REFRESH: Duration = Duration::from_secs(2);
const RETRY: Duration = Duration::from_secs(5);
const DEMAND_LEASE: Duration = Duration::from_secs(15);
const TICK: Duration = Duration::from_millis(100);

type Probe = dyn Fn(&Target, &ProbeBudget) -> Observation + Send + Sync;

pub(in crate::local) struct ControlObservations {
    shared: Arc<Shared>,
    scheduler: Option<JoinHandle<()>>,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

#[derive(Default)]
struct State {
    entries: HashMap<Target, Entry>,
    next_id: u64,
    stopped: bool,
}

struct Entry {
    generation: u64,
    last_requested: Instant,
    checking_since: Instant,
    cached: Option<Cached>,
    refresh_at: Instant,
    suspended: bool,
    flight: Option<Flight>,
}

struct Cached {
    observation: Observation,
    valid_until: Instant,
}

struct Flight {
    id: u64,
    generation: u64,
    budget: ProbeBudget,
    timeout_reported: bool,
}

struct Request {
    target: Target,
    id: u64,
    generation: u64,
    budget: ProbeBudget,
    dispatcher: tracing::Dispatch,
    span: tracing::Span,
}

impl ControlObservations {
    pub(in crate::local) fn new() -> std::io::Result<Self> {
        Self::start(Arc::new(Target::observe_with_budget))
    }

    fn start(probe: Arc<Probe>) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        });
        let (sender, receiver) = mpsc::sync_channel::<Request>(QUEUE);
        let receiver = Arc::new(Mutex::new(receiver));
        for index in 0..WORKERS {
            let shared = shared.clone();
            let receiver = receiver.clone();
            let probe = probe.clone();
            thread::Builder::new()
                .name(format!("velune-control-{index}"))
                .spawn(move || loop {
                    // Release the receiver mutex before doing IO: holding the
                    // while-let scrutinee guard would serialize every worker.
                    let request = receiver.lock().expect("control queue").recv();
                    let Ok(request) = request else { break };
                    tracing::dispatcher::with_default(&request.dispatcher, || {
                        let _span = request.span.enter();
                        let started = Instant::now();
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            if request.budget.finished() {
                                Observation::Unknown
                            } else {
                                probe(&request.target, &request.budget)
                            }
                        })).unwrap_or(Observation::Unknown);
                        let mut state = shared.state.lock().expect("control observations");
                        state.complete(&request, result, Instant::now());
                        tracing::debug!(target: "velune_application", event = "control_observation_finished",
                            runtime = ?request.target.kind, elapsed_ms = started.elapsed().as_millis() as u64,
                            cancelled = request.budget.cancelled(), expired = request.budget.expired(), observation = ?result);
                        drop(state);
                        shared.wake.notify_one();
                    });
                })?;
        }
        let scheduler_shared = shared.clone();
        let dispatcher = tracing::dispatcher::get_default(Clone::clone);
        let scheduler = thread::Builder::new()
            .name("velune-control-scheduler".into())
            .spawn(move || {
                tracing::dispatcher::with_default(&dispatcher, || {
                    let mut state = scheduler_shared.state.lock().expect("control observations");
                    while !state.stopped {
                        state.schedule(&sender, Instant::now());
                        state = scheduler_shared
                            .wake
                            .wait_timeout(state, TICK)
                            .expect("control scheduler")
                            .0;
                    }
                })
            })?;
        Ok(Self {
            shared,
            scheduler: Some(scheduler),
        })
    }

    /// Request a short demand lease and read the cached fact. Scheduling and IO
    /// continue in the background even if the caller stops polling.
    pub(in crate::local) fn access(&self, target: Target) -> InputAccess {
        if !target.reports_control() {
            return InputAccess::Editable;
        }
        let now = Instant::now();
        let mut state = self.shared.state.lock().expect("control observations");
        if state.stopped || !state.admit(&target, now) {
            return InputAccess::ControlUnavailable;
        }
        let entry = state
            .entries
            .get_mut(&target)
            .expect("admitted control target");
        if entry.suspended {
            entry.suspended = false;
            entry.checking_since = now;
            entry.refresh_at = now;
        }
        entry.last_requested = now;
        let access = entry.access(now);
        drop(state);
        self.shared.wake.notify_one();
        access
    }

    /// A local native handle is stronger evidence than a probe. Stop demand
    /// without cancelling/restarting the same generation on every snapshot.
    pub(in crate::local) fn suspend(&self, target: &Target) {
        let mut state = self.shared.state.lock().expect("control observations");
        state.suspend(target, Instant::now());
        drop(state);
        self.shared.wake.notify_one();
    }

    pub(in crate::local) fn invalidate(&self, target: &Target) {
        let mut state = self.shared.state.lock().expect("control observations");
        state.invalidate(target, Instant::now());
        drop(state);
        self.shared.wake.notify_one();
    }

    /// A native write rejection outranks any older observation in flight.
    pub(in crate::local) fn held_elsewhere(&self, target: Target) {
        let now = Instant::now();
        let mut state = self.shared.state.lock().expect("control observations");
        if state.admit(&target, now) {
            state.invalidate(&target, now);
            let entry = state
                .entries
                .get_mut(&target)
                .expect("admitted control target");
            entry.install(Observation::HeldElsewhere, now);
        }
        drop(state);
        self.shared.wake.notify_one();
    }
}

impl Drop for ControlObservations {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().expect("control observations");
        state.stopped = true;
        for entry in state.entries.values() {
            if let Some(flight) = &entry.flight {
                flight.budget.cancel();
            }
        }
        drop(state);
        self.shared.wake.notify_one();
        // Only join the metadata scheduler. A stalled filesystem syscall cannot
        // be cancelled; its fixed worker remains accounted for until it returns.
        if let Some(scheduler) = self.scheduler.take() {
            let _ = scheduler.join();
        }
    }
}

impl Entry {
    fn access(&self, now: Instant) -> InputAccess {
        match &self.cached {
            Some(cached) if now < cached.valid_until => match cached.observation {
                Observation::Available | Observation::NotReported => InputAccess::Editable,
                Observation::HeldElsewhere => InputAccess::ExternalController,
                Observation::Unknown => InputAccess::ControlUnavailable,
            },
            None if now.duration_since(self.checking_since) < PROBE_TIMEOUT => {
                InputAccess::CheckingControl
            }
            _ => InputAccess::ControlUnavailable,
        }
    }

    fn install(&mut self, observation: Observation, now: Instant) {
        let interval = if observation == Observation::Unknown {
            RETRY
        } else {
            REFRESH
        };
        self.refresh_at = now + interval;
        self.cached = Some(Cached {
            observation,
            valid_until: self.refresh_at + PROBE_TIMEOUT,
        });
    }
}

impl State {
    fn admit(&mut self, target: &Target, now: Instant) -> bool {
        if self.entries.contains_key(target) {
            return true;
        }
        if self.entries.len() == CACHE {
            let oldest = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.flight.is_none())
                .min_by_key(|(_, entry)| entry.last_requested)
                .map(|(target, _)| target.clone());
            let Some(oldest) = oldest else { return false };
            self.entries.remove(&oldest);
        }
        self.next_id += 1;
        self.entries.insert(
            target.clone(),
            Entry {
                generation: self.next_id,
                last_requested: now,
                checking_since: now,
                cached: None,
                refresh_at: now,
                suspended: false,
                flight: None,
            },
        );
        true
    }

    fn suspend(&mut self, target: &Target, now: Instant) {
        if self
            .entries
            .get(target)
            .is_some_and(|entry| !entry.suspended)
        {
            self.invalidate(target, now);
            self.entries
                .get_mut(target)
                .expect("suspended target")
                .suspended = true;
        }
    }

    fn invalidate(&mut self, target: &Target, now: Instant) {
        self.next_id += 1;
        if let Some(entry) = self.entries.get_mut(target) {
            entry.generation = self.next_id;
            entry.cached = None;
            entry.refresh_at = now;
            entry.checking_since = now;
            entry.last_requested = now;
            if let Some(flight) = &entry.flight {
                flight.budget.cancel();
            }
        }
    }

    fn schedule(&mut self, sender: &mpsc::SyncSender<Request>, now: Instant) {
        for entry in self.entries.values_mut() {
            if let Some(flight) = &mut entry.flight
                && flight.budget.finished()
                && !flight.timeout_reported
            {
                flight.timeout_reported = true;
                if flight.generation == entry.generation {
                    entry.install(Observation::Unknown, now);
                    tracing::warn!(target: "velune_application", event = "control_observation_deadline",
                        in_flight_retained = true);
                }
            }
        }
        // Oldest demand first; HashMap iteration alone can starve targets while
        // a slow execution location fills the queue.
        let mut due: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                !entry.suspended
                    && entry.flight.is_none()
                    && now >= entry.refresh_at
                    && now.duration_since(entry.last_requested) < DEMAND_LEASE
            })
            .map(|(target, entry)| (target.clone(), entry.refresh_at))
            .collect();
        due.sort_by_key(|(_, due)| *due);
        for (target, _) in due {
            self.next_id += 1;
            let budget = ProbeBudget::new(PROBE_TIMEOUT);
            let generation = self.entries[&target].generation;
            let request = Request {
                target: target.clone(),
                id: self.next_id,
                generation,
                budget: budget.clone(),
                dispatcher: tracing::dispatcher::get_default(Clone::clone),
                span: tracing::debug_span!(target: "velune_application", "control_observation", runtime = ?target.kind),
            };
            match sender.try_send(request) {
                Ok(()) => {
                    self.entries
                        .get_mut(&target)
                        .expect("scheduled control target")
                        .flight = Some(Flight {
                        id: self.next_id,
                        generation,
                        budget,
                        timeout_reported: false,
                    });
                }
                Err(mpsc::TrySendError::Full(_)) => break,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.stopped = true;
                    break;
                }
            }
        }
    }

    fn complete(&mut self, request: &Request, result: Observation, now: Instant) {
        let Some(entry) = self.entries.get_mut(&request.target) else {
            return;
        };
        if !entry
            .flight
            .as_ref()
            .is_some_and(|flight| flight.id == request.id)
        {
            return;
        }
        let flight = entry.flight.take().expect("matching control flight");
        if self.stopped || entry.generation != request.generation {
            return;
        }
        if request.budget.finished() {
            if !flight.timeout_reported {
                entry.install(Observation::Unknown, now);
            }
        } else {
            entry.install(result, now);
        }
    }
}

#[cfg(test)]
mod tests;
