//! Send into an existing conversation through the public service and a synthetic Codex peer.
#![cfg(unix)]

use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    time::{Duration, Instant},
};
use velune_application::{
    ApplicationService, ClientViewId, Command, ComposedTurn, DeliveredBlock, DeliveredSnapshot,
    ModelSelection, Options, Response, RuntimeExecution, RuntimeHome, SnapshotUpdate,
    SubmissionReceipt,
    config::RuntimeInstance,
    conversation::{MessageRole, RunState, RuntimeKind},
};
use velune_test_support::TempRoot;

const CONVERSATION: &str = "synthetic:abcd";
const NEW_TEXT: &str = "new question";
const PAGED_MESSAGES: usize = 2500;

const CODEX: &str = r#"#!/usr/bin/env python3
import json, pathlib, sys, time

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
paged = (root / 'paged').is_file()
count = int((root / 'paged').read_text()) if paged else 0
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
                             'modelProvider': 'openai', 'historyMode': 'paginated' if paged else 'legacy'}}
        if method == 'thread/read' and params.get('includeTurns'):
            assert not paged
            result['thread']['turns'] = [history]
    elif method == 'thread/items/list':
        assert paged
        start = int(params.get('cursor', '0'))
        end = min(start + params['limit'], count)
        result = {'data': [{'turnId': 'prior-turn', 'item':
                            {'id': f'prior-{i}', 'type': 'agentMessage', 'text': f'old answer {i}'}}
                           for i in range(start, end)],
                  'nextCursor': str(end) if end < count else None}
        if start == 0:
            result['data'][0]['item'] = history['items'][0]
    elif method == 'thread/turns/list':
        assert paged
        result = {'data': [{'id': 'prior-turn', 'status': 'completed'}], 'nextCursor': None}
    elif method == 'thread/fork':
        assert params['threadId'] == 'abcd' and params['ephemeral']
        result = {'thread': {'id': 'side-thread', 'cwd': str(root)}}
    elif method == 'turn/start':
        with (root / 'turns.jsonl').open('a') as wire:
            wire.write(line)
        context = {'threadId': params['threadId'], 'turnId': 'new-turn'}
        result = {'turn': {'id': 'new-turn'}}
        side = params['threadId'] == 'side-thread'
        if side:
            emit({'id': request['id'], 'result': result})
        emit({'method': 'turn/started', 'params': {**context, 'turn': {'id': 'new-turn'}}})
        emit({'method': 'item/completed', 'params': {**context, 'item': {
            'id': 'new-user', 'type': 'userMessage', 'content': params['input']}}})
        if side:
            deadline = time.monotonic() + 8
            while not (root / 'release-side-reply').exists():
                if time.monotonic() >= deadline:
                    sys.exit(2)
                time.sleep(0.005)
            emit({'method': 'item/completed', 'params': {**context, 'item': {
                'id': 'side-reply', 'type': 'agentMessage', 'text': 'side answer'}}})
        emit({'method': 'turn/completed', 'params': {**context, 'turn': {
            'id': 'new-turn', 'status': 'completed'}}})
        if side:
            continue
    emit({'id': request['id'], 'result': result})
"#;

struct Fixture {
    root: TempRoot,
    service: ApplicationService,
}

impl Fixture {
    fn new() -> Self {
        Self::with_history(None)
    }

