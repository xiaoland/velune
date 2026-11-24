use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Stdio},
};
use velune_test_support::TempRoot;

struct CodexWriter(Child);

impl CodexWriter {
    fn lock(path: &Path) -> Self {
        let mut writer = Self(
            velune_agent_runtime::process::command("python3")
                .args([
                    "-c",
                    r#"import fcntl, sys
with open(sys.argv[1], 'w') as writer:
    fcntl.lockf(writer, fcntl.LOCK_EX | fcntl.LOCK_NB)
    print('locked', flush=True)
    sys.stdin.buffer.read()
"#,
                ])
                .arg(path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let mut ready = String::new();
        BufReader::new(writer.0.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        assert_eq!(ready.trim_end(), "locked");
        writer
    }
}

impl Drop for CodexWriter {
    fn drop(&mut self) {
        // EOF releases the subprocess lock, also when an assertion panics.
        drop(self.0.stdin.take());
        let _ = self.0.wait();
    }
}

fn append_turn_event(rollout: &Path, event: &str, turn_id: &str) {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rollout)
        .unwrap();
    writeln!(
        file,
        "{}",
        json!({"type": "event_msg", "payload": {"type": event, "turn_id": turn_id}})
    )
    .unwrap();
}

fn assert_external_activity(core: &mut CoreRuntime, conversation_id: &str) {
    let active = core.active_conversations();
    let [entry] = active.as_slice() else {
        panic!("expected one externally running conversation, got {active:?}");
    };
    assert_eq!(entry.conversation_id, conversation_id);
    assert_eq!(entry.run_state, RunState::Elsewhere);
    assert!(entry.blocks_idle_submission);
}

#[test]
fn codex_snapshot_and_activity_follow_the_external_turn() {
    const RUNTIME_ID: &str = "synthetic-codex";
    const THREAD_ID: &str = "01234567-89ab-4cde-8fab-0123456789ab";
    const FIRST_TURN: &str = "11234567-89ab-4cde-8fab-0123456789ab";
    const SECOND_TURN: &str = "21234567-89ab-4cde-8fab-0123456789ab";

    let home = TempRoot::new("external");
    let agent_dir = home.join("codex");
    let sessions = agent_dir.join("sessions/2026/11/19");
    let locks = agent_dir.join("thread-writer-locks");
    fs::create_dir_all(&sessions).unwrap();
    fs::create_dir_all(&locks).unwrap();
    let rollout = sessions.join(format!("rollout-2026-11-19T00-00-00-{THREAD_ID}.jsonl"));
    let lock = locks.join(format!("{THREAD_ID}.lock"));
    append_turn_event(&rollout, "task_started", FIRST_TURN);
    let writer = CodexWriter::lock(&lock);

    let mut application = crate::Application::open(crate::Options {
        home_directory: home.join("app"),
        resources_directory: home.to_path_buf(),
    })
    .unwrap();
    let core = &mut application.runtime;
    core.runtime_instances = vec![RuntimeInstance {
        enabled: true,
        id: RUNTIME_ID.into(),
        name: "Synthetic Codex".into(),
        type_id: RuntimeKind::Codex166,
        gateway_id: String::new(),
        settings: BTreeMap::from([("agentDir".into(), agent_dir.to_str().unwrap().into())]),
    }];
    let conversation_id = format!("{RUNTIME_ID}:{THREAD_ID}");
    let snapshot = PiProjection::new(ConversationSummary {
        id: conversation_id.clone(),
        runtime_id: RUNTIME_ID.into(),
        title: crate::conversation::ConversationTitle::Untitled,
        cwd: None,
        created_at_unix_ms: None,
        updated_at_unix_ms: None,
        native_read_state: None,
        can_rename: false,
        can_delete: false,
        parent_conversation_id: None,
    })
    .snapshot
    .unwrap();
    assert_eq!(snapshot.run_state, RunState::Idle);
    core.session.set_history(snapshot);

    // An open history view must show external activity before the sidebar polls.
    let first = core.current_snapshot().unwrap();
    assert_eq!(first.run_state, RunState::Elsewhere);
    assert!(!first.actions.can_send);
    assert_external_activity(core, &conversation_id);

    // A loaded thread can retain its writer lock after its turn completes.
    append_turn_event(&rollout, "task_complete", FIRST_TURN);
    core.elsewhere.at = None;
    assert_eq!(core.current_snapshot().unwrap().run_state, RunState::Idle);
    assert!(core.active_conversations().is_empty());

    append_turn_event(&rollout, "task_started", SECOND_TURN);
    core.elsewhere.at = None;
    assert_external_activity(core, &conversation_id);
    assert_eq!(
        core.current_snapshot().unwrap().run_state,
        RunState::Elsewhere
    );

    // A continued conversation's public id differs from its running tail.
    let logical_id = "logical:external-codex";
    let link: crate::config::ConversationLink = serde_json::from_value(json!({
        "id": logical_id,
        "segments": [{
            "runtimeInstanceId": RUNTIME_ID,
            "runtimeTypeId": "codex-0.166",
            "nativeConversationId": conversation_id,
            "deleted": false,
        }],
    }))
    .unwrap();
    let native = core.session.projected().unwrap().clone();
    let mut origin = native.conversation.clone();
    origin.id = logical_id.into();
    core.conversation_links.push(link.clone());
    core.present_history(
        native,
        Some(super::super::continuation::LogicalProjection {
            link,
            origin,
            prefix: Vec::new(),
            retained_reply_ids: Vec::new(),
            work_notification_ids: Vec::new(),
            executions: Default::default(),
            sealed_sources: Default::default(),
            retention: Default::default(),
        }),
    );
    assert_external_activity(core, logical_id);
    let linked = core.current_snapshot().unwrap();
    assert_eq!(linked.conversation.id, logical_id);
    assert_eq!(linked.run_state, RunState::Elsewhere);

    // The same physical observation is excluded while local writes settle.
    core.session.last_execution = Some(Instant::now());
    assert!(core.active_conversations().is_empty());
    assert_eq!(core.current_snapshot().unwrap().run_state, RunState::Idle);
    core.session.last_execution = None;
    assert_external_activity(core, logical_id);

    // A stopped writer leaves its lock file and unfinished turn record behind.
    drop(writer);
    assert!(lock.is_file());
    core.elsewhere.at = None;
    assert!(core.active_conversations().is_empty());
    assert_eq!(core.current_snapshot().unwrap().run_state, RunState::Idle);
}
