use super::*;
use velune_agent_runtime::{execution::ExecutionTarget, version::RuntimeKind};

fn target() -> Target {
    Target {
        execution: ExecutionTarget::Local,
        os_home: None,
        kind: RuntimeKind::Codex166,
        agent_dir: "/synthetic/codex".into(),
        native_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(),
        node_binary: "/synthetic/node".into(),
        resources: "/synthetic/resources".into(),
    }
}

#[test]
fn aliases_share_single_flight_but_physical_sources_do_not() {
    let now = Instant::now();
    let mut state = State::default();
    let first = target();
    let mut other = first.clone();
    other.agent_dir = "/synthetic/other-codex".into();
    assert!(state.admit(&first, now));
    assert!(state.admit(&first.clone(), now));
    assert!(state.admit(&other, now));
    assert_eq!(state.entries.len(), 2);
    let (sender, receiver) = mpsc::sync_channel(QUEUE);
    state.schedule(&sender, now);
    state.schedule(&sender, now);
    assert_eq!(receiver.try_iter().count(), 2);
    assert!(state.entries.values().all(|entry| entry.flight.is_some()));
}

#[test]
fn invalidation_keeps_slot_and_rejects_late_results_even_after_native_rejection() {
    let now = Instant::now();
    let target = target();
    let mut state = State::default();
    state.admit(&target, now);
    let (sender, receiver) = mpsc::sync_channel(QUEUE);
    state.schedule(&sender, now);
    let old = receiver.try_recv().unwrap();
    state.invalidate(&target, now);
    state
        .entries
        .get_mut(&target)
        .unwrap()
        .install(Observation::HeldElsewhere, now);
    state.schedule(&sender, now + REFRESH);
    assert!(
        receiver.try_recv().is_err(),
        "cancelled work still owns its slot"
    );
    state.complete(&old, Observation::Available, now);
    assert_eq!(
        state.entries[&target].access(now),
        InputAccess::ExternalController
    );
    assert!(state.entries[&target].flight.is_none());

    state.invalidate(&target, now);
    state.schedule(&sender, now);
    let new = receiver.try_recv().unwrap();
    state.complete(&old, Observation::Available, now);
    assert_eq!(state.entries[&target].flight.as_ref().unwrap().id, new.id);
    assert_eq!(
        state.entries[&target].access(now),
        InputAccess::CheckingControl
    );
    state.complete(&new, Observation::Available, now);
    assert_eq!(state.entries[&target].access(now), InputAccess::Editable);

    state.suspend(&target, now);
    let suspended_generation = state.entries[&target].generation;
    state.suspend(&target, now + TICK);
    assert_eq!(state.entries[&target].generation, suspended_generation);
    state.schedule(&sender, now + RETRY);
    assert!(receiver.try_recv().is_err(), "local ownership stops demand");
    assert!(state.entries[&target].cached.is_none());
    // The first cache access after losing the handle restarts checking; it
    // must not reuse Available from before native ownership was acquired.
    let service = ControlObservations {
        shared: Arc::new(Shared {
            state: Mutex::new(state),
            wake: Condvar::new(),
        }),
        scheduler: None,
    };
    assert_eq!(service.access(target.clone()), InputAccess::CheckingControl);
    assert!(!service.shared.state.lock().unwrap().entries[&target].suspended);
}

#[test]
fn stalled_work_never_requeues_and_saturation_cannot_keep_checking_forever() {
    let now = Instant::now();
    let target = target();
    let mut state = State::default();
    state.admit(&target, now);
    let (sender, receiver) = mpsc::sync_channel(QUEUE);
    state.schedule(&sender, now);
    let request = receiver.try_recv().unwrap();
    request.budget.cancel();
    for _ in 0..10 {
        state.schedule(&sender, now + RETRY + PROBE_TIMEOUT);
    }
    assert!(receiver.try_recv().is_err());
    assert!(state.entries[&target].flight.is_some());
    assert_eq!(
        state.entries[&target].access(now + PROBE_TIMEOUT),
        InputAccess::ControlUnavailable
    );

    let (full_sender, _full_receiver) = mpsc::sync_channel(0);
    let mut saturated = State::default();
    saturated.admit(&target, now);
    saturated.schedule(&full_sender, now);
    assert_eq!(
        saturated.entries[&target].access(now),
        InputAccess::CheckingControl
    );
    assert_eq!(
        saturated.entries[&target].access(now + PROBE_TIMEOUT),
        InputAccess::ControlUnavailable
    );
    saturated
        .entries
        .get_mut(&target)
        .unwrap()
        .install(Observation::Available, now);
    assert_eq!(
        saturated.entries[&target].access(now + REFRESH + PROBE_TIMEOUT),
        InputAccess::ControlUnavailable
    );
}

#[test]
fn cache_and_worker_admission_are_bounded_without_evicting_in_flight_work() {
    let now = Instant::now();
    let mut state = State::default();
    for index in 0..CACHE {
        let mut target = target();
        target.native_id = format!("{index:x}");
        assert!(state.admit(&target, now));
    }
    let (sender, _receiver) = mpsc::sync_channel(QUEUE);
    state.schedule(&sender, now);
    assert_eq!(
        state
            .entries
            .values()
            .filter(|entry| entry.flight.is_some())
            .count(),
        QUEUE
    );
    let pinned: Vec<_> = state
        .entries
        .iter()
        .filter(|(_, entry)| entry.flight.is_some())
        .map(|(target, _)| target.clone())
        .collect();
    assert!(state.admit(&target(), now + TICK));
    assert_eq!(state.entries.len(), CACHE);
    assert!(
        pinned
            .iter()
            .all(|target| state.entries.contains_key(target))
    );
}

#[test]
fn service_uses_all_fixed_workers_and_shuts_down_without_waiting_for_blocked_probe() {
    let (started_tx, started_rx) = mpsc::channel();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_gate = gate.clone();
    let service = ControlObservations::start(Arc::new(move |_, budget| {
        started_tx.send(()).unwrap();
        let (lock, wake) = &*worker_gate;
        let mut released = lock.lock().unwrap();
        while !*released && !budget.finished() {
            released = wake
                .wait_timeout(released, Duration::from_millis(10))
                .unwrap()
                .0;
        }
        Observation::Available
    }))
    .unwrap();
    for index in 0..WORKERS {
        let mut target = target();
        target.native_id = format!("{index:x}");
        assert_eq!(service.access(target), InputAccess::CheckingControl);
    }
    let mut all_started = true;
    for _ in 0..WORKERS {
        all_started &= started_rx.recv_timeout(Duration::from_secs(2)).is_ok();
    }
    let started = Instant::now();
    drop(service);
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    assert!(
        all_started,
        "receiver lock must be released before probe IO"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}
