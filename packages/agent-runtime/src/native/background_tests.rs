//! Actual stream-json control writes with synthetic task events; no model runs.
use super::*;
use std::os::unix::fs::PermissionsExt;

struct Fixture {
    root: PathBuf,
    session: NativeSession,
}
impl Fixture {
    fn new() -> Self {
        Self::with_version("2.1.292")
    }
    fn with_version(version: &str) -> Self {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root =
            std::env::temp_dir().join(format!("velune-background-{:x}", u64::from_le_bytes(nonce)));
        std::fs::create_dir(&root).unwrap();
        let binary = root.join("claude");
        std::fs::write(&binary, r#"#!/usr/bin/python3
import json,sys,os
if '--version' in sys.argv:
 print('2.1.292 (Claude Code)');sys.exit(0)
for line in sys.stdin:
 request=json.loads(line)
 with open(os.path.join(os.path.dirname(__file__),'requests.jsonl'),'a') as f: f.write(json.dumps(request)+'\n')
 if request.get('type')=='control_request':
  print(json.dumps({'type':'control_response','response':{'subtype':'success','request_id':request['request_id'],'response':{}}}),flush=True)
"#.replace("2.1.292", version)).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut session = NativeSession::connect(NativeConfig {
            execution: crate::execution::ExecutionTarget::Local,
            os_home: Some(root.clone()),
            kind: NativeKind::ClaudeCode,
            binary,
            node_binary: None,
            agent_dir: root.clone(),
            resources_directory: root.clone(),
            projection_directory: root.clone(),
            gateway: None,
            mcp_servers: vec![],
            session_agents: vec![],
        })
        .unwrap();
        session.replace_snapshot("parent", Some(&root), "runtime", vec![]);
        session.native_id = Some("parent".into());
        session.running();
        Self { root, session }
    }
    fn event(&mut self, value: Value) {
        self.session.claude_handle_record(value).unwrap();
    }
    fn task(&self, title: &str) -> BackgroundTaskInfo {
        self.session
            .background_tasks()
            .tasks
            .into_iter()
            .find(|task| task.title == title)
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.session.shutdown();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn background_tasks_claude_shell_and_workflow_stay_separate_from_agent_relationships() {
    let mut f = Fixture::new();
    for (id, kind) in [("shell", "local_bash"), ("workflow", "local_workflow")] {
        f.event(json!({"type":"system","subtype":"task_started","task_id":id,"task_type":kind,"description":id}));
        assert!(f.task(id).conversation_id.is_none());
    }
    assert!(f.session.sub_agents.is_empty());
    assert!(
        sub_agent_conversations(&f.session.snapshot.as_ref().unwrap().messages)
            .next()
            .is_none()
    );

    f.event(json!({"type":"system","subtype":"task_started","task_id":"agent","task_type":"local_agent","description":"Agent task"}));
    let child = f.session.claude_agent_conversation("agent").unwrap();
    assert_eq!(
        f.task("Agent task").conversation_id.as_deref(),
        Some(child.as_str()),
        "the first agent receipt must link its child without waiting for progress"
    );
    assert_eq!(
        f.session.sub_agents.iter().cloned().collect::<Vec<_>>(),
        vec![child.clone()]
    );

    for id in ["shell", "workflow"] {
        f.event(json!({"type":"system","subtype":"task_notification","task_id":id,"status":"completed","summary":"Synthetic result"}));
        assert!(f.task(id).conversation_id.is_none());
    }
    assert_eq!(f.session.sub_agents.len(), 1);
    assert!(
        sub_agent_conversations(&f.session.snapshot.as_ref().unwrap().messages)
            .next()
            .is_none(),
        "ordinary background completion must not create a child conversation link"
    );
    f.event(json!({"type":"system","subtype":"task_notification","task_id":"agent","status":"completed","summary":"Agent result"}));
    assert_eq!(
        sub_agent_conversations(&f.session.snapshot.as_ref().unwrap().messages).collect::<Vec<_>>(),
        vec![child]
    );
}

#[test]
fn background_tasks_claude_exact_stop_preserves_parent_and_sibling_until_terminal() {
    let mut f = Fixture::new();
    f.event(
        json!({"type":"system","subtype":"background_tasks_changed","tasks":[
        {"task_id":"a","run_id":"one","task_type":"local_bash","description":"Shell A"},
        {"task_id":"b","run_id":"two","task_type":"local_bash","description":"Shell B"}]}),
    );
    let a = f.task("Shell A");
    assert!(!a.can_read_output, "description is not task output");
    assert_eq!(f.session.work().len(), 2);
    f.session.stop_background_task(&a.id).unwrap();
    assert_eq!(f.task("Shell A").status, BackgroundTaskStatus::Stopping);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !f.session.claude.task_stops.is_empty() && std::time::Instant::now() < deadline {
        f.session.poll().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        f.session.claude.task_stops.is_empty(),
        "matching control ACK must be consumed"
    );
    assert_eq!(
        f.task("Shell A").status,
        BackgroundTaskStatus::Stopping,
        "ACK is not completion"
    );
    assert_eq!(f.task("Shell B").status, BackgroundTaskStatus::Running);
    assert!(f.session.turn_running());
    let requests: Vec<Value> = std::fs::read_to_string(f.root.join("requests.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["request"]["subtype"] == "stop_task"
                && request["request"]["task_id"] == "a")
            .count(),
        1
    );
    assert!(
        !requests
            .iter()
            .any(|request| request["request"]["subtype"] == "interrupt")
    );
    f.event(json!({"type":"system","subtype":"task_notification","task_id":"a","run_id":"one","status":"stopped","summary":"Stopped shell A"}));
    assert_eq!(f.task("Shell A").status, BackgroundTaskStatus::Cancelled);
    assert_eq!(f.task("Shell B").status, BackgroundTaskStatus::Running);
    assert!(f.session.turn_running());
}

#[test]
fn background_tasks_claude_full_set_run_identity_and_verified_summary() {
    let mut f = Fixture::new();
    f.event(json!({"type":"system","subtype":"task_started","task_id":"same","run_id":"old","task_type":"local_bash","description":"Old run"}));
    let old = f.task("Old run");
    f.event(json!({"type":"system","subtype":"task_started","task_id":"same","run_id":"new","task_type":"local_bash","description":"New run"}));
    f.event(json!({"type":"system","subtype":"task_progress","task_id":"same","run_id":"old","description":"Late old progress"}));
    f.event(json!({"type":"system","subtype":"task_notification","task_id":"same","run_id":"old","status":"completed","summary":"Old result"}));
    assert_eq!(f.task("New run").status, BackgroundTaskStatus::Running);
    assert!(
        f.session.stop_background_task(&old.id).is_err(),
        "stale UI identity cannot stop reused native task ID"
    );
    let old = f.task("Old run");
    assert_eq!(old.status, BackgroundTaskStatus::Completed);
    let output = f
        .session
        .read_background_task_output(&old.id, None)
        .unwrap();
    assert_eq!(output.status, BackgroundTaskOutputStatus::Ready);
    assert_eq!(output.text, "Old result");
    let new_id = f.task("New run").id;
    assert!(
        f.session
            .read_background_task_output(&new_id, Some(&output.request_id))
            .is_err()
    );
    f.event(json!({"type":"system","subtype":"background_tasks_changed","tasks":[]}));
    assert_eq!(
        f.task("New run").status,
        BackgroundTaskStatus::Unknown,
        "absence proves no active work, not successful outcome"
    );
    assert!(f.session.work().is_empty());
}

#[test]
fn background_tasks_claude_late_negative_stop_ack_cannot_revive_terminal_run() {
    let mut f = Fixture::new();
    f.event(json!({"type":"system","subtype":"task_started","task_id":"a","run_id":"one","task_type":"local_bash","description":"Shell A"}));
    let task = f.task("Shell A");
    f.session.stop_background_task(&task.id).unwrap();
    let request = f.session.claude.task_stops.keys().next().unwrap().clone();
    f.event(json!({"type":"system","subtype":"task_notification","task_id":"a","run_id":"one","status":"completed","summary":"Completed before stop was accepted"}));
    f.event(json!({"type":"control_response","response":{"request_id":request,"subtype":"error","error":"already terminal"}}));
    assert_eq!(f.task("Shell A").status, BackgroundTaskStatus::Completed);
    assert!(!f.task("Shell A").can_stop);
    assert!(f.session.work().is_empty());
}

#[test]
fn background_tasks_claude_terminal_cache_and_stop_timeout_keep_live_work() {
    let mut f = Fixture::new();
    f.event(json!({"type":"system","subtype":"task_started","task_id":"active","run_id":"one","task_type":"local_bash","description":"Active shell"}));
    let active = f.task("Active shell");
    for index in 0..140 {
        f.event(json!({"type":"system","subtype":"task_notification","task_id":format!("done-{index}"),"run_id":"done","task_type":"local_bash","status":"completed","summary":"x".repeat(70_000)}));
    }
    assert!(f.session.claude.task_infos.len() <= 128);
    assert!(f.session.claude.task_outputs.len() <= 127);
    assert!(
        f.session
            .claude
            .task_outputs
            .values()
            .all(|(value, truncated)| value.len() <= 65_536 && *truncated)
    );
    assert_eq!(f.task("Active shell").status, BackgroundTaskStatus::Running);
    f.session.stop_background_task(&active.id).unwrap();
    for (_, started) in f.session.claude.task_stops.values_mut() {
        *started = std::time::Instant::now() - std::time::Duration::from_secs(11);
    }
    f.session.claude_expire_background_controls();
    assert_eq!(
        f.task("Active shell").status,
        BackgroundTaskStatus::Stopping
    );
    assert!(f.session.background_tasks().issue.is_some());
    assert_eq!(
        f.session.work().len(),
        1,
        "unconfirmed stop cannot release the live process"
    );
}

#[test]
fn background_tasks_claude_old_cli_observes_without_unproven_stop_capability() {
    let mut f = Fixture::with_version("2.1.286");
    f.event(json!({"type":"system","subtype":"task_started","task_id":"a","task_type":"local_bash","description":"Observed shell"}));
    let task = f.task("Observed shell");
    assert!(!task.can_stop);
    assert_eq!(task.status, BackgroundTaskStatus::Running);
    assert!(f.session.stop_background_task(&task.id).is_err());
}

#[test]
fn claude_api_failure_preserves_reason_and_settles_on_native_idle() {
    for marker in ["isApiErrorMessage", "is_api_error_message"] {
        let mut f = Fixture::new();
        f.session.turn_id = Some("turn".into());
        let mut error = json!({"type":"assistant","error":"authentication_failed",
            "message":{"id":"failure","stop_reason":"stop_sequence",
                "content":[{"type":"text","text":"Login expired · Please run /login"}]}});
        error[marker] = json!(true);
        f.event(error.clone());
        f.event(error);
        f.event(json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
        let snapshot = f.session.snapshot.as_ref().unwrap();
        assert_eq!(snapshot.run_state, RunState::Failed);
        assert_eq!(snapshot.messages.last().unwrap().blocks, vec![MessageBlock::RuntimeFailure {
            failure: RuntimeFailureKind::Other,
            detail: "Login expired · Please run /login\nClaude Code 错误类型：authentication_failed".into(),
            retry: None,
        }]);
        let notices: Vec<_> = snapshot.messages.iter().flat_map(|m| &m.blocks).collect();
        assert!(notices.iter().any(|b| matches!(b, MessageBlock::RuntimeFailure {detail: text, ..} if text.contains("Login expired · Please run /login"))));
        assert!(notices.iter().any(|b| matches!(b, MessageBlock::RuntimeFailure {detail: text, ..} if text.contains("authentication_failed"))));
    }
}

#[test]
fn claude_api_failure_marker_without_detail_is_not_a_success() {
    let mut f = Fixture::new();
    f.session.turn_id = Some("turn".into());
    f.event(json!({"type":"assistant","isApiErrorMessage":true,
        "message":{"id":"failure","stop_reason":"stop_sequence","content":[]}}));
    f.event(json!({"type":"result","subtype":"success","is_error":false}));
    f.event(json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
    let snapshot = f.session.snapshot.as_ref().unwrap();
    assert_eq!(snapshot.run_state, RunState::Failed);
    assert!(snapshot.messages.iter().flat_map(|m| &m.blocks).any(|b| matches!(b,
        MessageBlock::RuntimeFailure {detail: text, ..} if text == "Claude Code 执行失败（上游未提供错误详情）。")));
}

#[test]
fn claude_api_failure_can_recover_only_after_a_successful_assistant_response() {
    let mut f = Fixture::new();
    f.session.turn_id = Some("turn".into());
    f.event(json!({"type":"assistant","error":"authentication_failed",
        "message":{"id":"failure","stop_reason":"stop_sequence","content":[]}}));
    f.event(
        json!({"type":"assistant","message":{"id":"answer","stop_reason":"end_turn",
        "content":[{"type":"text","text":"recovered answer"}]}}),
    );
    f.event(json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
    let snapshot = f.session.snapshot.as_ref().unwrap();
    assert_eq!(snapshot.run_state, RunState::Idle);
    assert!(
        snapshot
            .messages
            .iter()
            .flat_map(|message| &message.blocks)
            .any(|block| matches!(
                block,
                MessageBlock::RuntimeFailure {
                    retry: Some(RuntimeRetry::Recovered),
                    ..
                }
            ))
    );
}

#[test]
fn claude_api_failure_cold_history_deserializes_and_stays_in_failed_work() {
    use crate::transcript::{TranscriptEvent, TranscriptProjection};
    let f = Fixture::new();
    let native_id = "00000000-0000-4000-8000-000000000003";
    let projects = f.root.join("projects");
    std::fs::create_dir(&projects).unwrap();
    let records = [
        json!({"type":"user","uuid":"u","parentUuid":null,"message":{"content":"question"}}),
        json!({"type":"assistant","uuid":"a","parentUuid":"u","message":{"content":"answer","stop_reason":"end_turn"}}),
        json!({"type":"user","uuid":"notification","parentUuid":"a","origin":{"kind":"task-notification"},"promptSource":"system","message":{"content":"<task-notification><task-id>job</task-id><status>completed</status><summary>done</summary></task-notification>"}}),
        json!({"type":"assistant","uuid":"error","parentUuid":"notification","isApiErrorMessage":true,"error":"authentication_failed","message":{"stop_reason":"stop_sequence","content":[{"type":"text","text":"Login expired · Please run /login"}]}}),
        json!({"type":"system","uuid":"duration","parentUuid":"error","subtype":"turn_duration","durationMs":2356}),
    ];
    std::fs::write(projects.join(format!("{native_id}.jsonl")), records.iter().map(|r| format!("{r}\n")).collect::<String>()).unwrap();
    let history = crate::history::read(&crate::history::HistoryConfig {
        kind: crate::version::RuntimeKind::ClaudeCode,
        execution: f.session.config.execution.clone(), os_home: Some(f.root.clone()),
        provider: "claude-code".into(), binary: f.session.config.binary.clone(),
        node_binary: PathBuf::new(), resources_directory: f.root.clone(),
        root: projects, home: f.root.clone(),
    }, "runtime", native_id).unwrap();
    let error = history.messages.last().unwrap();
    assert_eq!(error.role, MessageRole::System);
    assert_eq!(error.blocks, vec![MessageBlock::RuntimeFailure {
        failure: RuntimeFailureKind::Other,
        detail: "Login expired · Please run /login\nClaude Code 错误类型：authentication_failed".into(),
        retry: None,
    }]);
    let mut projection = TranscriptProjection::default();
    projection.apply(TranscriptEvent::Replace {messages: &history.messages, executions: &history.executions});
    let turns = projection.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].outcome, Some(TurnOutcome::Failed));
    assert!(turns[0].work_message_ids.contains(&"error".to_owned()));
}
