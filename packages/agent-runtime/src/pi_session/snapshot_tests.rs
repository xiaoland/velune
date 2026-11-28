use super::*;
use std::time::{Duration, Instant};

fn snapshot_event(request: &str, body: Value, path: &str) -> Value {
    let ids = body
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].clone())
        .collect::<Vec<_>>();
    json!({"type":"extension_ui_request","method":"setStatus","statusKey":"velune.session-metadata",
        "statusText":json!({"requestId":request,"snapshotVersion":1,"messages":body,"messageIds":ids,
            "identityConfirmations":[],"sessionFile":path,"name":"snapshot","cwd":"/synthetic"}).to_string()})
}

fn fixture(mode: &str) -> (PiSession, std::path::PathBuf) {
    let mut random = [0; 8];
    getrandom::fill(&mut random).unwrap();
    let root = std::env::temp_dir().join(format!(
        "velune-history-frame-{:x}",
        u64::from_le_bytes(random)
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("mode"), mode).unwrap();
    let helper = root.join("rpc.py");
    std::fs::write(&helper, r#"
import json,sys,os,time
def emit(v): print(json.dumps(v),flush=True)
mode=open(os.path.join(os.path.dirname(__file__),'mode')).read()
queries=0; previous=None
old=[{'id':'native:u:0','role':'user','content':'old user'}]
final=old+[{'id':'native:a:0','role':'assistant','content':'final answer'}]
def frame(q,body,path='/synthetic/session.jsonl'):
 return {'type':'extension_ui_request','method':'setStatus','statusKey':'velune.session-metadata','statusText':json.dumps({'requestId':q,'snapshotVersion':1,'messages':body,'messageIds':[m['id'] for m in body],'identityConfirmations':[],'sessionFile':path,'name':'snapshot'})}
for line in sys.stdin:
 r=json.loads(line); command=r['type']; data={}
 if command=='get_commands': data={'commands':[]}
 elif command=='prompt' and r['message'].startswith('/velune-background-work '):
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.background-work-v1','statusText':json.dumps({'version':1,'kind':'unsupported','supported':False,'tasks':[]})})
  data={'disposition':'handled'}
 elif command=='prompt':
  assert r['message']=='/velune-projection-sync '+r['id']
  queries+=1
  if mode=='silent': continue
  if mode=='stale' and queries==1:
   previous=r['id'];emit({'type':'response','id':r['id'],'command':command,'success':False,'error':'synthetic rejection'});continue
  if mode=='invalid':
   invalid=frame(r['id'],final,'/synthetic/wrong.jsonl'); payload=json.loads(invalid['statusText']); payload['messageIds']=['wrong:0']; invalid['statusText']=json.dumps(payload); emit(invalid)
  elif mode=='busy' and queries==1:
   emit({'type':'agent_start'})
   emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.message-identity','statusText':'pi-live:active'})
   emit({'type':'message_start','message':{'role':'assistant','content':[{'type':'text','text':'in flight'}]}})
   emit(frame(r['id'],old))
  else:
   if mode=='stale': emit(frame(previous,[{'id':'stale:0','role':'user','content':'wrong branch'}],'/synthetic/stale.jsonl'))
   emit(frame(r['id'],final,'/synthetic/new-session.jsonl' if mode=='late' else '/synthetic/session.jsonl'))
   if mode=='late':
    emit({'type':'message_start','message':{'id':'late','role':'custom','customType':'background-task-notification','content':'late notice'}})
    emit({'type':'message_end','message':{'id':'late','role':'custom','customType':'background-task-notification','content':'late notice'}})
    emit(frame(r['id'],final,'/synthetic/new-session.jsonl')) # duplicate must not erase the late append
    time.sleep(.03)
  data={'disposition':'handled'}
 elif command=='abort':
  emit({'type':'message_end','message':{'role':'assistant','content':[{'type':'text','text':'in flight'}],'stopReason':'aborted'}})
  emit({'type':'agent_settled','aborted':True})
 elif command=='get_state': data={'sessionFile':'/synthetic/session.jsonl'}
 elif command=='get_messages': data={'messages':old}
 emit({'type':'response','id':r['id'],'command':command,'success':True,'data':data})
"#).unwrap();
    let config: Config=serde_json::from_value(json!({"execution":{"kind":"local"},"os_home":root,"binary":"/usr/bin/python3","node_binary":"/usr/bin/python3","rpc_entry":helper})).unwrap();
    let client = Client::spawn(config.clone()).unwrap();
    let mut projection = PiProjection::new(ConversationSummary {
        native_read_state: None,
        id: "pi:/synthetic/session.jsonl".into(),
        runtime_id: "pi".into(),
        title: crate::conversation::conversation_title(None, None),
        cwd: None,
        created_at_unix_ms: None,
        updated_at_unix_ms: None,
        can_rename: false,
        can_delete: false,
        parent_conversation_id: None,
    });
    projection.replace_history(
        &json!({"messages":[{"id":"native:u:0","role":"user","content":"old user"}]}),
    );
    projection.invalidate_history();
    let mut session = PiSession::new(config);
    session.start(client, projection);
    until(&mut session, |s| !s.has_pending_metadata());
    (session, root)
}
fn until(session: &mut PiSession, check: impl Fn(&PiSession) -> bool) {
    let end = Instant::now() + Duration::from_secs(4);
    while !check(session) && Instant::now() < end {
        session.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(check(session));
}
fn ids(session: &PiSession) -> Vec<String> {
    session
        .snapshot()
        .unwrap()
        .messages
        .iter()
        .map(|m| m.id.clone())
        .collect()
}
fn finish(mut session: PiSession, root: std::path::PathBuf) {
    let _ = session.shutdown();
    std::fs::remove_dir_all(root).unwrap();
}

fn edit_fixture(mode: &str) -> (PiSession, std::path::PathBuf) {
    let mut random = [0; 8];
    getrandom::fill(&mut random).unwrap();
    let root =
        std::env::temp_dir().join(format!("velune-edit-sync-{:x}", u64::from_le_bytes(random)));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("mode"), mode).unwrap();
    let helper = root.join("rpc.py");
    std::fs::write(&helper, r#"
import json,os,sys,time
root=os.path.dirname(__file__)
mode=open(os.path.join(root,'mode')).read()
branched=False
old=[{'id':'pi:session:prefix-user:0','role':'user','content':'kept user'},
     {'id':'pi:session:prefix-reply:0','role':'assistant','content':'kept reply','stopReason':'stop'},
     {'id':'pi:session:edited-user:0','role':'user','content':'old selected body'},
     {'id':'pi:session:future-reply:0','role':'assistant','content':'discarded future','stopReason':'stop'}]
after=[{'id':'pi:session:prefix-user:0','role':'user','content':'kept user'},
       {'id':'pi:session:prefix-reply:0','role':'assistant','content':'kept reply','stopReason':'stop'}]
def emit(value): print(json.dumps(value),flush=True)
def frame(request,body,invalid=False):
 ids=[m['id'] for m in body]
 if invalid and ids: ids[0]='foreign-message'
 return {'type':'extension_ui_request','method':'setStatus','statusKey':'velune.session-metadata',
         'statusText':json.dumps({'requestId':request['id'],'snapshotVersion':1,'messages':body,
           'messageIds':ids,'identityConfirmations':[],'sessionFile':'/synthetic/session.jsonl',
           'name':'synthetic','cwd':'/synthetic'})}
for line in sys.stdin:
 request=json.loads(line); command=request['type']; data={}
 with open(os.path.join(root,'requests.jsonl'),'a') as log: log.write(json.dumps(request)+'\n')
 if command=='get_commands': data={'commands':[]}
 elif command=='prompt' and request['message'].startswith('/velune-projection-sync '):
  if mode=='timeout': continue
  elif mode=='missing-after-branch' and branched: pass
  else: emit(frame(request,after if branched else old,mode=='invalid-after-branch' and branched))
  if mode!='timeout': data={'disposition':'handled'}
 elif command=='prompt' and request['message'].startswith('/velune-rewind '):
  branched=True
  entry=request['message'].split(' ',1)[1]
  if mode=='rewind-error': emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.rewind','statusText':json.dumps({'error':'synthetic rewind rejected'})})
  else: emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.rewind','statusText':json.dumps({'entryId':entry})})
  data={'disposition':'handled'}
 emit({'type':'response','id':request['id'],'command':command,'success':True,'data':data})
"#).unwrap();
    let config: Config = serde_json::from_value(json!({
        "execution":{"kind":"local"},"os_home":root,
        "binary":"/usr/bin/python3","node_binary":"/usr/bin/python3","rpc_entry":helper,
    }))
    .unwrap();
    let client = Client::spawn(config.clone()).unwrap();
    let mut projection = PiProjection::new(ConversationSummary {
        native_read_state: None,
        id: "pi:/synthetic/session.jsonl".into(),
        runtime_id: "pi".into(),
        title: crate::conversation::conversation_title(None, None),
        cwd: None,
        created_at_unix_ms: None,
        updated_at_unix_ms: None,
        can_rename: false,
        can_delete: false,
        parent_conversation_id: None,
    });
    projection.replace_history(&json!({"messages":[
        {"id":"pi:session:prefix-user:0","role":"user","content":"kept user"},
        {"id":"pi:session:prefix-reply:0","role":"assistant","content":"kept reply","stopReason":"stop"},
        {"id":"pi:session:edited-user:0","role":"user","content":"old selected body"},
        {"id":"pi:session:future-reply:0","role":"assistant","content":"discarded future","stopReason":"stop"}
    ]}));
    projection.invalidate_history();
    let mut session = PiSession::new(config);
    session.start(client, projection);
    (session, root)
}

#[test]
fn edit_history_barrier_waits_for_the_validated_projection_frame() {
    let (mut session, root) = edit_fixture("success");
    let start = Instant::now();
    assert!(!session.sync_history("pi", None).unwrap());
    assert!(start.elapsed() < Duration::from_millis(250));
    session.sync_history_for_edit("pi", None).unwrap();
    assert_eq!(
        ids(&session),
        [
            "pi:session:prefix-user:0",
            "pi:session:prefix-reply:0",
            "pi:session:edited-user:0",
            "pi:session:future-reply:0",
        ]
    );
    assert!(!session.running.as_ref().unwrap().history_pending);
    assert!(!session.running.as_ref().unwrap().history_completed);
    finish(session, root);
}

#[test]
fn edit_history_barrier_returns_typed_port_timeout_without_authorizing_continue() {
    let (mut session, root) = edit_fixture("timeout");
    let start = Instant::now();
    let error = session
        .sync_history_for_edit("pi", None)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("Pi prompt deadline expired; delivery/outcome indeterminate"),
        "{error}"
    );
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(!session.projection().unwrap().history_synchronized());
    finish(session, root);
}

#[test]
fn edit_branch_waits_for_native_rewind_and_new_generation_snapshot() {
    let (mut session, root) = edit_fixture("success");
    assert!(!session.sync_history("pi", None).unwrap());
    session.sync_history_for_edit("pi", None).unwrap();
    let previous_generation = session
        .running
        .as_ref()
        .unwrap()
        .client
        .pi_session_generation()
        .unwrap();

    session.branch_for_edit("edited-user").unwrap();
    session.sync_history_for_edit("pi", None).unwrap();

    assert_eq!(
        ids(&session),
        ["pi:session:prefix-user:0", "pi:session:prefix-reply:0"]
    );
    assert_ne!(
        session
            .running
            .as_ref()
            .unwrap()
            .client
            .pi_session_generation()
            .unwrap(),
        previous_generation,
        "rewind must advance native generation before its confirming snapshot"
    );
    let requests = std::fs::read_to_string(root.join("requests.jsonl")).unwrap();
    let messages = requests
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|request| request["message"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.starts_with("/velune-projection-sync "))
            .count(),
        2,
        "the post-rewind frame must be fetched after the original preparation frame"
    );
    assert!(
        messages
            .iter()
            .any(|message| message == "/velune-rewind edited-user")
    );
    finish(session, root);
}

#[test]
fn edit_branch_does_not_complete_on_rewind_error_or_invalid_post_branch_frame() {
    for mode in [
        "rewind-error",
        "invalid-after-branch",
        "missing-after-branch",
    ] {
        let (mut session, root) = edit_fixture(mode);
        assert!(!session.sync_history("pi", None).unwrap());
        session.sync_history_for_edit("pi", None).unwrap();
        let error = if mode == "rewind-error" {
            session
                .branch_for_edit("edited-user")
                .unwrap_err()
                .to_string()
        } else {
            session.branch_for_edit("edited-user").unwrap();
            session
                .sync_history_for_edit("pi", None)
                .unwrap_err()
                .to_string()
        };
        let expected = match mode {
            "rewind-error" => "synthetic rewind rejected",
            "invalid-after-branch" => "原生会话消息身份不属于同一快照或重复",
            "missing-after-branch" => "原生会话投影缺少同源快照",
            _ => unreachable!(),
        };
        assert!(error.contains(expected), "{mode}: {error}");
        assert_eq!(
            ids(&session),
            [
                "pi:session:prefix-user:0",
                "pi:session:prefix-reply:0",
                "pi:session:edited-user:0",
                "pi:session:future-reply:0",
            ],
            "{mode}: a failed edit barrier must preserve the last validated history"
        );
        let requests = std::fs::read_to_string(root.join("requests.jsonl")).unwrap();
        assert!(!requests.lines().any(|line| {
            serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|request| request["message"].as_str().map(str::to_owned))
                .is_some_and(|message| {
                    !message.starts_with("/velune-") && message != "/velune-rewind edited-user"
                })
        }));
        finish(session, root);
    }
}

