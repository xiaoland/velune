//! Send into an existing conversation through the public service and a synthetic Codex peer.
#![cfg(unix)]

use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
    time::{Duration, Instant},
};
use velune_application::{
    ApplicationService, ClientViewId, Command, ComposedTurn, DeliveredBlock, DeliveredSnapshot,
    ModelSelection, Options, Response, RuntimeExecution, RuntimeHome, SubmissionReceipt,
    config::RuntimeInstance,
    conversation::{MessageRole, RunState, RuntimeKind},
};

const CONVERSATION: &str = "synthetic:abcd";
const NEW_TEXT: &str = "new question";

const CODEX: &str = r#"#!/usr/bin/env python3
import json, pathlib, sys

root = pathlib.Path(__file__).resolve().parent
if '--version' in sys.argv:
    print('codex-cli 0.166.0')
    sys.exit(0)
assert pathlib.Path.home().resolve() == root / 'home'

def emit(record):
    print(json.dumps(record), flush=True)

history = {'id': 'prior-turn', 'status': 'completed', 'items': [
    {'id': 'prior-user', 'type': 'userMessage', 'content': [{'type': 'text', 'text': 'old question'}]},
    {'id': 'prior-assistant', 'type': 'agentMessage', 'text': 'old answer'},
]}
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request['method']
    params = request.get('params', {})
    result = {}
    if method == 'config/read':
        result = {'config': {'model_provider': 'openai'}}
    elif method in ('thread/read', 'thread/resume'):
        result = {'thread': {'id': 'abcd', 'cwd': str(root), 'name': 'Synthetic',
                             'modelProvider': 'openai', 'historyMode': 'legacy'}}
        if method == 'thread/read' and params.get('includeTurns'):
            result['thread']['turns'] = [history]
    elif method == 'turn/start':
        with (root / 'turns.jsonl').open('a') as wire:
            wire.write(line)
        context = {'threadId': params['threadId'], 'turnId': 'new-turn'}
        emit({'method': 'turn/started', 'params': {**context, 'turn': {'id': 'new-turn'}}})
        emit({'method': 'item/completed', 'params': {**context, 'item': {
            'id': 'new-user', 'type': 'userMessage', 'content': params['input']}}})
        emit({'method': 'turn/completed', 'params': {**context, 'turn': {
            'id': 'new-turn', 'status': 'completed'}}})
        result = {'turn': {'id': 'new-turn'}}
    emit({'id': request['id'], 'result': result})
"#;

struct Fixture {
    root: PathBuf,
    service: ApplicationService,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "velune-send-turn-{:016x}",
            getrandom::u64().unwrap()
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let home = root.join("home");
        fs::create_dir(&home).unwrap();
        let binary = root.join("codex");
        fs::write(&binary, CODEX).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let service = ApplicationService::open(Options {
            home_directory: root.join("app"),
            resources_directory: root.clone(),
        })
        .unwrap();
        let fixture = Self { root, service };
        fixture.execute(Command::UpsertRuntimeWithExecution {
            runtime: RuntimeInstance {
                enabled: true,
                id: "synthetic".into(),
                name: "Synthetic Codex".into(),
                type_id: RuntimeKind::Codex166,
                gateway_id: "default".into(),
                settings: BTreeMap::from([
                    ("binary".into(), binary.display().to_string()),
                    ("nodeBinary".into(), "/synthetic/unused-node".into()),
                    ("agentDir".into(), home.join(".codex").display().to_string()),
                ]),
            },
            execution: RuntimeExecution {
                home: RuntimeHome::Existing {
                    path: home.display().to_string(),
                },
                ..Default::default()
            },
        });
        fixture
    }

    fn execute(&self, command: Command) -> Response {
        self.service
            .execute(ClientViewId("sender".into()), command)
            .unwrap()
    }

    fn snapshot_when(&self, ready: impl Fn(&DeliveredSnapshot) -> bool) -> DeliveredSnapshot {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let Response::ReadSnapshot(result) = self.execute(Command::ReadSnapshot {
                runtime_id: "synthetic".into(),
                conversation_id: CONVERSATION.into(),
            }) else {
                panic!("expected a snapshot response")
            };
            let snapshot = result.snapshot.unwrap();
            if ready(&snapshot) {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "unexpected final snapshot: {snapshot:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Close the service so its RPC owners reap their exact child processes.
        let _ = self.service.close(true);
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn send_turn_writes_once_and_binds_request_settings_to_the_new_user_echo() {
    let fixture = Fixture::new();
    let before = fixture.snapshot_when(|s| s.actions.can_send);
    let [prior] = before.transcript_turns.as_slice() else {
        panic!("expected a historical turn")
    };
    let selection = Some(ModelSelection::Native {
        provider_id: Some("openai".into()),
        model_id: "synthetic-model".into(),
    });
    let Response::SendTurn(result) = fixture.execute(Command::SendTurn {
        submission_id: None,
        source_runtime_id: "synthetic".into(),
        target_runtime_id: "synthetic".into(),
        conversation_id: CONVERSATION.into(),
        selection: selection.clone(),
        turn: ComposedTurn {
            text: NEW_TEXT.into(),
            effort: Some("high".into()),
            permission_mode: Some("read-only".into()),
            attachments: vec![],
        },
    }) else {
        panic!("expected a send response")
    };
    assert_eq!(result.submission_receipt, Some(SubmissionReceipt::Sending));

    let settled =
        |s: &DeliveredSnapshot| s.run_state == RunState::Idle && s.transcript_turns.len() >= 2;
    let after = result
        .snapshot
        .filter(settled)
        .unwrap_or_else(|| fixture.snapshot_when(settled));
    let users: Vec<_> = after
        .messages
        .iter()
        .filter(|m| m.role == MessageRole::User)
        .collect();
    let [old, new] = users.as_slice() else {
        panic!("expected exactly two user messages: {users:?}")
    };
    assert_eq!(old.id, prior.user_message_id);
    assert_eq!(new.id, "new-user", "expected the native user echo");
    assert_eq!(
        old.blocks,
        [DeliveredBlock::Text {
            text: "old question".into()
        }]
    );
    assert_eq!(
        new.blocks,
        [DeliveredBlock::Text {
            text: NEW_TEXT.into()
        }]
    );
    let [first, second] = after.transcript_turns.as_slice() else {
        panic!("expected two transcript turns")
    };
    assert_eq!(first.user_message_id, old.id);
    assert_eq!(first.settings, prior.settings);
    assert_eq!(second.user_message_id, new.id);
    let settings = second.settings.as_ref().expect("new turn request settings");
    assert_eq!(settings.model_selection, selection);
    assert_eq!(settings.effort.as_deref(), Some("high"));
    assert_eq!(settings.permission_mode.as_deref(), Some("read-only"));

    let wire = fs::read_to_string(fixture.root.join("turns.jsonl")).unwrap();
    let turns: Vec<Value> = wire
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let [record] = turns.as_slice() else {
        panic!("expected one turn/start: {wire}")
    };
    let params = &record["params"];
    assert_eq!(params["threadId"], "abcd");
    assert_eq!(params["input"], json!([{"type": "text", "text": NEW_TEXT}]));
    assert_eq!(params["model"], "synthetic-model");
    assert_eq!(params["effort"], "high");
    assert_eq!(params["sandboxPolicy"]["type"], "readOnly");
}
