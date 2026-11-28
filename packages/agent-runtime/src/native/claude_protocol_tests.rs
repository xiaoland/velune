//! Actual Claude send/steer/reply/poll entry points with a synthetic stream-json child.
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
    fn new(mode: &str) -> Self {
        let mut fixture = Self::connect(mode);
        let mut input = Self::input("once");
        input.permission_mode = Some("manual".into());
        fixture.session.send(&input).unwrap();
        fixture
    }
    fn connect(mode: &str) -> Self {
        let root = TempRoot::new("claude-protocol");
        std::fs::write(root.join("mode"), mode).unwrap();
        let binary = root.join("claude");
        std::fs::write(&binary, r#"#!/usr/bin/python3
import fcntl,json,sys,os
root=os.path.dirname(__file__)
if '--version' in sys.argv:
 print('2.1.327 (Claude Code)');sys.exit(0)
with open(os.path.join(root,'argv.json'),'w') as f: json.dump(sys.argv,f)
mode=open(os.path.join(root,'mode')).read()
started=False
def emit(record): print(json.dumps(record),flush=True)
def response(request,negative=False):
 emit({'type':'control_response','response':{'subtype':'error' if negative else 'success','request_id':request['request_id'],'error':'synthetic rejection' if negative else None,'response':{'models':[]}}})
def terminal():
 emit({'type':'system','subtype':'task_notification','task_id':'shell','run_id':'one','status':'completed','summary':'done'})
for line in sys.stdin:
 request=json.loads(line)
 with open(os.path.join(root,'requests.jsonl'),'a') as f:
  fcntl.flock(f,fcntl.LOCK_EX)
  f.write(json.dumps(request)+'\n')
 if request.get('type')=='fixture_emit': emit(request['record']);continue
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
  else: response(request,mode=='reject')
 elif request.get('type')=='user' and not started:
  started=True
  if mode.startswith('steer_'):
   emit({'type':'system','subtype':'init','permissionMode':'default'})
   if mode!='steer_success': continue
  emit({'type':'system','subtype':'task_started','task_id':'shell','run_id':'one','task_type':'local_bash','description':'Shell'})
  emit({'type':'control_request','request_id':'permission','request':{'subtype':'can_use_tool','tool_name':'Bash','input':{'command':'synthetic'}}})
 elif request.get('type')=='control_response':
  terminal()
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
            auto_compaction: None,
            mcp_servers: vec![],
            session_agents: vec![],
        })
        .unwrap();
        session.replace_snapshot("parent", Some(root.path()), "runtime", vec![]);
        session.claude.start = claude::ClaudeStart::Fresh { name: None };
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
    fn fence(&mut self) {
        self.session
            .rpc
            .claude_control(json!({"subtype":"initialize","hooks":{}}))
            .unwrap();
        self.session.poll().unwrap();
    }
    fn emit(&mut self, record: Value) {
        self.session
            .rpc
            .write(&json!({"type":"fixture_emit","record":record}))
            .unwrap();
    }
    fn requests(&self) -> Vec<Value> {
        // The child can still be appending a multi-megabyte JSON record.
        // Read a complete snapshot and keep malformed completed records fatal.
        let file = std::fs::File::open(self.root.join("requests.jsonl")).unwrap();
        file.lock_shared().unwrap();
        std::io::read_to_string(file)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.session.shutdown().unwrap();
    }
}

