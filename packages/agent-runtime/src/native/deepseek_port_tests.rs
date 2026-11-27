//! Account-free synthetic DeepSeek wire fixtures, under this source lane's tmp directory.
use super::*;
use std::{os::unix::fs::PermissionsExt, time::Duration};

struct Fixture {
    root: PathBuf,
    session: NativeSession,
}
impl Fixture {
    fn new(mode: &str) -> Self {
        Self::with_budget(mode, DEFAULT_BYTES)
    }
    fn with_budget(mode: &str, byte_budget: usize) -> Self {
        Self::build(mode, Some(byte_budget))
    }
    fn build(mode: &str, byte_budget: Option<usize>) -> Self {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::current_dir()
            .unwrap()
            .join(format!("tmp/deepseek-port-{:x}", u64::from_le_bytes(nonce)));
        std::fs::create_dir_all(&root).unwrap();
        let binary = root.join("dsh.py");
        let script = r#"import json,sys,time,threading,os
if '--version' in sys.argv:
 print('0.2.0-rc.2');sys.exit(0)
mode=MODE
log=LOG
if '--dump-config' in sys.argv:
 with open(log,'a') as f: f.write(json.dumps({'configDump':sys.argv})+'\n')
 with open(os.path.join(os.path.dirname(log),'composed.yml')) as f: sys.stdout.write(f.read())
 sys.exit(0)
catalog=[{'id':'model','currentValue':'[ "provider", "model" ]','options':[{'options':[{'value':'[ "provider", "model" ]','name':'Model'},{'value':'[ "provider", "other" ]','name':'Other'}]}]}]
with open(log,'a') as f:
 launch={'launch':sys.argv}
 for index,argument in enumerate(sys.argv[:-1]):
  if argument=='--patch':
   path=sys.argv[index+1]
   with open(path) as patch:
    if path.endswith('/compaction.patch.yml'): launch['compactionPatch']=patch.read()
    else: launch.setdefault('patch',json.load(patch))
 f.write(json.dumps(launch)+'\n')
turn=None
def emit(value): print(json.dumps(dict(jsonrpc='2.0',**value)),flush=True)
def result(r,value): emit({'id':r['id'],'result':value})
def permission(sid,identity,option='once'):
 emit({'id':identity,'method':'session/request_permission','params':{'sessionId':sid,'options':[{'optionId':option,'name':'Allow','kind':'allow_once'}]}})
def update(text):
 emit({'method':'session/update','params':{'sessionId':'session','update':{'sessionUpdate':'agent_message_chunk','content':{'text':text}}}})
for line in sys.stdin:
 r=json.loads(line)
 with open(log,'a') as f: f.write(json.dumps(r)+'\n')
 if mode=='gated-large' and r.get('id')=='large-permission' and 'result' in r:
  with open(os.path.join(os.path.dirname(log),'large-fifo-complete'),'w') as f: f.write('done')
 method=r.get('method')
 if method=='initialize':
  result(r,{'protocolVersion':1,'agentCapabilities':{'sessionCapabilities':{'resume':{}},'mcpCapabilities':{'http':True}}})
 elif method in ('session/new','session/resume'):
  if mode=='early' or mode=='resume-reject':
   permission('wrong','early-wrong');permission('session','early-session')
   update('Before identity response')
  if mode in ['reject','resume-reject']:
   emit({'id':r['id'],'error':{'code':-32602,'message':'Synthetic rejection','data':{'source':'fixture'}}});continue
  if mode=='overflow':
   for _ in range(40): update('Unknown identity')
  result(r,{'sessionId':r['params'].get('sessionId','session'),'configOptions':catalog})
  if mode=='exit-after-new': sys.exit(0)
  if mode in ['blocked-large','gated-large']:
   def large_later():
    time.sleep(0.05)
    if mode=='gated-large':
     while os.path.exists(os.path.join(os.path.dirname(log),'hold-large-permission')) and not os.path.exists(os.path.join(os.path.dirname(log),'allow-large-permission')): time.sleep(0.005)
    permission('session','large-permission');update('Oversized write still polling')
   threading.Thread(target=large_later,daemon=True).start()
   if mode=='gated-large':
    with open(os.path.join(os.path.dirname(log),'large-input-ready'),'w') as f: f.write('ready')
    while not os.path.exists(os.path.join(os.path.dirname(log),'release-large')): time.sleep(0.005)
   else: time.sleep(60)
 elif method=='session/set_config_option': result(r,{'configOptions':catalog})
 elif method=='session/close': result(r,{})
 elif method=='session/prompt':
  turn=r['id']
  if mode=='blocked':
   permission('session','blocked-permission','x'*262144)
   def later():
    time.sleep(0.05);update('While stdin blocked')
   threading.Thread(target=later,daemon=True).start()
   time.sleep(60)
  elif mode in ['late','turn-error','deadline']:
   update('Streaming')
   emit({'id':'unknown-capability','method':'elicitation/create','params':{'sessionId':'session'}})
   permission('wrong','wrong-permission');permission('session','permission')
  else: result(r,{'stopReason':'end_turn'})
 elif 'result' in r and r['id']=='permission' and turn:
  if mode=='turn-error': emit({'id':turn,'error':{'code':-32603,'message':'Late tool failure'}})
  else: emit({'id':turn,'result':{'stopReason':'end_turn'}})
 elif method=='session/cancel' and turn: emit({'id':turn,'result':{'stopReason':'cancelled'}})
"#.replace("MODE", &format!("{mode:?}")).replace("LOG", &format!("{:?}", root.join("wire.jsonl").to_string_lossy()));
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        // Python is this synthetic helper's interpreter only; production CLI/initialize gates
        // are unchanged and the fixture exercises the same declared executable-entry path.
        let mut session = NativeSession::connect(NativeConfig {
            execution: crate::execution::ExecutionTarget::Local,
            os_home: Some(root.clone()),
            kind: NativeKind::DeepSeek,
            binary,
            node_binary: Some(PathBuf::from("/usr/bin/python3")),
            agent_dir: root.clone(),
            resources_directory: root.clone(),
            projection_directory: root.clone(),
            gateway: None,
            auto_compaction: None,
            mcp_servers: vec![],
            session_agents: vec![],
        })
        .unwrap();
        if let Some(byte_budget) = byte_budget {
            session
                .enable_dsh_command_port_with_byte_budget(byte_budget)
                .unwrap();
        }
        Self { root, session }
    }
    fn enqueue(&mut self, command: DshCommand) -> Retained<DshTicket> {
        self.session
            .dsh_enqueue(command, Instant::now() + Duration::from_secs(2))
            .unwrap()
    }
    fn establish(&mut self) {
        self.enqueue(DshCommand::New {
            cwd: self.root.clone(),
            runtime_id: "runtime".into(),
        });
        self.until(|session| session.native_id.is_some());
        self.session.dsh_take_receipts().unwrap();
    }
    /// Complete unrelated session/helper preparation before arming a measured write D.
    /// The fixture's optional permission output is not a late-write precondition.
    fn prepare_blocked_input(&mut self) {
        std::fs::write(self.root.join("hold-large-permission"), b"hold").unwrap();
        let preparation_deadline = Instant::now() + rpc::CONTROL_TIMEOUT;
        self.session
            .dsh_enqueue(
                DshCommand::New {
                    cwd: self.root.clone(),
                    runtime_id: "runtime".into(),
                },
                preparation_deadline,
            )
            .expect("large-input session preparation admission");
        loop {
            assert!(
                Instant::now() < preparation_deadline,
                "large-input preparation expired before measured D was armed"
            );
            self.session
                .poll()
                .expect("large-input preparation poll (measured D not armed)");
            if self.session.native_id.is_some()
                && self
                    .session
                    .dsh_port
                    .as_ref()
                    .unwrap()
                    .operations
                    .values()
                    .all(|op| op.write.is_none())
                && std::fs::read_to_string(self.root.join("large-input-ready"))
                    .ok()
                    .as_deref()
                    == Some("ready")
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!self.session.has_pending_requests());
        self.session.dsh_take_receipts().unwrap();
    }
    fn until(&mut self, predicate: impl Fn(&NativeSession) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !predicate(&self.session) {
            assert!(
                Instant::now() < deadline,
                "synthetic DeepSeek condition timed out"
            );
            self.session.poll().unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn wait_written(&mut self) {
        self.until(|session| {
            session
                .dsh_port
                .as_ref()
                .unwrap()
                .operations
                .values()
                .all(|op| op.write.is_none())
        });
    }
    fn wire(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.join("wire.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.session.shutdown().unwrap();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
fn text_seen(session: &NativeSession, expected: &str) -> bool {
    session.snapshot().is_some_and(|snapshot| {
        snapshot.messages.iter().any(|message| {
            message
                .blocks
                .iter()
                .any(|block| matches!(block, MessageBlock::Text { text } if text == expected))
        })
    })
}
fn prompt(text: &str) -> DshCommand {
    DshCommand::Prompt {
        text: text.into(),
        run_deadline: None,
    }
}

#[test]
fn deepseek_prompt_completion_wait_keeps_updates_permission_reply_and_cancel_live() {
    let mut f = Fixture::new("late");
    f.establish();
    let ticket = f.enqueue(prompt("Only once"));
    f.until(|session| session.has_pending_requests());
    let written = f.session.dsh_take_receipts().unwrap();
    assert!(
        written
            .iter()
            .any(|receipt| receipt.ticket == *ticket && receipt.stage == DshReceiptStage::Written)
    );
    assert!(
        !written
            .iter()
            .any(|receipt| matches!(receipt.stage, DshReceiptStage::ProtocolSuccess { .. }))
    );
    assert!(text_seen(&f.session, "Streaming"));
    let interaction_id = f.session.interactions.keys().next().unwrap().clone();
    f.enqueue(DshCommand::PermissionReply {
        interaction_id,
        reply: RuntimeInteractionReply::Decision {
            option_id: "once".into(),
        },
    });
    f.enqueue(DshCommand::Cancel);
    f.until(|session| !session.turn_running() && !session.has_pending_requests());
    let receipts = f.session.dsh_take_receipts().unwrap();
    assert!(receipts.iter().any(|receipt| receipt.ticket == *ticket
        && matches!(&receipt.stage, DshReceiptStage::TurnCompleted { .. })));
    assert!(
        !receipts
            .iter()
            .any(|receipt| matches!(receipt.stage, DshReceiptStage::ProtocolSuccess { .. }))
    );
    let wire = f.wire();
    assert_eq!(
        wire.iter()
            .filter(|record| record["method"] == "session/prompt")
            .count(),
        1
    );
    assert!(wire.iter().any(|record| record["id"] == "unknown-capability" && record["error"]["code"] == -32601));
    assert!(wire.iter().any(|record| record["id"] == "wrong-permission"
        && record["result"]["outcome"]["outcome"] == "cancelled"));
}

#[test]
fn deepseek_native_rejection_and_written_late_turn_error_are_different() {
    let mut rejected = Fixture::new("reject");
    let new = rejected.enqueue(DshCommand::Resume {
        native_id: "session".into(),
        cwd: rejected.root.clone(),
        runtime_id: "runtime".into(),
        history: Retained::from_parts(vec![], DataRetention::default()),
    });
    rejected
        .until(|session| session.dsh_port.as_ref().unwrap().operations[&new.sequence].outcome_seen);
    assert!(rejected.session.native_id.is_none());
    assert!(
        rejected
            .session
            .dsh_enqueue(
                prompt("Never sent"),
                Instant::now() + Duration::from_secs(1)
            )
            .is_err()
    );
    assert!(rejected.session.dsh_take_receipts().unwrap().iter().any(|receipt| matches!(&receipt.stage, DshReceiptStage::ProtocolRejected { error } if error.code == -32602 && error.data.is_some())));
    let mut late = Fixture::new("turn-error");
    late.establish();
    let ticket = late.enqueue(prompt("Start tool"));
    late.until(|session| session.has_pending_requests());
    let interaction_id = late.session.interactions.keys().next().unwrap().clone();
    late.enqueue(DshCommand::PermissionReply {
        interaction_id,
        reply: RuntimeInteractionReply::Cancel,
    });
    late.until(|session| !session.turn_running());
    let receipts = late.session.dsh_take_receipts().unwrap();
    assert!(
        receipts
            .iter()
            .any(|receipt| receipt.ticket == *ticket && receipt.stage == DshReceiptStage::Written)
    );
    assert!(receipts.iter().any(|receipt| receipt.ticket == *ticket
        && matches!(receipt.stage, DshReceiptStage::TurnError { .. })));
    assert!(
        !receipts
            .iter()
            .any(|receipt| matches!(receipt.stage, DshReceiptStage::ProtocolRejected { .. }))
    );
}

#[test]
fn deepseek_new_early_unknown_identity_is_bounded_then_routed_exactly() {
    let mut f = Fixture::new("early");
    f.establish();
    f.until(|session| session.has_pending_requests());
    assert_eq!(f.session.interactions.len(), 1);
    assert_eq!(
        f.session.interactions.values().next().unwrap().id,
        "early-session"
    );
    f.wait_written();
    assert!(f.wire().iter().any(|record| record["id"] == "early-wrong"
        && record["result"]["outcome"]["outcome"] == "cancelled"));
    assert!(f.session.dsh_port.as_ref().unwrap().early.is_empty());
    assert!(text_seen(&f.session, "Before identity response"));
}

#[test]
fn deepseek_model_value_is_the_exact_opaque_catalog_string_and_sync_mutators_reject() {
    let mut f = Fixture::new("normal");
    f.establish();
    let ticket = f.enqueue(DshCommand::SetModel {
        model: Retained::from_parts(
            RuntimeModel {
                provider_id: Some("provider".into()),
                id: "other".into(),
                name: "other".into(),
            },
            DataRetention::default(),
        ),
    });
    f.until(|session| session.dsh_port.as_ref().unwrap().operations[&ticket.sequence].outcome_seen);
    let wire = f.wire();
    let model = wire
        .iter()
        .find(|record| record["method"] == "session/set_config_option")
        .unwrap();
    assert_eq!(model["params"]["value"], "[ \"provider\", \"other\" ]");
    assert_eq!(
        f.session
            .native_selection
            .as_ref()
            .unwrap()
            .provider_id
            .as_deref(),
        Some("provider")
    );
    assert!(
        f.session
            .select_native_model(Retained::from_parts(
                (Some("provider".into()), "model".into()),
                DataRetention::default(),
            ))
            .is_err()
    );
    assert!(f.session.create(Some(&f.root), "untracked").is_err());
    assert!(f.session.cancel().is_err());
    assert!(f.session.rename("untracked").is_err());
    f.enqueue(DshCommand::Close);
    f.until(|session| session.native_id.is_none());
}

#[test]
fn deepseek_resume_keeps_caller_history_and_rejected_expected_identity_is_not_sendable() {
    for mode in ["early", "resume-reject"] {
        let mut f = Fixture::new(mode);
        let history = vec![Message {
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
            completed: true,
            id: "saved-history".into(),
            role: MessageRole::User,
            timestamp_unix_ms: None,
            blocks: vec![MessageBlock::Text {
                text: "Caller history".into(),
            }],
        }];
        let ticket = f.enqueue(DshCommand::Resume {
            native_id: "session".into(),
            cwd: f.root.clone(),
            runtime_id: "runtime".into(),
            history: Retained::from_parts(history, DataRetention::default()),
        });
        f.until(|session| {
            session.dsh_port.as_ref().unwrap().operations[&ticket.sequence].outcome_seen
        });
        assert!(text_seen(&f.session, "Caller history"));
        assert!(text_seen(&f.session, "Before identity response"));
        if mode == "resume-reject" {
            assert!(f.session.native_id.is_none());
            f.until(|session| !session.has_pending_requests());
            assert!(f.wire().iter().any(|record| record["id"] == "early-session"
                && record["result"]["outcome"]["outcome"] == "cancelled"));
        } else {
            assert_eq!(f.session.native_id.as_deref(), Some("session"));
            assert_eq!(f.session.interactions.len(), 1);
        }
    }
}

#[test]
fn deepseek_written_prompt_does_not_inherit_write_ttl_and_absolute_run_deadline_cannot_revive() {
    let mut f = Fixture::new("deadline");
    f.establish();
    let dispatch_deadline = Instant::now() + Duration::from_millis(100);
    let ticket = f
        .session
        .dsh_enqueue(prompt("Wait for human"), dispatch_deadline)
        .unwrap();
    f.until(|session| session.has_pending_requests());
    f.wait_written();
    std::thread::sleep(Duration::from_millis(120));
    f.session.poll().unwrap();
    assert!(f.session.turn_running());
    assert!(!f.session.requires_failure_cleanup());
    // An old-generation response with a reused logical sequence is never a match.
    let generation = f.session.dsh_port.as_ref().unwrap().generation;
    f.session.dsh_port_record(json!({"jsonrpc":"2.0","id":format!("dsh:{}:{}", generation - 1, ticket.sequence),"result":{"stopReason":"end_turn"}})).unwrap();
    assert!(!f.session.dsh_port.as_ref().unwrap().operations[&ticket.sequence].outcome_seen);
    let id = f.session.interactions.keys().next().unwrap().clone();
    f.enqueue(DshCommand::PermissionReply {
        interaction_id: id,
        reply: RuntimeInteractionReply::Cancel,
    });
    f.until(|session| !session.turn_running());

    let mut hard = Fixture::new("deadline");
    hard.establish();
    let absolute = Instant::now() + Duration::from_millis(150);
    let ticket = hard.enqueue(DshCommand::Prompt {
        text: "Hard deadline".into(),
        run_deadline: Some(absolute),
    });
    hard.until(|session| session.has_pending_requests());
    hard.wait_written();
    // Queue a terminal fact on the real wire, but only poll after the absolute budget.
    let id = hard.session.interactions.keys().next().unwrap().clone();
    hard.enqueue(DshCommand::PermissionReply {
        interaction_id: id,
        reply: RuntimeInteractionReply::Cancel,
    });
    std::thread::sleep(Duration::from_millis(170));
    assert!(hard.session.poll().is_err());
    assert_eq!(hard.session.snapshot().unwrap().run_state, RunState::Failed);
    let receipts = hard.session.dsh_take_receipts().unwrap();
    assert!(receipts.iter().any(|receipt| receipt.ticket == *ticket
        && matches!(receipt.stage, DshReceiptStage::Indeterminate { .. })));
    assert!(receipts.iter().any(|receipt| receipt.ticket == *ticket
        && matches!(receipt.stage, DshReceiptStage::TurnCompleted { .. })));
    assert!(hard.session.requires_failure_cleanup());
    assert!(!hard.session.process_ended);
    assert!(
        hard.session
            .dsh_enqueue(
                prompt("Never replay"),
                Instant::now() + Duration::from_secs(1)
            )
            .is_err()
    );
}

#[test]
fn deepseek_blocked_stdin_keeps_owner_polling_and_holds_cleanup_and_bytes_after_receipts() {
    let mut f = Fixture::new("blocked");
    f.establish();
    f.enqueue(DshCommand::Prompt {
        text: "Blocked synthetic".into(),
        run_deadline: Some(Instant::now() + Duration::from_secs(3600)),
    });
    f.until(|session| session.has_pending_requests());
    let interaction_id = f.session.interactions.keys().next().unwrap().clone();
    let option_id = f.session.interactions[&interaction_id].options[0].clone();
    let started = Instant::now();
    let reply = f
        .session
        .dsh_enqueue(
            DshCommand::PermissionReply {
                interaction_id,
                reply: RuntimeInteractionReply::Decision { option_id },
            },
            Instant::now() + Duration::from_millis(200),
        )
        .unwrap();
    f.enqueue(DshCommand::Cancel);
    assert!(started.elapsed() < Duration::from_millis(150));
    f.until(|session| text_seen(session, "While stdin blocked"));
    let deadline = Instant::now() + Duration::from_secs(2);
    while !f.session.requires_failure_cleanup() {
        let _ = f.session.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let receipts = f.session.dsh_take_receipts().unwrap();
    assert!(receipts.iter().any(|receipt| receipt.ticket == *reply
        && matches!(receipt.stage, DshReceiptStage::Indeterminate { .. })));
    assert!(f.session.dsh_port.as_ref().unwrap().reserved_bytes >= 262144);
    assert!(f.session.work().unwrap().is_empty());
    assert!(f.session.dsh_has_pending_operations());
    assert!(!f.session.process_ended);
    assert!(f.root.exists());
    f.session.shutdown().unwrap();
    f.session.dsh_take_receipts().unwrap();
    assert!(!f.session.requires_failure_cleanup());
    assert!(!f.session.dsh_has_pending_operations());
    assert_eq!(f.session.dsh_port.as_ref().unwrap().reserved_bytes, 0);
}

#[test]
fn deepseek_unconsumed_results_bound_ordinary_admission_but_reserve_protocol_replies() {
    let mut f = Fixture::new("normal");
    f.establish();
    for _ in 0..OPERATIONS - REPLY_RESERVE {
        let ticket = f.enqueue(prompt("One bounded result"));
        f.until(|session| {
            session.dsh_port.as_ref().unwrap().operations[&ticket.sequence].outcome_seen
        });
        f.wait_written();
    }
    assert!(
        matches!(f.session.dsh_enqueue(prompt("No ordinary slot"), Instant::now() + Duration::from_secs(1)), Err(DshAdmissionError::NotDispatched { code, .. }) if code == "queue_full")
    );
    f.session.dsh_handle_record(json!({"jsonrpc":"2.0","id":"reserved-permission","method":"session/request_permission","params":{"sessionId":"session","options":[{"optionId":"once","name":"Allow","kind":"allow_once"}]}})).unwrap();
    let id = f.session.interactions.keys().next().unwrap().clone();
    f.enqueue(DshCommand::PermissionReply {
        interaction_id: id,
        reply: RuntimeInteractionReply::Cancel,
    });
    f.until(|session| !session.has_pending_requests());
    assert_eq!(
        f.session.dsh_port.as_ref().unwrap().operations.len(),
        OPERATIONS - REPLY_RESERVE + 1
    );
    f.session.dsh_take_receipts().unwrap();
    assert!(f.session.dsh_port.as_ref().unwrap().operations.is_empty());
    f.enqueue(prompt("Capacity recovered"));
}

#[test]
fn deepseek_auto_cancel_short_budget_is_fixed_not_parent_hour_and_new_early_overflow_is_terminal() {
    let mut f = Fixture::new("late");
    f.establish();
    f.enqueue(DshCommand::Prompt {
        text: "Long run".into(),
        run_deadline: Some(Instant::now() + Duration::from_secs(3600)),
    });
    f.until(|session| session.has_pending_requests());
    // Fill even the reply/evidence reserve, then retry automatic cancellation without
    // extending its first short write deadline or borrowing the hour-long run budget.
    while f.session.dsh_port.as_ref().unwrap().operations.len() < OPERATIONS {
        f.wait_written();
        let id = json!(format!(
            "synthetic-{}",
            f.session.dsh_port.as_ref().unwrap().operations.len()
        ));
        f.session
            .dsh_auto_reply(&id, &json!({"outcome":{"outcome":"cancelled"}}), None)
            .unwrap();
    }
    f.wait_written();
    f.session.dsh_mark_cancellations();
    f.session.dsh_schedule_cancellations().unwrap();
    let first = *f
        .session
        .dsh_port
        .as_ref()
        .unwrap()
        .cancellations
        .values()
        .next()
        .unwrap();
    assert!(first <= Instant::now() + rpc::CONTROL_TIMEOUT);
    std::thread::sleep(Duration::from_millis(5));
    f.session.dsh_schedule_cancellations().unwrap();
    f.session.dsh_mark_cancellations();
    assert_eq!(
        first,
        *f.session
            .dsh_port
            .as_ref()
            .unwrap()
            .cancellations
            .values()
            .next()
            .unwrap()
    );

    f.session.dsh_take_receipts().unwrap();
    f.until(|session| !session.turn_running() && !session.has_pending_requests());

    let mut early = Fixture::new("overflow");
    early.enqueue(DshCommand::New {
        cwd: early.root.clone(),
        runtime_id: "runtime".into(),
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !early.session.requires_failure_cleanup() {
        let _ = early.session.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(early.session.snapshot().is_none());
    assert!(early.session.native_id.is_none());
    assert!(early.session.dsh_port.as_ref().unwrap().early.len() <= OPERATIONS);
    // A full runtime run may split the overflow and matching response into different
    // ingress batches. The failed owner must retain the bound, then dispose early
    // unknown-identity records when it observes that later terminal response.
    let deadline = Instant::now() + Duration::from_secs(2);
    while !early
        .session
        .dsh_port
        .as_ref()
        .unwrap()
        .operations
        .values()
        .any(|op| op.outcome_seen)
    {
        let _ = early.session.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(early.session.dsh_port.as_ref().unwrap().early.is_empty());
    assert!(!early.session.process_ended);
}

#[test]
fn deepseek_exact_encoded_byte_admission_preserves_large_legal_prompts_with_prepared_budget() {
    let large = "x".repeat(DEFAULT_BYTES + 1);
    let mut small = Fixture::new("normal");
    small.establish();
    assert!(
        matches!(small.session.dsh_enqueue(prompt(&large), Instant::now() + Duration::from_secs(2)),
        Err(DshAdmissionError::NotDispatched { code, message }) if code == "queue_full" && message.contains("bytes"))
    );
    assert!(
        small
            .session
            .dsh_port
            .as_ref()
            .unwrap()
            .operations
            .is_empty()
    );
    assert!(!small.session.requires_failure_cleanup());
    assert_eq!(
        small
            .wire()
            .iter()
            .filter(|record| record["method"] == "session/prompt")
            .count(),
        0
    );
    let mut prepared = Fixture::with_budget("normal", DEFAULT_BYTES * 4);
    prepared.establish();
    let ticket = prepared.enqueue(prompt(&large));
    assert!(prepared.session.dsh_port.as_ref().unwrap().reserved_bytes > large.len());
    prepared.until(|session| {
        session.dsh_port.as_ref().unwrap().operations[&ticket.sequence].outcome_seen
    });
    prepared.wait_written();
    assert_eq!(
        prepared.session.dsh_port.as_ref().unwrap().reserved_bytes,
        0
    );
    assert_eq!(
        prepared
            .wire()
            .iter()
            .filter(|record| record["method"] == "session/prompt")
            .count(),
        1
    );
}

#[test]
fn deepseek_failure_barrier_preserves_real_control_success_without_installing_unconfirmed_identity()
{
    let mut f = Fixture::new("exit-after-new");
    let ticket = f.enqueue(DshCommand::New {
        cwd: f.root.clone(),
        runtime_id: "runtime".into(),
    });
    std::thread::sleep(Duration::from_millis(100));
    assert!(f.session.poll().is_err());
    assert!(f.session.native_id.is_none());
    assert!(f.session.snapshot().is_none());
    let receipts = f.session.dsh_take_receipts().unwrap();
    assert!(receipts.iter().any(|receipt| receipt.ticket == *ticket
        && matches!(receipt.stage, DshReceiptStage::ProtocolSuccess { .. })));
    assert!(receipts.iter().any(|receipt| receipt.ticket == *ticket
        && matches!(receipt.stage, DshReceiptStage::Indeterminate { .. })));
    assert!(f.session.requires_failure_cleanup());
    assert!(!f.session.process_ended);
}

#[test]
fn deepseek_admission_transport_failure_without_a_ticket_still_requires_explicit_cleanup() {
    let mut f = Fixture::new("normal");
    // Simulate an input unavailable at the transport boundary, with no admitted operation.
    f.session.rpc.shutdown();
    assert!(
        matches!(f.session.dsh_enqueue(DshCommand::New { cwd: f.root.clone(), runtime_id: "runtime".into() }, Instant::now() + Duration::from_secs(1)),
        Err(DshAdmissionError::NotDispatched { code, .. }) if code == "transport")
    );
    assert!(f.session.dsh_take_receipts().unwrap().is_empty());
    assert!(f.session.requires_failure_cleanup());
    assert!(f.session.dsh_has_pending_operations());
    f.session.shutdown().unwrap();
    assert!(!f.session.requires_failure_cleanup());
}

#[test]
fn deepseek_overlay_preserves_model_metadata_disables_old_endpoint_and_gates_handoff() {
    let mut f = Fixture::new("normal");
    f.establish();
    let target = crate::control::Target {
        execution: f.session.config.execution.clone(),
        os_home: f.session.config.os_home.clone(),
        kind: crate::version::RuntimeKind::DeepSeek,
        agent_dir: f.root.clone(),
        native_id: "session".into(),
        node_binary: PathBuf::from("/usr/bin/python3"),
        resources: f.root.clone(),
    };
    assert!(f.session.can_handoff_control(Some(&target)));
    let mut config = f.session.config.clone();
    config.gateway = Some(GatewayInjection {
        endpoint: "http://127.0.0.1:12345".into(),
        token: "synthetic-token".into(),
        model_alias: "one".into(),
        protocol: "openai-completions".into(),
        context_window: Some(10000),
        max_output_tokens: None,
        reasoning_levels: None,
    });
    assert!(NativeSession::dsh_command(&config).unwrap().1);
    config.gateway.as_mut().unwrap().model_alias = "two".into();
    NativeSession::dsh_command(&config).unwrap();
    config.gateway = None;
    assert!(NativeSession::dsh_command(&config).unwrap().1);
    let native: Value = serde_json::from_slice(
        &std::fs::read(f.root.join("deepseek-native-history.patch.yml")).unwrap(),
    )
    .unwrap();
    let provider = &native[0]["config"]["providers"]["velune-gateway"];
    assert_eq!(provider["baseURL"], "http://127.0.0.1:0");
    assert_eq!(provider["models"].as_array().unwrap().len(), 2);
    assert_eq!(provider["models"][0]["id"], "one");
    assert_eq!(provider["models"][1]["id"], "two");
    assert!(provider["models"][0].get("maxTokens").is_none());
    f.session.dsh_history_overlay = true;
    assert!(!f.session.can_handoff_control(Some(&target)));
}

#[test]
fn deepseek_auto_compaction_applies_on_same_session_resume_and_clear_restores_profile() {
    use velune_conversation::AutoCompactionSetting;

    let mut f = Fixture::build("normal", None);
    std::fs::write(
        f.root.join("composed.yml"),
        r#"- id: other
  config: {secret: synthetic-unrelated}
- id: agent
  group: true
  config:
  - id: compaction-basic
    name: '@deepseek-ai/dsh-compaction-basic'
    config:
      retainRatio: 0.22
      maxTokens: 3000
      headroomTokens: !!js '1024 + 1024'
      modelPolicies:
      - {provider: provider, model: model, thresholdRatio: 0.63, retainTokens: 1024}
      - {provider: provider, model: sibling, thresholdRatio: 0.58}
"#,
    )
    .unwrap();
    f.session.create(Some(&f.root), "runtime").unwrap();
    assert_eq!(
        f.session.dsh_current_model(),
        Some(("provider".into(), "model".into()))
    );
    f.session
        .set_auto_compaction(Some(AutoCompactionSetting::DeepSeek {
            threshold_percent: 74,
        }));
    f.session
        .send(&crate::TurnInput {
            text: "synthetic".into(),
            effort: None,
            permission_mode: None,
            images: Vec::new(),
        })
        .unwrap();
    f.until(|session| !session.turn_running());
    f.session.set_auto_compaction(None);
    f.session
        .send(&crate::TurnInput {
            text: "clear".into(),
            effort: None,
            permission_mode: None,
            images: Vec::new(),
        })
        .unwrap();
    f.until(|session| !session.turn_running());
    let wire = f.wire();
    let patch = wire
        .iter()
        .find_map(|record| record["compactionPatch"].as_str())
        .unwrap();
    assert!(patch.contains("!!js"));
    assert!(!patch.contains("synthetic-unrelated"));
    let patch: serde_yaml_ng::Value = serde_yaml_ng::from_str(patch).unwrap();
    let config = &patch[0]["config"];
    assert_eq!(config["retainRatio"].as_f64(), Some(0.22));
    assert_eq!(config["maxTokens"].as_u64(), Some(3000));
    assert_eq!(config["headroomTokens"].as_str(), Some("1024 + 1024"));
    let policies = config["modelPolicies"].as_sequence().unwrap();
    assert_eq!(policies.len(), 2);
    assert_eq!(policies[0]["thresholdRatio"].as_f64(), Some(0.74));
    assert_eq!(policies[0]["retainTokens"].as_u64(), Some(1024));
    assert_eq!(policies[1]["thresholdRatio"].as_f64(), Some(0.58));
    let resumes = wire
        .iter()
        .filter(|record| record["method"] == "session/resume")
        .map(|record| record["params"]["sessionId"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(resumes, ["session", "session"]);
    assert_eq!(
        wire.iter()
            .filter(|record| record.get("configDump").is_some())
            .count(),
        1
    );
    assert_eq!(
        wire.iter()
            .filter(|record| record.get("compactionPatch").is_some())
            .count(),
        1
    );
    assert!(std::fs::read_dir(&f.root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("dsh-compaction-")
    }));
}

#[test]
fn deepseek_compaction_config_uses_gateway_route_identity() {
    use velune_conversation::AutoCompactionSetting;

    let f = Fixture::new("normal");
    let mut config = f.session.config.clone();
    std::fs::write(
        f.root.join("composed.yml"),
        "- id: compaction-basic\n  name: '@deepseek-ai/dsh-compaction-basic'\n",
    )
    .unwrap();
    config.gateway = Some(GatewayInjection {
        endpoint: "http://127.0.0.1:12345".into(),
        token: "synthetic-token".into(),
        model_alias: "route-alias".into(),
        protocol: "openai-completions".into(),
        context_window: Some(180_000),
        max_output_tokens: None,
        reasoning_levels: None,
    });
    config.auto_compaction = Some(AutoCompactionSetting::DeepSeek {
        threshold_percent: 16,
    });
    let conflict = NativeSession::dsh_spawn(&config, None)
        .err()
        .expect("the threshold must exceed the native default retained fraction");
    assert!(conflict.to_string().contains("原生近期上下文保留比例"));
    config.auto_compaction = Some(AutoCompactionSetting::DeepSeek {
        threshold_percent: 85,
    });
    let (mut rpc, overlay) = NativeSession::dsh_spawn(&config, None).unwrap();
    assert!(overlay);
    rpc.finish();
    let launch = f
        .wire()
        .into_iter()
        .find(|record| record.get("compactionPatch").is_some())
        .unwrap();
    let patch: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(launch["compactionPatch"].as_str().unwrap()).unwrap();
    let policy = &patch[0]["config"]["modelPolicies"][0];
    assert_eq!(policy["provider"].as_str(), Some("velune-gateway"));
    assert_eq!(policy["model"].as_str(), Some("route-alias"));
    assert_eq!(policy["thresholdRatio"].as_f64(), Some(0.85));
}

#[test]
fn deepseek_blocking_catalog_keeps_private_persistence_lifecycle_and_removes_failure_artifacts() {
    for mode in ["normal", "reject"] {
        let mut f = Fixture::build(mode, None);
        let directory = f.root.join(format!("dsh-catalog-{}", f.session.epoch));
        let result = f.session.dsh_prepare_model_catalog_blocking();
        assert_eq!(result.is_ok(), mode == "normal");
        assert!(!directory.exists());
        assert!(f.session.native_id.is_none());
        let wire = f.wire();
        let launch = wire
            .iter()
            .find(|record| record.get("patch").is_some())
            .unwrap();
        assert_eq!(
            launch["patch"][0]["config"]["root"],
            directory.join("sessions").to_string_lossy().as_ref()
        );
        assert_eq!(
            launch["patch"][1]["config"]["root"],
            directory.join("storages").to_string_lossy().as_ref()
        );
        assert_eq!(launch["patch"][2]["config"]["path"], ":memory:");
        assert_eq!(
            wire.iter()
                .filter(|record| record["method"] == "session/new")
                .count(),
            1
        );
        assert_eq!(
            wire.iter()
                .filter(|record| record["method"] == "session/close")
                .count(),
            usize::from(mode == "normal")
        );
    }
}

#[test]
fn deepseek_shared_writer_oversized_inflight_frame_must_preserve_reserved_reply_admission() {
    for (text_bytes, budget) in [
        (DEFAULT_BYTES + 1, DEFAULT_BYTES * 4),
        // Fill the DSH ordinary allocation pool too: its reply reserve is separate.
        (DEFAULT_BYTES - 1024 * 1024, DEFAULT_BYTES),
    ] {
        let mut f = Fixture::with_budget("gated-large", budget);
        f.establish();
        let ticket = f
            .session
            .dsh_enqueue(
                prompt(&"x".repeat(text_bytes)),
                Instant::now() + Duration::from_secs(10),
            )
            .unwrap();
        f.until(|session| session.has_pending_requests());
        assert!(text_seen(&f.session, "Oversized write still polling"));
        assert!(f.session.dsh_port.as_ref().unwrap().reserved_bytes >= DEFAULT_BYTES);
        assert!(
            !f.session
                .dsh_take_receipts()
                .unwrap()
                .iter()
                .any(|receipt| receipt.ticket == *ticket
                    && receipt.stage == DshReceiptStage::Written)
        );
        let cancel = f
            .session
            .dsh_enqueue(DshCommand::Cancel, Instant::now() + Duration::from_secs(1));
        let interaction_id = f.session.interactions.keys().next().unwrap().clone();
        let reply = f.session.dsh_enqueue(
            DshCommand::PermissionReply {
                interaction_id,
                reply: RuntimeInteractionReply::Cancel,
            },
            Instant::now() + Duration::from_secs(1),
        );
        assert!(
            cancel.is_ok() && reply.is_ok(),
            "reserved commands not admitted: cancel={cancel:?}; reply={reply:?}"
        );
        // Admission while blocked is not Written and cannot bypass the partial prompt.
        assert!(
            !f.session
                .dsh_take_receipts()
                .unwrap()
                .iter()
                .any(|receipt| receipt.stage == DshReceiptStage::Written)
        );
        std::fs::write(f.root.join("release-large"), b"release").unwrap();
        f.wait_written();
        let deadline = Instant::now() + Duration::from_secs(3);
        while std::fs::read_to_string(f.root.join("large-fifo-complete"))
            .ok()
            .as_deref()
            != Some("done")
        {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        // Peer publication is a distinct boundary from physical Written.
        let wire = f.wire();
        let frames: Vec<_> = wire
            .iter()
            .filter(|record| {
                record["method"] == "session/prompt"
                    || record["method"] == "session/cancel"
                    || (record["id"] == "large-permission" && record.get("result").is_some())
            })
            .collect();
        assert_eq!(frames.len(), 3, "each actual input appears once");
        assert_eq!(frames[0]["method"], "session/prompt");
        assert_eq!(frames[1]["method"], "session/cancel");
        assert_eq!(frames[2]["id"], "large-permission");
    }
}

#[test]
fn deepseek_control_deadline_is_latched_before_matching_late_success() {
    let mut f = Fixture::new("normal");
    let ticket = f
        .session
        .dsh_enqueue(
            DshCommand::New {
                cwd: f.root.clone(),
                runtime_id: "runtime".into(),
            },
            Instant::now() + Duration::from_millis(30),
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(60));
    assert!(f.session.poll().is_err());
    assert!(f.session.native_id.is_none());
    let receipts = f.session.dsh_take_receipts().unwrap();
    assert!(receipts.iter().any(|receipt| receipt.ticket == *ticket
        && matches!(receipt.stage, DshReceiptStage::Indeterminate { .. })));
    assert!(receipts.iter().any(|receipt| receipt.ticket == *ticket
        && matches!(receipt.stage, DshReceiptStage::ProtocolSuccess { .. })));
    assert!(f.session.requires_failure_cleanup());
}

#[test]
fn deepseek_initialize_validates_protocol_resume_and_required_http_not_plugin_release() {
    let mut f = Fixture::build("normal", None);
    for (protocol, resume, http, require_http, accepted) in [
        (json!(2), json!({}), true, false, false),
        (json!(1), Value::Null, true, false, false),
        (json!(1), json!(false), true, false, false),
        (json!(1), json!({}), false, true, false),
        (json!(1), json!({}), true, true, true),
    ] {
        f.session.rpc.shutdown();
        let response = json!({"protocolVersion":protocol,
            "agentCapabilities":{"sessionCapabilities":{"resume":resume},"mcpCapabilities":{"http":http}},
            "agentInfo":{"name":"synthetic-plugin","version":"999.0.0"}});
        let script = format!(
            "import json,sys\nresponse=json.loads({:?})\nfor line in sys.stdin:\n r=json.loads(line)\n print(json.dumps(dict(jsonrpc='2.0',id=r['id'],result=response)),flush=True)\n",
            response.to_string()
        );
        let mut command = crate::process_tree::command("/usr/bin/python3");
        command.arg("-c").arg(script);
        f.session.rpc =
            rpc::RpcClient::spawn_at(command, &f.session.config.execution, Some(&f.root)).unwrap();
        f.session.config.mcp_servers = if require_http {
            vec![crate::mcp::Server {
                name: "synthetic".into(),
                endpoint: "http://127.0.0.1:0".into(),
                token: "synthetic".into(),
            }]
        } else {
            vec![]
        };
        assert_eq!(f.session.dsh_initialize().is_ok(), accepted);
    }
}

// Raw observation caches physical timestamps without settling the DSH operation.
// These private gates intentionally separate writer completion from owner harvesting.
fn review_wait_write(
    f: &Fixture,
    ticket: &DshTicket,
    predicate: impl Fn(&crate::transport::WriteOutcome) -> bool,
) -> crate::transport::WriteOutcome {
    let limit = Instant::now() + Duration::from_secs(5);
    loop {
        let write = f.session.dsh_port.as_ref().unwrap().operations[&ticket.sequence]
            .write
            .as_ref()
            .expect("unharvested physical receipt");
        if let Some(outcome) = write.try_outcome()
            && predicate(&outcome)
        {
            return outcome;
        }
        assert!(Instant::now() < limit, "physical fixture outcome missing");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn review_timely_unharvested_write(reply: bool) {
    let mut f = Fixture::new("late");
    f.establish();
    let deadline = Instant::now() + Duration::from_secs(3);
    let ticket = f
        .session
        .dsh_enqueue(prompt("Timely but not harvested"), deadline)
        .unwrap();
    let outcome = review_wait_write(&f, &ticket, |outcome| {
        matches!(outcome, crate::transport::WriteOutcome::Written(_))
    });
    assert!(
        matches!(outcome, crate::transport::WriteOutcome::Written(completion) if completion.met_deadline())
    );
    // Service real native ingress without harvesting the already-complete prompt write.
    // The parent deadline is absent: this is an ordinary prompt waiting for a human.
    let limit = Instant::now() + Duration::from_secs(2);
    while !f.session.has_pending_requests() {
        for record in f.session.rpc.poll().unwrap() {
            f.session
                .receive_native_record(record, NativeSession::dsh_port_record)
                .unwrap();
        }
        assert!(Instant::now() < limit);
        std::thread::sleep(Duration::from_millis(2));
    }
    f.session.dsh_port.as_mut().unwrap().test_now = Some(deadline + Duration::from_millis(1));
    let command = if reply {
        DshCommand::PermissionReply {
            interaction_id: f.session.interactions.keys().next().unwrap().clone(),
            reply: RuntimeInteractionReply::Cancel,
        }
    } else {
        DshCommand::Cancel
    };
    let result = f
        .session
        .dsh_enqueue(command, Instant::now() + Duration::from_secs(10));
    assert!(
        result.is_ok(),
        "timely physical Written must settle before expiry: {result:?}"
    );
    assert!(!f.session.requires_failure_cleanup());
    assert!(
        f.session
            .dsh_take_receipts()
            .unwrap()
            .iter()
            .any(|receipt| receipt.ticket == *ticket && receipt.stage == DshReceiptStage::Written)
    );
}
#[test]
fn deepseek_review_timely_unharvested_written_allows_cancel_after_write_deadline() {
    review_timely_unharvested_write(false);
}
#[test]
fn deepseek_review_timely_unharvested_written_allows_permission_reply_after_write_deadline() {
    review_timely_unharvested_write(true);
}

#[test]
fn deepseek_review_late_physical_written_keeps_fact_and_failed_cleanup_after_observation_timeout() {
    let mut f = Fixture::with_budget("gated-large", DEFAULT_BYTES * 4);
    f.prepare_blocked_input();
    let text = "x".repeat(DEFAULT_BYTES + 1);
    let deadline = Instant::now() + Duration::from_secs(1);
    let ticket = f
        .session
        .dsh_enqueue(
            DshCommand::Prompt {
                text,
                run_deadline: None,
            },
            deadline,
        )
        .unwrap();
    // Do not run an unrelated permission-readiness loop through this deliberately
    // expired write budget. The raw outcome is the measured fixture boundary.
    review_wait_write(&f, &ticket, |outcome| {
        matches!(
            outcome,
            crate::transport::WriteOutcome::ObservationTimeout { .. }
        )
    });
    assert!(
        f.session.poll().is_err(),
        "the measured write deadline must latch failure"
    );
    assert!(f.session.requires_failure_cleanup());
    assert!(
        f.session.dsh_port.as_ref().unwrap().operations[&ticket.sequence]
            .write
            .is_some()
    );
    assert!(f.session.dsh_port.as_ref().unwrap().reserved_bytes > DEFAULT_BYTES);
    std::fs::write(f.root.join("release-large"), b"release").unwrap();
    let outcome = review_wait_write(&f, &ticket, |outcome| {
        matches!(outcome, crate::transport::WriteOutcome::Written(_))
    });
    assert!(
        matches!(outcome, crate::transport::WriteOutcome::Written(completion)
        if completion.started_at < deadline && completion.completed_at >= deadline && !completion.met_deadline())
    );
    let _ = f.session.poll();
    let receipts = f.session.dsh_take_receipts().unwrap();
    assert!(
        receipts
            .iter()
            .any(|receipt| receipt.ticket == *ticket && receipt.stage == DshReceiptStage::Written),
        "late physical Written must not vanish into compatibility timeout: {receipts:?}"
    );
    assert!(receipts.iter().any(|receipt| receipt.ticket == *ticket
        && matches!(receipt.stage, DshReceiptStage::Indeterminate { .. })));
    assert!(f.session.requires_failure_cleanup());
    assert!(!f.session.process_ended);
    assert_eq!(f.session.snapshot().unwrap().run_state, RunState::Failed);
}

fn review_terminal_crosses_budget(command: &str) {
    let mut f = Fixture::new("normal");
    if command == "prompt" {
        f.establish();
    }
    let absolute = Instant::now() + Duration::from_secs(3600);
    let input = match command {
        "new" => DshCommand::New {
            cwd: f.root.clone(),
            runtime_id: "runtime".into(),
        },
        "resume" => DshCommand::Resume {
            native_id: "session".into(),
            cwd: f.root.clone(),
            runtime_id: "runtime".into(),
            history: Retained::from_parts(vec![], DataRetention::default()),
        },
        "prompt" => DshCommand::Prompt {
            text: "Actual terminal across deadline".into(),
            run_deadline: Some(absolute),
        },
        _ => unreachable!(),
    };
    let ticket = f.session.dsh_enqueue(input, absolute).unwrap();
    let outcome = review_wait_write(&f, &ticket, |outcome| {
        matches!(outcome, crate::transport::WriteOutcome::Written(_))
    });
    assert!(
        matches!(outcome, crate::transport::WriteOutcome::Written(completion) if completion.met_deadline())
    );
    // Poll enters at the real pre-deadline time. Only the matching terminal's
    // private synchronization point advances owner time across the absolute D.
    f.session.dsh_port.as_mut().unwrap().terminal_now = Some(absolute + Duration::from_millis(1));
    let limit = Instant::now() + Duration::from_secs(2);
    while !f.session.dsh_port.as_ref().unwrap().operations[&ticket.sequence].outcome_seen {
        let _ = f.session.poll();
        assert!(Instant::now() < limit);
        std::thread::sleep(Duration::from_millis(2));
    }
    let receipts = f.session.dsh_take_receipts().unwrap();
    let actual = receipts.iter().any(|receipt| {
        receipt.ticket == *ticket
            && if command == "prompt" {
                matches!(receipt.stage, DshReceiptStage::TurnCompleted { .. })
            } else {
                matches!(receipt.stage, DshReceiptStage::ProtocolSuccess { .. })
            }
    });
    assert!(actual, "real terminal evidence must survive the barrier");
    assert!(
        receipts.iter().any(|receipt| receipt.ticket == *ticket
            && matches!(receipt.stage, DshReceiptStage::Indeterminate { .. })),
        "terminal crossing D must latch uncertainty before healthy install: {receipts:?}"
    );
    assert!(f.session.requires_failure_cleanup());
    if command == "prompt" {
        assert_eq!(f.session.snapshot().unwrap().run_state, RunState::Failed);
    } else {
        assert!(
            f.session.native_id.is_none(),
            "no healthy unconfirmed identity may be installed"
        );
    }
}
#[test]
fn deepseek_review_new_terminal_crossing_deadline_never_installs_identity() {
    review_terminal_crosses_budget("new");
}
#[test]
fn deepseek_review_resume_terminal_crossing_deadline_never_installs_identity() {
    review_terminal_crosses_budget("resume");
}
#[test]
fn deepseek_review_prompt_terminal_crossing_deadline_never_revives_idle() {
    review_terminal_crosses_budget("prompt");
}

#[test]
fn deepseek_review_automatic_reply_dispatch_crosses_absolute_deadline_barrier() {
    let mut f = Fixture::new("late");
    f.establish();
    let absolute = Instant::now() + Duration::from_secs(3600);
    let ticket = f.enqueue(DshCommand::Prompt {
        text: "Budgeted run".into(),
        run_deadline: Some(absolute),
    });
    f.until(|session| session.has_pending_requests());
    f.wait_written();
    let sequence = f.session.dsh_port.as_ref().unwrap().next;
    f.session.dsh_port.as_mut().unwrap().test_now = Some(absolute + Duration::from_millis(1));
    let _ = f.session.dsh_handle_record(
        json!({"jsonrpc":"2.0","id":"after-absolute-D","method":"unsupported","params":{}}),
    );
    assert!(
        f.session.requires_failure_cleanup(),
        "automatic reply must cross the operation barrier too"
    );
    assert_eq!(
        f.session.dsh_port.as_ref().unwrap().next,
        sequence,
        "expired owner cannot admit an automatic wire frame"
    );
    assert!(
        f.session
            .dsh_take_receipts()
            .unwrap()
            .iter()
            .any(|receipt| receipt.ticket == *ticket
                && matches!(receipt.stage, DshReceiptStage::Indeterminate { .. }))
    );
}

#[test]
fn deepseek_review_late_completion_before_first_owner_poll_is_written_but_never_healthy() {
    let mut f = Fixture::with_budget("gated-large", DEFAULT_BYTES * 4);
    f.establish();
    let deadline = Instant::now() + Duration::from_secs(1);
    let ticket = f
        .session
        .dsh_enqueue(prompt(&"x".repeat(DEFAULT_BYTES + 1)), deadline)
        .unwrap();
    // Raw observation does not settle the owner or latch an operation failure.
    review_wait_write(&f, &ticket, |outcome| {
        matches!(
            outcome,
            crate::transport::WriteOutcome::ObservationTimeout { .. }
        )
    });
    assert!(!f.session.requires_failure_cleanup());
    std::fs::write(f.root.join("release-large"), b"release").unwrap();
    let outcome = review_wait_write(&f, &ticket, |outcome| {
        matches!(outcome, crate::transport::WriteOutcome::Written(_))
    });
    assert!(
        matches!(outcome, crate::transport::WriteOutcome::Written(completion)
        if completion.started_at < deadline && completion.completed_at >= deadline && !completion.met_deadline())
    );
    assert!(f.session.poll().is_err());
    let receipts = f.session.dsh_take_receipts().unwrap();
    assert!(
        receipts
            .iter()
            .any(|receipt| receipt.ticket == *ticket && receipt.stage == DshReceiptStage::Written)
    );
    assert!(receipts.iter().any(|receipt| receipt.ticket == *ticket
        && matches!(receipt.stage, DshReceiptStage::Indeterminate { .. })));
    assert!(f.session.requires_failure_cleanup());
    assert_eq!(f.session.snapshot().unwrap().run_state, RunState::Failed);
}

#[test]
fn deepseek_queued_cancel_known_not_written_retains_actual_fact_without_peer_delivery() {
    let mut f = Fixture::with_budget("gated-large", DEFAULT_BYTES * 4);
    f.establish();
    let prompt_ticket = f
        .session
        .dsh_enqueue(
            prompt(&"x".repeat(DEFAULT_BYTES + 1)),
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
    f.until(|session| session.has_pending_requests());
    let cancel = f
        .session
        .dsh_enqueue(
            DshCommand::Cancel,
            Instant::now() + Duration::from_millis(100),
        )
        .unwrap();
    review_wait_write(&f, &cancel, |outcome| {
        matches!(
            outcome,
            crate::transport::WriteOutcome::ObservationTimeout { .. }
        )
    });
    let _ = f.session.poll();
    assert!(f.session.requires_failure_cleanup());
    assert!(
        f.session.dsh_port.as_ref().unwrap().operations[&cancel.sequence]
            .write
            .is_some()
    );
    assert!(f.session.dsh_port.as_ref().unwrap().reserved_reply_bytes > 0);
    std::fs::write(f.root.join("release-large"), b"release").unwrap();
    review_wait_write(&f, &cancel, |outcome| {
        matches!(outcome, crate::transport::WriteOutcome::NotWritten(_))
    });
    let _ = f.session.poll();
    let receipts = f.session.dsh_take_receipts().unwrap();
    assert!(receipts.iter().any(|receipt| receipt.ticket == *cancel
        && matches!(receipt.stage, DshReceiptStage::NotWritten { .. })));
    assert!(
        !receipts
            .iter()
            .any(|receipt| receipt.ticket == *cancel && receipt.stage == DshReceiptStage::Written)
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while !f.session.dsh_port.as_ref().unwrap().operations[&prompt_ticket.sequence].outcome_seen {
        let _ = f.session.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !f.wire()
            .iter()
            .any(|record| record["method"] == "session/cancel")
    );
    assert_eq!(f.session.dsh_port.as_ref().unwrap().reserved_reply_bytes, 0);
    assert!(f.session.requires_failure_cleanup());
    assert!(!f.session.process_ended);
}

#[test]
fn deepseek_late_write_fixture_permission_readiness_is_not_a_measured_write_precondition() {
    let mut f = Fixture::with_budget("gated-large", DEFAULT_BYTES * 4);
    f.prepare_blocked_input();
    let text = "x".repeat(DEFAULT_BYTES + 1);
    let deadline = Instant::now() + Duration::from_secs(1);
    let ticket = f
        .session
        .dsh_enqueue(
            DshCommand::Prompt {
                text,
                run_deadline: None,
            },
            deadline,
        )
        .unwrap();
    // Recreate the old readiness dependency deterministically: peer permission output
    // is held while a legitimate pending physical write reaches its unchanged D.
    let limit = Instant::now() + Duration::from_secs(5);
    let readiness_error = loop {
        assert!(
            !f.session.has_pending_requests(),
            "permission output remains gated"
        );
        if let Err(error) = f.session.poll() {
            break error;
        }
        assert!(
            Instant::now() < limit,
            "controlled readiness reproduction did not reach D"
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(readiness_error.code(), "timeout");
    assert!(
        readiness_error
            .to_string()
            .contains("write outcome not yet observed")
    );
    assert_eq!(
        f.session.native_id.as_deref(),
        Some("session"),
        "session preparation had completed before measurement"
    );
    assert!(Instant::now() >= deadline);
    let pending = review_wait_write(&f, &ticket, |outcome| {
        matches!(
            outcome,
            crate::transport::WriteOutcome::ObservationTimeout { .. }
        )
    });
    assert!(
        matches!(pending, crate::transport::WriteOutcome::ObservationTimeout { deadline: observed_deadline, .. } if observed_deadline == deadline)
    );
    assert!(
        f.session.dsh_port.as_ref().unwrap().operations[&ticket.sequence]
            .write
            .is_some()
    );
    assert!(f.session.dsh_port.as_ref().unwrap().reserved_bytes > DEFAULT_BYTES);
    eprintln!(
        "causal fixture: session/helper prepared before D; permission deliberately withheld; readiness poll reached the unchanged 1s write D with retained ObservationTimeout, error={readiness_error}"
    );

    std::fs::write(f.root.join("allow-large-permission"), b"allow").unwrap();
    let mut peer_permission_seen = false;
    let limit = Instant::now() + Duration::from_secs(3);
    while !peer_permission_seen {
        for record in f.session.rpc.poll().unwrap() {
            peer_permission_seen |= record["id"] == "large-permission"
                && record["method"] == "session/request_permission";
        }
        assert!(Instant::now() < limit);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(Instant::now() >= deadline);
    std::fs::write(f.root.join("release-large"), b"release").unwrap();
    let outcome = review_wait_write(&f, &ticket, |outcome| {
        matches!(outcome, crate::transport::WriteOutcome::Written(_))
    });
    assert!(
        matches!(outcome, crate::transport::WriteOutcome::Written(completion)
        if completion.started_at < deadline && completion.completed_at >= deadline && !completion.met_deadline())
    );
    let _ = f.session.poll();
    let receipts = f.session.dsh_take_receipts().unwrap();
    assert!(
        receipts
            .iter()
            .any(|receipt| receipt.ticket == *ticket && receipt.stage == DshReceiptStage::Written)
    );
    assert!(receipts.iter().any(|receipt| receipt.ticket == *ticket
        && matches!(receipt.stage, DshReceiptStage::Indeterminate { .. })));
    assert!(f.session.requires_failure_cleanup());
    assert_eq!(f.session.snapshot().unwrap().run_state, RunState::Failed);
    eprintln!(
        "causal fixture: peer permission arrived only after D; actual started-before-D/completed-after-D Written is retained alongside failure and cleanup, not misclassified healthy"
    );
}
