use super::*;
use crate::LiveConversation;

fn poll_until(session: &mut PiSession, predicate: impl Fn(&PiSession) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        session.poll();
        if predicate(session) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "async Pi owner did not reach expected state"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn pi_subagent_live_metadata_keeps_terminal_identities_separate_from_active_work() {
    let mut random = [0; 8];
    getrandom::fill(&mut random).unwrap();
    let root = std::env::temp_dir().join(format!(
        "velune-child-descriptors-{:x}",
        u64::from_le_bytes(random)
    ));
    std::fs::create_dir(&root).unwrap();
    let helper = root.join("rpc.py");
    std::fs::write(&helper, r#"import json,sys
for line in sys.stdin:
 r=json.loads(line)
 data={'commands':[]} if r['type']=='get_commands' else {'messages':[]}
 print(json.dumps({'type':'response','id':r['id'],'command':r['type'],'success':True,'data':data}),flush=True)
"#).unwrap();
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
    session.config.session = Some("/synthetic/parent-a.jsonl".into());
    session.apply_background_frame(&json!({"version":1,"kind":"snapshot","parentSessionFile":"/synthetic/parent-b.jsonl",
            "tasks":[{"id":"foreign-task","status":"running"}],
            "children":[{"sessionFile":"/synthetic/foreign-child.jsonl","title":"foreign child","canStop":true}],
            "subAgents":[{"sessionFile":"/synthetic/foreign-child.jsonl","title":"foreign descriptor"}],
            "taskOperation":{"error":"foreign operation"},
            "persistedNotificationIds":["foreign-notification"]
        }));
    assert!(session.snapshot().unwrap().sub_agents.is_empty());
    assert!(session.work().is_empty());
    assert!(session.children.is_empty());
    assert!(session.task_items.is_empty());
    assert!(session.task_issue.is_none());
    assert!(
        !session
            .settled_notifications
            .contains("foreign-notification")
    );
    session.apply_background_frame(&json!({"version":1,"kind":"snapshot","parentSessionFile":"/synthetic/parent-a.jsonl","tasks":[
            {"id":"shell","status":"running","kind":"shell","description":"Ordinary shell","childSessionFile":"/synthetic/not-native.jsonl"}
        ],"subAgents":[{"sessionFile":"/synthetic/child.jsonl","title":"worker：缓存检查","failed":false}],
        "children":[{"sessionFile":"/synthetic/child.jsonl","title":"fallback","state":"running","canStop":true}]}));
    let agents = &session.snapshot().unwrap().sub_agents;
    assert_eq!(
        agents.len(),
        1,
        "ordinary background tasks do not create native identities"
    );
    assert_eq!(agents[0].title, "worker：缓存检查");
    assert!(
        session
            .work()
            .iter()
            .any(|task| task.conversation_id.as_deref() == Some("pi:/synthetic/child.jsonl"))
    );
    session.apply_background_frame(&json!({"version":1,"kind":"snapshot","tasks":[],"children":[],"subAgents":[{"sessionFile":"/synthetic/child.jsonl","title":"worker：缓存检查","failed":true}]}));
    assert!(session.work().is_empty());
    assert!(session.snapshot().unwrap().sub_agents[0].failed);
    session.apply_background_frame(
        &json!({"version":1,"kind":"unknown","childrenUnknown":true,"children":[]}),
    );
    assert_eq!(session.snapshot().unwrap().sub_agents.len(), 1);
    assert!(session.has_pending_metadata());
    // The SDK owner is newer than the launch configuration. Follow it in
    // both directions rather than accepting frames for the stale launch A.
    for (owner, rejected) in [("b", "a"), ("a", "b")] {
        let parent = format!("/synthetic/parent-{owner}.jsonl");
        session.apply_events(vec![json!({"type":"extension_ui_request","method":"setStatus",
                "statusKey":"velune.session-metadata","statusText":json!({"sessionFile":parent}).to_string()
            })]);
        assert!(
            session.snapshot().unwrap().sub_agents.is_empty(),
            "switching the SDK owner clears the previous parent's descriptors"
        );
        session.apply_background_frame(&json!({"version":1,"kind":"snapshot",
                "parentSessionFile":format!("/synthetic/parent-{rejected}.jsonl"),
                "tasks":[],"children":[],"subAgents":[{"sessionFile":"/synthetic/stale.jsonl","title":"stale parent"}]
            }));
        assert!(session.snapshot().unwrap().sub_agents.is_empty());
        session.apply_background_frame(&json!({"version":1,"kind":"snapshot","parentSessionFile":parent,
                "tasks":[],"children":[],"subAgents":[{"sessionFile":format!("/synthetic/child-{owner}.jsonl"),"title":format!("owner {owner}")}]
            }));
        assert_eq!(session.snapshot().unwrap().sub_agents.len(), 1);
        assert_eq!(
            session.snapshot().unwrap().sub_agents[0].title,
            format!("owner {owner}")
        );
    }
    session.shutdown().unwrap();
    drop(session);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn background_tasks_async_output_and_exact_stop_do_not_block_or_interrupt_parent() {
    let mut random = [0; 8];
    getrandom::fill(&mut random).unwrap();
    let root = std::env::temp_dir().join(format!(
        "velune-task-output-{:x}",
        u64::from_le_bytes(random)
    ));
    std::fs::create_dir(&root).unwrap();
    let helper = root.join("rpc.py");
    std::fs::write(&helper, r#"
import json,sys,time,base64,os
def emit(value): print(json.dumps(value),flush=True)
tasks=[{'id':'a','kind':'shell','description':'Shell A','status':'running','canStop':True,'canReadOutput':True},{'id':'b','kind':'shell','description':'Shell B','status':'running','canStop':True,'canReadOutput':True}]
for line in sys.stdin:
 r=json.loads(line); command=r['type']; data={}
 with open(os.path.join(os.path.dirname(__file__),'requests.jsonl'),'a') as f: f.write(json.dumps(r)+'\n')
 if command=='get_commands': data={'commands':[{'name':'velune-background-work'}]}
 elif command=='get_messages': data={'messages':[]}
 elif command=='prompt' and r['message'].startswith('/velune-projection-sync '):
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.session-metadata','statusText':json.dumps({'requestId':r['id'],'snapshotVersion':1,'messages':[],'messageIds':[],'identityConfirmations':[]})})
  data={'disposition':'handled'}
 elif command=='prompt':
  encoded=r['message'].split(' ',1)[1]; request=json.loads(base64.urlsafe_b64decode(encoded+'='*((-len(encoded))%4)))
  frame={'version':1,'kind':'snapshot','tasks':tasks}
  if request['operation']=='task_output':
   time.sleep(0.5)
   frame['taskOutput']={'taskId':request['taskId'],'requestId':request['requestId'],'text':'Shell A output','truncated':False}
  elif request['operation']=='stop_task':
   for task in tasks:
    if task['id']==request['taskId']: task['status']='killed'; task['canStop']=False
   frame['tasks']=tasks; frame['taskOperation']={'taskId':request['taskId']}
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.background-work-v1','statusText':json.dumps(frame)})
  data={'disposition':'handled'}
 emit({'type':'response','id':r['id'],'command':command,'success':True,'data':data})
"#).unwrap();
    let config: Config = serde_json::from_value(json!({"execution":{"kind":"local"},"os_home":root,"binary":"/usr/bin/python3","node_binary":"/usr/bin/python3","rpc_entry":helper})).unwrap();
    let mut client = Client::spawn(config.clone()).unwrap();
    // Production execution performs the patient readiness snapshot before
    // its first ordinary 3s metadata command. Keep the fixture faithful.
    client.state().unwrap();
    let mut session = PiSession::new(config);
    session.start(
        client,
        PiProjection::new(ConversationSummary {
            native_read_state: None,
            id: "pi:synthetic".into(),
            runtime_id: "pi".into(),
            title: crate::conversation::conversation_title(None, None),
            cwd: None,
            created_at_unix_ms: None,
            updated_at_unix_ms: None,
            can_rename: false,
            can_delete: false,
            parent_conversation_id: None,
        }),
    );
    poll_until(&mut session, |session| {
        !session.background_tasks().tasks.is_empty()
    });
    session.apply_events(vec![json!({"type":"agent_start"})]);
    session.apply_background_frame(&json!({"version":1,"kind":"unknown","error":"临时查询失败"}));
    assert!(!session.background_tasks().live);
    session.refresh_background_tasks();
    poll_until(&mut session, |session| session.background_tasks().live);
    assert!(session.background_tasks().issue.is_none());
    assert!(
        session
            .background_tasks()
            .tasks
            .iter()
            .any(|task| task.id == "a" && task.can_read_output),
        "initial task frame failed: {:?}; metadata={:?}",
        session.background_error,
        session.background_tasks()
    );
    let start = std::time::Instant::now();
    let output = session.read_background_task_output("a", None).unwrap();
    assert!(
        start.elapsed() < std::time::Duration::from_millis(250),
        "output request must not wait for the host"
    );
    assert_eq!(
        output.status,
        crate::conversation::BackgroundTaskOutputStatus::Checking
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        session.poll();
        if session
            .read_background_task_output("a", Some(&output.request_id))
            .unwrap()
            .status
            == crate::conversation::BackgroundTaskOutputStatus::Ready
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        session
            .read_background_task_output("a", Some(&output.request_id))
            .unwrap()
            .text,
        "Shell A output"
    );
    assert!(session.turn_running());
    session.stop_background_task("a").unwrap();
    while session
        .background_tasks()
        .tasks
        .iter()
        .any(|task| task.id == "a" && task.status != BackgroundTaskStatus::Cancelled)
    {
        session.poll();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(session.turn_running());
    assert_eq!(
        session
            .background_tasks()
            .tasks
            .iter()
            .find(|task| task.id == "b")
            .unwrap()
            .status,
        BackgroundTaskStatus::Running
    );
    let requests = std::fs::read_to_string(root.join("requests.jsonl")).unwrap();
    assert!(
        !requests
            .lines()
            .any(|line| serde_json::from_str::<Value>(line).unwrap()["type"] == "abort")
    );
    session.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn background_startup_frame_leaves_host_observation_pending() {
    let config: Config = serde_json::from_value(json!({
        "execution":{"kind":"local"}, "binary":"/bin/pi", "node_binary":"/bin/node",
    }))
    .unwrap();
    let mut session = PiSession::new(config);
    session.reset_background_session();
    let startup = json!({"version":1,"kind":"uninitialized","tasks":[]});
    session.apply_background_frame(&startup);
    assert!(session.has_pending_metadata());
    assert_eq!(session.background_error, None);

    session.apply_background_frame(&json!({"version":1,"kind":"unsupported","tasks":[]}));
    session.apply_background_frame(&startup);
    assert!(!session.has_pending_metadata());
    assert_eq!(session.background_error, None);
}

#[test]
fn background_snapshot_keeps_running_work_and_notification_handoff_separate() {
    let mut config: Config = serde_json::from_value(json!({
        "execution":{"kind":"local"}, "binary":"/bin/pi", "node_binary":"/bin/node",
    }))
    .unwrap();
    config.working_dir = None;
    let mut session = PiSession::new(config);
    session.apply_background_frame(&json!({
        "version":1,
        "kind":"snapshot",
        "tasks":[
            {"id":"running-1","status":"running"},
            {"id":"done-1","status":"completed","notifyOnCompletion":true,"notified":true},
            {"id":"done-2","status":"completed","notifyOnCompletion":false}
        ]
    }));
    let work = session.work();
    assert_eq!(work.len(), 1);
    assert_eq!(work[0].conversation_id, None);
    assert_eq!(work[0].state, BackgroundState::Running);
    assert!(session.has_pending_metadata());
    assert_eq!(session.background_error, None);

    session.observe_background_notification(&json!({
        "type":"message_end",
        "message":{"customType":"background-task-notification","details":{"id":"done-1"}}
    }));
    assert!(!session.has_pending_metadata());

    session.apply_background_frame(&json!({
            "version":1,"kind":"snapshot",
            "tasks":[
                {"id":"running-1","status":"running"},
                {"id":"previous-session-task","status":"completed","notifyOnCompletion":true,"notified":true}
            ]
        }));
    assert!(session.has_pending_metadata());
    PiSession::observe_persisted_background_notifications(
        &json!({"data":{"messages":[
            {"customType":"background-task-notification","details":{"id":"previous-session-task"}}
        ]}}),
        &mut session.settled_notifications,
        &mut session.pending_notifications,
    );
    assert!(!session.has_pending_metadata());

    session.apply_background_frame(&json!({
        "version":1,"kind":"unknown","supported":true,
        "error":"host query failed","tasks":[]
    }));
    assert_eq!(session.work().len(), 1);
    assert!(session.has_pending_metadata());
    assert_eq!(
        session.background_error.as_deref(),
        Some("host query failed")
    );

    session.apply_background_frame(
        &json!({"version":1,"kind":"unsupported","supported":false,"tasks":[]}),
    );
    assert!(session.work().is_empty());
    assert!(!session.has_pending_metadata());
}

#[test]
fn compacted_notification_receipts_settle_without_claiming_undelivered_work() {
    let config: Config = serde_json::from_value(json!({
        "execution":{"kind":"local"}, "binary":"/bin/pi", "node_binary":"/bin/node",
    }))
    .unwrap();
    let mut session = PiSession::new(config);
    // get_messages no longer includes this notification after compaction.
    // The SDK archive still proves it was delivered to this session.
    session.apply_background_frame(&json!({
            "version":1,"kind":"snapshot","tasks":[
                {"id":"before-compaction","status":"completed","notifyOnCompletion":true,"notified":true}
            ],"persistedNotificationIds":["before-compaction"]
        }));
    assert_eq!(session.work_status(), crate::work::WorkStatus::Settled);
    assert!(!session.has_pending_metadata());

    session.apply_background_frame(&json!({
            "version":1,"kind":"snapshot","tasks":[
                {"id":"before-compaction","status":"completed","notifyOnCompletion":true,"notified":true},
                {"id":"in-flight","status":"completed","notifyOnCompletion":true,"notified":true},
                {"id":"actual-work","status":"running"}
            ],"persistedNotificationIds":["before-compaction"]
        }));
    assert!(
        session.has_pending_metadata(),
        "notified alone is not delivery proof"
    );
    assert_eq!(session.work().len(), 1);
    session.apply_background_frame(&json!({
        "version":1,"kind":"snapshot","tasks":[
            {"id":"in-flight","status":"completed","notifyOnCompletion":true,"notified":true},
            {"id":"actual-work","status":"running"}
        ],"persistedNotificationIds":["in-flight"]
    }));
    assert!(!session.has_pending_metadata());
    assert!(
        session.work_status() == crate::work::WorkStatus::Active,
        "a receipt must not erase active work"
    );
}

#[test]
fn notification_receipt_snapshot_observes_preceding_agent_start() {
    let mut random = [0; 8];
    getrandom::fill(&mut random).unwrap();
    let root = std::env::temp_dir().join(format!(
        "velune-pi-receipt-order-{:x}",
        u64::from_le_bytes(random)
    ));
    std::fs::create_dir(&root).unwrap();
    let helper = root.join("rpc.py");
    std::fs::write(&helper, r#"
import json,sys
def emit(value): print(json.dumps(value),flush=True)
started=False
for line in sys.stdin:
 r=json.loads(line); command=r['type']; data={}
 if command=='get_commands': data={'commands':[{'name':'velune-background-work'}]}
 elif command=='prompt':
  if not started: emit({'type':'agent_start'}); started=True
  frame={'version':1,'kind':'snapshot','tasks':[{'id':'old','status':'completed','notifyOnCompletion':True,'notified':True}],'persistedNotificationIds':['old']}
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.background-work-v1','statusText':json.dumps(frame)})
  data={'disposition':'handled'}
 elif command=='abort': emit({'type':'agent_settled','aborted':True})
 emit({'type':'response','id':r['id'],'command':command,'success':True,'data':data})
"#).unwrap();
    let config: Config = serde_json::from_value(json!({
        "execution":{"kind":"local"}, "os_home":root,
        "binary":"/usr/bin/python3", "node_binary":"/usr/bin/python3", "rpc_entry":helper,
    }))
    .unwrap();
    let client = Client::spawn(config.clone()).unwrap();
    let mut session = PiSession::new(config);
    session.start(
        client,
        PiProjection::new(ConversationSummary {
            native_read_state: None,
            id: "pi:synthetic".into(),
            runtime_id: "pi".into(),
            title: crate::conversation::conversation_title(None, None),
            cwd: None,
            created_at_unix_ms: None,
            updated_at_unix_ms: None,
            can_rename: false,
            can_delete: false,
            parent_conversation_id: None,
        }),
    );
    poll_until(&mut session, |session| !session.has_pending_metadata());
    assert!(!session.has_pending_metadata());
    assert!(
        session.turn_running(),
        "settling old receipts must retain the new run"
    );
    assert_eq!(session.snapshot().unwrap().run_state, RunState::Running);
    assert!(session.can_steer());
    session.cancel().unwrap();
    poll_until(&mut session, |session| !session.turn_running());
    assert!(!session.turn_running());
    assert_eq!(session.snapshot().unwrap().run_state, RunState::Idle);
    session.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn extension_start_during_history_sync_keeps_steering_and_cancellation_available() {
    let mut random = [0; 8];
    getrandom::fill(&mut random).unwrap();
    let root = std::env::temp_dir().join(format!(
        "velune-pi-lifecycle-{:x}",
        u64::from_le_bytes(random)
    ));
    std::fs::create_dir(&root).unwrap();
    let helper = root.join("rpc.py");
    std::fs::write(
            &helper,
            r#"
import json, sys
def emit(value):
    print(json.dumps(value), flush=True)
started = False
for line in sys.stdin:
    request = json.loads(line)
    command = request['type']
    data = {}
    if command == 'prompt' and request['message'].startswith('/velune-background-work '):
        emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.background-work-v1','statusText':json.dumps({'version':1,'kind':'unsupported','supported':False,'tasks':[]})})
        data = {'disposition':'handled'}
    elif command == 'prompt':
        if not started:
            emit({'type':'agent_start'})
            started = True
        emit({'type':'extension_ui_request','method':'setStatus',
              'statusKey':'velune.session-metadata',
              'statusText':json.dumps({'requestId':request['id'],'snapshotVersion':1,'messages':[], 'messageIds':[], 'identityConfirmations':[]})})
        data = {'disposition':'handled'}
    elif command == 'get_state':
        if not started:
            emit({'type':'agent_start'})
            started = True
    elif command == 'get_messages':
        data = {'messages':[]}
    elif command == 'abort':
        emit({'type':'agent_settled','aborted':True})
    emit({'type':'response','id':request['id'],'command':command,'success':True,'data':data})
"#,
        )
        .unwrap();
    let config: Config = serde_json::from_value(json!({
        "execution":{"kind":"local"}, "os_home":root,
        "binary":"/usr/bin/python3", "node_binary":"/usr/bin/python3",
        "rpc_entry":helper,
    }))
    .unwrap();
    let client = Client::spawn(config.clone()).unwrap();
    let mut session = PiSession::new(config);
    session.start(
        client,
        PiProjection::new(ConversationSummary {
            native_read_state: None,
            id: "pi:synthetic".into(),
            runtime_id: "pi".into(),
            title: crate::conversation::conversation_title(None, None),
            cwd: None,
            created_at_unix_ms: None,
            updated_at_unix_ms: None,
            can_rename: false,
            can_delete: false,
            parent_conversation_id: None,
        }),
    );
    assert!(
        !session.sync_history("pi", None).unwrap(),
        "history admission is not a completed fence"
    );
    poll_until(&mut session, |session| {
        !session.running.as_ref().unwrap().history_pending
    });
    assert!(!session.running.as_ref().unwrap().history_completed);
    assert!(
        !session.sync_history("pi", None).unwrap(),
        "the new run defers native history replacement"
    );
    assert!(session.turn_running());
    assert!(session.can_steer());
    let snapshot = session.snapshot().unwrap();
    assert_eq!(snapshot.run_state, RunState::Running);
    assert!(!snapshot.actions.can_send);
    assert!(snapshot.actions.can_cancel);
    session.cancel().unwrap();
    poll_until(&mut session, |session| !session.turn_running());
    assert!(!session.turn_running());
    assert!(!session.can_steer());
    assert!(session.snapshot().unwrap().actions.can_send);
    session.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn preflight_dialog_does_not_wait_for_prompt_ack_and_routes_reply() {
    use crate::conversation::{InteractionAnswer, InteractionKind, RuntimeInteractionReply};
    let mut random = [0; 8];
    getrandom::fill(&mut random).unwrap();
    let root =
        std::env::temp_dir().join(format!("velune-pi-dialog-{:x}", u64::from_le_bytes(random)));
    std::fs::create_dir(&root).unwrap();
    let helper = root.join("rpc.py");
    std::fs::write(&helper, r#"
import json, sys
pending = None
def emit(value): print(json.dumps(value), flush=True)
for line in sys.stdin:
    request = json.loads(line)
    command = request['type']
    data = {}
    if command == 'prompt':
        if request['message'].startswith('/velune'):
            data = {'disposition':'handled'}
        else:
            pending = request
            emit({'type':'extension_ui_request','id':'dialog','method':'editor','title':'Edit','prefill':'initial'})
            continue
    elif command == 'extension_ui_response':
        assert request.get('value') == 'edited' or request.get('cancelled') is True
        failed = pending['message'] == 'reject'
        emit({'type':'response','id':pending['id'],'command':'prompt','success':not failed,
              'error':'synthetic rejection','data':{'disposition':'handled'}})
        continue
    elif command == 'get_messages': data = {'messages':[]}
    elif command == 'abort': emit({'type':'agent_settled','aborted':True})
    emit({'type':'response','id':request['id'],'command':command,'success':True,'data':data})
"#).unwrap();
    let config: Config = serde_json::from_value(json!({
        "execution":{"kind":"local"},"os_home":root,
        "binary":"/usr/bin/python3","node_binary":"/usr/bin/python3","rpc_entry":helper,
    }))
    .unwrap();
    let client = Client::spawn(config.clone()).unwrap();
    let mut session = PiSession::new(config);
    session.start(
        client,
        PiProjection::new(ConversationSummary {
            native_read_state: None,
            id: "pi:synthetic".into(),
            runtime_id: "pi".into(),
            title: crate::conversation::conversation_title(None, None),
            cwd: None,
            created_at_unix_ms: None,
            updated_at_unix_ms: None,
            can_rename: false,
            can_delete: false,
            parent_conversation_id: None,
        }),
    );
    for text in ["handled", "reject", "cancel"] {
        session
            .send(&TurnInput {
                text: text.into(),
                effort: None,
                permission_mode: None,
                images: vec![],
            })
            .unwrap();
        poll_until(&mut session, |session| {
            !session.snapshot().unwrap().pending_interactions.is_empty()
        });
        let snapshot = session.snapshot().unwrap();
        assert_eq!(snapshot.pending_interactions.len(), 1);
        let InteractionKind::UserInput { questions } = &snapshot.pending_interactions[0].kind
        else {
            panic!("editor question")
        };
        assert_eq!(questions[0].default_text.as_deref(), Some("initial"));
        assert!(questions[0].multiline);
        let revision = snapshot.revision;
        if text == "cancel" {
            session.cancel().unwrap();
        } else {
            session
                .reply(
                    "dialog",
                    RuntimeInteractionReply::Answers {
                        answers: vec![InteractionAnswer {
                            question_id: "dialog".into(),
                            values: vec!["edited".into()],
                        }],
                    },
                )
                .unwrap();
        }
        assert!(session.snapshot().unwrap().pending_interactions.is_empty());
        assert!(session.snapshot().unwrap().revision > revision);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while session.turn_running() && std::time::Instant::now() < deadline {
            session.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!session.turn_running());
        assert!(session.snapshot().unwrap().actions.can_send);
        if text == "reject" {
            assert!(session.turn_failed());
            assert!(session.snapshot().unwrap().messages.iter().any(|message|
                    message.blocks.iter().any(|block| matches!(block,crate::conversation::MessageBlock::Notice {text} if text == "synthetic rejection"))));
        }
    }
    session.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
