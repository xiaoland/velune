use super::*;
use std::time::{Duration, Instant};

struct Fixture {
    session: PiSession,
    root: std::path::PathBuf,
}
impl Fixture {
    fn new(script: &str) -> Self {
        let mut random = [0; 8];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-pi-owner-port-{:x}",
            u64::from_le_bytes(random)
        ));
        std::fs::create_dir(&root).unwrap();
        let helper = root.join("rpc.py");
        std::fs::write(&helper, script).unwrap();
        let config: Config = serde_json::from_value(json!({"execution":{"kind":"local"},"os_home":root,"binary":"/usr/bin/python3","node_binary":"/usr/bin/python3","rpc_entry":helper})).unwrap();
        let client = Client::spawn(config.clone()).unwrap();
        let mut session = PiSession::new(config);
        session.start(
            client,
            PiProjection::new(ConversationSummary {
                native_read_state: None,
                id: "pi:synthetic".into(),
                runtime_id: "pi".into(),
                title: crate::conversation::ConversationTitle::Untitled,
                cwd: None,
                created_at_unix_ms: None,
                updated_at_unix_ms: None,
                can_rename: false,
                can_delete: false,
                parent_conversation_id: None,
            }),
        );
        Self { session, root }
    }
    fn until(&mut self, predicate: impl Fn(&PiSession) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let start = Instant::now();
            self.session.poll();
            assert!(
                start.elapsed() < Duration::from_millis(200),
                "owner poll must not wait for writes or ACKs"
            );
            if predicate(&self.session) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "expected Pi state did not arrive"
            );
            thread_sleep();
        }
    }
}
fn thread_sleep() {
    std::thread::sleep(Duration::from_millis(5));
}
fn input(text: String) -> TurnInput {
    TurnInput {
        text,
        effort: None,
        permission_mode: None,
        images: vec![],
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.session.shutdown().unwrap();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn pi_session_blocked_stdin_keeps_poll_dialog_reply_and_cancel_nonblocking_without_replaying_input()
{
    let mut fixture = Fixture::new(
        r#"
import json,sys,os,time
root=os.path.dirname(__file__)
session_file=os.path.join(root,'session.jsonl')
def emit(v): print(json.dumps(v),flush=True)
emit({'type':'extension_ui_request','id':'dialog','method':'editor','title':'Edit'})
while not os.path.exists(os.path.join(root,'release')): time.sleep(.005)
pending=None
messages=[]
for line in sys.stdin:
 r=json.loads(line); c=r['type']; message=r.get('message',''); data={}
 with open(os.path.join(root,'requests'),'a') as f: f.write(json.dumps(r)+'\n')
 if c=='get_commands': data={'commands':[{'name':'velune-background-work'}]}
 elif c=='prompt' and message.startswith('/velune-projection-sync '):
  token=message.split(' ',1)[1]
  assert token==r['id']
  frame={'requestId':token,'snapshotVersion':1,'messages':messages,
         'messageIds':[m['id'] for m in messages],'identityConfirmations':[],
         'sessionFile':session_file,'cwd':root,'name':'Blocked input','createdUnixMs':1}
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.session-metadata','statusText':json.dumps(frame)})
  data={'disposition':'handled'}
 elif c=='prompt' and message.startswith('/velune-background-work '):
  frame={'version':1,'kind':'snapshot','parentSessionFile':session_file,'tasks':[]}
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.background-work-v1','statusText':json.dumps(frame)})
  data={'disposition':'handled'}
 elif c=='prompt':
  pending=r
  messages.append({'id':'user-once','role':'user','content':[{'type':'text','text':message}]})
  continue
 elif c=='extension_ui_response':
  if pending:
   emit({'type':'response','command':'prompt','id':pending['id'],'success':True,'data':{'disposition':'handled'}})
   pending=None
  continue
 if c=='abort': emit({'type':'agent_settled','aborted':True})
 emit({'type':'response','command':c,'id':r['id'],'success':True,'data':data})
"#,
    );
    fixture
        .session
        .send(&input("x".repeat(4 * 1024 * 1024)))
        .unwrap();
    fixture.until(|session| !session.snapshot().unwrap().pending_interactions.is_empty());
    // The synthetic peer demonstrably does not read stdin before release.
    for _ in 0..6 {
        let start = Instant::now();
        fixture.session.poll();
        assert!(start.elapsed() < Duration::from_millis(200));
        thread_sleep();
    }
    let start = Instant::now();
    fixture
        .session
        .reply(
            "dialog",
            crate::conversation::RuntimeInteractionReply::Cancel,
        )
        .unwrap();
    fixture
        .session
        .steer(&input("unsent steering".into()))
        .unwrap();
    fixture.session.cancel().unwrap();
    assert!(start.elapsed() < Duration::from_millis(200));
    assert!(!fixture.root.join("requests").exists());
    std::fs::write(fixture.root.join("release"), b"release").unwrap();
    fixture.until(|session| !session.turn_running() && !session.has_pending_metadata());
    let requests: Vec<Value> = std::fs::read_to_string(fixture.root.join("requests"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    // Bridge queries use prompt on the wire, but are not user input. Retain
    // every decoded command and classify it instead of dropping extra prompts.
    let user_prompts: Vec<_> = requests
        .iter()
        .filter(|request| {
            let message = request["message"].as_str().unwrap_or_default();
            request["type"] == "prompt"
                && !message.starts_with("/velune-projection-sync ")
                && !message.starts_with("/velune-background-work ")
        })
        .collect();
    assert_eq!(user_prompts.len(), 1);
    assert_eq!(user_prompts[0]["message"], "x".repeat(4 * 1024 * 1024));
    assert!(requests.iter().any(|request| {
        request["type"] == "prompt"
            && request["message"]
                .as_str()
                .unwrap()
                .starts_with("/velune-background-work ")
    }));
    for request in &requests {
        if request["type"] == "prompt"
            && let Some(token) = request["message"]
                .as_str()
                .unwrap()
                .strip_prefix("/velune-projection-sync ")
        {
            assert_eq!(Some(token), request["id"].as_str());
        }
    }
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["type"] == "steer")
            .count(),
        0
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["type"] == "extension_ui_response")
            .count(),
        1
    );
    let position = |kind| {
        requests
            .iter()
            .position(|request| request["type"] == kind)
            .unwrap()
    };
    assert!(position("extension_ui_response") < position("abort"));
    assert!(position("clear_queue") < position("abort"));
}

#[test]
fn pi_session_thinking_ack_dependency_does_not_hide_early_dialog_or_agent_start() {
    let mut fixture = Fixture::new(
        r#"
import json,sys,os
pending=None
def emit(v): print(json.dumps(v),flush=True)
def response(r): emit({'type':'response','id':r['id'],'command':r['type'],'success':True,'data':{'disposition':'handled','commands':[]}})
for line in sys.stdin:
 r=json.loads(line); c=r['type']
 with open(os.path.join(os.path.dirname(__file__),'requests'),'a') as f: f.write(c+'\n')
 if c=='set_thinking_level':
  pending=r
  emit({'type':'agent_start'})
  emit({'type':'extension_ui_request','id':'thinking-dialog','method':'editor','title':'Before ACK'})
  continue
 if c=='extension_ui_response': response(pending); continue
 response(r)
 if c=='prompt': emit({'type':'agent_settled'})
"#,
    );
    let mut turn = input("once".into());
    turn.effort = Some("high".into());
    fixture.session.send(&turn).unwrap();
    fixture.until(|session| !session.snapshot().unwrap().pending_interactions.is_empty());
    assert!(fixture.session.turn_running());
    assert!(
        !std::fs::read_to_string(fixture.root.join("requests"))
            .unwrap()
            .lines()
            .any(|line| line == "prompt")
    );
    fixture
        .session
        .reply(
            "thinking-dialog",
            crate::conversation::RuntimeInteractionReply::Cancel,
        )
        .unwrap();
    fixture.until(|session| !session.turn_running());
    let requests = std::fs::read_to_string(fixture.root.join("requests")).unwrap();
    assert_eq!(requests.lines().filter(|line| *line == "prompt").count(), 1);
    assert!(requests.find("extension_ui_response").unwrap() < requests.find("prompt").unwrap());
}

#[test]
fn pi_session_get_messages_receipt_is_ordered_history_fence_without_loss_or_replay() {
    let mut fixture = Fixture::new(
        r#"
import json,sys,os,time
root=os.path.dirname(__file__)
queries=0
def emit(v): print(json.dumps(v),flush=True)
def wait_for(name):
 while not os.path.exists(os.path.join(root,name)): time.sleep(.005)
old={'id':'m1','role':'user','content':[{'type':'text','text':'before history fence'}]}
new={'id':'m2','role':'user','content':[{'type':'text','text':'after history fence'}]}
for line in sys.stdin:
 r=json.loads(line); c=r['type']; message=r.get('message',''); data={'commands':[]}
 with open(os.path.join(root,'requests'),'a') as f: f.write(json.dumps(r)+'\n')
 if c=='prompt' and message.startswith('/velune-projection-sync '):
  queries+=1
  token=message.split(' ',1)[1]
  assert token==r['id']
  if queries==1:
   emit({'type':'agent_start'})
   emit({'type':'message_end','message':old})
  frame={'requestId':token,'snapshotVersion':1,'messages':[old],'messageIds':['m1'],
         'identityConfirmations':[],'sessionFile':os.path.join(root,'session.jsonl'),
         'cwd':root,'name':'History fence','createdUnixMs':1}
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.session-metadata','statusText':json.dumps(frame)})
  if queries==2: wait_for('receipt')
  data={'disposition':'handled'}
 elif c=='prompt' and message.startswith('/velune-background-work '):
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.background-work-v1',
        'statusText':json.dumps({'version':1,'kind':'snapshot','tasks':[]})})
  data={'disposition':'handled'}
 emit({'type':'response','command':c,'id':r['id'],'success':True,'data':data})
 if c=='prompt' and message.startswith('/velune-projection-sync '):
  if queries==1:
   wait_for('settle')
   emit({'type':'agent_settled'})
  else:
   emit({'type':'agent_start'})
   emit({'type':'message_end','message':new})
"#,
    );
    assert!(!fixture.session.sync_history("pi", None).unwrap());
    fixture.until(|session| {
        !session.running.as_ref().unwrap().history_pending
            && session.snapshot().unwrap().messages.len() == 1
    });
    assert!(!fixture.session.running.as_ref().unwrap().history_completed);
    assert!(fixture.session.turn_running());
    assert!(!fixture.session.snapshot().unwrap().actions.can_send);
    assert!(
        !fixture.session.sync_history("pi", None).unwrap(),
        "an early agent_start defers history replacement without losing its live row"
    );
    std::fs::write(fixture.root.join("settle"), b"settle").unwrap();
    fixture.until(|session| !session.turn_running() && !session.has_pending_metadata());
    assert!(!fixture.session.sync_history("pi", None).unwrap());
    fixture.until(|session| {
        session
            .running
            .as_ref()
            .unwrap()
            .pending
            .values()
            .any(|action| {
                matches!(action, PendingAction::HistoryMetadata(request)
                if matches!(request.snapshot, HistorySnapshot::Installed))
            })
    });
    assert!(fixture.session.running.as_ref().unwrap().history_pending);
    assert!(
        !fixture.session.running.as_ref().unwrap().history_completed,
        "the atomic frame alone cannot complete the receipt fence"
    );
    assert_eq!(fixture.session.snapshot().unwrap().messages.len(), 1);
    std::fs::write(fixture.root.join("receipt"), b"receipt").unwrap();
    fixture.until(|session| {
        session.running.as_ref().unwrap().history_completed
            && session.snapshot().unwrap().messages.len() == 2
    });
    let snapshot = fixture.session.snapshot().unwrap();
    assert_eq!(
        snapshot
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m1", "m2"],
        "events before and after the atomic history fence each survive exactly once"
    );
    assert_eq!(
        snapshot.conversation.id,
        format!("pi:{}", fixture.root.join("session.jsonl").display())
    );
    assert_eq!(snapshot.conversation.cwd.as_deref(), fixture.root.to_str());
    assert_eq!(snapshot.conversation.title.display_text(), "History fence");
    assert_eq!(snapshot.conversation.created_at_unix_ms, Some(1));
    assert!(fixture.session.turn_running());
    assert!(!snapshot.actions.can_send);
    let requests: Vec<Value> = std::fs::read_to_string(fixture.root.join("requests"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let queries: Vec<_> = requests
        .iter()
        .filter(|request| {
            request["type"] == "prompt"
                && request["message"]
                    .as_str()
                    .unwrap()
                    .starts_with("/velune-projection-sync ")
        })
        .collect();
    assert_eq!(queries.len(), 2);
    for query in queries {
        assert_eq!(
            query["message"]
                .as_str()
                .unwrap()
                .strip_prefix("/velune-projection-sync "),
            query["id"].as_str()
        );
    }
    assert!(
        !requests
            .iter()
            .any(|request| { request["type"] == "get_state" || request["type"] == "get_messages" })
    );
}

#[test]
fn pi_session_parent_settlement_requeries_inflight_background_snapshot_and_retains_live_work() {
    let mut fixture = Fixture::new(
        r#"
import json,sys,time
snapshots=0
def emit(v): print(json.dumps(v),flush=True)
for line in sys.stdin:
 r=json.loads(line); c=r['type']; data={}
 if c=='get_commands': data={'commands':[{'name':'velune-background-work'}]}
 if c=='prompt':
  snapshots+=1
  if snapshots==1:
   emit({'type':'agent_start'}); emit({'type':'agent_settled'})
   time.sleep(.15)
  frame={'version':1,'kind':'snapshot','tasks':[] if snapshots==1 else [{'id':'survivor','status':'running'}]}
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.background-work-v1','statusText':json.dumps(frame)})
  data={'disposition':'handled'}
 emit({'type':'response','command':c,'id':r['id'],'success':True,'data':data})
"#,
    );
    fixture.until(|session| session.background_refresh_queued);
    assert!(fixture.session.has_pending_metadata());
    assert!(!fixture.session.turn_running());
    fixture.until(|session| !session.has_pending_metadata());
    assert_eq!(fixture.session.background_tasks().tasks[0].id, "survivor");
    assert!(
        fixture.session.work_status() == crate::work::WorkStatus::Active,
        "parent settlement never releases surviving background work"
    );
}

#[test]
fn pi_session_cancel_does_not_start_a_queued_compaction_after_abort_or_wait_for_its_result() {
    let mut fixture = Fixture::new(
        r#"
import json,sys,os
compact=None
def emit(v): print(json.dumps(v),flush=True)
for line in sys.stdin:
 r=json.loads(line); c=r['type']
 with open(os.path.join(os.path.dirname(__file__),'requests'),'a') as f: f.write(c+'\n')
 if c=='compact':
  compact=r
  emit({'type':'compaction_start'})
  continue
 if c=='abort' and compact:
  emit({'type':'compaction_end','aborted':True})
  emit({'type':'response','command':'compact','id':compact['id'],'success':False,'error':'aborted'})
  emit({'type':'agent_settled','aborted':True})
 emit({'type':'response','command':c,'id':r['id'],'success':True,'data':{'commands':[]}})
"#,
    );
    fixture.session.compact().unwrap();
    fixture.session.cancel().unwrap();
    fixture.until(|session| !session.turn_running());
    let requests = std::fs::read_to_string(fixture.root.join("requests")).unwrap();
    assert_eq!(
        requests.lines().filter(|line| *line == "compact").count(),
        1
    );
    assert!(requests.find("compact").unwrap() < requests.find("clear_queue").unwrap());
    assert!(requests.find("clear_queue").unwrap() < requests.find("abort").unwrap());
    assert!(fixture.session.snapshot().unwrap().actions.can_send);
}

#[test]
fn pi_background_run_end_keeps_the_pending_prompt_live() {
    let mut fixture = Fixture::new(
        r#"
import json, os, sys, time
root = os.path.dirname(__file__)
def emit(value): print(json.dumps(value), flush=True)
for line in sys.stdin:
 request = json.loads(line)
 command = request['type']
 if command == 'prompt':
  emit({'type':'agent_start'})
  emit({'type':'agent_settled'})
  deadline = time.monotonic() + 5
  while not os.path.exists(os.path.join(root, 'release-prompt')):
   if time.monotonic() >= deadline: sys.exit(2)
   time.sleep(.005)
  emit({'type':'agent_start'})
  emit({'type':'response','command':command,'id':request['id'],'success':True,'data':{}})
  emit({'type':'agent_settled'})
 else:
  emit({'type':'response','command':command,'id':request['id'],'success':True,'data':{'commands':[]}})
"#,
    );
    fixture.until(|session| !session.has_pending_metadata());
    fixture.session.send(&input("next input".into())).unwrap();
    fixture.until(|session| session.running.as_ref().unwrap().projection.turn_ended());
    // Unblock the worker before asserting so a failing test can shut down.
    let pending_kept_busy = fixture.session.turn_running();
    std::fs::write(fixture.root.join("release-prompt"), []).unwrap();
    assert!(
        pending_kept_busy,
        "the previous run must not release this input"
    );
    fixture.until(|session| !session.turn_running());
    assert!(
        fixture
            .session
            .running
            .as_ref()
            .unwrap()
            .pending_prompt
            .is_none()
    );
}

#[test]
fn pi_review_owner_reader_failure_keeps_late_handled_and_rejected_prompts_terminal() {
    for success in [true, false] {
        let mut fixture = Fixture::new(
            "import sys,json,os\nfor line in sys.stdin:\n r=json.loads(line)\n if r['type']=='prompt': open(os.path.join(os.path.dirname(__file__),'prompt-seen'),'w').close()\n",
        );
        fixture.session.send(&input("pending".into())).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !fixture.root.join("prompt-seen").exists() {
            fixture.session.poll();
            assert!(Instant::now() < deadline);
            thread_sleep();
        }
        let running = fixture.session.running.as_mut().unwrap();
        let id = running.pending_prompt.clone().unwrap();
        running.client.test_reader_failure_batch(vec![json!({"type":"response","command":"prompt","id":id,"success":success,"data":{"disposition":"handled"},"error":"late rejection"})]);
        let items = running.client.poll_pi();
        assert!(items.iter().any(|item| matches!(item, PiPortItem::Receipt(receipt) if matches!(receipt.phase, PiReceiptPhase::Outcome(_) | PiReceiptPhase::Rejected(_)))), "real late outcome evidence must survive");
        fixture.session.apply_port_items(items);
        let running = fixture.session.running.as_ref().unwrap();
        assert_eq!(
            running.projection.snapshot.as_ref().unwrap().run_state,
            RunState::Failed
        );
        assert!(
            !running
                .projection
                .snapshot
                .as_ref()
                .unwrap()
                .actions
                .can_send
        );
        assert!(!running.projection.can_sync_history());
        assert!(running.client.pi_cleanup_required());
        assert!(running.pending_prompt.is_none());
    }
}

#[test]
fn pi_review_scope_transition_fences_old_prompt_until_owner_confirmation() {
    for command in ["fork", "clone", "new_session"] {
        for buffered in [true, false] {
            let mut fixture = Fixture::new(
                r#"
import json,sys,os,select
root=os.path.dirname(__file__); pending=None; buffer=b''
def emit(v): print(json.dumps(v),flush=True)
while True:
 if pending and os.path.exists(os.path.join(root,'release')):
  emit({'type':'response','id':pending['id'],'command':pending['type'],'success':True,'data':{}})
  pending=None
  open(os.path.join(root,'response-sent'),'w').close()
 ready,_,_=select.select([sys.stdin],[],[],.005)
 if not ready: continue
 chunk=os.read(0,65536)
 if not chunk: break
 buffer+=chunk
 while b'\n' in buffer:
  line,buffer=buffer.split(b'\n',1)
  r=json.loads(line); c=r['type']
  with open(os.path.join(root,'requests'),'a') as f: f.write(json.dumps(r)+'\n')
  if c in ['fork','clone','new_session']:
   pending=r;open(os.path.join(root,'transition-seen'),'w').close();continue
  emit({'type':'response','id':r['id'],'command':c,'success':True,'data':{'commands':[],'disposition':'handled'}})
"#,
            );
            match command {
                "fork" => fixture.session.fork_at(Some("entry")).unwrap(),
                "clone" => fixture.session.fork_at(None).unwrap(),
                _ => fixture.session.new_session().unwrap(),
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while !fixture.root.join("transition-seen").exists() {
                fixture.session.poll();
                assert!(Instant::now() < deadline);
                thread_sleep();
            }
            assert!(matches!(
                fixture.session.compact(),
                Err(Error::ScopeTransitionBusy)
            ));
            assert!(
                !fixture.session.turn_running(),
                "NotAdmitted compact must not publish compacting state"
            );
            assert!(
                !fixture
                    .session
                    .running
                    .as_ref()
                    .unwrap()
                    .pending
                    .values()
                    .any(|action| matches!(action, PendingAction::Control)),
                "NotAdmitted compact must not retain a pending action"
            );
            fixture.session.cancel().unwrap();
            while !std::fs::read_to_string(fixture.root.join("requests"))
                .unwrap()
                .contains("abort")
            {
                fixture.session.poll();
                assert!(Instant::now() < deadline);
                thread_sleep();
            }
            // Harvest Written without releasing the native scope response.
            for _ in 0..3 {
                fixture.session.poll();
                thread_sleep();
            }
            if buffered {
                std::fs::write(fixture.root.join("release"), b"release").unwrap();
                while !fixture.root.join("response-sent").exists() {
                    assert!(Instant::now() < deadline);
                    thread_sleep();
                }
            }
            let running = fixture.session.running.as_mut().unwrap();
            let old = running
                .client
                .enqueue_pi(PiOperation::Prompt(input("old-leaf".into())), None)
                .unwrap();
            let mut not_sent = false;
            for _ in 0..3 {
                let items = fixture.session.running.as_mut().unwrap().client.poll_pi();
                not_sent |= items.iter().any(|item|matches!(item,PiPortItem::Receipt(receipt) if receipt.ticket==old && receipt.phase==PiReceiptPhase::NotSent));
                fixture.session.apply_port_items(items);
                thread_sleep();
            }
            if !buffered {
                std::fs::write(fixture.root.join("release"), b"release").unwrap();
            }
            while fixture
                .session
                .running
                .as_ref()
                .unwrap()
                .client
                .pi_session_generation()
                == Some(old.session_generation)
                || !not_sent
            {
                let items = fixture.session.running.as_mut().unwrap().client.poll_pi();
                not_sent |= items.iter().any(|item|matches!(item,PiPortItem::Receipt(receipt) if receipt.ticket==old && receipt.phase==PiReceiptPhase::NotSent));
                fixture.session.apply_port_items(items);
                assert!(
                    Instant::now() < deadline,
                    "scope response must settle without dispatching old input"
                );
                thread_sleep();
            }
            fixture.session.send(&input("new-leaf".into())).unwrap();
            fixture.until(|session| !session.turn_running());
            let requests = std::fs::read_to_string(fixture.root.join("requests")).unwrap();
            assert!(
                !requests.contains("old-leaf"),
                "old-scope prompt reached native leaf: {requests}"
            );
            assert!(requests.contains("new-leaf"));
            assert!(!requests.contains("compact"));
            assert!(requests.find("clear_queue").unwrap() < requests.find("abort").unwrap());
            fixture.session.compact().unwrap();
            fixture.until(|session| !session.turn_running());
            assert!(
                std::fs::read_to_string(fixture.root.join("requests"))
                    .unwrap()
                    .contains("compact")
            );
        }
    }
}

#[test]
fn pi_review_owner_install_rechecks_deadline_after_port_observed_native_outcome() {
    let mut fixture = Fixture::new(
        r#"
import sys,json
for line in sys.stdin:
 r=json.loads(line)
 print(json.dumps({'type':'response','command':r['type'],'id':r['id'],'success':True,'data':{'commands':[],'disposition':'handled'}}),flush=True)
"#,
    );
    fixture.until(|session| !session.has_pending_metadata());
    let deadline = Instant::now() + Duration::from_millis(150);
    let running = fixture.session.running.as_mut().unwrap();
    let ticket = running
        .client
        .enqueue_pi(
            PiOperation::Prompt(input("deadline".into())),
            Some(deadline),
        )
        .unwrap();
    running.pending.insert(ticket, PendingAction::Prompt);
    running.pending_prompt = Some(format!(
        "velune-pi-{}-{}",
        ticket.generation, ticket.sequence
    ));
    running.projection.begin_turn();
    running.busy = true;
    let mut items = Vec::new();
    loop {
        items.extend(fixture.session.running.as_mut().unwrap().client.poll_pi());
        if items.iter().any(|item|matches!(item,PiPortItem::Receipt(receipt) if receipt.ticket==ticket && matches!(receipt.phase,PiReceiptPhase::Outcome(_)))) { break; }
        assert!(Instant::now() < deadline);
        thread_sleep();
    }
    std::thread::sleep(
        deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(5),
    );
    fixture.session.apply_port_items(items);
    let running = fixture.session.running.as_ref().unwrap();
    assert_eq!(
        running.projection.snapshot.as_ref().unwrap().run_state,
        RunState::Failed
    );
    assert!(
        !running
            .projection
            .snapshot
            .as_ref()
            .unwrap()
            .actions
            .can_send
    );
    assert!(!running.projection.can_sync_history());
    assert!(running.client.pi_cleanup_required());
}

#[test]
fn pi_review_owner_rejected_scope_transition_preserves_original_leaf_and_prompt() {
    for command in ["fork", "clone", "new_session"] {
        let mut fixture = Fixture::new(
            r#"
import sys,json,os
for line in sys.stdin:
 r=json.loads(line);c=r['type']
 with open(os.path.join(os.path.dirname(__file__),'requests'),'a') as f:f.write(json.dumps(r)+'\n')
 print(json.dumps({'type':'response','command':c,'id':r['id'],'success':c not in ['fork','clone','new_session'],'error':'scope rejected','data':{'commands':[],'disposition':'handled'}}),flush=True)
"#,
        );
        match command {
            "fork" => fixture.session.fork_at(Some("entry")).unwrap(),
            "clone" => fixture.session.fork_at(None).unwrap(),
            _ => fixture.session.new_session().unwrap(),
        }
        let ticket = fixture
            .session
            .running
            .as_mut()
            .unwrap()
            .client
            .enqueue_pi(PiOperation::Prompt(input("original-leaf".into())), None)
            .unwrap();
        let requests_path = fixture.root.join("requests");
        fixture.until(|session| {
            !session
                .running
                .as_ref()
                .unwrap()
                .pending
                .values()
                .any(|action| matches!(action, PendingAction::Branch))
                && std::fs::read_to_string(&requests_path)
                    .unwrap_or_default()
                    .contains("original-leaf")
        });
        assert_eq!(
            fixture
                .session
                .running
                .as_ref()
                .unwrap()
                .client
                .pi_session_generation(),
            Some(ticket.session_generation)
        );
        assert!(
            !fixture
                .session
                .running
                .as_ref()
                .unwrap()
                .client
                .pi_cleanup_required()
        );
        fixture
            .session
            .send(&input("still-original".into()))
            .unwrap();
        fixture.until(|session| !session.turn_running());
        assert!(
            std::fs::read_to_string(requests_path)
                .unwrap()
                .contains("still-original")
        );
    }
}

fn old_scope_deadline_after_branch_confirmation(operation_event: bool) {
    let mut fixture = Fixture::new(
        r#"
import sys,json,os
old_query=None
def emit(value): print(json.dumps(value),flush=True)
def response(request,data):
 emit({'type':'response','command':request['type'],'id':request['id'],'success':True,'data':data})
for line in sys.stdin:
 request=json.loads(line); command=request['type']; message=request.get('message','')
 with open(os.path.join(os.path.dirname(__file__),'requests'),'a') as output:
  output.write(json.dumps(request)+'\n')
 if command=='get_state' or message.startswith('/velune-projection-sync '):
  old_query=request
  continue
 if command=='fork':
  response(request,{})
  response(old_query,{'disposition':'handled','sessionName':'old-query-title'})
  old_query=None
  continue
 if message.startswith('/velune-background-work '):
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.background-work-v1',
        'statusText':json.dumps({'version':1,'kind':'snapshot','tasks':[]})})
 response(request,{'commands':[],'disposition':'handled'})
"#,
    );
    fixture.until(|session| !session.has_pending_metadata());
    // Set D only after the peer is ready. The peer emits the branch response
    // before the old query response; both are actually collected before D.
    let deadline = Instant::now() + Duration::from_secs(1);
    let running = fixture.session.running.as_mut().unwrap();
    let old = running
        .client
        .enqueue_pi(
            if operation_event {
                PiOperation::ProjectionSync
            } else {
                PiOperation::GetState
            },
            Some(deadline),
        )
        .unwrap();
    running.pending.insert(old, PendingAction::RenameState);
    fixture.session.fork_at(Some("entry")).unwrap();
    let branch = *fixture
        .session
        .running
        .as_ref()
        .unwrap()
        .pending
        .iter()
        .find(|(_, action)| matches!(action, PendingAction::Branch))
        .unwrap()
        .0;
    let mut collected = Vec::new();
    loop {
        collected.extend(fixture.session.running.as_mut().unwrap().client.poll_pi());
        let has_old = collected.iter().any(|item| {
            matches!(item, PiPortItem::Receipt(receipt)
            if receipt.ticket == old && matches!(receipt.phase, PiReceiptPhase::Accepted(_)))
        });
        let has_branch = collected.iter().any(|item| {
            matches!(item, PiPortItem::Receipt(receipt)
            if receipt.ticket == branch && matches!(receipt.phase, PiReceiptPhase::Accepted(_)))
        });
        assert!(
            Instant::now() < deadline,
            "native-response-before-D precondition"
        );
        if has_old && has_branch {
            break;
        }
        thread_sleep();
    }
    let old_receipt = collected
        .iter()
        .find_map(|item| match item {
            PiPortItem::Receipt(receipt)
                if receipt.ticket == old
                    && matches!(receipt.phase, PiReceiptPhase::Accepted(_)) =>
            {
                Some(receipt.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(old_receipt.deadline, Some(deadline));
    assert!(
        matches!(&old_receipt.phase, PiReceiptPhase::Accepted(value) if value["data"]["sessionName"] == "old-query-title")
    );
    let branch_position = collected
        .iter()
        .position(|item| {
            matches!(item, PiPortItem::Receipt(receipt)
        if receipt.ticket == branch && matches!(receipt.phase, PiReceiptPhase::Accepted(_)))
        })
        .unwrap();
    let old_position = collected
        .iter()
        .position(|item| {
            matches!(item, PiPortItem::Receipt(receipt)
        if receipt.ticket == old && matches!(receipt.phase, PiReceiptPhase::Accepted(_)))
        })
        .unwrap();
    assert!(
        branch_position < old_position,
        "real wire facts must have branch-first order"
    );
    let (old_items, branch_items): (Vec<_>, Vec<_>) =
        collected.into_iter().partition(|item| match item {
            PiPortItem::Receipt(receipt) => receipt.ticket == old,
            PiPortItem::OperationEvent { ticket, .. } => *ticket == old,
            _ => false,
        });
    fixture.session.apply_port_items(branch_items);
    assert_eq!(
        fixture
            .session
            .running
            .as_ref()
            .unwrap()
            .client
            .pi_session_generation(),
        Some(old.session_generation + 1)
    );
    assert!(
        !fixture
            .session
            .running
            .as_ref()
            .unwrap()
            .client
            .pi_cleanup_required()
    );
    assert!(
        !fixture
            .session
            .running
            .as_ref()
            .unwrap()
            .pending
            .contains_key(&old)
    );
    fixture
        .session
        .send(&input("new-scope-work".into()))
        .unwrap();
    let admitted_at = Instant::now();
    let running = fixture.session.running.as_ref().unwrap();
    let new_ticket = *running
        .pending
        .iter()
        .find(|(_, action)| matches!(action, PendingAction::Prompt))
        .unwrap()
        .0;
    assert_eq!(new_ticket.session_generation, old.session_generation + 1);
    let revision = running.projection.snapshot.as_ref().unwrap().revision;
    let conversation = running
        .projection
        .snapshot
        .as_ref()
        .unwrap()
        .conversation
        .id
        .clone();
    // This is an absolute boundary wait, not a guessed delay for peer progress:
    // native facts are already in hand, and only old D is allowed to expire.
    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
    assert!(Instant::now() >= deadline);
    assert!(
        Instant::now().duration_since(admitted_at) < Duration::from_secs(2),
        "current queued work must still be inside its independent 3s write budget"
    );
    let install: Vec<_> = old_items
        .into_iter()
        .filter(|item| match item {
            PiPortItem::OperationEvent { .. } => operation_event,
            PiPortItem::Receipt(receipt) => {
                !operation_event && matches!(receipt.phase, PiReceiptPhase::Accepted(_))
            }
            _ => false,
        })
        .collect();
    assert_eq!(install.len(), 1);
    fixture.session.apply_port_items(install);
    let running = fixture.session.running.as_ref().unwrap();
    assert!(
        !running.client.pi_cleanup_required(),
        "an expired old-scope item must not poison the current owner"
    );
    assert!(running.projection.can_sync_history());
    assert_eq!(
        running.projection.snapshot.as_ref().unwrap().run_state,
        RunState::Running
    );
    assert_eq!(
        running.projection.snapshot.as_ref().unwrap().revision,
        revision,
        "old query outcome must not install into the new projection"
    );
    assert_eq!(
        running
            .projection
            .snapshot
            .as_ref()
            .unwrap()
            .conversation
            .id,
        conversation
    );
    assert!(running.pending.contains_key(&new_ticket));
    assert!(
        matches!(&old_receipt.phase, PiReceiptPhase::Accepted(value)
        if value["data"]["sessionName"] == "old-query-title"),
        "old native evidence remains observable"
    );
    if operation_event {
        fixture
            .session
            .apply_port_items(vec![PiPortItem::Receipt(old_receipt)]);
    }
    fixture.until(|session| !session.turn_running() && !session.has_pending_metadata());
    assert!(
        !fixture
            .session
            .running
            .as_ref()
            .unwrap()
            .client
            .pi_cleanup_required()
    );
    let requests = std::fs::read_to_string(fixture.root.join("requests")).unwrap();
    assert_eq!(
        requests
            .lines()
            .filter(
                |line| serde_json::from_str::<Value>(line).unwrap()["message"] == "new-scope-work"
            )
            .count(),
        1,
        "new-scope queued work must survive and complete exactly once"
    );
}

#[test]
fn pi_review_old_scope_receipt_deadline_does_not_poison_new_owner() {
    old_scope_deadline_after_branch_confirmation(false);
}

#[test]
fn pi_review_old_scope_operation_event_deadline_does_not_poison_new_owner() {
    old_scope_deadline_after_branch_confirmation(true);
}

#[test]
fn pi_integration_history_install_rechecks_receipt_deadline() {
    let mut fixture = Fixture::new("import sys\nfor line in sys.stdin: pass\n");
    let previous = fixture.session.snapshot().unwrap().conversation.id.clone();
    let request = HistoryRequest {
        runtime_id: "pi".into(),
        model_record_key: None,
        snapshot: HistorySnapshot::Awaiting,
        deadline: None,
    };
    assert!(
        fixture
            .session
            .install_history(
                request,
                json!({"data":{"messages":[]}}),
                Some(Instant::now() - Duration::from_millis(1)),
            )
            .is_err()
    );
    assert_eq!(
        fixture.session.snapshot().unwrap().conversation.id,
        previous
    );
    assert!(
        fixture
            .session
            .running
            .as_ref()
            .unwrap()
            .client
            .pi_cleanup_required()
    );
}
