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
