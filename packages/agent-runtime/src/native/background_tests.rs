//! Actual stream-json control writes with synthetic task events; no model runs.
use super::*;
use std::os::unix::fs::PermissionsExt;
use velune_test_support::TempRoot;

struct Fixture {
    root: TempRoot,
    session: NativeSession,
}
impl Fixture {
    fn new() -> Self {
        Self::with_version("2.1.292")
    }
    fn with_version(version: &str) -> Self {
        let root = TempRoot::new("background");
        let binary = root.join("claude");
        std::fs::write(&binary, r#"#!/usr/bin/python3
import json,sys,os
if '--version' in sys.argv:
 print('2.1.292 (Claude Code)');sys.exit(0)
with open(os.path.join(os.path.dirname(__file__),'launches'),'a') as f: f.write(str(os.getpid())+'\n')
for line in sys.stdin:
 request=json.loads(line)
 with open(os.path.join(os.path.dirname(__file__),'requests.jsonl'),'a') as f: f.write(json.dumps(request)+'\n')
 if request.get('type')=='control_request':
  print(json.dumps({'type':'control_response','response':{'subtype':'success','request_id':request['request_id'],'response':{}}}),flush=True)
"#.replace("2.1.292", version)).unwrap();
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
        session.native_id = Some("parent".into());
        session.running();
        Self { root, session }
    }
    fn event(&mut self, value: Value) {
        self.session.claude_handle_record(value).unwrap();
    }
    fn task(&self, title: &str) -> Retained<BackgroundTaskInfo> {
        self.session.background_tasks().unwrap().map(|list| {
            list.tasks
                .into_iter()
                .find(|task| task.title == title)
                .unwrap()
        })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.session.shutdown();
    }
}

fn composed_input() -> crate::TurnInput {
    crate::TurnInput {
        text: "composed prompt".into(),
        images: vec![],
        effort: None,
        permission_mode: None,
    }
}

#[test]
fn claude_composed_waits_for_the_exact_native_start_receipt() {
    let mut f = Fixture::with_version("2.1.286");
    f.session.settle(RunState::Idle);
    assert!(f.session.supports_claude_composed_acceptance());
    let id = f.session.send_claude_composed(&composed_input()).unwrap();
    assert!(f.session.turn_running());
    assert!(f.session.snapshot().unwrap().messages.is_empty());

    // A flushed writer does not count as the runtime's acceptance.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        f.session.poll().unwrap();
        let requests = std::fs::read_to_string(f.root.join("requests.jsonl")).unwrap();
        if requests
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|frame| frame["type"] == "user" && frame["uuid"] == id)
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Claude did not read the input"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(f.session.take_claude_composed_acceptance().is_none());
    assert!(f.session.snapshot().unwrap().messages.is_empty());

    for (state, session, command) in [
        ("started", "different-session", id.as_str()),
        ("started", "parent", "other-command"),
        ("queued", "parent", id.as_str()),
    ] {
        f.event(json!({
            "type":"command_lifecycle", "state":state,
            "command_uuid":command, "session_id":session
        }));
    }
    f.event(json!({
        "type":"result", "subtype":"success",
        "origin":{"kind":"task-notification","runId":"other-run"}
    }));
    f.event(json!({"type":"system", "subtype":"session_state_changed", "state":"idle"}));
    assert!(f.session.turn_running());
    assert!(f.session.take_claude_composed_acceptance().is_none());
    assert!(f.session.snapshot().unwrap().messages.is_empty());

    for _ in 0..2 {
        f.event(json!({
            "type":"command_lifecycle", "state":"started",
            "command_uuid":id, "session_id":"parent"
        }));
    }
    assert_eq!(
        f.session.take_claude_composed_acceptance(),
        Some(ClaudeComposedAcceptance::Accepted {
            message_id: id.clone()
        })
    );
    assert!(f.session.take_claude_composed_acceptance().is_none());
    let messages = &f.session.snapshot().unwrap().messages;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, id);
    assert_eq!(messages[0].role, MessageRole::User);
}

