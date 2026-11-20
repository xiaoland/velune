use super::*;
use crate::{DeliveredSnapshot, TranscriptCursor, conversation::InputAccess};
use std::time::{Duration, Instant};
use velune_agent_runtime::work::BackgroundState;

fn wait_for_child_work(
    parent: &mut session::Session,
    child: &str,
    expected: Option<BackgroundState>,
) {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        parent.pi().unwrap().poll();
        let work = parent.work().unwrap();
        if work.iter().any(|task| task.conversation_id.is_none())
            && work
                .iter()
                .find(|task| task.conversation_id.as_deref() == Some(child))
                .map(|task| task.state)
                == expected
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Pi did not report {expected:?} for child: {work:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn assert_child_view(view: &DeliveredSnapshot, state: RunState) {
    assert_eq!(view.run_state, state);
    assert!(view.messages.is_empty() && view.message_ids.is_empty());
    assert_eq!(view.actions.input_access, InputAccess::ReadOnly);
    assert!(!view.actions.can_send && !view.actions.can_cancel);
    assert!(!view.follow_up.can_queue && !view.follow_up.can_steer);
}

fn next_child_view(core: &mut CoreRuntime, since: TranscriptCursor) -> DeliveredSnapshot {
    let snapshot = core.snapshot_result().unwrap().snapshot.unwrap();
    assert_eq!(snapshot.cursor.generation, since.generation);
    assert!(snapshot.cursor.sequence > since.sequence);
    snapshot
}

#[test]
fn child_footer_activity_follows_exact_pi_work_without_transcript_changes() {
    let root =
        std::env::temp_dir().join(format!("velune-child-activity-{}", crate::new_record_key()));
    fs::create_dir(&root).unwrap();
    let helper = root.join("rpc.py");
    fs::write(
        &helper,
        r#"import json, pathlib, select, sys

root = pathlib.Path(__file__).parent

def publish(state):
    children = [{'sessionFile': str(root / 'sibling.jsonl'), 'state': 'running', 'canStop': True}]
    if state != 'finished':
        children.append({'sessionFile': str(root / 'child.jsonl'), 'state': state, 'canStop': False})
    frame = {'version': 1, 'kind': 'snapshot', 'tasks': [{'id': 'shell', 'status': 'running'}], 'children': children}
    print(json.dumps({'type': 'extension_ui_request', 'method': 'setStatus',
        'statusKey': 'velune.background-work-v1', 'statusText': json.dumps(frame)}), flush=True)

last = None
while True:
    if select.select([sys.stdin], [], [], 0.02)[0]:
        line = sys.stdin.readline()
        if not line:
            break
        request = json.loads(line)
        kind = request['type']
        if kind == 'get_commands':
            data = {'commands': [{'name': 'velune-background-work'}]}
        elif kind == 'prompt':
            assert request['message'].startswith('/velune-background-work ')
            last = 'running'
            publish(last)
            data = {'disposition': 'handled'}
        else:
            raise AssertionError(kind)
        print(json.dumps({'type': 'response', 'id': request['id'], 'command': kind,
            'success': True, 'data': data}), flush=True)
    if last is not None:
        state = 'finished' if (root / 'finished').exists() else 'stopping' if (root / 'stopping').exists() else 'running'
        if state != last:
            publish(state)
            last = state
"#,
    )
    .unwrap();
    let mut application = crate::Application::open(crate::Options {
        home_directory: root.clone(),
        resources_directory: root.clone(),
    })
    .unwrap();
    let summary = |id: String, parent_conversation_id| ConversationSummary {
        id,
        runtime_id: "pi".into(),
        title: crate::conversation::ConversationTitle::Untitled,
        cwd: Some(root.to_string_lossy().into_owned()),
        updated_at_unix_ms: None,
        created_at_unix_ms: None,
        parent_conversation_id,
        native_read_state: None,
        can_rename: false,
        can_delete: false,
    };
    let parent_id = "pi:parent".to_owned();
    let child_id = format!("pi:{}", root.join("child.jsonl").display());
    let sibling_id = format!("pi:{}", root.join("sibling.jsonl").display());
    let config: PiConfig = serde_json::from_value(json!({
        "execution":{"kind":"local"}, "os_home":root, "working_dir":root,
        "binary":"/usr/bin/python3", "node_binary":"/usr/bin/python3", "rpc_entry":helper,
    }))
    .unwrap();
    let client = pi::Client::spawn(config.clone()).unwrap();
    let mut parent = pi::PiSession::new(config);
    parent.start(client, PiProjection::new(summary(parent_id.clone(), None)));
    let core = &mut application.runtime;
    core.runtime_instances = vec![RuntimeInstance {
        enabled: true,
        id: "pi".into(),
        name: "Synthetic Pi".into(),
        type_id: RuntimeKind::Pi,
        gateway_id: String::new(),
        settings: Default::default(),
    }];
    core.session.active_state = ActiveState::Pi(Box::new(parent));
    core.focus_new();
    let child = PiProjection::new(summary(child_id.clone(), Some(parent_id)))
        .snapshot
        .unwrap();
    core.session.set_history(child);

    wait_for_child_work(
        &mut core.parked[0],
        &child_id,
        Some(BackgroundState::Running),
    );
    assert!(!core.parked[0].turn_running());
    // The first delivery has only the native Idle history and the owner's live work.
    let first = core.snapshot_result().unwrap().snapshot.unwrap();
    assert_child_view(&first, RunState::Elsewhere);

    fs::write(root.join("stopping"), []).unwrap();
    wait_for_child_work(
        &mut core.parked[0],
        &child_id,
        Some(BackgroundState::Stopping),
    );
    let stopping = next_child_view(core, first.cursor);
    assert_child_view(&stopping, RunState::Stopping);

    fs::write(root.join("finished"), []).unwrap();
    wait_for_child_work(&mut core.parked[0], &child_id, None);
    let ended = next_child_view(core, stopping.cursor);
    assert_child_view(&ended, RunState::Idle);
    assert!(core.parked[0].work().unwrap().iter().any(|task| {
        task.conversation_id.as_deref() == Some(sibling_id.as_str())
            && task.state == BackgroundState::Running
    }));
    assert_eq!(core.native_snapshot().unwrap().run_state, RunState::Idle);
    assert!(!root.join("child.jsonl").exists());

    core.parked[0].pi().unwrap().shutdown().unwrap();
    drop(application);
    fs::remove_dir_all(root).unwrap();
}