    fn with_history(paged: Option<usize>) -> Self {
        let root = TempRoot::new("send-turn");
        let home = root.join("home");
        fs::create_dir(&home).unwrap();
        let binary = root.join("codex");
        fs::write(&binary, CODEX).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        if let Some(count) = paged {
            fs::write(root.join("paged"), count.to_string()).unwrap();
        }
        let service = ApplicationService::open(Options {
            home_directory: root.join("app"),
            resources_directory: root.to_path_buf(),
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

#[test]
fn send_turn_keeps_long_paginated_history_and_dispatches_once() {
    let fixture = Fixture::with_history(Some(PAGED_MESSAGES));
    let before = fixture.snapshot_when(|snapshot| snapshot.actions.can_send);
    assert_eq!(before.messages.len(), PAGED_MESSAGES);
    let Response::SendTurn(result) = fixture.execute(Command::SendTurn {
        submission_id: None,
        source_runtime_id: "synthetic".into(),
        target_runtime_id: "synthetic".into(),
        conversation_id: CONVERSATION.into(),
        selection: None,
        turn: ComposedTurn {
            text: NEW_TEXT.into(),
            effort: None,
            permission_mode: None,
            attachments: vec![],
        },
    }) else {
        panic!("expected a send response")
    };
    let settled = |snapshot: &DeliveredSnapshot| {
        snapshot.run_state == RunState::Idle && snapshot.messages.len() == PAGED_MESSAGES + 1
    };
    let after = result
        .snapshot
        .filter(settled)
        .unwrap_or_else(|| fixture.snapshot_when(settled));
    assert_eq!(after.messages[..PAGED_MESSAGES], before.messages);
    assert_eq!(after.messages.last().unwrap().id, "new-user");
    let wire = fs::read_to_string(fixture.root.join("turns.jsonl")).unwrap();
    assert_eq!(wire.lines().count(), 1);
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Close the service so its RPC owners reap their exact child processes.
        let _ = self.service.close(true);
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

#[test]
fn side_chat_delivers_a_delayed_reply_and_settles_through_snapshot_polling() {
    let fixture = Fixture::new();
    let parent = fixture.snapshot_when(|snapshot| snapshot.actions.can_send);
    let Response::OpenSideChat(result) = fixture.execute(Command::OpenSideChat {
        runtime_id: "synthetic".into(),
        conversation_id: CONVERSATION.into(),
        selection: None,
        effort: None,
        permission_mode: None,
    }) else {
        panic!("expected a side chat response")
    };
    let opened = result.snapshot.unwrap();
    let side_id = opened.conversation.id;
    assert_ne!(side_id, CONVERSATION);
    assert!(opened.messages.is_empty());

    let Response::SendSideChat(result) = fixture.execute(Command::SendSideChat {
        side_chat_id: side_id.clone(),
        target_runtime_id: "synthetic".into(),
        selection: None,
        turn: ComposedTurn {
            text: "side question".into(),
            effort: None,
            permission_mode: None,
            attachments: vec![],
        },
    }) else {
        panic!("expected a side chat send response")
    };
    let sent = result.snapshot.unwrap();
    assert!(!sent.actions.can_send);

    let runtime_id = sent.conversation.runtime_id;
    let poll = |since| {
        let Response::ReadSnapshotUpdate(update) = fixture.execute(Command::ReadSnapshotUpdate {
            runtime_id: runtime_id.clone(),
            conversation_id: side_id.clone(),
            since,
        }) else {
            panic!("expected a side chat update")
        };
        update
    };
    assert!(!matches!(poll(sent.cursor), SnapshotUpdate::Absent));
    fs::write(fixture.root.join("release-side-reply"), "").unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let settled = loop {
        if let SnapshotUpdate::Changed { snapshot } = poll(sent.cursor)
            && snapshot.run_state == RunState::Idle
        {
            break snapshot;
        }
        assert!(Instant::now() < deadline, "side chat did not settle");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(settled.conversation.id, side_id);
    assert!(settled.actions.can_send);
    assert!(settled.messages.iter().any(|message| {
        message.role == MessageRole::Assistant
            && message.blocks
                == [DeliveredBlock::Text {
                    text: "side answer".into(),
                }]
    }));
    assert!(matches!(poll(settled.cursor), SnapshotUpdate::Unchanged));

    let Response::ReadSnapshot(result) = fixture.execute(Command::ReadSnapshot {
        runtime_id,
        conversation_id: side_id,
    }) else {
        panic!("expected a full side chat snapshot")
    };
    assert_eq!(result.snapshot.unwrap().message_ids, settled.message_ids);
    assert_eq!(
        fixture
            .snapshot_when(|snapshot| snapshot.actions.can_send)
            .messages,
        parent.messages
    );
}