#[test]
fn claude_composed_terminal_before_receipt_preserves_background_and_is_indeterminate() {
    let mut f = Fixture::with_version("2.1.327");
    f.session.claude.bound_session_id = f.session.native_id.clone();
    f.event(json!({
        "type":"system", "subtype":"task_started", "task_id":"shell",
        "run_id":"run", "task_type":"local_bash", "is_backgrounded":true
    }));
    f.session.settle(RunState::Idle);
    assert!(f.session.has_background_work());
    let id = f.session.send_claude_composed(&composed_input()).unwrap();
    f.event(json!({"type":"result", "subtype":"success", "user_message_uuid":id}));
    f.event(json!({"type":"system", "subtype":"session_state_changed", "state":"idle"}));
    assert!(matches!(
        f.session.take_claude_composed_acceptance(),
        Some(ClaudeComposedAcceptance::Indeterminate { message_id, .. }) if message_id == id
    ));
    assert!(f.session.snapshot().unwrap().messages.is_empty());
    assert!(f.session.has_background_work());
    assert!(!f.session.process_ended);
}

#[test]
fn claude_composed_compatibility_send_still_publishes_without_receipt() {
    let mut f = Fixture::with_version("2.1.286");
    f.session.settle(RunState::Idle);
    f.session.send(&composed_input()).unwrap();
    assert_eq!(f.session.snapshot().unwrap().messages.len(), 1);
    assert!(f.session.take_claude_composed_acceptance().is_none());
}

#[test]
fn claude_composed_rejects_a_write_the_transport_proves_was_skipped() {
    let mut f = Fixture::with_version("2.1.327");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    let (writer, receipt) = crate::transport::WriteReceipt::test_pending(deadline);
    f.session.claude.composed_prompt = Some(claude_composed::PendingPrompt::new(
        Message {
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
            completed: true,
            id: "skipped".into(),
            role: MessageRole::User,
            timestamp_unix_ms: None,
            blocks: vec![],
        },
        receipt,
        deadline,
    ));
    writer
        .send(crate::transport::WriteOutcome::NotWritten(
            std::io::Error::new(std::io::ErrorKind::TimedOut, "write expired in queue"),
        ))
        .unwrap();
    f.session.poll().unwrap();
    assert_eq!(
        f.session.take_claude_composed_acceptance(),
        Some(ClaudeComposedAcceptance::Rejected {
            message_id: "skipped".into(),
            detail: "write expired in queue".into(),
        })
    );
    assert!(!f.session.turn_running());
    assert!(f.session.snapshot().unwrap().messages.is_empty());
}

