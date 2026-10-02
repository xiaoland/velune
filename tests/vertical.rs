use velune_core::{
    Core, Delivery,
    adapter::{Fault, Harness, HarnessAdapter, NativeUnavailable, SimHarness, Submission},
    routing::{self, Decision, Intent, Resource},
};
fn resources() -> (Vec<Resource>, Vec<String>) {
    let r: Vec<_> = [Harness::Codex, Harness::ClaudeCode, Harness::Pi]
        .into_iter()
        .map(Resource::fixture)
        .collect();
    let a = r.iter().map(|r| r.slot.clone()).collect();
    (r, a)
}
fn setup(core: &mut Core) -> (i64, i64) {
    let t = core.create_task("synthetic").unwrap();
    (
        core.session(t, Harness::Codex, "parent").unwrap(),
        core.session(t, Harness::ClaudeCode, "review").unwrap(),
    )
}
#[test]
fn three_harness_roundtrip_and_duplicate_delivery() {
    let mut core = Core::open(":memory:").unwrap();
    let task = core.create_task("synthetic").unwrap();
    let a = core.session(task, Harness::Codex, "parent").unwrap();
    let b = core.session(task, Harness::ClaudeCode, "review").unwrap();
    let c = core.session(task, Harness::Pi, "check").unwrap();
    let (r, allowed) = resources();
    assert_eq!(core.discover(a).unwrap().len(), 2);
    for (target, h) in [(b, Harness::ClaudeCode), (c, Harness::Pi)] {
        let id = core
            .delegate(a, target, h.name(), "2 + 3 = 5", 100, 0)
            .unwrap();
        assert_eq!(
            core.delegate(a, target, h.name(), "2 + 3 = 5", 100, 0)
                .unwrap(),
            id
        );
        assert!(
            core.delegate(a, target, h.name(), "changed", 100, 0)
                .is_err()
        );
        let mut worker = SimHarness::new(h);
        assert_eq!(
            core.deliver(id, &mut worker, &r, &allowed, 1).unwrap(),
            Delivery::Consumed
        );
        assert_eq!(
            core.deliver(id, &mut worker, &r, &allowed, 1).unwrap(),
            Delivery::AlreadyHandled
        );
        assert_eq!(worker.consumed.len(), 1);
        let reply = core.reply(id).unwrap().unwrap();
        assert!(core.verify(reply).is_err());
        assert_eq!(
            core.deliver(reply, &mut SimHarness::new(Harness::Codex), &r, &allowed, 2)
                .unwrap(),
            Delivery::Consumed
        );
        assert!(core.verify(reply).unwrap());
    }
}
#[test]
fn busy_not_sent_expiry_scope_and_wrong_adapter() {
    let mut c = Core::open(":memory:").unwrap();
    let (a, b) = setup(&mut c);
    let (r, allow) = resources();
    let id = c.delegate(a, b, "busy", "2 + 3 = 5", 10, 0).unwrap();
    let mut adapter = SimHarness::new(Harness::ClaudeCode);
    for fault in [Fault::Busy, Fault::BeforeSend] {
        adapter.fault = fault;
        assert_eq!(
            c.deliver(id, &mut adapter, &r, &allow, 1).unwrap(),
            Delivery::Queued
        );
        assert_eq!(c.state(id).unwrap(), "accepted");
    }
    assert!(
        c.deliver(id, &mut SimHarness::new(Harness::Pi), &r, &allow, 1)
            .is_err()
    );
    assert_eq!(
        c.deliver(id, &mut adapter, &r, &allow, 10).unwrap(),
        Delivery::Expired
    );
    assert!(adapter.consumed.is_empty());
    let task = c.create_task("private synthetic scope").unwrap();
    let other = c.session(task, Harness::Pi, "private").unwrap();
    assert!(
        c.delegate(a, other, "forbidden", "2 + 3 = 5", 100, 0)
            .is_err()
    );
    assert_eq!(c.discover(other).unwrap().len(), 0);
}
struct Crash(Harness);
impl HarnessAdapter for Crash {
    fn harness(&self) -> Harness {
        self.0
    }
    fn submit(&mut self, _: i64, _: &str, _: bool) -> Submission {
        panic!("simulated process loss after submit, before durable ack")
    }
}
#[test]
fn restart_unknown_never_replays_or_runs_another_message_on_segment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let (r, allow) = resources();
    let id;
    let second;
    {
        let mut c = Core::open(&path).unwrap();
        let (a, b) = setup(&mut c);
        assert!(
            Core::open(&path).is_err(),
            "second host must not steal ownership"
        );
        id = c.delegate(a, b, "crash", "2 + 3 = 5", 100, 0).unwrap();
        second = c.delegate(a, b, "another", "2 + 3 = 5", 100, 0).unwrap();
        let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            c.deliver(id, &mut Crash(Harness::ClaudeCode), &r, &allow, 1)
        }));
        assert!(crash.is_err());
    }
    let mut c = Core::open(&path).unwrap();
    assert_eq!(c.state(id).unwrap(), "unknown");
    let mut adapter = SimHarness::new(Harness::ClaudeCode);
    for message in [id, second] {
        assert_eq!(
            c.deliver(message, &mut adapter, &r, &allow, 2).unwrap(),
            Delivery::Unknown
        );
    }
    assert!(adapter.consumed.is_empty());
    assert!(c.reply(id).unwrap().is_none());
    assert!(
        c.events()
            .unwrap()
            .iter()
            .any(|e| e.event == "restart:delivery_unknown")
    );
}
#[test]
fn accepted_queue_and_result_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let (r, allow) = resources();
    let id;
    {
        let mut c = Core::open(&path).unwrap();
        let (a, b) = setup(&mut c);
        id = c.delegate(a, b, "queued", "2 + 3 = 5", 100, 0).unwrap();
    }
    let reply;
    {
        let mut c = Core::open(&path).unwrap();
        assert_eq!(
            c.deliver(id, &mut SimHarness::new(Harness::ClaudeCode), &r, &allow, 1)
                .unwrap(),
            Delivery::Consumed
        );
        reply = c.reply(id).unwrap().unwrap();
    }
    let mut c = Core::open(&path).unwrap();
    assert_eq!(
        c.deliver(reply, &mut SimHarness::new(Harness::Codex), &r, &allow, 2)
            .unwrap(),
        Delivery::Consumed
    );
    assert!(c.verify(reply).unwrap());
}
#[test]
fn route_fails_closed_preserves_identity_and_waits() {
    let (mut r, allow) = resources();
    let intent = Intent {
        harness: Harness::Codex,
        allowed_slots: &allow,
        bound_account: Some("synthetic-codex"),
    };
    assert_eq!(routing::choose(&intent, &r), Decision::Dispatch(0));
    r[0].available = false;
    assert_eq!(routing::choose(&intent, &r), Decision::Wait);
    r[0].account = "other-account".into();
    assert_eq!(routing::choose(&intent, &r), Decision::HandoffRequired);
    r[0].simulated = false;
    assert_eq!(routing::choose(&intent, &r), Decision::NeedsDecision);
    r[0].simulated = true;
    r[0].authorized = false;
    assert_eq!(routing::choose(&intent, &r), Decision::NeedsDecision);
    assert!(NativeUnavailable(Harness::Codex).start().is_err());
}
#[test]
fn reported_failure_is_not_completion_and_unknown_ack_is_not_retryable() {
    let mut c = Core::open(":memory:").unwrap();
    let (a, b) = setup(&mut c);
    let (r, allow) = resources();
    let id = c.delegate(a, b, "bad", "not arithmetic", 100, 0).unwrap();
    c.deliver(id, &mut SimHarness::new(Harness::ClaudeCode), &r, &allow, 1)
        .unwrap();
    let reply = c.reply(id).unwrap().unwrap();
    c.deliver(reply, &mut SimHarness::new(Harness::Codex), &r, &allow, 2)
        .unwrap();
    assert!(!c.verify(reply).unwrap());
    let id = c.delegate(a, b, "unknown", "2 + 3 = 5", 100, 0).unwrap();
    let mut adapter = SimHarness::new(Harness::ClaudeCode);
    adapter.fault = Fault::AfterAccept;
    assert_eq!(
        c.deliver(id, &mut adapter, &r, &allow, 3).unwrap(),
        Delivery::Unknown
    );
    adapter.fault = Fault::None;
    assert_eq!(
        c.deliver(id, &mut adapter, &r, &allow, 4).unwrap(),
        Delivery::Unknown
    );
    assert_eq!(adapter.consumed.len(), 1);
}

#[test]
fn provider_fallback_is_resource_selection_not_harness_delegation() {
    let mut first = Resource::fixture(Harness::Codex);
    first.available = false;
    let mut alternative = first.clone();
    alternative.available = true;
    alternative.slot = "second-responses-slot".into();
    alternative.provider = "second-local-provider".into();
    let allowed = vec![first.slot.clone(), alternative.slot.clone()];
    let intent = Intent {
        harness: Harness::Codex,
        allowed_slots: &allowed,
        bound_account: Some("synthetic-codex"),
    };
    assert_eq!(
        routing::choose(&intent, &[first, alternative]),
        Decision::Dispatch(1)
    );
    for h in [Harness::Codex, Harness::ClaudeCode, Harness::Pi] {
        assert!(!h.boundary().native_verified);
        assert!(NativeUnavailable(h).start().is_err());
    }
}
