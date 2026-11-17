use super::*;
use std::time::{Duration, Instant};

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
   emit(frame(r['id'],old,'/synthetic/deferred-other-session.jsonl'))
  else:
   if mode=='stale': emit(frame(previous,[{'id':'stale:0','role':'user','content':'wrong branch'}],'/synthetic/stale.jsonl'))
   emit(frame(r['id'],final,'/synthetic/new-session.jsonl' if mode=='late' else '/synthetic/session.jsonl'))
   if mode=='stale-late': emit(frame('previous-query',[{'id':'stale:0','role':'user','content':'stale body'}],'/synthetic/stale-late.jsonl'))
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

#[test]
fn snapshot_installs_at_frame_before_late_append_and_delayed_receipt() {
    let (mut session, root) = fixture("late");
    session.sync_history("pi", None).unwrap();
    session.poll();
    assert_eq!(ids(&session), ["native:u:0", "native:a:0", "late"]);
    assert_eq!(
        session.snapshot().unwrap().conversation.id,
        "pi:/synthetic/new-session.jsonl"
    );
    finish(session, root);
}
#[test]
fn run_start_before_frame_defers_replace_and_preserves_live_only_message_until_resync() {
    let (mut session, root) = fixture("busy");
    session.sync_history("pi", None).unwrap();
    session.poll();
    assert!(session.turn_running());
    assert_eq!(
        session.snapshot().unwrap().conversation.id,
        "pi:/synthetic/session.jsonl"
    );
    assert!(ids(&session).contains(&"pi-live:active".into()));
    assert!(ids(&session).contains(&"native:u:0".into()));
    session.cancel().unwrap();
    until(&mut session, |s| !s.turn_running());
    session.sync_history("pi", None).unwrap();
    session.poll();
    assert!(ids(&session).contains(&"native:u:0".into()));
    assert!(ids(&session).contains(&"native:a:0".into()));
    finish(session, root);
}
#[test]
fn rejected_query_frame_cannot_satisfy_a_new_query_or_change_its_session_header() {
    let (mut session, root) = fixture("stale");
    assert!(session.sync_history("pi", None).is_err());
    session.poll();
    assert!(ids(&session).contains(&"native:u:0".into()));
    session.sync_history("pi", None).unwrap();
    session.poll();
    assert!(ids(&session).contains(&"native:a:0".into()));
    assert!(!ids(&session).contains(&"stale:0".into()));
    assert_eq!(
        session.snapshot().unwrap().conversation.id,
        "pi:/synthetic/session.jsonl"
    );
    finish(session, root);
}
#[test]
fn malformed_snapshot_keeps_old_history_and_session_header() {
    let (mut session, root) = fixture("invalid");
    assert!(session.sync_history("pi", None).is_err());
    session.poll();
    assert!(ids(&session).contains(&"native:u:0".into()));
    assert!(!ids(&session).contains(&"native:a:0".into()));
    assert_eq!(
        session.snapshot().unwrap().conversation.id,
        "pi:/synthetic/session.jsonl"
    );
    finish(session, root);
}

#[test]
fn obsolete_query_frame_after_matching_snapshot_cannot_retarget_installed_body() {
    let (mut session, root) = fixture("stale-late");
    session.sync_history("pi", None).unwrap();
    session.poll();
    assert_eq!(
        session.snapshot().unwrap().conversation.id,
        "pi:/synthetic/session.jsonl"
    );
    assert_eq!(ids(&session), ["native:u:0", "native:a:0"]);
    finish(session, root);
}
