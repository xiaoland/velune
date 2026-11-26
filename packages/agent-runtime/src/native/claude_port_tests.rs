//! Synthetic stream-json child: held ACKs and blocked stdin, no model/account IO.
use super::*;
use std::{
    os::unix::fs::PermissionsExt,
    time::{Duration, Instant},
};
use velune_test_support::TempRoot;

pub(super) struct Fixture {
    root: TempRoot,
    pub(super) session: NativeSession,
}
impl Fixture {
    pub(super) fn new(mode: &str) -> Self {
        Self::with_port(mode, true)
    }
    fn with_port(mode: &str, port: bool) -> Self {
        let root = TempRoot::new("claude-port");
        std::fs::write(root.join("mode"), mode).unwrap();
        let binary = root.join("claude");
        std::fs::write(&binary, r#"#!/usr/bin/python3
import json,sys,os,time
root=os.path.dirname(__file__)
if '--version' in sys.argv:
 print('2.1.292 (Claude Code)');sys.exit(0)
with open(os.path.join(root,'argv.json'),'w') as f: json.dump(sys.argv,f)
mode=open(os.path.join(root,'mode')).read()
held=[]
started=False
def emit(record): print(json.dumps(record),flush=True)
def response(request,negative=False):
 emit({'type':'control_response','response':{'subtype':'error' if negative else 'success','request_id':request['request_id'],'error':'synthetic rejection' if negative else None,'response':{'models':[]}}})
def terminal():
 emit({'type':'system','subtype':'task_notification','task_id':'shell','run_id':'one','status':'completed','summary':'done'})
for line in sys.stdin:
 request=json.loads(line)
 with open(os.path.join(root,'requests.jsonl'),'a') as f: f.write(json.dumps(request)+'\n')
 if request.get('type')=='fixture_emit': emit(request['record']);continue
 if request.get('type')=='fixture_batch':
  for record in request['records']: emit(record)
  continue
 if request.get('type')=='fixture_exit':
  for record in request['records']: emit(record)
  sys.exit(0)
 if request.get('type')=='control_request':
  subtype=request['request']['subtype']
  if subtype=='initialize':
   if mode=='refresh' and 'hooks' not in request['request']: continue
   old=os.path.join(root,'old-response')
   if os.path.exists(old): emit(json.load(open(old)))
   response(request)
  elif subtype=='side_question':
   emit({'type':'control_response','response':{'subtype':'success','request_id':request['request_id'],'response':{'response':'synthetic side answer'}}})
  elif subtype=='set_permission_mode':
   response({'request_id':'unrelated-control'})
   if mode=='steer_disconnect': sys.exit(0)
   if mode=='steer_ended':
    emit({'type':'result','subtype':'success','is_error':False})
    emit({'type':'system','subtype':'session_state_changed','state':'idle'})
   response(request,mode=='steer_rejected')
   if mode=='steer_success': response(request)
  elif mode=='delay':
   time.sleep(.12);terminal();response(request,True)
  elif subtype in ['interrupt','stop_task'] and mode in ['hold','block']:
   held.append(request)
  else: response(request,mode=='reject')
 elif request.get('type')=='user' and not started:
  started=True
  if mode.startswith('steer_'):
   emit({'type':'system','subtype':'init','permissionMode':'default'})
   if mode!='steer_success': continue
  emit({'type':'system','subtype':'task_started','task_id':'shell','run_id':'one','task_type':'local_bash','description':'Shell'})
  emit({'type':'control_request','request_id':'permission','request':{'subtype':'can_use_tool','tool_name':'Bash','input':{'command':'synthetic'}}})
  if mode=='block':
   while not os.path.exists(os.path.join(root,'release')):
    emit({'type':'system','subtype':'task_progress','task_id':'shell','run_id':'one','description':'pulse'})
    time.sleep(.02)
 elif request.get('type')=='control_response':
  terminal()
  for pending in held: response(pending,pending['request']['subtype']=='stop_task')
  held=[]
"#).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut session = NativeSession::connect(NativeConfig {
            execution: crate::execution::ExecutionTarget::Local,
            os_home: Some(root.to_path_buf()),
            kind: NativeKind::ClaudeCode,
            binary,
            node_binary: None,
            agent_dir: root.to_path_buf(),
            resources_directory: root.to_path_buf(),
            projection_directory: root.to_path_buf(),
            gateway: None,
            mcp_servers: vec![],
            session_agents: vec![],
        })
        .unwrap();
        session.replace_snapshot("parent", Some(root.path()), "runtime", vec![]);
        session.claude.start = claude::ClaudeStart::Fresh { name: None };
        if port {
            session
                .prepare_claude_commands(None, None, 64 * 1024 * 1024)
                .unwrap();
        } else {
            let mut input = Self::input("once");
            input.permission_mode = Some("manual".into());
            session.send(&input).unwrap();
        }
        Self { root, session }
    }
    fn input(text: &str) -> crate::TurnInput {
        crate::TurnInput {
            text: text.into(),
            effort: None,
            permission_mode: None,
            images: vec![],
        }
    }
    fn enqueue(&mut self, command: ClaudeCommand) -> Retained<ClaudeOperationTicket> {
        self.session
            .enqueue_claude_command(command, Instant::now() + Duration::from_secs(3), None)
            .unwrap()
    }
    fn poll_until(&mut self, condition: impl Fn(&NativeSession) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !condition(&self.session) {
            self.session.poll().unwrap();
            assert!(
                Instant::now() < deadline,
                "synthetic child did not progress"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn user(&mut self) -> Retained<ClaudeOperationTicket> {
        let ticket = self.enqueue(ClaudeCommand::User(Self::input("once")));
        self.poll_until(|session| session.has_pending_requests());
        ticket
    }
    fn permission(&mut self) -> Retained<ClaudeOperationTicket> {
        let id = self.session.interactions.keys().next().unwrap().clone();
        self.enqueue(ClaudeCommand::PermissionReply {
            interaction_id: id,
            reply: RuntimeInteractionReply::Decision {
                option_id: "allow".into(),
            },
        })
    }
    fn emit(&mut self, record: Value) {
        self.session
            .rpc
            .write(&json!({"type":"fixture_emit","record":record}))
            .unwrap();
    }
    fn requests(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.join("requests.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    // Stage real child output in Rpc's deferred records before latching failure.
    // The initialize response is a test-only stdout fence, not a port admission:
    // all preceding records are received/deferred before this helper returns.
    fn expired_interrupt_terminal_batch(
        &mut self,
        progress: usize,
        result_subtypes: &[&str],
    ) -> Retained<ClaudeOperationTicket> {
        let deadline = Instant::now() + Duration::from_millis(120);
        let interrupt = self
            .session
            .enqueue_claude_command(ClaudeCommand::Interrupt, deadline, None)
            .unwrap();
        let mut records = (0..progress)
            .map(|id| json!({"type":"system","subtype":"task_progress","task_id":"shell","run_id":"one","description":format!("pulse-{id}")}))
            .collect::<Vec<_>>();
        records.push(json!({"type":"system","subtype":"task_notification","task_id":"shell","run_id":"one","status":"completed"}));
        for subtype in result_subtypes {
            records
                .push(json!({"type":"result","subtype":subtype,"is_error":*subtype != "success"}));
            records.push(json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
        }
        records.push(json!({"type":"control_request","request_id":"blocked-late-request","request":{"subtype":"unknown"}}));
        records.push(json!({"type":"control_response","response":{"subtype":"success","request_id":interrupt.request_id,"response":{}}}));
        self.session
            .rpc
            .write(&json!({"type":"fixture_batch","records":records}))
            .unwrap();
        self.session
            .rpc
            .claude_control(json!({"subtype":"initialize","hooks":{}}))
            .unwrap();
        std::thread::sleep(
            deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(5),
        );
        interrupt
    }
    fn assert_failed_terminal_owner(&mut self, interrupt: &ClaudeOperationTicket) {
        let receipts = self.session.take_claude_receipts();
        let receipt = receipts
            .iter()
            .find(|receipt| receipt.ticket == *interrupt)
            .unwrap();
        assert!(matches!(
            receipt.delivery,
            ClaudeDelivery::Indeterminate { .. }
        ));
        assert_eq!(
            receipt.late_response.as_ref().unwrap()["subtype"],
            "success"
        );
        let snapshot = self.session.snapshot().unwrap();
        assert_eq!(snapshot.run_state, RunState::Failed);
        assert!(!snapshot.actions.can_send);
        assert!(self.session.claude_cleanup_required());
        assert!(!self.session.process_ended);
        assert!(!self.session.turn_running());
        assert!(
            !self
                .requests()
                .iter()
                .any(|record| record["type"] == "control_response")
        );
        assert_eq!(
            self.requests()
                .iter()
                .filter(|record| record["type"] == "user")
                .count(),
            1
        );
        assert!(
            self.session
                .enqueue_claude_command(
                    ClaudeCommand::Resume,
                    Instant::now() + Duration::from_secs(1),
                    None,
                )
                .is_err()
        );
    }
    fn complete(&mut self) {
        self.permission();
        self.poll_until(|session| {
            !session.has_pending_requests() && session.work().unwrap().is_empty()
        });
        self.emit(json!({"type":"result","subtype":"success","is_error":false}));
        self.emit(json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
        self.poll_until(|session| !session.turn_running());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.session.shutdown().unwrap();
    }
}

#[test]
fn claude_legacy_steer_updates_permission_without_answering_an_existing_tool_prompt() {
    let mut fixture = Fixture::with_port("steer_success", false);
    fixture.poll_until(|session| session.has_pending_requests());
    let active_turn = fixture.session.turn_id.clone();
    let mut steer = Fixture::input("steered");
    steer.permission_mode = Some("bypassPermissions".into());
    fixture.session.steer(&steer).unwrap();
    steer.text = "steered again".into();
    fixture.session.steer(&steer).unwrap();
    // The test-only stdout fence proves the child consumed the user frame.
    fixture
        .session
        .rpc
        .claude_control(json!({"subtype":"initialize","hooks":{}}))
        .unwrap();
    fixture.session.poll().unwrap();

    let argv: Vec<String> =
        serde_json::from_slice(&std::fs::read(fixture.root.join("argv.json")).unwrap()).unwrap();
    assert!(
        argv.iter()
            .any(|arg| arg == "--allow-dangerously-skip-permissions")
    );
    assert!(
        !argv
            .iter()
            .any(|arg| arg == "--dangerously-skip-permissions")
    );
    assert!(
        argv.windows(2)
            .any(|pair| pair == ["--permission-mode", "manual"])
    );
    assert_eq!(fixture.session.turn_id, active_turn);
    assert!(fixture.session.has_pending_requests());
    let requests = fixture.requests();
    let user_and_permission = requests
        .iter()
        .filter(|record| {
            record["type"] == "user" || record["request"]["subtype"] == "set_permission_mode"
        })
        .collect::<Vec<_>>();
    assert_eq!(user_and_permission.len(), 4);
    assert_eq!(
        user_and_permission[0]["message"]["content"][0]["text"],
        "once"
    );
    assert_eq!(
        user_and_permission[1]["request"]["mode"],
        "bypassPermissions"
    );
    assert_eq!(
        user_and_permission[2]["message"]["content"][0]["text"],
        "steered"
    );
    assert_eq!(
        user_and_permission[3]["message"]["content"][0]["text"],
        "steered again"
    );
    assert!(
        !requests
            .iter()
            .any(|record| record["type"] == "control_response")
    );
    let snapshot = fixture.session.snapshot().unwrap();
    let last = snapshot.messages.last().unwrap();
    assert_eq!(
        last.turn_settings
            .as_ref()
            .unwrap()
            .permission_mode
            .as_deref(),
        Some("bypassPermissions")
    );
}

#[test]
fn claude_legacy_steer_never_sends_text_when_permission_is_rejected_or_the_turn_ends() {
    for (mode, error) in [
        ("steer_rejected", "steer_permission_rejected"),
        ("steer_disconnect", "steer_permission_unconfirmed"),
        ("steer_ended", "steer_settings_changed"),
    ] {
        let mut fixture = Fixture::with_port(mode, false);
        let mut steer = Fixture::input("steered");
        steer.permission_mode = Some("bypassPermissions".into());
        assert_eq!(fixture.session.steer(&steer).unwrap_err().code(), error);
        let requests = fixture.requests();
        assert_eq!(
            requests
                .iter()
                .filter(|frame| frame["type"] == "user")
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|frame| frame["request"]["subtype"] == "set_permission_mode")
                .count(),
            1
        );
    }
}

#[test]
fn claude_input_uses_the_wire_uuid_for_its_live_row() {
    let mut fixture = Fixture::new("hold");
    fixture.user();
    let messages = &fixture.session.snapshot().unwrap().messages;
    let id = &messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .unwrap()
        .id;
    let requests = fixture.requests();
    let frame = requests
        .iter()
        .find(|request| request["type"] == "user")
        .unwrap();
    assert_eq!(frame["uuid"].as_str(), Some(id.as_str()));
    assert_eq!(id.len(), 36);
}

#[test]
fn claude_port_rejects_unapplied_steer_configuration_without_user_write() {
    let mut fixture = Fixture::new("hold");
    fixture.user();
    let mut steer = Fixture::input("must not be sent");
    steer.permission_mode = Some("bypassPermissions".into());
    let Err(ClaudeAdmissionError::Rejected(error)) = fixture.session.enqueue_claude_command(
        ClaudeCommand::Steer(steer),
        Instant::now() + Duration::from_secs(3),
        None,
    ) else {
        panic!("steer with unconfirmed settings was admitted")
    };
    assert_eq!(error.code(), "steer_settings_unsupported");
    assert_eq!(
        fixture
            .requests()
            .iter()
            .filter(|frame| frame["type"] == "user")
            .count(),
        1
    );
}

#[test]
fn claude_port_written_is_not_accepted_and_held_ack_does_not_block_permission() {
    let mut f = Fixture::new("hold");
    let user = f.user();
    let interrupt = f.enqueue(ClaudeCommand::Interrupt);
    let reply = f.permission();
    f.poll_until(|session| !session.claude.commands.pending());
    let receipts = f.session.take_claude_receipts();
    assert_eq!(receipts.len(), 3);
    assert!(
        receipts
            .iter()
            .any(|r| r.ticket == *user && matches!(r.delivery, ClaudeDelivery::Written))
    );
    assert!(
        receipts
            .iter()
            .any(|r| r.ticket == *reply && matches!(r.delivery, ClaudeDelivery::Written))
    );
    assert!(
        receipts.iter().any(
            |r| r.ticket == *interrupt && matches!(r.delivery, ClaudeDelivery::Accepted { .. })
        )
    );
    assert!(f.session.turn_running(), "control ACK is not completion");
    assert_eq!(
        f.requests().iter().filter(|r| r["type"] == "user").count(),
        1
    );
    assert_eq!(
        f.requests()
            .iter()
            .filter(|r| r["type"] == "control_response")
            .count(),
        1
    );
    assert!(f.session.steer(&Fixture::input("untracked")).is_err());
    assert!(f.session.cancel().is_err());
    assert!(f.session.rename("untracked").is_err());
}

#[test]
fn claude_port_blocked_stdin_keeps_poll_and_reserved_reply_admission_live() {
    let mut f = Fixture::new("block");
    f.user();
    let steer = f.enqueue(ClaudeCommand::Steer(Fixture::input(
        &"x".repeat(2 * 1024 * 1024),
    )));
    let interrupt = f.enqueue(ClaudeCommand::Interrupt);
    let reply = f.permission();
    let started = Instant::now();
    for _ in 0..10 {
        f.session.poll().unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(
        f.session
            .background_tasks()
            .unwrap()
            .tasks
            .iter()
            .any(|task| task.title == "pulse")
    );
    assert!(f.session.has_pending_requests(), "queued is not written");
    assert!(f.session.claude.commands.pending());
    std::fs::write(f.root.join("release"), "release").unwrap();
    f.poll_until(|session| !session.claude.commands.pending());
    let receipts = f.session.take_claude_receipts();
    assert!(
        receipts
            .iter()
            .any(|r| r.ticket == *steer && matches!(r.delivery, ClaudeDelivery::Written))
    );
    assert!(
        receipts
            .iter()
            .any(|r| r.ticket == *reply && matches!(r.delivery, ClaudeDelivery::Written))
    );
    assert!(
        receipts.iter().any(
            |r| r.ticket == *interrupt && matches!(r.delivery, ClaudeDelivery::Accepted { .. })
        )
    );
    assert_eq!(
        f.requests().iter().filter(|r| r["type"] == "user").count(),
        2
    );
    assert_eq!(
        f.requests()
            .iter()
            .filter(|r| r["type"] == "control_response")
            .count(),
        1
    );
}

#[test]
fn claude_port_stop_task_negative_ack_cannot_revive_terminal_and_ack_is_not_stop() {
    let mut f = Fixture::new("hold");
    f.user();
    let task = f.session.background_tasks().unwrap().tasks[0].id.clone();
    let stop = f.enqueue(ClaudeCommand::StopTask {
        task_id: task.clone(),
    });
    assert_eq!(
        f.session.background_tasks().unwrap().tasks[0].status,
        BackgroundTaskStatus::Stopping
    );
    f.permission();
    f.poll_until(|session| !session.claude.commands.pending());
    assert_eq!(
        f.session.background_tasks().unwrap().tasks[0].status,
        BackgroundTaskStatus::Completed
    );
    assert!(
        f.session
            .take_claude_receipts()
            .iter()
            .any(|r| r.ticket == *stop && matches!(r.delivery, ClaudeDelivery::Rejected { .. }))
    );
    let requests = f.requests();
    let wire = requests
        .iter()
        .find(|r| r["request"]["subtype"] == "stop_task")
        .unwrap();
    assert_eq!(wire["request"]["task_id"], "shell");
    assert!(
        f.session
            .enqueue_claude_command(
                ClaudeCommand::StopTask { task_id: task },
                Instant::now() + Duration::from_secs(1),
                None
            )
            .is_err()
    );
}

#[test]
fn claude_background_refresh_requires_a_matching_ack_and_new_snapshot() {
    let mut f = Fixture::new("refresh");
    f.user();
    let known = f.session.background_tasks().unwrap().tasks[0].clone();
    let approvals = f.session.snapshot().unwrap().pending_interactions.clone();
    assert_eq!(approvals.len(), 1);
    f.session.available_models = vec![json!({"value":"existing-model"})];

    f.session.claude_refresh_background_tasks();
    let first = f
        .session
        .claude
        .task_refresh
        .as_ref()
        .unwrap()
        .request_id
        .clone();
    f.session.claude_refresh_background_tasks();
    let pending = f.session.claude.task_refresh.as_mut().unwrap();
    assert_eq!(pending.request_id, first);
    pending.deadline = Instant::now();
    f.session.poll().unwrap();
    let timed_out = f.session.background_tasks().unwrap();
    assert!(timed_out.live && timed_out.issue.as_ref().unwrap().contains("超时"));
    assert_eq!(timed_out.tasks, vec![known.clone()]);
    assert!(f.session.turn_running() && !f.session.failed);

    f.session.claude_refresh_background_tasks();
    let retry = f
        .session
        .claude
        .task_refresh
        .as_ref()
        .unwrap()
        .request_id
        .clone();
    assert_ne!(retry, first);
    let ack = |request_id| {
        json!({"type":"control_response","response":{
            "subtype":"success", "request_id":request_id, "response":{"models":[]},
            "pending_permission_requests":[{"type":"control_request","request_id":"permission",
                "request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"synthetic"}}}]
        }})
    };
    let snapshot = |description| {
        json!({"type":"system","subtype":"background_tasks_changed",
            "session_id":"parent","tasks":[{
                "task_id":"shell","run_id":"one","task_type":"local_bash","description":description
            }]
        })
    };
    f.emit(ack(first.clone()));
    f.emit(snapshot("原生推送"));
    f.poll_until(|session| session.background_tasks().unwrap().tasks[0].title == "原生推送");
    assert!(!f.session.claude.task_refresh.as_ref().unwrap().accepted);
    assert!(f.session.background_tasks().unwrap().issue.is_some());

    f.emit(ack(retry.clone()));
    f.poll_until(|session| session.claude.task_refresh.as_ref().unwrap().accepted);
    assert!(f.session.background_tasks().unwrap().issue.is_some());
    f.emit(snapshot("刷新清单"));
    f.poll_until(|session| session.claude.task_refresh.is_none());
    let updated = f.session.background_tasks().unwrap();
    assert!(updated.live && updated.issue.is_none());
    assert_eq!(updated.tasks[0].id, known.id);
    assert_eq!(updated.tasks[0].title, "刷新清单");
    assert_eq!(
        f.session.snapshot().unwrap().pending_interactions,
        approvals
    );
    assert_eq!(
        f.session.available_models,
        vec![json!({"value":"existing-model"})]
    );
    let refreshes = f
        .requests()
        .into_iter()
        .filter(|frame| {
            frame["request"]["subtype"] == "initialize" && frame["request"].get("hooks").is_none()
        })
        .map(|frame| frame["request_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(refreshes, vec![first, retry]);
}

#[test]
fn claude_port_expiry_precedes_late_ack_preserves_terminal_and_cleanup_owner() {
    let mut f = Fixture::new("delay");
    f.user();
    let ticket = f
        .session
        .enqueue_claude_command(
            ClaudeCommand::Interrupt,
            Instant::now() + Duration::from_millis(30),
            None,
        )
        .unwrap();
    f.session
        .rpc
        .write(&json!({"type":"fixture_batch","records":[
            {"type":"result","subtype":"success","is_error":false},
            {"type":"system","subtype":"session_state_changed","state":"idle"}
        ]}))
        .unwrap();
    std::thread::sleep(Duration::from_millis(180));
    assert!(f.session.poll().is_err());
    let receipts = f.session.take_claude_receipts();
    let receipt = receipts.iter().find(|r| r.ticket == *ticket).unwrap();
    assert!(matches!(
        receipt.delivery,
        ClaudeDelivery::Indeterminate { .. }
    ));
    assert_eq!(receipt.late_response.as_ref().unwrap()["subtype"], "error");
    assert_eq!(
        f.session.background_tasks().unwrap().tasks[0].status,
        BackgroundTaskStatus::Completed
    );
    assert!(f.session.requires_failure_cleanup());
    assert!(!f.session.process_ended);
    let _ = f.session.poll();
    assert!(f.session.claude_cleanup_required());
    let snapshot = f.session.snapshot().unwrap();
    assert_eq!(snapshot.run_state, RunState::Failed);
    assert!(!snapshot.actions.can_send);
    assert_eq!(
        snapshot.executions.records.last().unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    assert!(!f.session.turn_running());
    assert!(
        f.session
            .enqueue_claude_command(
                ClaudeCommand::Steer(Fixture::input("must not replay")),
                Instant::now() + Duration::from_secs(1),
                None
            )
            .is_err()
    );
    assert_eq!(
        f.requests().iter().filter(|r| r["type"] == "user").count(),
        1
    );
}

#[test]
fn claude_port_cross_batch_terminal_refines_only_connection_failure() {
    let mut f = Fixture::new("hold");
    f.user();
    let interrupt = f.expired_interrupt_terminal_batch(64, &["success"]);
    assert!(f.session.poll().is_err());
    let snapshot = f.session.snapshot().unwrap();
    assert_eq!(
        snapshot.executions.records.last().unwrap().outcome,
        Some(TurnOutcome::Failed)
    );
    assert!(f.session.claude_has_failed_turn());
    assert_ne!(
        f.session.background_tasks().unwrap().tasks[0].status,
        BackgroundTaskStatus::Completed
    );
    let _ = f.session.poll();
    assert_eq!(
        f.session.background_tasks().unwrap().tasks[0].status,
        BackgroundTaskStatus::Completed
    );
    assert_eq!(
        f.session
            .snapshot()
            .unwrap()
            .executions
            .records
            .last()
            .unwrap()
            .outcome,
        Some(TurnOutcome::Completed)
    );
    assert!(!f.session.claude_has_failed_turn());
    f.assert_failed_terminal_owner(&interrupt);
}

#[test]
fn claude_port_enqueue_failure_then_poll_refines_received_terminal() {
    let mut f = Fixture::new("hold");
    f.user();
    let interrupt = f.expired_interrupt_terminal_batch(0, &["success"]);
    assert!(
        f.session
            .enqueue_claude_command(
                ClaudeCommand::Steer(Fixture::input("must not dispatch")),
                Instant::now() + Duration::from_secs(1),
                None,
            )
            .is_err()
    );
    assert!(f.session.claude_has_failed_turn());
    assert_eq!(
        f.session
            .snapshot()
            .unwrap()
            .executions
            .records
            .last()
            .unwrap()
            .outcome,
        Some(TurnOutcome::Failed)
    );
    let _ = f.session.poll();
    assert_eq!(
        f.session
            .snapshot()
            .unwrap()
            .executions
            .records
            .last()
            .unwrap()
            .outcome,
        Some(TurnOutcome::Completed)
    );
    f.assert_failed_terminal_owner(&interrupt);
}

#[test]
fn claude_port_late_terminal_cannot_refine_replaced_generation_or_execution() {
    for replacement in ["generation", "execution", "activity"] {
        let mut f = Fixture::new("hold");
        f.user();
        f.expired_interrupt_terminal_batch(0, &["success"]);
        assert!(
            f.session
                .enqueue_claude_command(
                    ClaudeCommand::Steer(Fixture::input("must not dispatch")),
                    Instant::now() + Duration::from_secs(1),
                    None,
                )
                .is_err()
        );
        assert!(f.session.claude_has_failed_turn());
        let old_execution = f
            .session
            .snapshot()
            .unwrap()
            .executions
            .current
            .as_ref()
            .unwrap()
            .execution_id
            .clone();
        if replacement == "generation" {
            // Keep the old proof and barrier to exercise the generation guard,
            // not merely next_generation's eager proof invalidation.
            f.session.claude.commands.generation += 1;
        } else {
            f.session
                .snapshot
                .as_mut()
                .unwrap()
                .executions
                .begin(ExecutionKind::UserTurn, replacement == "activity");
        }
        assert!(!f.session.claude_has_failed_turn());
        let _ = f.session.poll();
        let snapshot = f.session.snapshot().unwrap();
        let old = snapshot
            .executions
            .records
            .iter()
            .find(|record| record.id == old_execution)
            .unwrap();
        assert_eq!(
            old.outcome,
            if replacement == "activity" {
                None
            } else {
                Some(TurnOutcome::Failed)
            }
        );
        assert!(
            !snapshot
                .executions
                .records
                .iter()
                .any(|record| record.outcome == Some(TurnOutcome::Completed))
        );
        assert_eq!(snapshot.run_state, RunState::Failed);
        assert!(!snapshot.actions.can_send);
        assert!(!f.session.process_ended);
        assert!(f.session.claude_cleanup_required());
    }
}

#[test]
fn claude_port_late_success_cannot_replace_known_native_failure() {
    let mut f = Fixture::new("hold");
    f.user();
    let interrupt = f.expired_interrupt_terminal_batch(64, &["error", "success"]);
    assert!(f.session.poll().is_err());
    let _ = f.session.poll();
    assert_eq!(
        f.session
            .snapshot()
            .unwrap()
            .executions
            .records
            .last()
            .unwrap()
            .outcome,
        Some(TurnOutcome::Failed)
    );
    assert!(!f.session.claude_has_failed_turn());
    f.assert_failed_terminal_owner(&interrupt);
}

#[test]
fn claude_port_barrier_preserves_prior_native_result_in_both_directions() {
    for (first, duplicate, expected) in [
        ("error", "success", TurnOutcome::Failed),
        ("success", "error", TurnOutcome::Completed),
    ] {
        let mut f = Fixture::new("hold");
        f.user();
        f.emit(json!({"type":"result","subtype":first,"is_error":first != "success"}));
        f.poll_until(|session| session.claude.result.is_some());
        assert_eq!(f.session.claude.result.as_ref().unwrap().0, expected);
        assert!(f.session.turn_running(), "result alone is not idle");
        assert!(!f.session.failed);
        assert!(
            f.session
                .snapshot()
                .unwrap()
                .executions
                .records
                .last()
                .unwrap()
                .outcome
                .is_none()
        );
        // No successful assistant response intervenes. The next poll latches
        // expiry before projecting this duplicate result and its idle record.
        let interrupt = f.expired_interrupt_terminal_batch(0, &[duplicate]);
        assert!(f.session.poll().is_err());
        assert_eq!(
            f.session
                .snapshot()
                .unwrap()
                .executions
                .records
                .last()
                .unwrap()
                .outcome,
            Some(expected)
        );
        f.assert_failed_terminal_owner(&interrupt);
    }
}

#[test]
fn claude_port_barrier_keeps_api_and_refusal_recovery_from_successful_assistant() {
    for transient in [
        json!({"type":"assistant","is_api_error_message":true,"error":"synthetic_error","message":{"id":"transient-api","stop_reason":"stop_sequence","content":[{"type":"text","text":"synthetic API error"}]}}),
        json!({"type":"assistant","message":{"id":"transient-refusal","stop_reason":"refusal","content":[{"type":"text","text":"synthetic refusal"}]}}),
    ] {
        let mut f = Fixture::new("hold");
        f.user();
        f.emit(transient);
        f.poll_until(|session| session.claude.result.is_some());
        assert_eq!(
            f.session.claude.result.as_ref().unwrap().0,
            TurnOutcome::Failed
        );
        // Queue a genuine successful assistant response, but do not project it
        // until the following poll has latched the interrupt's failure barrier.
        f.emit(json!({"type":"assistant","message":{"id":"native-recovery","stop_reason":"end_turn","content":[{"type":"text","text":"recovered"}]}}));
        let interrupt = f.expired_interrupt_terminal_batch(0, &["success"]);
        assert!(f.session.poll().is_err());
        assert_eq!(
            f.session
                .snapshot()
                .unwrap()
                .executions
                .records
                .last()
                .unwrap()
                .outcome,
            Some(TurnOutcome::Completed)
        );
        f.assert_failed_terminal_owner(&interrupt);
    }
}

#[test]
fn claude_port_written_clears_short_write_budget_but_not_explicit_run_deadline() {
    let mut f = Fixture::new("hold");
    let ticket = f
        .session
        .enqueue_claude_command(
            ClaudeCommand::User(Fixture::input("once")),
            Instant::now() + Duration::from_millis(100),
            Some(Instant::now() + Duration::from_millis(220)),
        )
        .unwrap();
    f.poll_until(|session| session.has_pending_requests());
    f.poll_until(|session| !session.claude.commands.pending());
    assert!(
        f.session
            .take_claude_receipts()
            .iter()
            .any(|r| r.ticket == *ticket && matches!(r.delivery, ClaudeDelivery::Written))
    );
    std::thread::sleep(Duration::from_millis(120));
    f.session.poll().unwrap();
    assert!(f.session.turn_running());
    std::thread::sleep(Duration::from_millis(120));
    assert!(f.session.poll().is_err());
    assert!(f.session.requires_failure_cleanup());
    assert!(!f.session.process_ended);
}

#[test]
fn claude_port_no_model_ttl_from_written_or_receipt_consumption() {
    let mut f = Fixture::new("hold");
    f.session
        .enqueue_claude_command(
            ClaudeCommand::User(Fixture::input("once")),
            Instant::now() + Duration::from_millis(100),
            None,
        )
        .unwrap();
    f.poll_until(|session| session.has_pending_requests() && !session.claude.commands.pending());
    f.session.take_claude_receipts();
    std::thread::sleep(Duration::from_millis(150));
    f.session.poll().unwrap();
    assert!(f.session.turn_running());
    assert!(!f.session.claude_cleanup_required());
}

#[test]
fn claude_port_unconsumed_evidence_reserves_reply_and_control_capacity() {
    let mut f = Fixture::new("hold");
    f.user();
    for _ in 0..11 {
        f.enqueue(ClaudeCommand::Steer(Fixture::input("steer")));
        f.poll_until(|session| !session.claude.commands.pending());
    }
    assert_eq!(
        f.session
            .enqueue_claude_command(
                ClaudeCommand::Steer(Fixture::input("overflow")),
                Instant::now() + Duration::from_secs(1),
                None
            )
            .unwrap_err()
            .to_string(),
        "Claude command/evidence capacity full"
    );
    f.enqueue(ClaudeCommand::Interrupt);
    f.permission();
    f.poll_until(|session| !session.claude.commands.pending());
    assert_eq!(f.session.take_claude_receipts().len(), 14);
}

#[test]
fn claude_port_large_frame_is_not_silently_removed_and_budget_rejects_before_dispatch() {
    let mut f = Fixture::new("hold");
    f.session
        .prepare_claude_commands(None, None, 128 * 1024)
        .unwrap();
    assert!(
        f.session
            .enqueue_claude_command(
                ClaudeCommand::User(Fixture::input(&"x".repeat(128 * 1024))),
                Instant::now() + Duration::from_secs(1),
                None
            )
            .is_err()
    );
    assert!(!f.session.turn_running());
    assert!(!f.requests().iter().any(|r| r["type"] == "user"));
    f.session
        .prepare_claude_commands(None, None, 64 * 1024 * 1024)
        .unwrap();
    let ticket = f.enqueue(ClaudeCommand::User(Fixture::input(
        &"x".repeat(9 * 1024 * 1024),
    )));
    f.poll_until(|session| session.has_pending_requests() && !session.claude.commands.pending());
    assert!(
        f.session
            .take_claude_receipts()
            .iter()
            .any(|r| r.ticket == *ticket && matches!(r.delivery, ClaudeDelivery::Written))
    );
    assert_eq!(
        f.requests().iter().find(|r| r["type"] == "user").unwrap()["message"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .len(),
        9 * 1024 * 1024
    );
}

#[test]
fn claude_port_restart_fences_old_generation_and_keeps_fixed_fork_and_options() {
    let mut f = Fixture::new("hold");
    f.user();
    let old = f.enqueue(ClaudeCommand::Initialize);
    f.poll_until(|session| !session.claude.commands.pending());
    f.complete();
    assert!(f.session.claude_cleanup_required());
    assert!(!f.session.process_ended);
    f.session.take_claude_receipts();
    f.session
        .with_claude_blocking_preparation(|session| {
            session.select_native_model(Retained::from_parts(
                (None, "synthetic-model".into()),
                DataRetention::default(),
            ))?;
            session.claude.start = claude::ClaudeStart::Fork {
                from: "fixed-source".into(),
                at: Some("fixed-boundary".into()),
                name: Some("fork-name".into()),
            };
            Ok(())
        })
        .unwrap();
    f.session
        .prepare_claude_commands(Some("high"), Some("manual"), 64 * 1024 * 1024)
        .unwrap();
    std::fs::write(f.root.join("old-response"), serde_json::to_vec(&json!({"type":"control_response","response":{"subtype":"success","request_id":old.request_id,"response":{"models":[{"value":"stale"}]}}})).unwrap()).unwrap();
    let init = f.enqueue(ClaudeCommand::Initialize);
    f.poll_until(|session| !session.claude.commands.pending());
    assert!(init.generation > old.generation);
    assert!(f.session.available_models.is_empty());
    assert!(
        f.session
            .take_claude_receipts()
            .iter()
            .all(|r| r.ticket == *init)
    );
    let args: Vec<String> =
        serde_json::from_slice(&std::fs::read(f.root.join("argv.json")).unwrap()).unwrap();
    for pair in [
        ["--resume", "fixed-source"],
        ["--resume-session-at", "fixed-boundary"],
        ["--model", "synthetic-model"],
        ["--effort", "high"],
        ["--permission-mode", "manual"],
        ["--name", "fork-name"],
    ] {
        assert!(args.windows(2).any(|args| args == pair));
    }
    assert!(args.iter().any(|arg| arg == "--fork-session"));
}

#[test]
fn claude_port_side_result_is_not_admission_ack_and_cancellation_is_explicit_io() {
    let mut f = Fixture::new("hold");
    f.session
        .with_claude_blocking_preparation(|session| {
            session.side = true;
            session.claude.start = claude::ClaudeStart::Side;
            Ok(())
        })
        .unwrap();
    f.session
        .prepare_claude_commands(None, None, 64 * 1024 * 1024)
        .unwrap();
    let ticket = f.enqueue(ClaudeCommand::User(Fixture::input("side question")));
    f.poll_until(|session| !session.claude.commands.pending());
    assert!(
        f.session
            .take_claude_receipts()
            .iter()
            .any(|r| r.ticket == *ticket && matches!(r.delivery, ClaudeDelivery::SideResult { .. }))
    );
    assert_eq!(f.session.claude.side_history.len(), 1);
    assert!(!f.session.turn_running());
    assert!(f.session.claude_cleanup_required());
    assert!(!f.session.process_ended);
    assert!(matches!(
        f.session.enqueue_claude_command(
            ClaudeCommand::Interrupt,
            Instant::now() + Duration::from_secs(1),
            None
        ),
        Err(ClaudeAdmissionError::NeedsPreparation)
    ));
}

#[test]
fn claude_port_unknown_request_automatic_rejection_does_not_wait_for_blocked_stdin() {
    let mut f = Fixture::new("block");
    f.user();
    f.enqueue(ClaudeCommand::Steer(Fixture::input(
        &"x".repeat(2 * 1024 * 1024),
    )));
    let started = Instant::now();
    f.session.claude_handle_record(json!({"type":"control_request","request_id":"unknown","request":{"subtype":"not-supported"}})).unwrap();
    assert!(started.elapsed() < Duration::from_millis(100));
    f.session.poll().unwrap();
    std::fs::write(f.root.join("release"), "release").unwrap();
    f.poll_until(|session| !session.claude.commands.pending());
    let deadline = Instant::now() + Duration::from_secs(2);
    while !f
        .requests()
        .iter()
        .any(|r| r["response"]["request_id"] == "unknown")
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        f.requests()
            .iter()
            .filter(|r| r["response"]["request_id"] == "unknown")
            .count(),
        1
    );
}

#[test]
fn claude_port_partial_write_timeout_keeps_cleanup_fact_after_receipt_drain() {
    let mut f = Fixture::new("block");
    f.user();
    let ticket = f
        .session
        .enqueue_claude_command(
            ClaudeCommand::Steer(Fixture::input(&"x".repeat(2 * 1024 * 1024))),
            Instant::now() + Duration::from_millis(80),
            None,
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(120));
    assert!(f.session.poll().is_err());
    assert!(
        f.session
            .take_claude_receipts()
            .iter()
            .any(|r| r.ticket == *ticket
                && matches!(r.delivery, ClaudeDelivery::Indeterminate { .. }))
    );
    assert!(f.session.requires_failure_cleanup());
    assert!(!f.session.process_ended);
    assert!(
        f.session
            .enqueue_claude_command(
                ClaudeCommand::Initialize,
                Instant::now() + Duration::from_secs(1),
                None
            )
            .is_err()
    );
    f.session.shutdown().unwrap();
    assert!(f.session.process_ended);
    assert!(!f.session.claude_cleanup_required());
}

#[test]
fn claude_port_enqueue_latches_expiry_before_dispatch_and_without_a_new_ticket() {
    let mut f = Fixture::new("hold");
    f.user();
    f.session
        .enqueue_claude_command(
            ClaudeCommand::Interrupt,
            Instant::now() + Duration::from_millis(30),
            None,
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(70));
    assert!(
        f.session
            .enqueue_claude_command(
                ClaudeCommand::Steer(Fixture::input("must not dispatch")),
                Instant::now() + Duration::from_secs(1),
                None
            )
            .is_err()
    );
    assert!(f.session.requires_failure_cleanup());
    assert!(!f.session.process_ended);
    assert_eq!(
        f.requests().iter().filter(|r| r["type"] == "user").count(),
        1
    );
}

#[test]
fn claude_port_recorded_transport_failure_is_a_dispatch_barrier_but_keeps_terminal() {
    let mut f = Fixture::new("hold");
    f.user();
    let interrupt = f.enqueue(ClaudeCommand::Interrupt);
    // The same synthetic stdout batch contains terminal/control evidence, an
    // unknown request that would auto-reply, and EOF. No mutation crosses EOF.
    f.session.rpc.write(&json!({"type":"fixture_exit","records":[
        {"type":"system","subtype":"task_notification","task_id":"shell","run_id":"one","status":"completed"},
        {"type":"result","subtype":"success","is_error":false},
        {"type":"system","subtype":"session_state_changed","state":"idle"},
        {"type":"control_request","request_id":"blocked-by-barrier","request":{"subtype":"unknown"}},
        {"type":"control_response","response":{"subtype":"success","request_id":interrupt.request_id,"response":{}}}
    ]})).unwrap();
    std::thread::sleep(Duration::from_millis(40));
    assert!(f.session.poll().is_err());
    assert_eq!(
        f.session.background_tasks().unwrap().tasks[0].status,
        BackgroundTaskStatus::Completed
    );
    assert!(
        !f.requests()
            .iter()
            .any(|r| r["response"]["request_id"] == "blocked-by-barrier")
    );
    let receipts = f.session.take_claude_receipts();
    let receipt = receipts.iter().find(|r| r.ticket == *interrupt).unwrap();
    assert!(matches!(
        receipt.delivery,
        ClaudeDelivery::Indeterminate { .. }
    ));
    assert_eq!(
        receipt.late_response.as_ref().unwrap()["subtype"],
        "success"
    );
    let snapshot = f.session.snapshot().unwrap();
    assert_eq!(snapshot.run_state, RunState::Failed);
    assert!(!snapshot.actions.can_send);
    assert_eq!(
        snapshot.executions.records.last().unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    assert!(f.session.claude_cleanup_required());
    assert!(!f.session.process_ended);
}

#[test]
fn claude_port_result_is_not_idle_and_compaction_history_is_an_explicit_boundary() {
    let mut f = Fixture::new("hold");
    f.user();
    f.emit(json!({"type":"result","subtype":"success","is_error":false}));
    f.poll_until(|session| session.claude.result.is_some());
    assert!(f.session.turn_running());
    assert!(!f.session.claude_cleanup_required());
    f.emit(json!({"type":"system","subtype":"session_state_changed","state":"requires_action"}));
    f.poll_until(|session| session.claude.requires_action);
    assert!(f.session.has_pending_requests());
    f.emit(json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
    f.poll_until(|session| !session.turn_running());
    assert!(
        f.session.has_pending_requests(),
        "idle must not release unanswered approval"
    );
    assert!(!f.session.process_ended);
    f.permission();
    f.poll_until(|session| session.claude_cleanup_required());
    f.session.take_claude_receipts();
    f.session
        .prepare_claude_commands(None, None, 64 * 1024 * 1024)
        .unwrap();
    let compact = f.enqueue(ClaudeCommand::Compact);
    f.poll_until(|session| session.has_pending_requests() && !session.claude.commands.pending());
    assert!(
        f.session
            .take_claude_receipts()
            .iter()
            .any(|receipt| receipt.ticket == *compact
                && matches!(receipt.delivery, ClaudeDelivery::Written))
    );
    f.emit(json!({"type":"result","subtype":"success","is_error":false}));
    f.poll_until(|session| session.claude_context_refresh_required());
    assert!(!f.session.process_ended);
}

#[test]
fn claude_port_checked_sequence_and_duplicate_reply_never_write_untracked_frames() {
    let mut f = Fixture::new("hold");
    f.user();
    let id = f.session.interactions.keys().next().unwrap().clone();
    f.permission();
    assert!(
        f.session
            .enqueue_claude_command(
                ClaudeCommand::PermissionReply {
                    interaction_id: id,
                    reply: RuntimeInteractionReply::Cancel
                },
                Instant::now() + Duration::from_secs(1),
                None
            )
            .is_err()
    );
    f.session.claude.steer_sequence = u64::MAX;
    assert!(
        f.session
            .enqueue_claude_command(
                ClaudeCommand::Steer(Fixture::input("overflow")),
                Instant::now() + Duration::from_secs(1),
                None
            )
            .is_err()
    );
    f.poll_until(|session| {
        !session.claude.commands.pending() && session.work().unwrap().is_empty()
    });
    assert_eq!(
        f.requests().iter().filter(|r| r["type"] == "user").count(),
        1
    );
    assert_eq!(
        f.requests()
            .iter()
            .filter(|r| r["type"] == "control_response")
            .count(),
        1
    );
}

#[test]
fn claude_port_automatic_overload_latches_before_same_batch_control_ack() {
    let mut f = Fixture::new("hold");
    f.user();
    for _ in 0..11 {
        f.enqueue(ClaudeCommand::Steer(Fixture::input("steer")));
        f.poll_until(|session| !session.claude.commands.pending());
    }
    // Twelve retained one-way receipts occupy ordinary capacity. Four real
    // held control operations fill the reserve regardless of write harvesting;
    // flushed automatic replies alone do not retain an occupied slot.
    let interrupts = (0..4)
        .map(|_| f.enqueue(ClaudeCommand::Interrupt))
        .collect::<Vec<_>>();
    assert!(matches!(
        f.session.enqueue_claude_command(
            ClaudeCommand::Interrupt,
            Instant::now() + Duration::from_secs(1),
            None
        ),
        Err(ClaudeAdmissionError::Rejected(error)) if error.code() == "queue_full"
    ));
    let interrupt = &interrupts[0];
    let mut records = (0..4).map(|id| json!({"type":"control_request","request_id":format!("unknown-{id}"),"request":{"subtype":"unknown"}})).collect::<Vec<_>>();
    records.push(json!({"type":"control_response","response":{"subtype":"success","request_id":interrupt.request_id,"response":{}}}));
    f.session
        .rpc
        .write(&json!({"type":"fixture_batch","records":records}))
        .unwrap();
    std::thread::sleep(Duration::from_millis(50));
    assert!(f.session.poll().is_err());
    let receipts = f.session.take_claude_receipts();
    let receipt = receipts
        .iter()
        .find(|receipt| receipt.ticket == **interrupt)
        .unwrap();
    assert!(interrupts.iter().all(|ticket| {
        receipts.iter().any(|receipt| {
            receipt.ticket == **ticket
                && matches!(receipt.delivery, ClaudeDelivery::Indeterminate { .. })
        })
    }));
    assert_eq!(
        receipt.late_response.as_ref().unwrap()["subtype"],
        "success"
    );
    assert!(f.session.claude_cleanup_required());
    assert!(!f.session.process_ended);
    assert!(!f.session.snapshot().unwrap().actions.can_send);
    assert!(!f.requests().iter().any(|r| r["type"] == "control_response"));
}

#[test]
fn claude_port_successful_stop_ack_and_resume_keep_native_semantics() {
    let mut f = Fixture::new("ack");
    let resume = f.enqueue(ClaudeCommand::Resume);
    f.poll_until(|session| session.has_pending_requests() && !session.claude.commands.pending());
    let task = f.session.background_tasks().unwrap().tasks[0].id.clone();
    let stop = f.enqueue(ClaudeCommand::StopTask { task_id: task });
    f.poll_until(|session| !session.claude.commands.pending());
    let receipts = f.session.take_claude_receipts();
    assert!(
        receipts.iter().any(|receipt| receipt.ticket == *resume
            && matches!(receipt.delivery, ClaudeDelivery::Written))
    );
    assert!(receipts.iter().any(|receipt| receipt.ticket == *stop
        && matches!(receipt.delivery, ClaudeDelivery::Accepted { .. })));
    assert_eq!(
        f.session.background_tasks().unwrap().tasks[0].status,
        BackgroundTaskStatus::Stopping
    );
    assert!(!f.session.work().unwrap().is_empty());
    assert!(f.session.turn_running());
}

#[test]
fn claude_port_rejected_create_side_context_preserves_prepared_user_routing() {
    let mut f = Fixture::new("hold");
    let native_id = f.session.native_id.clone();
    let generation = f.session.claude.commands.generation;
    assert!(!f.session.side);
    assert!(matches!(
        f.session.claude.start,
        claude::ClaudeStart::Fresh { name: None }
    ));

    let error = f
        .session
        .create_side_context(Some(&f.root), "runtime", "rejected context")
        .expect_err("ordinary synchronous mutation must be rejected in port mode");
    assert_eq!(error.code(), "operation_pending");
    assert!(!f.session.side, "rejection must not enable side routing");
    assert_eq!(f.session.native_id, native_id);
    assert!(matches!(
        f.session.claude.start,
        claude::ClaudeStart::Fresh { name: None }
    ));
    assert_eq!(f.session.claude.commands.generation, generation);
    assert!(
        !f.root
            .join(format!("side-context-{}", f.session.epoch))
            .exists()
    );

    // Admission without another preparation proves the prepared state survived.
    let user = f.enqueue(ClaudeCommand::User(Fixture::input("ordinary user")));
    f.poll_until(|session| session.has_pending_requests() && !session.claude.commands.pending());
    let receipts = f.session.take_claude_receipts();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].ticket, *user);
    assert!(matches!(receipts[0].delivery, ClaudeDelivery::Written));
    let requests = f.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|record| record["type"] == "user")
            .count(),
        1
    );
    assert!(
        !requests
            .iter()
            .any(|record| record["request"]["subtype"] == "side_question")
    );
}

#[test]
fn claude_port_create_side_context_remains_available_in_blocking_preparation() {
    let mut f = Fixture::new("hold");
    let cwd = f.root.to_path_buf();
    f.session
        .with_claude_blocking_preparation(|session| {
            session.create_side_context(Some(&cwd), "runtime", "portable side context")?;
            Ok(())
        })
        .unwrap();
    assert!(f.session.side);
    let claude::ClaudeStart::SideContext { path } = &f.session.claude.start else {
        panic!("explicit preparation must preserve the real SideContext start mode");
    };
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "portable side context"
    );
    f.session
        .prepare_claude_commands(None, None, 64 * 1024 * 1024)
        .unwrap();
    let question = f.enqueue(ClaudeCommand::User(Fixture::input("side question")));
    f.poll_until(|session| !session.claude.commands.pending());
    let receipts = f.session.take_claude_receipts();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].ticket, *question);
    assert!(matches!(
        receipts[0].delivery,
        ClaudeDelivery::SideResult { .. }
    ));
    assert!(
        f.requests()
            .iter()
            .any(|record| record["request"]["subtype"] == "side_question")
    );
}

#[test]
fn claude_port_api_failure_reaches_typed_recovery_after_native_idle() {
    let mut f = Fixture::new("accept");
    f.user();
    f.permission();
    f.poll_until(|session| !session.has_pending_requests() && session.work().unwrap().is_empty());
    f.emit(
        json!({"type":"assistant","is_api_error_message":true,"error":"authentication_failed",
        "message":{"id":"failure","stop_reason":"stop_sequence",
            "content":[{"type":"text","text":"Login expired · Please run /login"}]}}),
    );
    f.emit(json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
    f.poll_until(|session| !session.turn_running());
    let snapshot = f.session.snapshot.as_ref().unwrap();
    assert_eq!(snapshot.run_state, RunState::Failed);
    assert!(snapshot.messages.iter().flat_map(|message| &message.blocks).any(|block| matches!(block,
        MessageBlock::RuntimeFailure {failure: RuntimeFailureKind::Other, detail, retry: None}
            if detail == "Login expired · Please run /login\nClaude Code 错误类型：authentication_failed")));
    assert_eq!(
        snapshot.executions.records.last().unwrap().outcome,
        Some(TurnOutcome::Failed)
    );
}