#[test]
fn snapshot_installs_at_frame_before_late_append_and_delayed_receipt() {
    let (mut session, root) = fixture("late");
    session.sync_history("pi", None).unwrap();
    until(&mut session, |s| {
        !s.running.as_ref().unwrap().history_pending
    });
    assert_eq!(ids(&session), ["native:u:0", "native:a:0", "late"]);
    assert_eq!(
        session.snapshot().unwrap().conversation.id,
        "pi:/synthetic/new-session.jsonl"
    );
    assert!(session.running.as_ref().unwrap().history_completed);
    finish(session, root);
}
#[test]
fn run_start_before_frame_defers_replace_and_preserves_live_only_message_until_resync() {
    let (mut session, root) = fixture("busy");
    session.sync_history("pi", None).unwrap();
    until(&mut session, |s| {
        !s.running.as_ref().unwrap().history_pending
    });
    assert!(session.turn_running());
    assert!(ids(&session).contains(&"pi-live:active".into()));
    assert!(ids(&session).contains(&"native:u:0".into()));
    assert!(!session.running.as_ref().unwrap().history_completed);
    session.cancel().unwrap();
    until(&mut session, |s| !s.turn_running());
    session.sync_history("pi", None).unwrap();
    until(&mut session, |s| {
        !s.running.as_ref().unwrap().history_pending
    });
    assert!(
        session.running.as_ref().unwrap().history_completed,
        "deferred history must be retried after the active run settles"
    );
    assert!(ids(&session).contains(&"native:u:0".into()));
    assert!(ids(&session).contains(&"native:a:0".into()));
    finish(session, root);
}
#[test]
fn rejected_query_frame_cannot_satisfy_a_new_query_or_change_its_session_header() {
    let (mut session, root) = fixture("stale");
    session.sync_history("pi", None).unwrap();
    until(&mut session, |s| {
        !s.running.as_ref().unwrap().history_pending
    });
    assert!(ids(&session).contains(&"native:u:0".into()));
    session.sync_history("pi", None).unwrap();
    until(&mut session, |s| {
        !s.running.as_ref().unwrap().history_pending
    });
    assert!(ids(&session).contains(&"native:a:0".into()));
    assert!(!ids(&session).contains(&"stale:0".into()));
    assert_eq!(
        session.snapshot().unwrap().conversation.id,
        "pi:/synthetic/session.jsonl"
    );
    finish(session, root);
}
#[test]
fn expired_install_deadline_rejects_snapshot_without_replacing_old_body_or_header() {
    let (mut session, root) = fixture("silent");
    session.sync_history("pi", None).unwrap();
    until(&mut session, |s| {
        s.running
            .as_ref()
            .unwrap()
            .pending
            .values()
            .any(|a| matches!(a,PendingAction::HistoryMetadata(r) if r.deadline.is_some()))
    });
    let running = session.running.as_mut().unwrap();
    let (&ticket, request) = running
        .pending
        .iter_mut()
        .find_map(|(t, a)| match a {
            PendingAction::HistoryMetadata(r) => Some((t, r)),
            _ => None,
        })
        .unwrap();
    request.deadline = Some(Instant::now() - Duration::from_millis(1));
    let request_id = format!("velune-pi-{}-{}", ticket.generation, ticket.sequence);
    session.apply_events(vec![snapshot_event(
        &request_id,
        json!([{"id":"wrong:0","role":"user","content":"must not install"}]),
        "/synthetic/wrong.jsonl",
    )]);
    assert!(ids(&session).contains(&"native:u:0".into()));
    assert!(!ids(&session).contains(&"wrong:0".into()));
    assert_eq!(
        session.snapshot().unwrap().conversation.id,
        "pi:/synthetic/session.jsonl"
    );
    assert!(
        session
            .running
            .as_ref()
            .unwrap()
            .client
            .pi_cleanup_required()
    );
    finish(session, root);
}

#[test]
fn malformed_snapshot_keeps_old_history_and_session_header() {
    let (mut session, root) = fixture("invalid");
    session.sync_history("pi", None).unwrap();
    until(&mut session, |s| {
        !s.running.as_ref().unwrap().history_pending
    });
    assert!(ids(&session).contains(&"native:u:0".into()));
    assert!(!ids(&session).contains(&"native:a:0".into()));
    assert_eq!(
        session.snapshot().unwrap().conversation.id,
        "pi:/synthetic/session.jsonl"
    );
    assert!(!session.running.as_ref().unwrap().history_completed);
    finish(session, root);
}