#[test]
fn claude_steer_updates_permission_without_answering_an_existing_tool_prompt() {
    let mut fixture = Fixture::new("steer_success");
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
fn claude_steer_never_sends_text_when_permission_is_rejected_or_the_turn_ends() {
    for (mode, error) in [
        ("steer_rejected", "steer_permission_rejected"),
        ("steer_disconnect", "steer_permission_unconfirmed"),
        ("steer_ended", "steer_settings_changed"),
    ] {
        let mut fixture = Fixture::new(mode);
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
    let mut fixture = Fixture::new("ordinary");
    fixture.poll_until(|session| session.has_pending_requests());
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
fn claude_background_refresh_requires_a_matching_ack_and_new_snapshot() {
    let mut f = Fixture::new("refresh");
    f.poll_until(|session| session.has_pending_requests());
    let known = f.session.background_tasks().unwrap().tasks[0].clone();
    let approvals = f.session.snapshot().unwrap().pending_interactions.clone();
    assert_eq!(approvals.len(), 1);
    f.session.available_models = vec![json!({"value":"existing-model"})];

    f.session.refresh_background_tasks(false);
    f.fence();
    assert!(!f.requests().iter().any(|frame| {
        frame["request"]["subtype"] == "initialize" && frame["request"].get("hooks").is_none()
    }));

    f.session.refresh_background_tasks(true);
    let first = f
        .session
        .claude
        .task_refresh
        .as_ref()
        .unwrap()
        .request_id
        .clone();
    f.session.refresh_background_tasks(true);
    let pending = f.session.claude.task_refresh.as_mut().unwrap();
    assert_eq!(pending.request_id, first);
    pending.deadline = Instant::now();
    f.session.poll().unwrap();
    let timed_out = f.session.background_tasks().unwrap();
    assert!(timed_out.live && timed_out.issue.as_ref().unwrap().contains("超时"));
    assert_eq!(timed_out.tasks, vec![known.clone()]);
    assert!(f.session.turn_running() && !f.session.failed);

    f.session.refresh_background_tasks(true);
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
fn claude_permission_replies_use_exact_native_id_and_cannot_be_sent_twice() {
    for (reply, expected) in [
        (
            RuntimeInteractionReply::Decision {
                option_id: "allow".into(),
            },
            json!({"behavior":"allow","updatedInput":{"command":"synthetic"}}),
        ),
        (
            RuntimeInteractionReply::Decision {
                option_id: "deny".into(),
            },
            json!({"behavior":"deny","message":"User denied this operation"}),
        ),
        (
            RuntimeInteractionReply::Cancel,
            json!({"behavior":"deny","message":"User cancelled this operation","interrupt":true}),
        ),
    ] {
        let mut f = Fixture::new("ordinary");
        f.poll_until(|session| session.has_pending_requests());
        let id = f.session.snapshot().unwrap().pending_interactions[0]
            .id
            .clone();
        f.session.reply(&id, reply.clone()).unwrap();
        assert!(!f.session.has_pending_requests());
        assert_eq!(
            f.session.reply(&id, reply).unwrap_err().to_string(),
            "此交互请求已经失效"
        );
        f.fence();
        let replies: Vec<_> = f
            .requests()
            .into_iter()
            .filter(|r| r["type"] == "control_response")
            .collect();
        assert_eq!(
            replies,
            vec![json!({"type":"control_response","response":{
                "subtype":"success","request_id":"permission","response":expected
            }})]
        );
        assert!(
            f.session.turn_running(),
            "permission reply is not a turn terminal"
        );
    }
}

#[test]
fn claude_result_waits_for_native_idle_and_blocks_further_steering() {
    let mut f = Fixture::new("ordinary");
    f.poll_until(|session| session.has_pending_requests());
    let id = f.session.snapshot().unwrap().pending_interactions[0]
        .id
        .clone();
    f.session
        .reply(
            &id,
            RuntimeInteractionReply::Decision {
                option_id: "allow".into(),
            },
        )
        .unwrap();
    f.emit(json!({"type":"result","subtype":"success","is_error":false}));
    f.fence();
    assert!(f.session.turn_running(), "result alone is not idle");
    assert_eq!(
        f.session
            .steer(&Fixture::input("must not send"))
            .unwrap_err()
            .to_string(),
        "Claude Code 当前轮次不能插入消息"
    );
    assert_eq!(
        f.requests().iter().filter(|r| r["type"] == "user").count(),
        1
    );
    f.emit(json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
    f.poll_until(|session| !session.turn_running());
    assert_eq!(f.session.snapshot().unwrap().run_state, RunState::Idle);
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
}

#[test]
fn claude_unknown_control_is_rejected_without_answering_existing_permission() {
    let mut f = Fixture::new("ordinary");
    f.poll_until(|session| session.has_pending_requests());
    let existing = f.session.snapshot().unwrap().pending_interactions.clone();
    f.emit(json!({"type":"control_request","request_id":"unknown-control","request":{"subtype":"unsupported"}}));
    f.fence();
    f.fence();
    assert_eq!(f.session.snapshot().unwrap().pending_interactions, existing);
    let replies: Vec<_> = f
        .requests()
        .into_iter()
        .filter(|r| r["type"] == "control_response")
        .collect();
    assert_eq!(
        replies,
        vec![json!({"type":"control_response","response":{
            "subtype":"error","request_id":"unknown-control","error":"Unsupported control request"
        }})]
    );
    assert!(f.session.turn_running());
}

#[test]
fn claude_normal_send_preserves_a_large_user_frame() {
    let mut f = Fixture::connect("ordinary");
    let text = format!("prefix{}suffix", "x".repeat(9 * 1024 * 1024));
    f.session.send(&Fixture::input(&text)).unwrap();
    f.fence();
    let frames = f.requests();
    let users: Vec<_> = frames
        .iter()
        .filter(|frame| frame["type"] == "user")
        .collect();
    assert_eq!(users.len(), 1);
    assert_eq!(
        users[0]["message"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .len(),
        text.len()
    );
    assert_eq!(
        users[0]["message"]["content"][0]["text"].as_str(),
        Some(text.as_str())
    );
}

#[test]
fn claude_normal_restart_applies_model_effort_and_clears_auto_compaction() {
    let mut f = Fixture::connect("ordinary");
    f.session
        .select_native_model(Retained::from_parts(
            (None, "synthetic-model".into()),
            DataRetention::default(),
        ))
        .unwrap();
    f.session
        .set_auto_compaction(Some(AutoCompactionSetting::Claude {
            window_tokens: 700_000,
        }));
    let mut input = Fixture::input("first");
    input.effort = Some("high".into());
    input.permission_mode = Some("manual".into());
    f.session.send(&input).unwrap();
    f.fence();
    let argv: Vec<String> =
        serde_json::from_slice(&std::fs::read(f.root.join("argv.json")).unwrap()).unwrap();
    for pair in [
        ["--model", "synthetic-model"],
        ["--effort", "high"],
        ["--autocompact", "700000"],
        ["--permission-mode", "manual"],
    ] {
        assert!(
            argv.windows(2).any(|args| args == pair),
            "{pair:?}: {argv:?}"
        );
    }
    let permission = f.session.snapshot().unwrap().pending_interactions[0]
        .id
        .clone();
    f.session
        .reply(&permission, RuntimeInteractionReply::Cancel)
        .unwrap();
    f.emit(json!({"type":"result","subtype":"success","is_error":false}));
    f.emit(json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
    f.poll_until(|session| !session.turn_running());
    f.session.set_auto_compaction(None);
    f.session.send(&Fixture::input("second")).unwrap();
    f.fence();
    let argv: Vec<String> =
        serde_json::from_slice(&std::fs::read(f.root.join("argv.json")).unwrap()).unwrap();
    assert!(
        argv.windows(2)
            .any(|args| args == ["--model", "synthetic-model"])
    );
    for removed in ["--effort", "--autocompact"] {
        assert!(
            !argv.iter().any(|arg| arg == removed),
            "{removed}: {argv:?}"
        );
    }
}

#[test]
fn claude_portable_side_context_is_loaded_without_persistence_and_used_for_questions() {
    let mut f = Fixture::connect("ordinary");
    let context = "Reference context\nUTF-8: 合成来源\n";
    f.session
        .create_side_context(Some(f.root.path()), "runtime", context)
        .unwrap();
    f.session
        .send(&Fixture::input("first side question"))
        .unwrap();
    f.poll_until(|session| !session.turn_running());
    let argv: Vec<String> =
        serde_json::from_slice(&std::fs::read(f.root.join("argv.json")).unwrap()).unwrap();
    assert!(argv.iter().any(|arg| arg == "--no-session-persistence"));
    assert!(!argv.iter().any(|arg| arg == "--resume"));
    let context_path = argv
        .windows(2)
        .find(|args| args[0] == "--append-system-prompt-file")
        .unwrap();
    assert_eq!(std::fs::read_to_string(&context_path[1]).unwrap(), context);
    f.session
        .send(&Fixture::input("second side question"))
        .unwrap();
    f.poll_until(|session| !session.turn_running());
    let requests = f.requests();
    assert!(!requests.iter().any(|r| r["type"] == "user"));
    let questions: Vec<_> = requests
        .iter()
        .filter(|r| r["request"]["subtype"] == "side_question")
        .collect();
    assert_eq!(questions.len(), 2);
    assert_eq!(
        questions[0]["request"],
        json!({"subtype":"side_question","question":"first side question","history":[]})
    );
    assert_eq!(
        questions[1]["request"],
        json!({"subtype":"side_question","question":"second side question",
        "history":[{"question":"first side question","response":"synthetic side answer"}]})
    );
    assert_eq!(
        f.session
            .snapshot()
            .unwrap()
            .messages
            .last()
            .unwrap()
            .blocks,
        [MessageBlock::Text {
            text: "synthetic side answer".into()
        }]
    );
}