#[test]
fn claude_reuses_the_background_child_and_settles_only_the_matching_foreground() {
    let mut f = Fixture::with_version("2.1.327");
    f.session.claude.bound_session_id = f.session.native_id.clone();
    f.session.claude.launched_effort = Some("medium".into());
    let input = |text: &str, effort: &str| crate::TurnInput {
        text: text.into(),
        effort: Some(effort.into()),
        permission_mode: None,
        images: vec![],
    };
    f.session.claude_send(&input("one", "medium")).unwrap();
    let first = f
        .session
        .snapshot()
        .unwrap()
        .messages
        .last()
        .unwrap()
        .id
        .clone();
    f.event(json!({"type":"system","subtype":"task_started","task_id":"inline","run_id":"inline-run","task_type":"local_agent","is_backgrounded":false}));
    f.event(json!({"type":"system","subtype":"task_updated","task_id":"inline","run_id":"inline-run","patch":{"status":"completed"}}));
    assert!(
        !f.session.waits_for_background_followup(),
        "foreground tool completion needs no automatic response"
    );
    f.event(json!({"type":"system","subtype":"task_started","task_id":"shell","run_id":"run","task_type":"local_bash","description":"shell","is_backgrounded":true}));
    f.event(json!({"type":"result","subtype":"success","user_message_uuid":first}));
    f.event(json!({"type":"command_lifecycle","state":"completed","command_uuid":first}));
    f.event(json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
    assert!(!f.session.turn_running());
    assert!(f.session.has_background_work() && !f.session.process_ended);
    assert_eq!(
        f.session
            .send(&input("changed", "high"))
            .unwrap_err()
            .code(),
        "outstanding_work"
    );
    let launches = std::fs::read(f.root.join("launches")).unwrap();
    f.session.send(&input("two", "medium")).unwrap();
    assert_eq!(std::fs::read(f.root.join("launches")).unwrap(), launches);
    let second = f
        .session
        .snapshot()
        .unwrap()
        .messages
        .last()
        .unwrap()
        .id
        .clone();

    f.event(json!({"type":"system","subtype":"task_updated","task_id":"shell","run_id":"run","patch":{"status":"completed"}}));
    f.event(json!({"type":"system","subtype":"background_tasks_changed","tasks":[]}));
    assert!(!f.session.has_background_work() && f.session.waits_for_background_followup());
    f.event(
        json!({"type":"result","subtype":"success","user_message_uuid":second,
        "usage":{"input_tokens":10,"output_tokens":4}}),
    );
    f.event(json!({"type":"command_lifecycle","state":"completed","command_uuid":first}));
    assert!(
        f.session.turn_running(),
        "a late completion must match the original UUID"
    );
    f.event(json!({"type":"command_lifecycle","state":"completed","command_uuid":second}));
    assert!(!f.session.turn_running());
    f.session.end_spent_process();
    assert!(!f.session.process_ended);
    f.event(
        json!({"type":"result","subtype":"error_during_execution","is_error":true,
        "origin":{"kind":"task-notification","runId":"run"},"usage":{"input_tokens":999}}),
    );
    assert!(
        f.session.waits_for_background_followup(),
        "keep the child until native idle"
    );
    f.event(json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
    f.session.end_spent_process();
    let snapshot = f.session.snapshot().unwrap();
    assert_eq!(
        snapshot.executions.records.last().unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    assert_eq!(
        snapshot
            .messages
            .last()
            .unwrap()
            .turn_usage
            .as_ref()
            .unwrap()
            .input_tokens,
        Some(10)
    );
    assert!(!f.session.waits_for_background_followup());
}

#[test]
fn claude_auto_compaction_change_waits_for_the_background_child() {
    let mut f = Fixture::with_version("2.1.327");
    f.session.claude.bound_session_id = f.session.native_id.clone();
    f.event(json!({"type":"system","subtype":"task_started","task_id":"shell","run_id":"run","task_type":"local_bash","is_backgrounded":true}));
    f.session.settle(RunState::Idle);
    assert!(!f.session.restart_needed_for_send(None, None));

    f.session
        .set_auto_compaction(Some(AutoCompactionSetting::Claude {
            window_tokens: 700_000,
        }));
    let input = crate::TurnInput {
        text: "Next turn".into(),
        effort: None,
        permission_mode: None,
        images: vec![],
    };
    assert_eq!(
        f.session.send(&input).unwrap_err().code(),
        "outstanding_work"
    );
    assert!(f.session.has_background_work());
    assert!(!f.session.process_ended);
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
    assert_eq!(f.session.work().unwrap().len(), 2);
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
    let new_id = f.task("New run").map(|task| task.id);
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
    assert!(f.session.work().unwrap().is_empty());
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
    assert!(f.session.work().unwrap().is_empty());
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
    assert!(f.session.background_tasks().unwrap().issue.is_some());
    assert_eq!(
        f.session.work().unwrap().len(),
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
fn claude_compact_synthetic_sdk_message_uses_native_boundary_and_keeps_history() {
    let mut f = Fixture::new();
    let native_id = "00000000-0000-4000-8000-000000000001";
    f.session.native_id = Some(native_id.into());
    let projects = f.root.join("projects");
    std::fs::create_dir(&projects).unwrap();
    // Synthetic native JSONL: the SDK drops the summary flag.
    std::fs::write(projects.join(format!("{native_id}.jsonl")), [
        json!({"type":"user","uuid":"u","parentUuid":null,"message":{"content":"old question"}}),
        json!({"type":"system","uuid":"b","parentUuid":null,"logicalParentUuid":"u","subtype":"compact_boundary"}),
        json!({"type":"user","uuid":"s","parentUuid":"b","isCompactSummary":true,"message":{"content":"native summary"}}),
    ].into_iter().map(|r| format!("{r}\n")).collect::<String>()).unwrap();
    f.session.message(Message {
        id: "u".into(),
        role: MessageRole::User,
        timestamp_unix_ms: None,
        blocks: vec![MessageBlock::Text {
            text: "old question".into(),
        }],
        completed: true,
        turn_settings: None,
        turn_usage: None,
        branch_point: None,
    });
    f.event(json!({"type":"system","subtype":"compact_boundary","uuid":"b"}));
    f.event(
        json!({"type":"user","uuid":"s","isSynthetic":true,"message":{"content":"native summary"}}),
    );
    let messages = &f.session.snapshot.as_ref().unwrap().messages;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].id, "u");
    assert_eq!(messages[1].id, "b");
    assert_eq!(
        messages[1].blocks,
        vec![MessageBlock::ContextCompaction {
            text: "上下文已压缩\n\nnative summary".into()
        }]
    );
    // Once the snapshot owns a full summary, unrelated synthetic messages
    // must not reread history. Changing only this synthetic native fixture
    // makes a redundant read observable as an incorrect detail replacement.
    let path = projects.join(format!("{native_id}.jsonl"));
    let native = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        native.replace("native summary", "changed native summary"),
    )
    .unwrap();
    f.event(json!({"type":"user","isSynthetic":true,"message":{"content":"unrelated metadata"}}));
    assert!(
        matches!(&f.session.snapshot.as_ref().unwrap().messages[1].blocks[0],
        MessageBlock::ContextCompaction {text} if text == "上下文已压缩\n\nnative summary")
    );
    assert_eq!(f.session.snapshot.as_ref().unwrap().messages.len(), 2);
    f.event(json!({"type":"system","subtype":"status","compact_result":"success"}));
    assert!(
        matches!(&f.session.snapshot.as_ref().unwrap().messages[1].blocks[0], MessageBlock::ContextCompaction {text} if text == "上下文已压缩\n\nnative summary")
    );
}

#[test]
fn claude_compaction_order_matches_cold_history_through_repeated_boundaries() {
    use crate::transcript::{TranscriptEvent, TranscriptProjection};
    let mut f = Fixture::new();
    let native_id = "00000000-0000-4000-8000-000000000002";
    f.session.native_id = Some(native_id.into());
    let projects = f.root.join("projects");
    std::fs::create_dir(&projects).unwrap();
    let mut records = Vec::new();
    let user = json!({"type":"user","uuid":"u","parentUuid":null,"message":{"content":"question"}});
    records.push(user);
    f.session
        .snapshot
        .as_mut()
        .unwrap()
        .executions
        .begin(ExecutionKind::UserTurn, false);
    f.session.message(serde_json::from_value(json!({"id":"u","role":"user","completed":true,"blocks":[{"kind":"text","text":"question"}]})).unwrap());
    let mut parent = "u".to_string();
    for (id, content) in [
        (
            "a1",
            json!([{"type":"text","text":"working"},{"type":"tool_use","id":"call1","name":"Shell"}]),
        ),
        (
            "a2",
            json!([{"type":"thinking","thinking":"checking"},{"type":"tool_use","id":"call2","name":"Read"}]),
        ),
        ("answer", json!([{"type":"text","text":"final answer"}])),
    ] {
        let record = json!({"type":"assistant","uuid":id,"parentUuid":parent,"message":{"id":id,"content":content,"stop_reason":if id == "answer" {"end_turn"} else {"tool_use"}}});
        f.event(record.clone());
        records.push(record);
        parent = id.into();
        if id == "answer" {
            break;
        }
        let boundary = format!("b-{id}");
        let summary = format!("s-{id}");
        let result = json!({"type":"user","uuid":format!("r-{id}"),"parentUuid":parent,"message":{"content":[{"type":"tool_result","tool_use_id":if id == "a1" {"call1"} else {"call2"},"content":"tool output"}]}});
        parent = format!("r-{id}");
        f.event(result.clone());
        records.push(result);
        // The first compaction has provisional status; the second arrives as
        // a native boundary directly, which must not rename the first one.
        if id == "a1" {
            f.event(json!({"type":"system","subtype":"status","status":"compacting"}));
            let snapshot = f.session.snapshot.as_ref().unwrap();
            let mut projection = TranscriptProjection::default();
            projection.apply(TranscriptEvent::Replace {
                messages: &snapshot.messages,
                executions: &snapshot.executions,
            });
            assert_eq!(projection.turns()[0].work_message_ids.len(), 2);
        }
        let record = json!({"type":"system","subtype":"compact_boundary","uuid":boundary,"parentUuid":null,"logicalParentUuid":parent,"compactMetadata":{"trigger":"auto"}});
        f.event(record.clone());
        records.push(record);
        let record = json!({"type":"user","uuid":summary,"parentUuid":boundary,"isCompactSummary":true,"message":{"content":format!("summary {id}")}});
        f.event(record.clone());
        records.push(record);
        // Repeated delivery of the same boundary must update, not append.
        f.event(json!({"type":"system","subtype":"compact_boundary","uuid":boundary}));
        parent = summary;
    }
    let live = f.session.snapshot.as_ref().unwrap();
    let mut live_projection = TranscriptProjection::default();
    live_projection.apply(TranscriptEvent::Replace {
        messages: &live.messages,
        executions: &live.executions,
    });
    let turn = live_projection.turns().remove(0);
    assert_eq!(turn.work_message_ids, ["a1", "b-a1", "a2", "b-a2"]);
    assert_eq!(turn.last_message_id.as_deref(), Some("answer"));
    assert_eq!(live.messages.len(), 6);
    // A manual maintenance operation after the answer is independent, and a
    // subsequent user turn must not absorb either earlier compaction.
    f.session
        .snapshot
        .as_mut()
        .unwrap()
        .executions
        .finish(Some(TurnOutcome::Completed));
    f.session
        .snapshot
        .as_mut()
        .unwrap()
        .executions
        .begin(ExecutionKind::ContextMaintenance, false);
    for record in [
        json!({"type":"system","uuid":"manual","parentUuid":null,"logicalParentUuid":"answer","subtype":"compact_boundary","compactMetadata":{"trigger":"manual"}}),
        json!({"type":"user","uuid":"sm","parentUuid":"manual","isCompactSummary":true,"message":{"content":"manual summary"}}),
    ] {
        f.event(record.clone());
        records.push(record);
    }
    f.session
        .snapshot
        .as_mut()
        .unwrap()
        .executions
        .finish(Some(TurnOutcome::Completed));
    f.session
        .snapshot
        .as_mut()
        .unwrap()
        .executions
        .begin(ExecutionKind::UserTurn, false);
    records.push(
        json!({"type":"user","uuid":"u2","parentUuid":"sm","message":{"content":"next question"}}),
    );
    f.session.message(serde_json::from_value(json!({"id":"u2","role":"user","completed":true,"blocks":[{"kind":"text","text":"next question"}]})).unwrap());
    let last = json!({"type":"assistant","uuid":"answer2","parentUuid":"u2","message":{"id":"answer2","content":"next answer","stop_reason":"end_turn"}});
    f.event(last.clone());
    records.push(last);
    let live = f.session.snapshot.as_ref().unwrap();
    live_projection.apply(TranscriptEvent::Replace {
        messages: &live.messages,
        executions: &live.executions,
    });
    let expanded: Vec<String> = live_projection
        .items()
        .into_iter()
        .flat_map(|item| match item {
            crate::conversation::TranscriptItem::Message { message_id, .. } => vec![message_id],
            crate::conversation::TranscriptItem::Work { turn_id } => {
                live_projection
                    .turns()
                    .into_iter()
                    .find(|turn| turn.id == turn_id)
                    .unwrap()
                    .work_message_ids
            }
        })
        .collect();
    assert_eq!(
        expanded,
        [
            "u", "a1", "b-a1", "a2", "b-a2", "answer", "manual", "u2", "answer2"
        ]
    );
    assert!(live_projection.turns()[1].work_message_ids.is_empty());
    std::fs::write(
        projects.join(format!("{native_id}.jsonl")),
        records
            .into_iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let history = crate::history::read(
        &crate::history::HistoryConfig {
            kind: crate::version::RuntimeKind::ClaudeCode,
            execution: f.session.config.execution.clone(),
            os_home: Some(f.root.to_path_buf()),
            binary: f.session.config.binary.clone(),
            node_binary: PathBuf::new(),
            resources_directory: f.root.to_path_buf(),
            root: projects,
            home: f.root.to_path_buf(),
        },
        "runtime",
        native_id,
    )
    .unwrap();
    assert_eq!(
        live.messages
            .iter()
            .map(|message| &message.id)
            .collect::<Vec<_>>(),
        history
            .messages
            .iter()
            .map(|message| &message.id)
            .collect::<Vec<_>>()
    );
    let mut cold_projection = TranscriptProjection::default();
    cold_projection.apply(TranscriptEvent::Replace {
        messages: &history.messages,
        executions: &history.executions,
    });
    assert_eq!(
        cold_projection.turns()[0].work_message_ids,
        turn.work_message_ids
    );
    assert_eq!(
        cold_projection.turns()[0].last_message_id,
        turn.last_message_id
    );
    assert_eq!(cold_projection.items(), live_projection.items());
}

#[test]
fn claude_safety_refusal_survives_success_result_and_native_idle() {
    let mut f = Fixture::new();
    f.session.turn_id = Some("turn".into());
    f.event(
        json!({"type":"user","isSynthetic":true,"refused_message_id":"msg_refused",
        "message":{"content":"Synthetic reason: response stopped by safety classifier"}}),
    );
    f.event(json!({"type":"result","subtype":"success","is_error":false,"result":""}));
    assert_eq!(
        f.session.claude.result.as_ref().unwrap().0,
        TurnOutcome::Failed
    );
    f.event(json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
    let snapshot = f.session.snapshot.as_ref().unwrap();
    assert_eq!(snapshot.run_state, RunState::Failed);
    assert!(
        snapshot
            .messages
            .iter()
            .flat_map(|m| &m.blocks)
            .any(|b| matches!(b, MessageBlock::RuntimeFailure {detail: text, ..} if text.contains("Synthetic reason")))
    );
    assert!(
        snapshot
            .messages
            .iter()
            .all(|m| m.role != MessageRole::User)
    );
}

#[test]
fn claude_result_refusal_cannot_be_reported_as_success_or_retry() {
    let mut f = Fixture::new();
    f.session.turn_id = Some("turn".into());
    f.event(json!({"type":"result","subtype":"success","is_error":false,"stop_reason":"refusal","result":""}));
    assert_eq!(
        f.session.claude.result.as_ref().unwrap().0,
        TurnOutcome::Failed
    );
    assert!(
        f.session
            .snapshot
            .as_ref()
            .unwrap()
            .messages
            .iter()
            .flat_map(|m| &m.blocks)
            .any(|b| matches!(b, MessageBlock::RuntimeFailure {detail: text, ..} if text.contains("refusal")))
    );
}

#[test]
fn claude_native_safe_answer_can_recover_after_a_refused_response() {
    let mut f = Fixture::new();
    f.session.turn_id = Some("turn".into());
    f.event(json!({"type":"user","isSynthetic":true,"refused_message_id":"msg_refused","message":{"content":"safety stop"}}));
    f.event(json!({"type":"assistant","message":{"id":"safe-answer","stop_reason":"end_turn","content":[{"type":"text","text":"safe answer"}]}}));
    f.event(json!({"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn","result":"safe answer"}));
    assert_eq!(
        f.session.claude.result.as_ref().unwrap().0,
        TurnOutcome::Completed
    );
}

#[test]
fn claude_safety_marker_then_native_idle_needs_no_success_result() {
    let mut f = Fixture::new();
    f.session.turn_id = Some("turn".into());
    f.event(json!({"type":"user","isSynthetic":true,"refused_message_id":"msg_refused","message":{"content":"safety stop"}}));
    f.event(json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
    assert_eq!(
        f.session.snapshot.as_ref().unwrap().run_state,
        RunState::Failed
    );
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
        assert_eq!(
            snapshot.messages.last().unwrap().blocks,
            vec![MessageBlock::RuntimeFailure {
                failure: RuntimeFailureKind::Other,
                detail:
                    "Login expired · Please run /login\nClaude Code 错误类型：authentication_failed"
                        .into(),
                retry: None,
            }]
        );
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
    std::fs::write(
        projects.join(format!("{native_id}.jsonl")),
        records.iter().map(|r| format!("{r}\n")).collect::<String>(),
    )
    .unwrap();
    let history = crate::history::read(
        &crate::history::HistoryConfig {
            kind: crate::version::RuntimeKind::ClaudeCode,
            execution: f.session.config.execution.clone(),
            os_home: Some(f.root.to_path_buf()),
            binary: f.session.config.binary.clone(),
            node_binary: PathBuf::new(),
            resources_directory: f.root.to_path_buf(),
            root: projects,
            home: f.root.to_path_buf(),
        },
        "runtime",
        native_id,
    )
    .unwrap();
    let error = history.messages.last().unwrap();
    assert_eq!(error.role, MessageRole::System);
    assert_eq!(
        error.blocks,
        vec![MessageBlock::RuntimeFailure {
            failure: RuntimeFailureKind::Other,
            detail:
                "Login expired · Please run /login\nClaude Code 错误类型：authentication_failed"
                    .into(),
            retry: None,
        }]
    );
    let mut projection = TranscriptProjection::default();
    projection.apply(TranscriptEvent::Replace {
        messages: &history.messages,
        executions: &history.executions,
    });
    let turns = projection.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].outcome, Some(TurnOutcome::Failed));
    assert!(turns[0].work_message_ids.contains(&"error".to_owned()));
}
