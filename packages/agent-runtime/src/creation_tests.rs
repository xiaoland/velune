use super::*;
use crate::{
    acp::{AcpCommand, Client, Config},
    native::{DshCommand, NativeConfig, NativeKind, NativeSession},
};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        let path = std::env::temp_dir().join(format!(
            "velune-creation-budget-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn until(mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !check() {
        assert!(
            Instant::now() < deadline,
            "actual helper/owner did not progress"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn helper(home: &Home, mode: &str, id: &str) -> PathBuf {
    fs::write(
        home.0.join("identity.json"),
        serde_json::to_vec(id).unwrap(),
    )
    .unwrap();
    let path = home.0.join("harness.py");
    let script = r#"import json,sys,time,pathlib
if '--version' in sys.argv:
 print('0.2.0-rc.2');sys.exit(0)
root=pathlib.Path(ROOT)
mode=MODE
def emit(r,v): print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':v}),flush=True)
for line in sys.stdin:
 r=json.loads(line)
 with (root/'wire.jsonl').open('a') as f: f.write(json.dumps(r)+'\n')
 method=r.get('method')
 if method=='initialize':
  value={'protocolVersion':1,'agentCapabilities':{'mcpCapabilities':{'http':True},'sessionCapabilities':{'resume':{}}}}
  if mode=='large-init': value['agentInfo']={'name':'x'*1048576,'version':'synthetic'}
  emit(r,value)
 elif method=='fixture/read': emit(r,{'payload':'x'*1048576})
 elif method=='session/new':
  (root/'native-created').touch()
  if mode=='hold':
   while not (root/'release').exists(): time.sleep(.001)
  if mode=='malformed-error':
   print(json.dumps({'jsonrpc':'2.0','id':r['id'],'error':None}),flush=True)
   while not (root/'release-created').exists(): time.sleep(.001)
  if mode in ('flood','interleave'):
   for n in range(16):
    print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'not-known-yet','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'x'*8192}}}}),flush=True)
  if mode=='incomplete':
   sys.stdout.write('{"jsonrpc":"2.0","id":'+json.dumps(r['id'])+',"result":{"sessionId":"'+'x'*4096)
   sys.stdout.flush()
  else: emit(r,{'sessionId':json.loads((root/'identity.json').read_text()),'configOptions':[{'id':'model','name':'Model','type':'select','currentValue':'[\"provider\",\"model\"]','options':[{'value':'[\"provider\",\"model\"]','name':'Synthetic'}]}]})
 elif method=='session/prompt': emit(r,{'stopReason':'end_turn'})
 else: emit(r,{})
"#.replace("ROOT", &format!("{:?}", home.0.to_string_lossy())).replace("MODE", &format!("{mode:?}"));
    fs::write(&path, script).unwrap();
    path
}
enum Owner {
    Acp(Box<Client>),
    Dsh(Box<NativeSession>),
}
impl Owner {
    fn new(home: &Home, dsh: bool, mode: &str, id: &str) -> Self {
        let script = helper(home, mode, id);
        if dsh {
            let mut session = NativeSession::connect(NativeConfig {
                execution: crate::execution::ExecutionTarget::Local,
                os_home: Some(home.0.clone()),
                kind: NativeKind::DeepSeek,
                binary: script,
                node_binary: Some("/usr/bin/python3".into()),
                agent_dir: home.0.clone(),
                resources_directory: home.0.clone(),
                projection_directory: home.0.clone(),
                gateway: None,
                mcp_servers: Vec::new(),
                session_agents: Vec::new(),
            })
            .unwrap();
            session.enable_dsh_command_port().unwrap();
            Self::Dsh(Box::new(session))
        } else {
            let mut client = Client::connect(Config {
                binary: "/usr/bin/python3".into(),
                args: vec![script.to_string_lossy().into_owned()],
                environment: BTreeMap::new(),
                execution: crate::execution::ExecutionTarget::Local,
                os_home: Some(home.0.clone()),
                mcp_servers: Vec::new(),
            })
            .unwrap();
            client.enable_command_port().unwrap();
            Self::Acp(Box::new(client))
        }
    }
    fn new_request(&mut self, home: &Home, budget: CreationBudget, deadline: Instant) -> Ticket {
        match self {
            Self::Acp(owner) => Ticket::Acp(
                owner
                    .enqueue_creation(home.0.clone(), "synthetic".into(), budget, deadline)
                    .unwrap(),
            ),
            Self::Dsh(owner) => Ticket::Dsh(
                owner
                    .dsh_enqueue_creation(home.0.clone(), "synthetic".into(), budget, deadline)
                    .unwrap(),
            ),
        }
    }
    fn poll(&mut self) {
        match self {
            Self::Acp(owner) => {
                let _ = owner.poll();
            }
            Self::Dsh(owner) => {
                let _ = owner.poll();
            }
        }
    }
    fn outcome(&mut self, ticket: &Ticket) -> Option<ObservedCreation> {
        self.poll();
        match (self, ticket) {
            (Self::Acp(owner), Ticket::Acp(ticket)) => owner
                .take_creation_result(ticket)
                .map(ObservedCreation::Acp),
            (Self::Dsh(owner), Ticket::Dsh(ticket)) => owner
                .dsh_take_creation_result(ticket)
                .map(ObservedCreation::Dsh),
            _ => unreachable!(),
        }
    }
    fn expire(&mut self, ticket: &Ticket) {
        match (self, ticket) {
            (Self::Acp(owner), Ticket::Acp(ticket)) => owner.expire_creation_test(ticket),
            (Self::Dsh(owner), Ticket::Dsh(ticket)) => owner.dsh_expire_creation_test(ticket),
            _ => unreachable!(),
        }
    }
    fn written(&mut self) -> bool {
        match self {
            Self::Acp(owner) => owner
                .take_receipts()
                .iter()
                .any(|r| matches!(r.stage, crate::acp::AcpReceiptStage::Written)),
            Self::Dsh(owner) => owner
                .dsh_take_receipts()
                .unwrap()
                .iter()
                .any(|r| matches!(r.stage, crate::native::DshReceiptStage::Written)),
        }
    }
    fn prompt_refused(&mut self) -> bool {
        match self {
            Self::Acp(owner) => owner
                .enqueue(
                    AcpCommand::Prompt {
                        text: "synthetic".into(),
                        images: Vec::new(),
                        run_deadline: None,
                    },
                    Instant::now() + Duration::from_secs(5),
                )
                .is_err(),
            Self::Dsh(owner) => owner
                .dsh_enqueue(
                    DshCommand::Prompt {
                        text: "synthetic".into(),
                        run_deadline: None,
                    },
                    Instant::now() + Duration::from_secs(5),
                )
                .is_err(),
        }
    }
    fn seal(&mut self, ticket: &Ticket) -> bool {
        self.poll();
        match (self, ticket) {
            (Self::Acp(owner), Ticket::Acp(ticket)) => owner.settle_creation_result(ticket).is_ok(),
            (Self::Dsh(owner), Ticket::Dsh(ticket)) => {
                owner.dsh_settle_creation_result(ticket).is_ok()
            }
            _ => unreachable!(),
        }
    }
    fn acknowledge(&mut self, ticket: &Ticket) {
        match (self, ticket) {
            (Self::Acp(owner), Ticket::Acp(ticket)) => owner.complete_creation(ticket).unwrap(),
            (Self::Dsh(owner), Ticket::Dsh(ticket)) => owner.dsh_complete_creation(ticket).unwrap(),
            _ => unreachable!(),
        }
    }
    fn prompt_accepted(&mut self) -> bool {
        match self {
            Self::Acp(owner) => owner
                .enqueue(
                    AcpCommand::Prompt {
                        text: "synthetic local fixture only".into(),
                        images: Vec::new(),
                        run_deadline: None,
                    },
                    Instant::now() + Duration::from_secs(5),
                )
                .is_ok(),
            Self::Dsh(owner) => owner
                .dsh_enqueue(
                    DshCommand::Prompt {
                        text: "synthetic local fixture only".into(),
                        run_deadline: None,
                    },
                    Instant::now() + Duration::from_secs(5),
                )
                .is_ok(),
        }
    }
    fn has_snapshot(&self) -> bool {
        match self {
            Self::Acp(owner) => owner.snapshot().is_some(),
            Self::Dsh(owner) => owner.snapshot().is_some(),
        }
    }
}
enum Ticket {
    Acp(crate::acp::AcpTicket),
    Dsh(Retained<crate::native::DshTicket>),
}
enum ObservedCreation {
    Acp(CreationOutcome),
    Dsh(Retained<CreationOutcome>),
}
impl std::ops::Deref for ObservedCreation {
    type Target = CreationOutcome;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Acp(outcome) => outcome,
            Self::Dsh(outcome) => outcome,
        }
    }
}
fn budget(identity_bytes: usize, record_bytes: usize) -> CreationBudget {
    CreationBudget::new(CreationLimits {
        identity_bytes,
        record_bytes,
        retained_bytes: 8 * 1024 * 1024,
        records: 32,
        known_bytes: 4096,
    })
    .unwrap()
}
fn methods(home: &Home, method: &str) -> usize {
    fs::read_to_string(home.0.join("wire.jsonl"))
        .unwrap()
        .lines()
        .filter(|line| serde_json::from_str::<Value>(line).unwrap()["method"] == method)
        .count()
}

#[test]
fn complex_identity_over_four_kib_uses_typed_created_and_waits_for_association_ack() {
    for dsh in [false, true] {
        let home = Home::new();
        let identity = "名\"\\\n\0".repeat(1024);
        assert!(identity.len() > 4096);
        let mut owner = Owner::new(&home, dsh, "full", &identity);
        let policy = budget(32 * 1024, 128 * 1024);
        assert_eq!(
            policy.peak_record_capacity(),
            0,
            "no maximum-buffer preallocation"
        );
        let ticket = owner.new_request(
            &home,
            policy.clone(),
            Instant::now() + Duration::from_secs(5),
        );
        let mut created = None;
        until(|| {
            created = owner.outcome(&ticket);
            created.is_some()
        });
        let Some(CreationOutcome::Created(created)) = created.as_deref() else {
            panic!("not real Created");
        };
        assert_eq!(created.native_id(), identity);
        assert!(created.within_policy());
        assert!(owner.has_snapshot());
        let _ = owner.written(); // consume actual flush evidence, not a fabricated receipt
        assert!(
            owner.prompt_refused(),
            "native Created is not association completion"
        );
        until(|| owner.seal(&ticket));
        assert!(!policy.is_in_flight());
        assert!(
            owner.prompt_refused(),
            "sealing receive credit must not release prompt dependency"
        );
        assert_eq!(methods(&home, "session/prompt"), 0);
        owner.acknowledge(&ticket);
        assert!(
            owner.prompt_accepted(),
            "strict association ACK releases the successor dependency"
        );
        until(|| methods(&home, "session/prompt") == 1);
        assert!(policy.peak_record_capacity() < policy.limits().record_bytes);
        assert_eq!(methods(&home, "session/new"), 1);
    }
}

#[test]
fn full_oversize_is_created_but_incomplete_record_budget_is_unknown_neither_replays() {
    for dsh in [false, true] {
        for incomplete in [false, true] {
            let home = Home::new();
            let identity = "large-identity-名".repeat(512);
            let mut owner = Owner::new(
                &home,
                dsh,
                if incomplete { "incomplete" } else { "full" },
                &identity,
            );
            let policy = budget(64, if incomplete { 512 } else { 128 * 1024 });
            let ticket = owner.new_request(
                &home,
                policy.clone(),
                Instant::now() + Duration::from_secs(5),
            );
            let mut outcome = None;
            until(|| {
                outcome = owner.outcome(&ticket);
                outcome.is_some()
            });
            if incomplete {
                assert!(matches!(
                    outcome.as_deref(),
                    Some(CreationOutcome::Unknown { .. })
                ));
            } else {
                let Some(CreationOutcome::Created(created)) = outcome.as_deref() else {
                    panic!("full identity lost");
                };
                assert_eq!(created.native_id(), identity);
                assert!(!created.within_policy());
            }
            assert!(
                policy.is_in_flight(),
                "failure/receipt consumption does not free actual owner"
            );
            assert!(
                !owner.has_snapshot(),
                "over-policy New must not install a usable target"
            );
            assert!(owner.prompt_refused());
            assert!(!owner.seal(&ticket));
            assert_eq!(methods(&home, "session/new"), 1);
            assert_eq!(methods(&home, "session/prompt"), 0);
            assert!(home.0.join("native-created").exists());
            drop(owner);
            until(|| !policy.is_in_flight());
        }
    }
}

#[test]
fn held_creation_timeout_and_dropped_observation_retain_actual_lifetime_until_cleanup() {
    struct Lifetime(mpsc::Sender<()>);
    impl Drop for Lifetime {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    for dsh in [false, true] {
        let home = Home::new();
        let mut owner = Owner::new(&home, dsh, "hold", "known-after-timeout");
        let policy = budget(8192, 64 * 1024);
        let (done, completed) = mpsc::channel();
        policy
            .retain_until_settled(Arc::new(Lifetime(done)))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let ticket = owner.new_request(&home, policy.clone(), deadline);
        until(|| home.0.join("native-created").exists());
        let mut written = false;
        until(|| {
            owner.poll();
            written |= owner.written();
            written
        });
        assert_eq!(policy.peak_record_capacity(), 0);
        assert!(matches!(
            completed.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        owner.expire(&ticket);
        let mut outcome = None;
        until(|| {
            outcome = owner.outcome(&ticket);
            outcome.is_some()
        });
        assert!(matches!(
            outcome.as_deref(),
            Some(CreationOutcome::Unknown { .. })
        ));
        // A view/receipt no longer observing is not a native cancellation.
        drop(policy.clone());
        assert!(policy.is_in_flight());
        assert!(matches!(
            completed.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(methods(&home, "session/new"), 1);
        assert_eq!(methods(&home, "session/prompt"), 0);
        fs::write(home.0.join("release"), []).unwrap();
        let mut late = None;
        until(|| {
            late = owner.outcome(&ticket);
            matches!(late.as_deref(), Some(CreationOutcome::Created(_)))
        });
        assert!(
            !owner.seal(&ticket),
            "late Written/Created must not unblock unhealthy connection"
        );
        assert!(policy.is_in_flight());
        drop(owner);
        completed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!policy.is_in_flight());
    }
}

#[test]
fn queued_and_parsed_creation_records_remain_byte_charged_when_the_owner_polls() {
    for dsh in [false, true] {
        let home = Home::new();
        let mut owner = Owner::new(&home, dsh, "flood", "identity-after-oversized-early-output");
        let policy = CreationBudget::new(CreationLimits {
            identity_bytes: 8192,
            record_bytes: 64 * 1024,
            retained_bytes: 3 * 1024 * 1024,
            records: 32,
            known_bytes: 4096,
        })
        .unwrap();
        let ticket = owner.new_request(
            &home,
            policy.clone(),
            Instant::now() + Duration::from_secs(5),
        );
        let mut outcome = None;
        until(|| {
            outcome = owner.outcome(&ticket);
            outcome.is_some()
        });
        assert!(matches!(
            outcome.as_deref(),
            Some(CreationOutcome::Unknown { .. })
        ));
        assert!(
            policy.peak_record_capacity() < policy.limits().record_bytes,
            "many individually valid small records must hit retained credit, not the single-frame cap"
        );
        assert!(policy.is_in_flight());
        assert!(owner.prompt_refused());
        assert_eq!(methods(&home, "session/new"), 1);
        assert_eq!(methods(&home, "session/prompt"), 0);
        drop(owner);
        until(|| !policy.is_in_flight());
    }
}

#[test]
fn common_deferred_creation_records_cannot_escape_retained_credit_by_leaving_the_receive_channel() {
    let home = Home::new();
    let script = helper(&home, "interleave", "not-observed-after-budget-exhaustion");
    let mut command = crate::process::command("/usr/bin/python3");
    command.arg(script);
    let mut rpc = crate::native::rpc::RpcClient::spawn_at(
        command,
        &crate::execution::ExecutionTarget::Local,
        Some(&home.0),
    )
    .unwrap();
    let policy = CreationBudget::new(CreationLimits {
        identity_bytes: 8192,
        record_bytes: 64 * 1024,
        retained_bytes: 3 * 1024 * 1024,
        records: 32,
        known_bytes: 0,
    })
    .unwrap();
    rpc.arm_creation(&policy).unwrap();
    let result = rpc.request("session/new", &serde_json::json!({"cwd":home.0}));
    assert!(
        result.is_err(),
        "deferred DOMs and encoded staging must stay charged"
    );
    assert!(policy.is_in_flight());
    assert_eq!(methods(&home, "session/new"), 1);
    assert_eq!(methods(&home, "session/prompt"), 0);
    drop(rpc);
    until(|| !policy.is_in_flight());
}

#[test]
fn legacy_common_reader_keeps_large_single_record_contract_without_creation_policy() {
    let home = Home::new();
    let identity = "x".repeat(9 * 1024 * 1024);
    let script = helper(&home, "full", &identity);
    let mut command = crate::process::command("/usr/bin/python3");
    command.arg(script);
    let mut rpc = crate::native::rpc::RpcClient::spawn_at(
        command,
        &crate::execution::ExecutionTarget::Local,
        Some(&home.0),
    )
    .unwrap();
    let response = rpc
        .request("session/new", &serde_json::json!({"cwd":home.0}))
        .unwrap();
    assert_eq!(
        response["sessionId"].as_str().unwrap().len(),
        identity.len()
    );
    assert_eq!(methods(&home, "session/new"), 1);
}

#[test]
fn malformed_creation_error_stays_unknown_and_late_created_is_recovered_for_both_adapters() {
    for dsh in [false, true] {
        let home = Home::new();
        let identity = "recovery-名\"\\\n".repeat(512);
        let mut owner = Owner::new(&home, dsh, "malformed-error", &identity);
        let policy = budget(32 * 1024, 128 * 1024);
        let ticket = owner.new_request(
            &home,
            policy.clone(),
            Instant::now() + Duration::from_secs(5),
        );
        let mut first = None;
        until(|| {
            first = owner.outcome(&ticket);
            first.is_some()
        });
        assert!(
            matches!(first.as_deref(), Some(CreationOutcome::Unknown { .. })),
            "error:null is not a schema-validated native rejection"
        );
        assert!(policy.is_in_flight());
        assert!(!owner.has_snapshot());
        assert!(owner.prompt_refused());
        assert_eq!(methods(&home, "session/new"), 1);
        assert_eq!(methods(&home, "session/prompt"), 0);
        fs::write(home.0.join("release-created"), []).unwrap();
        let mut late = None;
        until(|| {
            late = owner.outcome(&ticket);
            matches!(late.as_deref(), Some(CreationOutcome::Created(_)))
        });
        let Some(CreationOutcome::Created(created)) = late.as_deref() else {
            unreachable!()
        };
        assert_eq!(created.native_id(), identity);
        assert!(created.within_policy());
        assert!(
            !owner.has_snapshot(),
            "failed connection must not install a usable target"
        );
        assert!(owner.prompt_refused());
        assert!(!owner.seal(&ticket));
        assert_eq!(methods(&home, "session/new"), 1);
        assert_eq!(methods(&home, "session/prompt"), 0);
        drop(owner);
        until(|| !policy.is_in_flight());
    }
}

// These two non-ignored regressions specify the still-open retained-ownership
// P1. They are intentionally not claimed green until return/native owners hold
// their permits, and dropping an owner really returns its charge.
#[test]
fn retained_initialization_fields_reject_small_creation_before_dispatch() {
    let home = Home::new();
    let mut owner = Owner::new(&home, false, "large-init", "must-not-be-created");
    let Owner::Acp(client) = &mut owner else {
        unreachable!()
    };
    assert_eq!(client.agent_info().unwrap().name.len(), 1024 * 1024);
    let policy = CreationBudget::new(CreationLimits {
        identity_bytes: 1024,
        record_bytes: 64 * 1024,
        retained_bytes: 3 * 1024 * 1024,
        records: 32,
        known_bytes: 4096,
    })
    .unwrap();
    let result = client.enqueue_creation(
        home.0.clone(),
        "synthetic".into(),
        policy.clone(),
        Instant::now() + Duration::from_secs(5),
    );
    assert!(
        result.is_err(),
        "live typed initialize data must be charged before writer admission"
    );
    assert!(!policy.is_in_flight());
    assert!(
        !policy.is_settled(),
        "pre-admission rejection must not start the policy"
    );
    assert_eq!(methods(&home, "session/new"), 0);
}

#[test]
fn held_common_response_keeps_credit_through_poll_and_returns_it_on_owner_drop() {
    let home = Home::new();
    let script = helper(&home, "large-read", "unused");
    let mut command = crate::process::command("/usr/bin/python3");
    command.arg(script);
    let mut rpc = crate::native::rpc::RpcClient::spawn_at(
        command,
        &crate::execution::ExecutionTarget::Local,
        Some(&home.0),
    )
    .unwrap();
    let held = rpc.request("fixture/read", &serde_json::json!({})).unwrap();
    assert_eq!(held["payload"].as_str().unwrap().len(), 1024 * 1024);
    let _ = rpc.poll();
    let policy = CreationBudget::new(CreationLimits {
        identity_bytes: 1024,
        record_bytes: 64 * 1024,
        retained_bytes: 3 * 1024 * 1024,
        records: 32,
        known_bytes: 0,
    })
    .unwrap();
    assert!(
        rpc.arm_creation(&policy).is_err(),
        "poll/arm must not forget a still-owned response"
    );
    assert!(!policy.is_in_flight());
    assert!(!policy.is_settled());
    assert_eq!(held["payload"].as_str().unwrap().len(), 1024 * 1024);
    assert_eq!(methods(&home, "session/new"), 0);
    drop(held);
    rpc.arm_creation(&policy).unwrap();
    rpc.cancel_creation_admission(&policy);
    assert!(
        policy.is_settled(),
        "disposed transient results must not accumulate as permanent history"
    );
}

#[test]
fn retained_cold_data_does_not_pin_actual_connection_or_home_cleanup() {
    struct Lifetime(mpsc::Sender<()>);
    impl Drop for Lifetime {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let home = Home::new();
    let mut command = crate::process::command("/usr/bin/python3");
    command.arg(helper(&home, "large-read", "unused"));
    let mut rpc = crate::native::rpc::RpcClient::spawn_at(
        command,
        &crate::execution::ExecutionTarget::Local,
        Some(&home.0),
    )
    .unwrap();
    let policy = CreationBudget::new(CreationLimits {
        identity_bytes: 1024,
        record_bytes: 2 * 1024 * 1024,
        retained_bytes: 128 * 1024 * 1024,
        records: 32,
        known_bytes: 0,
    })
    .unwrap();
    let (released, lifetime_released) = mpsc::channel();
    policy
        .retain_until_settled(Arc::new(Lifetime(released)))
        .unwrap();
    rpc.arm_creation(&policy).unwrap();
    let data = rpc.request("fixture/read", &serde_json::json!({})).unwrap();
    assert_eq!(data["payload"].as_str().unwrap().len(), 1024 * 1024);
    assert!(matches!(
        lifetime_released.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    drop(rpc);
    lifetime_released
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(
        policy.is_settled(),
        "actual IO, not cached data, holds the HOME lease"
    );
    assert_eq!(data["payload"].as_str().unwrap().len(), 1024 * 1024);
    drop(data);
}
