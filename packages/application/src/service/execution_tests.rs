//! A real protocol wait must not occupy the application owner.
use super::*;
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use velune_test_support::TempRoot;

struct Fixture(TempRoot);
impl Fixture {
    fn new() -> Self {
        let root = TempRoot::new("control-gate");
        let binary = root.join("claude");
        fs::write(
            &binary,
            r#"#!/usr/bin/env python3
import json, os, pathlib, sys, time
root = pathlib.Path(__file__).parent
if '--version' in sys.argv:
    print('2.1.286 (Claude Code)')
    sys.exit(0)
for line in sys.stdin:
    record = json.loads(line)
    if record.get('type') == 'user':
        text = record['message']['content'][0]['text']
        with (root / 'steer-wire').open('a') as log:
            log.write(text + '\n')
        if record.get('uuid'):
            lifecycle = {'type': 'command_lifecycle', 'command_uuid': record['uuid'],
                         'session_id': record['session_id']}
            if text == 'same settings follow-up':
                print(json.dumps(dict(lifecycle, state='queued')), flush=True)
                (root / 'composed-queued').touch()
                deadline = time.monotonic() + 8
                while not (root / 'composed-release').exists():
                    if time.monotonic() >= deadline: sys.exit(2)
                    time.sleep(0.005)
            print(json.dumps(dict(lifecycle, state='started')), flush=True)
        if text in ('synthetic insert', 'synthetic queue insert'):
            (root / text.replace(' ', '-')).touch()
        if text in ('start background work', 'same settings follow-up'):
            with (root / 'background-owner').open('a') as log:
                log.write(str(os.getpid()) + '\n')
            if text == 'start background work':
                print(json.dumps({'type': 'system', 'subtype': 'task_started',
                                  'task_id': 'synthetic-work', 'task_type': 'local_workflow',
                                  'description': 'Synthetic background work'}), flush=True)
            print(json.dumps({'type': 'result', 'subtype': 'success', 'is_error': False,
                              'session_id': record['session_id']}), flush=True)
            print(json.dumps({'type': 'system', 'subtype': 'session_state_changed', 'state': 'idle'}),
                  flush=True)
    if record.get('type') != 'control_request':
        continue
    request = record['request']
    subtype = request['subtype']
    response = {'models': []}
    result_type = 'success'
    if subtype == 'interrupt':
        attempt = len(list(root.glob('entered-*'))) + 1
        with (root / 'wire').open('a') as log:
            log.write('interrupt\n')
        (root / ('entered-' + str(attempt))).touch()
        deadline = time.monotonic() + 8
        while not (root / ('release-' + str(attempt))).exists():
            if time.monotonic() >= deadline:
                sys.exit(2)
            time.sleep(0.005)
        if attempt == 1:
            result_type = 'error'
    elif subtype == 'mcp_status':
        response = {'mcpServers': []}
    elif subtype == 'mcp_set_servers':
        response = {'errors': {}}
    elif subtype == 'set_permission_mode':
        with (root / 'steer-wire').open('a') as log:
            log.write(request['mode'] + '\n')
        (root / 'steer-entered').touch()
        deadline = time.monotonic() + 8
        while not (root / 'release-steer').exists():
            if time.monotonic() >= deadline:
                sys.exit(2)
            time.sleep(0.005)
    print(json.dumps({'type': 'control_response', 'response': {
        'subtype': result_type, 'request_id': record['request_id'], 'response': response
    }}), flush=True)
"#,
        )
        .unwrap();
        fs::set_permissions(binary, fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }
    fn codex() -> Self {
        let root = TempRoot::new("bootstrap-gate");
        let binary = root.join("codex");
        fs::write(
            &binary,
            r#"#!/usr/bin/env python3
import json, pathlib, sys, time
root = pathlib.Path(__file__).parent
if '--version' in sys.argv:
    print('codex-cli 0.166.0')
    sys.exit(0)
for line in sys.stdin:
    record = json.loads(line)
    if 'id' not in record:
        continue
    method = record['method']
    result = {}
    if method == 'config/read':
        result = {'config': {'model_provider': 'openai', 'model': 'synthetic-native'}}
    elif method == 'thread/start':
        with (root / 'wire').open('a') as log: log.write('thread/start\n')
        (root / 'entered-1').touch()
        deadline = time.monotonic() + 8
        while not (root / 'release-1').exists():
            if time.monotonic() >= deadline: sys.exit(2)
            time.sleep(0.005)
        result = {'thread': {'id': 'created-thread', 'cwd': str(root)}, 'model': 'synthetic-native'}
    print(json.dumps({'id': record['id'], 'result': result}), flush=True)
"#,
        )
        .unwrap();
        fs::set_permissions(binary, fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }

    fn codex_list_index() -> Self {
        let root = TempRoot::new("codex-list");
        root.dir("native/sessions/2026/11/23");
        let binary = root.join("codex");
        fs::write(
            &binary,
            r#"#!/usr/bin/env python3
import json, os, pathlib, sys, time
root = pathlib.Path(__file__).parent
if '--version' in sys.argv:
    print('codex-cli 0.166.0')
    sys.exit(0)
for line in sys.stdin:
    record = json.loads(line)
    if 'id' not in record:
        continue
    method = record['method']
    result = {}
    if method == 'project/list':
        result = {'data': [], 'nextCursor': None}
    elif method == 'thread/list':
        params = record['params']
        indexed = params.get('useStateDbOnly', False)
        with (root / 'list-wire').open('a') as log:
            log.write('indexed\n' if indexed else 'repair\n')
        database = root / 'list-db'
        if not indexed:
            (root / 'entered-1').touch()
            deadline = time.monotonic() + 8
            while not (root / 'release-1').exists():
                if time.monotonic() >= deadline: sys.exit(2)
                time.sleep(0.005)
            rows = [{'id': 'known-thread', 'cwd': str(root), 'preview': 'Original thread',
                     'createdAt': 100, 'updatedAt': 100, 'historyMode': 'paginated'}]
            if any((pathlib.Path(os.environ['CODEX_HOME']) / 'sessions').rglob('rollout-*.jsonl')):
                rows.append({'id': 'imported-thread', 'cwd': str(root), 'preview': 'Imported thread',
                             'createdAt': 90, 'updatedAt': 90, 'historyMode': 'paginated'})
            database.write_text(json.dumps(rows))
        rows = json.loads(database.read_text()) if database.exists() else []
        result = {'data': rows, 'nextCursor': None}
    elif method == 'thread/read':
        result = {'thread': {'id': record['params']['threadId'], 'cwd': str(root),
                             'preview': 'Original thread', 'historyMode': 'paginated'}}
    elif method == 'thread/items/list':
        item = {'id': 'message', 'type': 'agentMessage', 'text': 'Readable while indexing'}
        result = {'data': [{'turnId': 'turn', 'item': item}], 'nextCursor': None}
    elif method == 'thread/turns/list':
        result = {'data': [{'id': 'turn', 'status': 'completed'}], 'nextCursor': None}
    print(json.dumps({'id': record['id'], 'result': result}), flush=True)
"#,
        )
        .unwrap();
        fs::set_permissions(binary, fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }

    fn claude_naming() -> Self {
        let root = TempRoot::new("claude-naming");
        let binary = root.join("claude");
        fs::write(
            &binary,
            r#"#!/usr/bin/env python3
import datetime, json, os, pathlib, re, sys, uuid

if '--version' in sys.argv:
    print('2.1.286 (Claude Code)')
    sys.exit(0)

for line in sys.stdin:
    record = json.loads(line)
    if record['type'] == 'control_request':
        response = {'models': [], 'mcpServers': [], 'errors': {}}
        print(json.dumps({'type': 'control_response', 'response': {
            'subtype': 'success', 'request_id': record['request_id'], 'response': response
        }}), flush=True)
    elif record['type'] == 'user':
        session = sys.argv[sys.argv.index('--session-id') + 1]
        cwd = str(pathlib.Path.cwd())
        project = pathlib.Path(os.environ['CLAUDE_CONFIG_DIR']) / 'projects' / re.sub(r'[^a-zA-Z0-9]', '-', cwd)
        project.mkdir(parents=True, exist_ok=True)
        now = datetime.datetime.now(datetime.timezone.utc).isoformat()
        user = {'type': 'user', 'uuid': record['uuid'], 'parentUuid': None,
                'sessionId': session, 'cwd': cwd, 'timestamp': now, 'message': record['message']}
        assistant = {'type': 'assistant', 'uuid': str(uuid.uuid4()), 'parentUuid': user['uuid'],
                     'sessionId': session, 'session_id': session, 'cwd': cwd, 'timestamp': now,
                     'message': {'role': 'assistant', 'id': 'reply', 'stop_reason': 'end_turn',
                                 'content': [{'type': 'text', 'text': 'synthetic assistant reply'}]}}
        path = project / (session + '.jsonl')
        with path.open('w') as history:
            for row in [user, assistant]:
                history.write(json.dumps(row) + '\n')
        print(json.dumps(assistant), flush=True)
        print(json.dumps({'type': 'result', 'subtype': 'success', 'is_error': False,
                          'session_id': session}), flush=True)
        # Claude's title can be persisted separately from the stdout transcript.
        if record['message']['content'][0]['text'] == 'native titled prompt':
            with path.open('a') as history:
                history.write(json.dumps({'type': 'ai-title', 'sessionId': session,
                                          'aiTitle': 'Native Claude title'}) + '\n')
        print(json.dumps({'type': 'system', 'subtype': 'session_state_changed', 'state': 'idle'}),
              flush=True)
"#,
        )
        .unwrap();
        fs::set_permissions(binary, fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }

    fn wait(&self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.0.join(name).exists() {
            assert!(
                Instant::now() < deadline,
                "synthetic helper did not enter {name}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Release a helper even when an assertion fails, then let its exact
        // ProcessTree owner perform cleanup. No process-name termination.
        for attempt in 1..=2 {
            let _ = fs::write(self.0.join(format!("release-{attempt}")), []);
        }
    }
}

fn request_later(
    service: &ApplicationService,
    view: &str,
    command: Command,
) -> mpsc::Receiver<Result<Response, Failure>> {
    let (reply, result) = mpsc::channel();
    let service = service.clone();
    let view = ClientViewId(view.into());
    std::thread::spawn(move || {
        let _ = reply.send(service.execute(view, command));
    });
    result
}

fn assert_read_before_release(service: &ApplicationService) {
    let result = request_later(
        service,
        "other-view",
        Command::ReadRuntimeHomeDirectory {
            runtime_id: "synthetic".into(),
        },
    );
    assert!(matches!(
        result
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap(),
        Response::ReadRuntimeHomeDirectory(_)
    ));
}

#[test]
fn codex_startup_prewarms_the_active_list_without_blocking_the_owner() {
    let fixture = Fixture::codex_list_index();
    let options = Options {
        home_directory: fixture.0.join("app"),
        resources_directory: fixture.0.to_path_buf(),
    };
    let mut application = Application::open(options.clone()).unwrap();
    application
        .runtime
        .upsert_runtime_with_execution(
            crate::config::RuntimeInstance {
                enabled: true,
                id: "synthetic".into(),
                name: "Synthetic Codex".into(),
                type_id: crate::conversation::RuntimeKind::Codex166,
                gateway_id: "default".into(),
                settings: BTreeMap::from([
                    (
                        "binary".into(),
                        fixture.0.join("codex").display().to_string(),
                    ),
                    (
                        "agentDir".into(),
                        fixture.0.join("native").display().to_string(),
                    ),
                    ("nodeBinary".into(), "/synthetic/not-executed-node".into()),
                ]),
            },
            crate::RuntimeExecution {
                home: crate::RuntimeHome::Existing {
                    path: fixture.0.display().to_string(),
                },
                ..Default::default()
            },
        )
        .unwrap();
    drop(application);

    let app = ApplicationService::open(options).unwrap();
    fixture.wait("entered-1");
    // Enqueue the list before the transcript, including on a busy scheduler.
    let (reply, first) = mpsc::channel();
    app.sender
        .send(Job::Execute {
            view: ClientViewId("list".into()),
            presentation_isolated: false,
            context: None,
            command: Box::new(Command::ReadConfiguration),
            cancellation: None,
            queued_at: Instant::now(),
            history_completion: app.sender.clone(),
            dispatch: tracing::dispatcher::get_default(Clone::clone),
            span: tracing::Span::none(),
            reply,
        })
        .unwrap();
    assert_read_before_release(&app);
    let transcript = request_later(
        &app,
        "transcript",
        Command::ReadConversation {
            runtime_id: "synthetic".into(),
            conversation_id: "synthetic:known-thread".into(),
        },
    );
    let read = transcript.recv_timeout(Duration::from_secs(3));
    let list_pending = matches!(first.try_recv(), Err(mpsc::TryRecvError::Empty));
    fs::write(fixture.0.join("release-1"), []).unwrap();
    assert!(list_pending);
    let Response::ReadConversation(history) = read.unwrap().unwrap() else {
        panic!("transcript response")
    };
    let history = history.snapshot.unwrap();
    assert_eq!(history.conversation.id, "synthetic:known-thread");
    assert_eq!(history.messages.len(), 1);

    let ids = |response| {
        let Response::ReadConfiguration(configuration) = response else {
            panic!("list response");
        };
        assert!(configuration.history_failures.is_empty());
        configuration
            .conversations
            .into_iter()
            .map(|row| row.conversation.id)
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        ids(first
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap()
            .response),
        BTreeSet::from(["synthetic:known-thread".into()])
    );
    let wire = fixture.0.join("list-wire");
    assert_eq!(fs::read_to_string(&wire).unwrap(), "repair\nindexed\n");

    fs::write(
        fixture.0.join(
            "native/sessions/2026/11/23/rollout-2026-11-23T01-02-03-00000000-0000-7000-8000-000000000001.jsonl",
        ),
        "{}\n",
    )
    .unwrap();
    let refresh = || {
        ids(app
            .execute(ClientViewId("list".into()), Command::ReadConfiguration)
            .unwrap())
    };
    let complete = BTreeSet::from([
        "synthetic:known-thread".into(),
        "synthetic:imported-thread".into(),
    ]);
    assert_eq!(refresh(), complete);
    assert_eq!(
        fs::read_to_string(&wire).unwrap(),
        "repair\nindexed\nrepair\n"
    );

    fs::write(fixture.0.join("list-db"), "[]").unwrap();
    assert_eq!(refresh(), complete);
    assert_eq!(
        fs::read_to_string(&wire).unwrap(),
        "repair\nindexed\nrepair\nindexed\nrepair\n"
    );
    app.close(false).unwrap();
}

#[test]
fn codex_active_index_persists_across_restarts_and_repairs_on_external_change() {
    let fixture = Fixture::codex_list_index();
    let options = Options {
        home_directory: fixture.0.join("app"),
        resources_directory: fixture.0.clone(),
    };
    let mut application = Application::open(options.clone()).unwrap();
    application
        .runtime
        .upsert_runtime_with_execution(
            crate::config::RuntimeInstance {
                enabled: true,
                id: "synthetic".into(),
                name: "Synthetic Codex".into(),
                type_id: crate::conversation::RuntimeKind::Codex166,
                gateway_id: "default".into(),
                settings: BTreeMap::from([
                    ("binary".into(), fixture.0.join("codex").display().to_string()),
                    (
                        "agentDir".into(),
                        fixture.0.join("native").display().to_string(),
                    ),
                    ("nodeBinary".into(), "/synthetic/not-executed-node".into()),
                ]),
            },
            crate::RuntimeExecution {
                home: crate::RuntimeHome::Existing {
                    path: fixture.0.display().to_string(),
                },
                ..Default::default()
            },
        )
        .unwrap();
    drop(application);

    let ids = |response| {
        let Response::ReadConfiguration(configuration) = response else {
            panic!("list response");
        };
        assert!(configuration.history_failures.is_empty());
        configuration
            .conversations
            .into_iter()
            .map(|row| row.conversation.id)
            .collect::<BTreeSet<_>>()
    };
    let wire = fixture.0.join("list-wire");

    // First launch: warms via repair and persists proof.
    fs::write(fixture.0.join("release-1"), []).unwrap();
    let app = ApplicationService::open(options.clone()).unwrap();
    assert_eq!(
        ids(app
            .execute(ClientViewId("list".into()), Command::ReadConfiguration)
            .unwrap()),
        BTreeSet::from(["synthetic:known-thread".into()])
    );
    assert_eq!(fs::read_to_string(&wire).unwrap(), "repair\nindexed\n");
    app.close(false).unwrap();

    let proof_dir = options.home_directory.join("codex-active-index");
    let proof_files = fs::read_dir(&proof_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(proof_files.len(), 1);
    let proof_path = proof_files[0].clone();

    // Second launch with unchanged rollouts: warmup skips Codex, list uses indexed path.
    fs::remove_file(&wire).unwrap();
    fs::remove_file(fixture.0.join("release-1")).unwrap();
    let app = ApplicationService::open(options.clone()).unwrap();
    assert_eq!(
        ids(app
            .execute(ClientViewId("list".into()), Command::ReadConfiguration)
            .unwrap()),
        BTreeSet::from(["synthetic:known-thread".into()])
    );
    assert_eq!(fs::read_to_string(&wire).unwrap(), "indexed\n");
    app.close(false).unwrap();

    // Third launch with externally wiped list-db: indexed fails result digest and repairs.
    fs::write(fixture.0.join("list-db"), "[]").unwrap();
    fs::remove_file(&wire).unwrap();
    fs::write(fixture.0.join("release-1"), []).unwrap();
    let app = ApplicationService::open(options.clone()).unwrap();
    assert_eq!(
        ids(app
            .execute(ClientViewId("list".into()), Command::ReadConfiguration)
            .unwrap()),
        BTreeSet::from(["synthetic:known-thread".into()])
    );
    assert_eq!(fs::read_to_string(&wire).unwrap(), "indexed\nrepair\n");
    app.close(false).unwrap();

    // Fourth launch with a new rollout file: warmup detects rollout change and repairs.
    fs::write(
        fixture.0.join(
            "native/sessions/2026/11/23/rollout-2026-11-23T01-02-03-00000000-0000-7000-8000-000000000001.jsonl",
        ),
        "{}\n",
    )
    .unwrap();
    fs::remove_file(&wire).unwrap();
    let complete = BTreeSet::from([
        "synthetic:known-thread".into(),
        "synthetic:imported-thread".into(),
    ]);
    let app = ApplicationService::open(options.clone()).unwrap();
    assert_eq!(
        ids(app
            .execute(ClientViewId("list".into()), Command::ReadConfiguration)
            .unwrap()),
        complete
    );
    assert_eq!(fs::read_to_string(&wire).unwrap(), "repair\nindexed\n");
    app.close(false).unwrap();

    // Fifth launch with unknown proof schema: falls back to repair and preserves original bytes.
    let unknown = br#"{"schemaVersion":99,"future":true}"#;
    fs::write(&proof_path, unknown).unwrap();
    fs::remove_file(&wire).unwrap();
    let app = ApplicationService::open(options).unwrap();
    assert_eq!(
        ids(app
            .execute(ClientViewId("list".into()), Command::ReadConfiguration)
            .unwrap()),
        complete
    );
    assert_eq!(fs::read_to_string(&wire).unwrap(), "repair\nindexed\n");
    assert_eq!(fs::read(&proof_path).unwrap(), unknown);
    app.close(false).unwrap();
}

#[test]
fn codex_native_creation_wait_keeps_owner_responsive_and_creates_once() {
    let fixture = Fixture::codex();
    let app = ApplicationService::open(Options {
        home_directory: fixture.0.join("app"),
        resources_directory: Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../agent-runtime/resources"),
    })
    .unwrap();
    app.execute(
        ClientViewId("creator".into()),
        Command::UpsertRuntimeWithExecution {
            runtime: crate::config::RuntimeInstance {
                enabled: true,
                id: "synthetic".into(),
                name: "Synthetic Codex".into(),
                type_id: crate::conversation::RuntimeKind::Codex166,
                gateway_id: "default".into(),
                settings: BTreeMap::from([
                    (
                        "binary".into(),
                        fixture.0.join("codex").to_string_lossy().into_owned(),
                    ),
                    // Required configuration field; Codex RPC never launches this helper.
                    ("nodeBinary".into(), "/synthetic/not-executed-node".into()),
                    (
                        "agentDir".into(),
                        fixture.0.join("native").to_string_lossy().into_owned(),
                    ),
                ]),
            },
            execution: crate::RuntimeExecution {
                home: crate::RuntimeHome::Managed { remote_root: None },
                ..Default::default()
            },
        },
    )
    .unwrap();
    let created = request_later(
        &app,
        "creator",
        Command::CreateConversation {
            runtime_id: "synthetic".into(),
            cwd: Some(fixture.0.to_string_lossy().into_owned()),
            project: None,
            selection: Some(crate::ModelSelection::Native {
                provider_id: Some("openai".into()),
                model_id: "synthetic-native".into(),
            }),
        },
    );
    fixture.wait("entered-1");
    assert_read_before_release(&app);
    assert!(matches!(created.try_recv(), Err(mpsc::TryRecvError::Empty)));
    fs::write(fixture.0.join("release-1"), []).unwrap();
    let Response::CreateConversation(result) = created
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap()
    else {
        panic!("creation response")
    };
    let snapshot = result.snapshot.unwrap();
    assert_eq!(snapshot.conversation.id, "synthetic:created-thread");
    assert_eq!(snapshot.context_runtime_id, "synthetic");
    assert_eq!(
        fs::read_to_string(fixture.0.join("wire")).unwrap(),
        "thread/start\n"
    );
    app.close(true).unwrap();
}

#[test]
fn native_control_wait_keeps_owner_responsive_and_rejection_retry_is_explicit() {
    let fixture = Fixture::new();
    let app = ApplicationService::open(Options {
        home_directory: fixture.0.join("app"),
        resources_directory: Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../agent-runtime/resources"),
    })
    .unwrap();
    let view = ClientViewId("control-view".into());
    app.execute(
        view.clone(),
        Command::UpsertRuntimeWithExecution {
            runtime: crate::config::RuntimeInstance {
                enabled: true,
                id: "synthetic".into(),
                name: "Synthetic Claude".into(),
                type_id: crate::conversation::RuntimeKind::ClaudeCode,
                gateway_id: "default".into(),
                settings: BTreeMap::from([
                    (
                        "binary".into(),
                        fixture.0.join("claude").to_string_lossy().into_owned(),
                    ),
                    (
                        "agentDir".into(),
                        fixture.0.join("native").to_string_lossy().into_owned(),
                    ),
                ]),
            },
            execution: crate::RuntimeExecution {
                home: crate::RuntimeHome::Managed { remote_root: None },
                ..Default::default()
            },
        },
    )
    .unwrap();
    let Response::CreateConversation(created) = app
        .execute(
            view.clone(),
            Command::CreateConversation {
                runtime_id: "synthetic".into(),
                cwd: Some(fixture.0.to_string_lossy().into_owned()),
                project: None,
                selection: None,
            },
        )
        .unwrap()
    else {
        panic!("create response")
    };
    let id = created.snapshot.unwrap().conversation.id;
    app.execute(
        view.clone(),
        Command::SendTurn {
            submission_id: None,
            source_runtime_id: "synthetic".into(),
            conversation_id: id.clone(),
            target_runtime_id: "synthetic".into(),
            selection: None,
            turn: crate::ComposedTurn {
                text: "synthetic input".into(),
                effort: None,
                permission_mode: None,
                attachments: Vec::new(),
            },
        },
    )
    .unwrap();

    let insert = request_later(
        &app,
        "control-view",
        Command::SteerTurn {
            runtime_id: "synthetic".into(),
            conversation_id: id.clone(),
            target_runtime_id: Some("synthetic".into()),
            turn: crate::ComposedTurn {
                text: "synthetic insert".into(),
                effort: Some("xhigh".into()),
                permission_mode: Some("bypassPermissions".into()),
                attachments: Vec::new(),
            },
        },
    );
    fixture.wait("steer-entered");
    assert_read_before_release(&app);
    assert!(matches!(insert.try_recv(), Err(mpsc::TryRecvError::Empty)));
    let steer_wire = fixture.0.join("steer-wire");
    assert_eq!(
        fs::read_to_string(&steer_wire).unwrap(),
        "synthetic input\nbypassPermissions\n"
    );
    fs::write(fixture.0.join("release-steer"), []).unwrap();
    assert!(matches!(
        insert
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap(),
        Response::SteerTurn(_)
    ));
    fixture.wait("synthetic-insert");
    assert_eq!(
        fs::read_to_string(&steer_wire).unwrap(),
        "synthetic input\nbypassPermissions\nsynthetic insert\n"
    );

    let queue = |operation| Command::EditSendQueue {
        submission_id: None,
        runtime_id: "synthetic".into(),
        conversation_id: id.clone(),
        operation,
    };
    app.execute(
        view.clone(),
        queue(crate::SendQueueOperation::Add {
            source_runtime_id: "synthetic".into(),
            target_runtime_id: "synthetic".into(),
            selection: None,
            turn: crate::ComposedTurn {
                text: "synthetic queue insert".into(),
                effort: None,
                permission_mode: Some("acceptEdits".into()),
                attachments: Vec::new(),
            },
        }),
    )
    .unwrap();
    let pending = control_snapshot(&app, &id).follow_up.queue;
    assert_eq!(pending.len(), 1);
    app.execute(
        view.clone(),
        queue(crate::SendQueueOperation::Steer {
            id: pending[0].id.clone(),
        }),
    )
    .unwrap();
    fixture.wait("synthetic-queue-insert");
    assert!(control_snapshot(&app, &id).follow_up.queue.is_empty());
    assert_eq!(
        fs::read_to_string(&steer_wire).unwrap(),
        "synthetic input\nbypassPermissions\nsynthetic insert\nacceptEdits\nsynthetic queue insert\n"
    );

    let cancel = || Command::Cancel {
        runtime_id: "synthetic".into(),
        conversation_id: id.clone(),
    };
    let first = request_later(&app, "control-view", cancel());
    fixture.wait("entered-1");
    assert_read_before_release(&app);
    assert!(matches!(first.try_recv(), Err(mpsc::TryRecvError::Empty)));
    fs::write(fixture.0.join("release-1"), []).unwrap();
    let rejected = match first.recv_timeout(Duration::from_secs(3)).unwrap() {
        Err(error) => error,
        Ok(_) => panic!("synthetic rejection must reach the caller"),
    };
    assert_eq!(rejected.kind, FailureKind::Invalid);
    let Response::ReadSnapshot(current) = app
        .execute(
            view.clone(),
            Command::ReadSnapshot {
                runtime_id: "synthetic".into(),
                conversation_id: id.clone(),
            },
        )
        .unwrap()
    else {
        panic!("snapshot response")
    };
    assert!(
        current.snapshot.unwrap().actions.can_cancel,
        "an explicitly rejected cancellation must leave an explicit retry available"
    );

    let second = request_later(&app, "control-view", cancel());
    fixture.wait("entered-2");
    assert_read_before_release(&app);
    fs::write(fixture.0.join("release-2"), []).unwrap();
    let Response::Cancel(stopped) = second
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap()
    else {
        panic!("cancel response")
    };
    assert_eq!(
        stopped.snapshot.unwrap().run_state,
        crate::conversation::RunState::Stopping
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("wire"))
            .unwrap()
            .lines()
            .count(),
        2,
        "only the two explicit user commands may write interrupt"
    );
    app.close(true).unwrap();
}

impl Fixture {
    fn codex_control(mode: &str) -> Self {
        let fixture = Self::codex();
        fs::write(fixture.0.join("mode"), mode).unwrap();
        fs::write(fixture.0.join("codex"), r#"#!/usr/bin/env python3
import json, os, pathlib, select, sys, time
root = pathlib.Path(__file__).parent
if '--version' in sys.argv:
    print('codex-cli 0.166.0'); sys.exit(0)
def emit(method, params):
    print(json.dumps({'method':method, 'params':params}), flush=True)
def ack(request, result):
    print(json.dumps({'id':request['id'], 'result':result}), flush=True)
def approval(turn='turn'):
    print(json.dumps({'id':'approval', 'method':'item/commandExecution/requestApproval',
        'params':{'threadId':'thread','turnId':turn,'command':'true','availableDecisions':['accept','decline']}}), flush=True)
held = None
held_send = None
held_steer = None
held_echo = None
flooded = False
current_turn = 'turn'
held_children = []
managed_background_active = False
attempt = 0
turn_count = 0
buffer = ''
while True:
    if managed_background_active and (root / 'finish-managed-background').exists():
        managed_background_active = False
        for child in ('child-a', 'child-b'):
            emit('turn/completed', {'threadId':child,'turn':{'id':'turn-' + child,'status':'completed'}})
            emit('thread/status/changed', {'threadId':child,'status':{'type':'idle'}})
        # Fill the pipe after the terminal events, forcing the Rust reader to
        # queue them before the read-only helper can unblock the Core owner.
        emit('synthetic/pipeBarrier', {'threadId':'thread','padding':'x' * (2 * 1024 * 1024)})
        (root / 'managed-background-finished').touch()
    if held_echo and (root / 'finish-without-echo').exists():
        emit('turn/completed', {'threadId':'thread','turn':{'id':current_turn,'status':'completed'}})
        held_echo = None
        (root / 'no-echo-terminal').touch()
    if held_echo and (root / 'release-echo').exists():
        emit('item/completed', {'threadId':'thread','turnId':current_turn,'item':{'id':'user-' + str(turn_count),'type':'userMessage','content':held_echo['params']['input']}})
        held_echo = None
        (root / 'echo-written').touch()
    if not flooded and (root / 'saturate-writer').exists():
        flooded = True
        for i in range(4):
            print(json.dumps({'id':str(i) + ('x' * (64 * 1024)), 'method':'synthetic/unsupported','params':{'threadId':'thread'}}), flush=True)
        emit('item/started', {'threadId':'thread','item':{'id':'writer-ready','type':'agentMessage','text':'synthetic writer receipts primed'}})
        (root / 'writer-blocked').touch()
        while root.exists() and not (root / 'fail-connection').exists():
            if (root / 'check-alive').exists(): (root / 'alive-before-cleanup').touch()
            time.sleep(0.005)
        sys.stdout.write('{malformed\n'); sys.stdout.flush()
        (root / 'failure-written').touch()
        while root.exists() and not (root / 'release-stdin').exists(): time.sleep(0.005)
    if held_send and (root / 'release-send').exists():
        ack(held_send, {'turn':{'id':current_turn}}); held_send = None
    if held_steer and (root / 'release-steer').exists():
        ack(held_steer, {}); held_steer = None
    if (root / 'check-alive').exists():
        (root / 'alive-before-cleanup').touch()
    if held_children and (root / 'release-children').exists():
        for child in held_children:
            print(json.dumps({'id':child['id'], 'error':{'code':-32600,'message':'child rejected'}}), flush=True)
        held_children.clear()
    if held and (root / ('release-' + str(attempt))).exists():
        mode = (root / 'mode').read_text()
        if mode == 'reject':
            print(json.dumps({'id':held['id'], 'error':{'code':-32600,'message':'parent rejected'}}), flush=True)
        else:
            ack(held, {})
        held = None
    if '\n' not in buffer:
        ready, _, _ = select.select([sys.stdin], [], [], 0.005)
        if not ready: continue
        chunk = os.read(0, 65536)
        if not chunk: break
        buffer += chunk.decode()
        if '\n' not in buffer: continue
    line, buffer = buffer.split('\n', 1)
    request = json.loads(line)
    with (root / 'wire').open('a') as log: log.write(json.dumps(request) + '\n')
    method = request.get('method')
    if method == 'initialize': ack(request, {})
    elif method == 'config/read': ack(request, {'config':{'model_provider':'changed-default' if (root / 'changed-default').exists() else 'openai','model':'synthetic-native'}})
    elif method == 'thread/start':
        with (root / 'thread-processes').open('a') as log: log.write(str(os.getpid()) + '\n')
        ack(request, {'thread':{'id':'thread','name':'Synthetic','cwd':str(root)},'model':'synthetic-native'})
    elif method == 'thread/resume':
        provider = 'velune' if (root / 'mode').read_text() == 'managed-queue-background' else 'openai'
        ack(request, {'thread':{'id':'thread','modelProvider':provider}})
    elif method == 'thread/read':
        if (root / 'reject-provider-check').exists():
            print(json.dumps({'id':request['id'], 'error':{'code':-32600,'message':'provider-check-required'}}), flush=True)
            continue
        thread = request['params']['threadId']
        provider = 'openai'
        if (root / 'mode').read_text() == 'managed-queue-background':
            provider = 'velune'
            armed = root / 'finish-background-during-provider-read'
            if thread == 'thread' and request['params'].get('includeTurns') is False and armed.exists():
                armed.unlink()
                (root / 'finish-managed-background').touch()
                deadline = time.monotonic() + 5
                while not (root / 'managed-background-finished').exists():
                    if time.monotonic() >= deadline: sys.exit(2)
                    time.sleep(0.005)
        ack(request, {'thread':{'id':thread,'modelProvider':provider,'historyMode':'legacy','turns':[],
            'parentThreadId':'thread' if thread.startswith('child-') else None}})
    elif method == 'turn/start':
        with (root / 'turn-processes').open('a') as log: log.write(str(os.getpid()) + '\n')
        turn_count += 1
        mode = (root / 'mode').read_text()
        if mode == 'managed-queue-background':
            text = request['params']['input'][0]['text']
            current_turn = 'managed-first' if text == 'start managed background' else 'managed-next'
            ack(request, {'turn':{'id':current_turn}})
            emit('turn/started', {'threadId':'thread','turn':{'id':current_turn}})
            emit('item/completed', {'threadId':'thread','turnId':current_turn,'item':{'id':'user-' + current_turn,'type':'userMessage','content':request['params']['input']}})
            if current_turn == 'managed-first':
                managed_background_active = True
                for child in ('child-a', 'child-b'):
                    emit('thread/status/changed', {'threadId':child,'status':{'type':'active','activeFlags':[]}})
                    emit('turn/started', {'threadId':child,'turn':{'id':'turn-' + child}})
            emit('turn/completed', {'threadId':'thread','turn':{'id':current_turn,'status':'completed'}})
            continue
        if mode == 'draft-first-reject':
            print(json.dumps({'id':request['id'], 'error':{'code':-32600,'message':'first input explicitly rejected'}}), flush=True)
            (root / 'send-entered').touch()
            continue
        if mode == 'draft-first-disconnect':
            (root / 'send-entered').touch()
            sys.exit(0)
        if mode == 'draft-first-hold':
            current_turn = 'draft-turn'
            held_send = request
            (root / 'send-entered').touch()
            (root / ('send-entered-' + str(os.getpid()))).touch()
            emit('turn/started', {'threadId':'thread','turn':{'id':current_turn}})
            emit('item/completed', {'threadId':'thread','turnId':current_turn,'item':{'id':'draft-user','type':'userMessage','content':request['params']['input']}})
            emit('item/started', {'threadId':'thread','turnId':current_turn,'item':{'id':'assistant','type':'agentMessage','text':'draft output before send ACK'}})
            approval(current_turn)
            continue
        if mode.startswith('send-'):
            current_turn = 'turn-' + str(turn_count)
            if turn_count > 1 and mode == 'send-reject':
                print(json.dumps({'id':request['id'], 'error':{'code':-32600,'message':'input explicitly rejected'}}), flush=True)
                (root / 'send-entered').touch()
                continue
            if turn_count > 1 and mode == 'send-disconnect':
                (root / 'send-entered').touch()
                sys.exit(0)
            if turn_count > 2 and mode == 'send-late-echo':
                ack(request, {'turn':{'id':current_turn}})
                held_echo = request
                (root / 'send-acked').touch()
                continue
            if turn_count == 1: ack(request, {'turn':{'id':current_turn}})
            else:
                held_send = request
                (root / 'send-entered').touch()
            if mode != 'send-no-id' or turn_count == 1:
                emit('turn/started', {'threadId':'thread','turn':{'id':current_turn}})
            if request['params']['input']:
                emit('item/completed', {'threadId':'thread','turnId':current_turn,'item':{'id':'user-' + str(turn_count),'type':'userMessage','content':request['params']['input']}})
            emit('item/started', {'threadId':'thread','turnId':current_turn,'item':{'id':'assistant-' + str(turn_count),'type':'agentMessage','text':'output before send ACK'}})
            if turn_count > 1 and mode != 'send-late-echo': approval(current_turn)
            if (turn_count == 1 and mode != 'send-writer-full') or mode == 'send-terminal' or (mode == 'send-late-echo' and turn_count == 2):
                emit('turn/completed', {'threadId':'thread','turn':{'id':current_turn,'status':'interrupted' if turn_count == 1 else 'completed'}})
            continue
        if mode == 'sync-send-failure' and turn_count == 2:
            print(json.dumps({'id':request['id'], 'error':{'code':-32600,'message':'synthetic sync send rejected'}}), flush=True)
            (root / 'sync-send-rejected').touch()
            continue
        ack(request, {'turn':{'id':'turn'}})
        emit('turn/started', {'threadId':'thread','turn':{'id':'turn'}})
        emit('item/started', {'threadId':'thread','turnId':'turn','item':{'id':'assistant','type':'agentMessage','text':'initial output'}})
        if mode == 'sync-send-failure':
            emit('turn/completed', {'threadId':'thread','turn':{'id':'turn','status':'completed'}})
        else:
            for child in ('child-a', 'child-b'):
                emit('thread/status/changed', {'threadId':child,'status':{'type':'active','activeFlags':[]}})
                emit('turn/started', {'threadId':child,'turn':{'id':'turn-' + child}})
            if mode == 'queue-background':
                emit('turn/completed', {'threadId':'thread','turn':{'id':'turn','status':'completed'}})
    elif method == 'turn/interrupt':
        if request['params']['threadId'] == 'thread':
            attempt += 1
            held = request
            (root / ('entered-' + str(attempt))).touch()
            emit('item/agentMessage/delta', {'threadId':'thread','turnId':current_turn,'itemId':'assistant','delta':' while interrupt pending'})
            approval(current_turn)
            mode = (root / 'mode').read_text()
            if mode == 'terminal':
                emit('turn/completed', {'threadId':'thread','turn':{'id':'turn','status':'completed'}})
            if mode == 'failure':
                sys.stdout.write('{malformed\n'); sys.stdout.flush()
                (root / 'malformed-written').touch()
        elif (root / 'mode').read_text() == 'children-held-reject':
            held_children.append(request)
            if len(held_children) == 2: (root / 'children-entered').touch()
        elif request['params']['threadId'] == 'child-b' and (root / 'mode').read_text() != 'terminal':
            print(json.dumps({'id':request['id'], 'error':{'code':-32600,'message':'child rejected'}}), flush=True)
        else: ack(request, {})
    elif method == 'turn/steer':
        if (root / 'mode').read_text() == 'steer-hold':
            held_steer = request
            (root / 'steer-entered').touch()
            emit('item/agentMessage/delta', {'threadId':'thread','turnId':current_turn,'itemId':'assistant','delta':' output before steer ACK'})
            approval(current_turn)
        else: ack(request, {})
    elif method is None and request.get('id') == 'approval':
        (root / 'reply-written').touch()
    elif 'id' in request: ack(request, {})
"#).unwrap();
        fixture
    }

    fn wire(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(self.0.join("wire"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn native_selection() -> Option<crate::ModelSelection> {
    Some(crate::ModelSelection::Native {
        provider_id: Some("openai".into()),
        model_id: "synthetic-native".into(),
    })
}

fn control_turn(text: &str) -> crate::ComposedTurn {
    crate::ComposedTurn {
        text: text.into(),
        effort: None,
        permission_mode: None,
        attachments: vec![],
    }
}

fn codex_control_runtime(fixture: &Fixture, runtime_id: &str) -> crate::config::RuntimeInstance {
    crate::config::RuntimeInstance {
        enabled: true,
        id: runtime_id.into(),
        name: "Synthetic Codex".into(),
        type_id: crate::conversation::RuntimeKind::Codex166,
        gateway_id: "default".into(),
        settings: BTreeMap::from([
            (
                "binary".into(),
                fixture.0.join("codex").to_string_lossy().into_owned(),
            ),
            ("nodeBinary".into(), "/synthetic/not-executed-node".into()),
            (
                "agentDir".into(),
                fixture.0.join("native").to_string_lossy().into_owned(),
            ),
        ]),
    }
}

fn created_codex(
    app: &ApplicationService,
    fixture: &Fixture,
    runtime_id: &str,
    selection: Option<crate::ModelSelection>,
) -> String {
    app.execute(
        ClientViewId("control-view".into()),
        Command::UpsertRuntimeWithExecution {
            runtime: codex_control_runtime(fixture, runtime_id),
            execution: crate::RuntimeExecution {
                home: crate::RuntimeHome::Managed { remote_root: None },
                ..Default::default()
            },
        },
    )
    .unwrap();
    let Response::CreateConversation(created) = app
        .execute(
            ClientViewId("control-view".into()),
            Command::CreateConversation {
                runtime_id: runtime_id.into(),
                cwd: Some(fixture.0.to_string_lossy().into_owned()),
                project: None,
                selection,
            },
        )
        .unwrap()
    else {
        panic!("creation response")
    };
    created.snapshot.unwrap().conversation.id
}

fn running_codex(app: &ApplicationService, fixture: &Fixture, runtime_id: &str) -> String {
    let id = created_codex(app, fixture, runtime_id, native_selection());
    app.execute(
        ClientViewId("control-view".into()),
        Command::SendTurn {
            submission_id: None,
            source_runtime_id: runtime_id.into(),
            conversation_id: id.clone(),
            target_runtime_id: runtime_id.into(),
            selection: native_selection(),
            turn: control_turn("synthetic input"),
        },
    )
    .unwrap();
    id
}

fn control_snapshot(app: &ApplicationService, id: &str) -> crate::DeliveredSnapshot {
    runtime_control_snapshot(app, "synthetic", id)
}

fn runtime_control_snapshot(
    app: &ApplicationService,
    runtime_id: &str,
    id: &str,
) -> crate::DeliveredSnapshot {
    let response = request_later(
        app,
        "observer",
        Command::ReadSnapshot {
            runtime_id: runtime_id.into(),
            conversation_id: id.into(),
        },
    )
    .recv_timeout(Duration::from_secs(2))
    .unwrap()
    .unwrap();
    let Response::ReadSnapshot(result) = response else {
        panic!("snapshot response")
    };
    result.snapshot.unwrap()
}

fn open_control_service(fixture: &Fixture) -> ApplicationService {
    ApplicationService::open(Options {
        home_directory: fixture.0.join("app"),
        resources_directory: Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../agent-runtime/resources"),
    })
    .unwrap()
}

#[tokio::test]
async fn claude_automatic_naming_uses_configured_model_and_preserves_native_title() {
    use crate::config::{ExecutionBackend, Protocol, ProviderQuotaSource, RequestProfile};
    use crate::conversation::{ConversationTitle, RunState};
    use axum::{Router, body::Bytes, http::header, routing::post};
    use serde_json::{Value, json};
    use velune_application_api::{ProviderDraft, ProviderModel};

    let (requests, received) = mpsc::channel();
    let server = Router::new().route(
        "/v1/chat/completions",
        post(move |body: Bytes| {
            let requests = requests.clone();
            async move {
                let body = serde_json::from_slice::<Value>(&body).unwrap();
                requests.send(body).unwrap();
                (
                    [(header::CONTENT_TYPE, "application/json")],
                    json!({"choices":[{"message":{"content":"Velune fallback title"},"finish_reason":"stop"}]}).to_string(),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });

    tokio::task::spawn_blocking(move || {
        let fixture = Fixture::claude_naming();
        let app = open_control_service(&fixture);
        let view = ClientViewId("naming-observer".into());
        let execute = |command| app.execute(view.clone(), command).unwrap();
        let Response::SaveProvider(saved) = execute(Command::SaveProvider {
            gateway_id: "default".into(),
            provider: ProviderDraft {
                id: "synthetic-text".into(),
                name: "Synthetic text provider".into(),
                protocol: Protocol::ChatCompletions,
                endpoint: format!("http://{address}/v1"),
                execution_backend: ExecutionBackend::Http,
                request_profile: RequestProfile::Native,
                codex_client_version: None,
                authentication_source: crate::ProviderAuthenticationSource::ApiKey,
                quota_source: ProviderQuotaSource::None,
                quota_limits: Vec::new(),
                models: ["synthetic-namer", "synthetic-turn-model"]
                    .into_iter()
                    .map(|id| ProviderModel {
                        record_key: String::new(),
                        provider_model_id: id.into(),
                        nickname: String::new(),
                        icon: None,
                        context_window: None,
                        max_output_tokens: None,
                        reasoning_levels: None,
                        request_parameters: None,
                    })
                    .collect(),
            },
            authentication_edit: crate::AuthenticationEdit::SetApiKey {
                value: "synthetic-key".into(),
            },
            quota_authentication_edit: None,
        }) else {
            panic!("save naming provider response")
        };
        let model_key = |id| {
            saved.gateways[0].providers[0]
                .models
                .iter()
                .find(|model| model.provider_model_id == id)
                .unwrap()
                .record_key
                .clone()
        };
        let naming_model = model_key("synthetic-namer");
        let instruction = "Name this synthetic conversation with a short descriptive title.";
        let set_naming = |enabled: bool| {
            execute(Command::UpsertRuntimeWithExecution {
                runtime: crate::config::RuntimeInstance {
                    enabled: true,
                    id: "synthetic".into(),
                    name: "Synthetic Claude".into(),
                    type_id: crate::conversation::RuntimeKind::ClaudeCode,
                    gateway_id: "default".into(),
                    settings: BTreeMap::from([
                        (
                            "binary".into(),
                            fixture.0.join("claude").to_string_lossy().into_owned(),
                        ),
                        ("automaticNaming".into(), enabled.to_string()),
                        ("automaticNamingModel".into(), naming_model.clone()),
                        ("automaticNamingPrompt".into(), instruction.into()),
                    ]),
                },
                execution: crate::RuntimeExecution {
                    home: crate::RuntimeHome::Managed { remote_root: None },
                    ..Default::default()
                },
            });
        };
        let run = |prompt, expected, selection: Option<crate::ModelSelection>| {
            let Response::CreateConversation(created) = execute(Command::CreateConversation {
                runtime_id: "synthetic".into(),
                cwd: Some(fixture.0.to_string_lossy().into_owned()),
                project: None,
                selection: selection.clone(),
            }) else {
                panic!("create response")
            };
            let id = created.snapshot.unwrap().conversation.id;
            execute(Command::SendTurn {
                submission_id: None,
                source_runtime_id: "synthetic".into(),
                conversation_id: id.clone(),
                target_runtime_id: "synthetic".into(),
                selection,
                turn: control_turn(prompt),
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let snapshot = control_snapshot(&app, &id);
                if snapshot.conversation.title == expected && snapshot.run_state == RunState::Idle {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "{id} did not receive {expected:?}; last title was {:?} ({:?})",
                    snapshot.conversation.title,
                    snapshot.run_state,
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            id
        };
        set_naming(false);
        run(
            "fallback disabled",
            ConversationTitle::FirstMessage("fallback disabled".into()),
            None,
        );
        set_naming(true);
        let native = ConversationTitle::Native("Native Claude title".into());
        let generated = ConversationTitle::Generated("Velune fallback title".into());
        let generated_id = run(
            "name this synthetic prompt",
            generated.clone(),
            Some(crate::ModelSelection::Managed {
                model_record_key: model_key("synthetic-turn-model"),
            }),
        );
        let native_id = run("native titled prompt", native.clone(), None);
        let assert_list = |configuration: Response| {
            let Response::ReadConfiguration(configuration) = configuration else {
                panic!("read configuration response")
            };
            for (id, expected) in [(&native_id, &native), (&generated_id, &generated)] {
                let listed = configuration
                    .conversations
                    .iter()
                    .find(|row| &row.conversation.id == id);
                assert_eq!(listed.map(|row| &row.conversation.title), Some(expected));
            }
        };
        assert_list(execute(Command::ReadConfiguration));
        app.close(true).unwrap();
        let app = open_control_service(&fixture);
        assert_list(app.execute(view, Command::ReadConfiguration).unwrap());
        app.close(false).unwrap();

        let request = received.try_recv().expect("fallback naming request");
        assert_eq!(request["model"], "synthetic-namer");
        let prompt = request["messages"][0]["content"].as_str().unwrap();
        assert!(prompt.starts_with(instruction));
        assert_eq!(
            serde_json::from_str::<Value>(prompt.lines().last().unwrap()).unwrap(),
            json!({"user": "name this synthetic prompt", "assistant": "synthetic assistant reply"})
        );
        assert!(
            received.try_recv().is_err(),
            "the disabled fallback and the native title must not request naming"
        );
    })
    .await
    .unwrap();
    server.abort();
}

#[test]
fn codex_control_owner_polls_output_and_reply_during_ack_then_preserves_partial_parent_stop() {
    let fixture = Fixture::codex_control("reject");
    let app = open_control_service(&fixture);
    let id = running_codex(&app, &fixture, "synthetic");
    let cancel = || Command::Cancel {
        runtime_id: "synthetic".into(),
        conversation_id: id.clone(),
    };
    let first = request_later(&app, "control-view", cancel());
    fixture.wait("entered-1");
    assert_read_before_release(&app);
    let deadline = Instant::now() + Duration::from_secs(2);
    let interaction = loop {
        let snapshot = control_snapshot(&app, &id);
        if let Some(interaction) = snapshot.pending_interactions.first() {
            assert!(snapshot.messages.iter().any(|message| message.blocks.iter().any(|block|
                matches!(block, crate::DeliveredBlock::Text { text } if text.contains("while interrupt pending")))));
            assert_ne!(snapshot.run_state, crate::conversation::RunState::Stopping);
            break interaction.id.clone();
        }
        assert!(Instant::now() < deadline);
    };
    let reply = || Command::ReplyRuntimeInteraction {
        runtime_id: "synthetic".into(),
        conversation_id: id.clone(),
        interaction_id: interaction.clone(),
        reply: crate::conversation::RuntimeInteractionReply::Decision {
            option_id: "accept".into(),
        },
    };
    assert!(matches!(
        request_later(&app, "replier", reply())
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap(),
        Response::ReplyRuntimeInteraction(_)
    ));
    fixture.wait("reply-written");
    assert!(
        app.execute(ClientViewId("replier".into()), reply())
            .is_err(),
        "interaction replies must not replay"
    );
    assert!(matches!(first.try_recv(), Err(mpsc::TryRecvError::Empty)));
    fs::write(fixture.0.join("release-1"), []).unwrap();
    assert_eq!(
        first
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .err()
            .expect("expected control failure")
            .kind,
        FailureKind::Invalid
    );
    let snapshot = control_snapshot(&app, &id);
    assert_eq!(snapshot.run_state, crate::conversation::RunState::Running);
    assert!(snapshot.actions.can_cancel);
    app.execute(
        ClientViewId("control-view".into()),
        Command::SteerTurn {
            runtime_id: "synthetic".into(),
            conversation_id: id.clone(),
            target_runtime_id: None,
            turn: control_turn("still steerable"),
        },
    )
    .unwrap();
    fs::write(fixture.0.join("mode"), "partial").unwrap();
    let second = request_later(&app, "control-view", cancel());
    fixture.wait("entered-2");
    fs::write(fixture.0.join("release-2"), []).unwrap();
    let partial = second
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .err()
        .expect("expected control failure");
    assert_eq!(partial.code, "runtime_operation_completed");
    assert!(
        partial.message.contains("1/2")
            && partial.message.contains("child-b")
            && partial.message.contains("Rejected")
    );
    assert_eq!(
        control_snapshot(&app, &id).run_state,
        crate::conversation::RunState::Stopping
    );
    let wire = fixture.wire();
    let interrupts: Vec<_> = wire
        .iter()
        .filter(|record| record["method"] == "turn/interrupt")
        .collect();
    assert_eq!(
        interrupts.len(),
        4,
        "two explicit parents and exactly one frame per child"
    );
    assert_eq!(
        wire.iter()
            .filter(|record| record["id"] == "approval" && record.get("method").is_none())
            .count(),
        1
    );
    assert_eq!(
        wire.iter()
            .filter(|record| record["method"] == "thread/start")
            .count(),
        1
    );
    app.close(true).unwrap();
}

#[test]
fn codex_terminal_before_cancel_ack_pauses_queue_and_closed_view_is_not_recreated() {
    let fixture = Fixture::codex_control("terminal");
    let app = open_control_service(&fixture);
    let id = running_codex(&app, &fixture, "synthetic");
    app.execute(
        ClientViewId("control-view".into()),
        Command::EditSendQueue {
            submission_id: None,
            runtime_id: "synthetic".into(),
            conversation_id: id.clone(),
            operation: crate::SendQueueOperation::Add {
                source_runtime_id: "synthetic".into(),
                target_runtime_id: "synthetic".into(),
                selection: native_selection(),
                turn: control_turn("must not automatically send"),
            },
        },
    )
    .unwrap();
    let cancel = request_later(
        &app,
        "control-view",
        Command::Cancel {
            runtime_id: "synthetic".into(),
            conversation_id: id.clone(),
        },
    );
    fixture.wait("entered-1");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = control_snapshot(&app, &id);
        if snapshot.run_state == crate::conversation::RunState::Idle {
            break;
        }
        assert!(Instant::now() < deadline);
    }
    app.execute(ClientViewId("control-view".into()), Command::ReleaseView)
        .unwrap();
    assert_read_before_release(&app);
    assert!(matches!(cancel.try_recv(), Err(mpsc::TryRecvError::Empty)));
    fs::write(fixture.0.join("release-1"), []).unwrap();
    let result = cancel
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .err()
        .expect("expected control failure");
    assert_eq!(result.code, "runtime_operation_completed");
    assert!(result.message.contains("原请求视图已关闭"), "{result:?}");
    let wire = fixture.wire();
    assert_eq!(
        wire.iter()
            .filter(|record| record["method"] == "turn/start")
            .count(),
        1
    );
    assert_eq!(
        wire.iter()
            .filter(|record| record["method"] == "thread/start")
            .count(),
        1
    );
    app.close(true).unwrap();
}

#[test]
fn codex_stop_child_is_exact_and_stop_background_preserves_each_delivery() {
    for child_only in [true, false] {
        let fixture = Fixture::codex_control("partial");
        let app = open_control_service(&fixture);
        let id = running_codex(&app, &fixture, "synthetic");
        let command = if child_only {
            let child_id = "synthetic:child-a";
            let child = control_snapshot(&app, child_id);
            assert_eq!(
                child.conversation.parent_conversation_id.as_deref(),
                Some(id.as_str())
            );
            Command::Cancel {
                runtime_id: "synthetic".into(),
                conversation_id: child_id.into(),
            }
        } else {
            Command::StopSubAgents {
                runtime_id: "synthetic".into(),
                conversation_id: id.clone(),
            }
        };
        let result = request_later(&app, "control-view", command)
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        if child_only {
            assert!(matches!(result.unwrap(), Response::Cancel(_)));
            app.execute(
                ClientViewId("control-view".into()),
                Command::SteerTurn {
                    runtime_id: "synthetic".into(),
                    conversation_id: id.clone(),
                    target_runtime_id: None,
                    turn: control_turn("parent was not cancelled"),
                },
            )
            .unwrap();
        } else {
            let error = result
                .err()
                .expect("partial stop must not become all-rejected");
            assert_eq!(error.code, "runtime_operation_completed");
            assert!(error.message.contains("1/2") && error.message.contains("child-b"));
        }
        let snapshot = control_snapshot(&app, &id);
        assert_eq!(snapshot.run_state, crate::conversation::RunState::Running);
        let wire = fixture.wire();
        let stops: Vec<_> = wire
            .iter()
            .filter(|record| record["method"] == "turn/interrupt")
            .collect();
        assert_eq!(stops.len(), if child_only { 1 } else { 2 });
        assert!(
            stops
                .iter()
                .all(|record| record["params"]["threadId"] != "thread")
        );
        if child_only {
            assert_eq!(stops[0]["params"]["threadId"], "child-a")
        }
        app.close(true).unwrap();
    }
}

#[test]
fn codex_control_source_change_does_not_refresh_or_replay_the_old_operation() {
    let fixture = Fixture::codex_control("partial");
    let app = open_control_service(&fixture);
    let id = running_codex(&app, &fixture, "synthetic");
    let cancel = request_later(
        &app,
        "control-view",
        Command::Cancel {
            runtime_id: "synthetic".into(),
            conversation_id: id,
        },
    );
    fixture.wait("entered-1");
    let mut runtime = codex_control_runtime(&fixture, "synthetic");
    runtime.settings.insert(
        "binary".into(),
        fixture
            .0
            .join("different-source/codex")
            .to_string_lossy()
            .into_owned(),
    );
    app.execute(
        ClientViewId("editor".into()),
        Command::UpsertRuntimeWithExecution {
            runtime,
            execution: crate::RuntimeExecution {
                home: crate::RuntimeHome::Managed { remote_root: None },
                ..Default::default()
            },
        },
    )
    .unwrap();
    fs::write(fixture.0.join("release-1"), []).unwrap();
    let error = cancel
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .err()
        .expect("old source result must not refresh the new source");
    assert_eq!(
        error.code, "runtime_operation_completed",
        "native acceptance must not become an all-rejected fact"
    );
    assert!(error.message.contains("来源已改变"), "{error:?}");
    let wire = fixture.wire();
    assert_eq!(
        wire.iter()
            .filter(|record| record["method"] == "thread/start")
            .count(),
        1
    );
    assert_eq!(
        wire.iter()
            .filter(|record| record["method"] == "turn/interrupt")
            .count(),
        1
    );
    assert!(
        !fixture.0.join("different-source").exists(),
        "completion must not prepare a new source"
    );
    app.close(true).unwrap();
}

#[test]
fn codex_source_change_during_background_rejections_preserves_parent_acceptance() {
    let fixture = Fixture::codex_control("children-held-reject");
    let app = open_control_service(&fixture);
    let id = running_codex(&app, &fixture, "synthetic");
    let cancel = request_later(
        &app,
        "control-view",
        Command::Cancel {
            runtime_id: "synthetic".into(),
            conversation_id: id,
        },
    );
    fixture.wait("entered-1");
    fs::write(fixture.0.join("release-1"), []).unwrap();
    fixture.wait("children-entered");
    assert!(matches!(cancel.try_recv(), Err(mpsc::TryRecvError::Empty)));
    let mut runtime = codex_control_runtime(&fixture, "synthetic");
    runtime.settings.insert(
        "binary".into(),
        fixture
            .0
            .join("different-source/codex")
            .to_string_lossy()
            .into_owned(),
    );
    app.execute(
        ClientViewId("editor".into()),
        Command::UpsertRuntimeWithExecution {
            runtime,
            execution: crate::RuntimeExecution {
                home: crate::RuntimeHome::Managed { remote_root: None },
                ..Default::default()
            },
        },
    )
    .unwrap();
    fs::write(fixture.0.join("release-children"), []).unwrap();
    let error = cancel
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .err()
        .expect("source changed after parent acceptance");
    assert_eq!(error.code, "runtime_operation_completed");
    assert!(error.message.contains("父 turn 停止已受理"), "{error:?}");
    assert!(error.message.contains("来源已改变"), "{error:?}");
    assert!(
        error.message.contains("0/2 已受理"),
        "child rejection must not become native acceptance: {error:?}"
    );
    assert!(error.message.contains("child-a") && error.message.contains("child-b"));
    assert_eq!(error.message.matches("Rejected").count(), 2);
    assert!(!error.message.contains("Indeterminate"));
    let wire = fixture.wire();
    assert_eq!(
        wire.iter()
            .filter(|record| record["method"] == "thread/start")
            .count(),
        1
    );
    let interrupts: Vec<_> = wire
        .iter()
        .filter(|record| record["method"] == "turn/interrupt")
        .collect();
    assert_eq!(
        interrupts.len(),
        3,
        "parent and each child dispatched exactly once"
    );
    for thread in ["thread", "child-a", "child-b"] {
        assert_eq!(
            interrupts
                .iter()
                .filter(|record| record["params"]["threadId"] == thread)
                .count(),
            1
        );
    }
    assert!(!fixture.0.join("different-source").exists());
    app.close(true).unwrap();
}

#[test]
fn codex_failed_control_cleanup_retains_home_and_saturated_pool_resources_until_worker_finishes() {
    assert_failed_cleanup_retains_saturated_pool_resources(false);
}

#[test]
fn codex_compat_sync_send_failure_after_typed_control_retries_cleanup_even_when_poll_is_ok() {
    assert_failed_cleanup_retains_saturated_pool_resources(true);
}

fn assert_failed_cleanup_retains_saturated_pool_resources(compat_sync_send: bool) {
    use crate::local::execution_workers::cleanup_test_gate;
    let fixtures: Vec<_> = (0..3)
        .map(|index| {
            Fixture::codex_control(if compat_sync_send && index == 2 {
                "sync-send-failure"
            } else {
                "failure"
            })
        })
        .collect();
    let app = open_control_service(&fixtures[0]);
    let ids: Vec<_> = fixtures
        .iter()
        .enumerate()
        .map(|(index, fixture)| running_codex(&app, fixture, &format!("runtime-{index}")))
        .collect();
    let gates: Vec<_> = fixtures
        .iter()
        .map(|fixture| cleanup_test_gate::register(&fixture.0.join("codex")))
        .collect();
    let mut replies = Vec::new();
    for (index, id) in ids.into_iter().enumerate() {
        let command = if compat_sync_send && index == 2 {
            let Response::StopSubAgents(result) = app
                .execute(
                    ClientViewId("control-view".into()),
                    Command::StopSubAgents {
                        runtime_id: "runtime-2".into(),
                        conversation_id: id.clone(),
                    },
                )
                .unwrap()
            else {
                panic!("typed control response")
            };
            assert_eq!(
                result.snapshot.unwrap().run_state,
                crate::conversation::RunState::Idle
            );
            Command::SendTurn {
                submission_id: None,
                source_runtime_id: "runtime-2".into(),
                conversation_id: id,
                target_runtime_id: "runtime-2".into(),
                selection: native_selection(),
                // Attachments deliberately retain the compatibility path;
                // attachment-free same-context sends now use typed admission.
                turn: crate::ComposedTurn {
                    attachments: vec![
                        fixtures[index]
                            .0
                            .join("mode")
                            .to_string_lossy()
                            .into_owned(),
                    ],
                    ..control_turn("compatibility send fails on the enabled connection")
                },
            }
        } else {
            Command::Cancel {
                runtime_id: format!("runtime-{index}"),
                conversation_id: id,
            }
        };
        replies.push(request_later(&app, "control-view", command));
        fixtures[index].wait(if compat_sync_send && index == 2 {
            "sync-send-rejected"
        } else {
            "malformed-written"
        });
        if index < 2 {
            gates[index].0.recv_timeout(Duration::from_secs(2)).unwrap();
        }
    }
    for (index, reply) in replies.into_iter().enumerate() {
        let error = reply
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .err()
            .expect("failed native connection");
        if compat_sync_send && index == 2 {
            assert!(
                error.message.contains("synthetic sync send rejected"),
                "{error:?}"
            );
            let wire = fixtures[index].wire();
            let processes = fs::read_to_string(fixtures[index].0.join("turn-processes")).unwrap();
            let processes: Vec<_> = processes.lines().collect();
            assert_eq!(processes.len(), 2);
            assert_eq!(
                processes[0], processes[1],
                "compatibility send must use the already enabled connection"
            );
            assert_eq!(
                wire.iter()
                    .filter(|record| record["method"] == "thread/start")
                    .count(),
                1
            );
            assert_eq!(
                wire.iter()
                    .filter(|record| record["method"] == "turn/start")
                    .count(),
                2
            );
            fs::write(fixtures[index].0.join("check-alive"), []).unwrap();
            fixtures[index].wait("alive-before-cleanup");
        } else {
            assert_eq!(error.code, "runtime_operation_indeterminate");
        }
    }
    // Two local lifecycle leases are occupied. The third failed NativeSession
    // must stay retained on the owner, not be dropped as failed admission.
    assert!(matches!(
        gates[2].0.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    assert!(crate::repository::Repository::lock_home(&fixtures[0].0.join("app")).is_err());
    assert!(
        app.close(true).is_err(),
        "pending cleanup must keep the service open"
    );
    let read = request_later(
        &app,
        "observer",
        Command::ReadRuntimeHomeDirectory {
            runtime_id: "runtime-2".into(),
        },
    );
    assert!(read.recv_timeout(Duration::from_secs(2)).unwrap().is_ok());
    gates[0].1.send(()).unwrap();
    gates[2].0.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(
        app.close(true).is_err(),
        "a cleanup lease is not shutdown confirmation"
    );
    assert!(crate::repository::Repository::lock_home(&fixtures[0].0.join("app")).is_err());
    gates[1].1.send(()).unwrap();
    gates[2].1.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if app.close(true).is_ok() {
            drop(crate::repository::Repository::lock_home(&fixtures[0].0.join("app")).unwrap());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "home lock not released after lifecycle workers finish"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn next_codex_send(id: &str) -> Command {
    Command::SendTurn {
        submission_id: None,
        source_runtime_id: "synthetic".into(),
        conversation_id: id.into(),
        target_runtime_id: "synthetic".into(),
        selection: native_selection(),
        turn: control_turn("one typed input"),
    }
}

fn provider_wire_reads(fixture: &Fixture) -> usize {
    fixture
        .wire()
        .iter()
        .filter(|record| {
            matches!(
                record["method"].as_str(),
                Some("initialize" | "config/read" | "thread/read" | "thread/resume")
            )
        })
        .count()
}

fn reply_before_submission_ack(app: &ApplicationService, fixture: &Fixture, id: &str) {
    assert_read_before_release(app);
    let deadline = Instant::now() + Duration::from_secs(2);
    let interaction = loop {
        let snapshot = control_snapshot(app, id);
        if let Some(interaction) = snapshot.pending_interactions.first() {
            assert!(snapshot.messages.iter().any(|message| message.blocks.iter().any(|block|
                matches!(block, crate::DeliveredBlock::Text { text } if text.contains("before send ACK") || text.contains("before steer ACK")))));
            break interaction.id.clone();
        }
        assert!(Instant::now() < deadline);
    };
    assert!(matches!(
        request_later(
            app,
            "replier",
            Command::ReplyRuntimeInteraction {
                runtime_id: "synthetic".into(),
                conversation_id: id.into(),
                interaction_id: interaction,
                reply: crate::conversation::RuntimeInteractionReply::Decision {
                    option_id: "accept".into()
                },
            }
        )
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap(),
        Response::ReplyRuntimeInteraction(_)
    ));
    fixture.wait("reply-written");
}

#[test]
fn codex_prepared_send_and_resume_keep_polling_and_reuse_the_confirmed_provider_owner() {
    for submission in ["send", "resume", "current"] {
        let resume = submission == "resume";
        let fixture = Fixture::codex_control("send-hold");
        let app = open_control_service(&fixture);
        let id = running_codex(&app, &fixture, "synthetic");
        let before = provider_wire_reads(&fixture);
        // A new helper would see a different dynamic default. The already
        // confirmed owner must keep its captured provider and exact context.
        fs::write(fixture.0.join("changed-default"), []).unwrap();
        let command = if resume {
            Command::ResumeTurn {
                runtime_id: "synthetic".into(),
                conversation_id: id.clone(),
                selection: native_selection(),
                effort: None,
                permission_mode: None,
            }
        } else if submission == "current" {
            Command::SendCurrentTurn {
                source_runtime_id: "synthetic".into(),
                conversation_id: id.clone(),
                turn: control_turn("one typed input"),
            }
        } else {
            next_codex_send(&id)
        };
        let submitted = request_later(&app, "control-view", command);
        fixture.wait("send-entered");
        reply_before_submission_ack(&app, &fixture, &id);
        assert!(matches!(
            submitted.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(
            provider_wire_reads(&fixture),
            before,
            "confirmed context must not sample a second helper's provider defaults"
        );
        fs::write(fixture.0.join("release-send"), []).unwrap();
        let response = submitted
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        let result = match response {
            Response::SendTurn(result) | Response::ResumeTurn(result) => result,
            _ => panic!("submission response"),
        };
        assert_eq!(
            result.snapshot.as_ref().unwrap().run_state,
            crate::conversation::RunState::Running
        );
        if resume {
            assert!(result.submitted_message_id.is_none());
        } else {
            assert!(
                result
                    .submitted_message_id
                    .as_ref()
                    .is_some_and(|id| id.contains("user-2"))
            );
            let settings = result
                .snapshot
                .as_ref()
                .unwrap()
                .transcript_turns
                .iter()
                .find(|turn| turn.user_message_id.contains("user-2"))
                .and_then(|turn| turn.settings.as_ref())
                .expect("accepted requested settings");
            assert_eq!(settings.model_selection, native_selection());
        }
        let wire = fixture.wire();
        let turns: Vec<_> = wire
            .iter()
            .filter(|record| record["method"] == "turn/start")
            .collect();
        assert_eq!(turns.len(), 2);
        assert_eq!(
            turns[1]["params"]["input"].as_array().unwrap().len(),
            usize::from(!resume)
        );
        let processes = fs::read_to_string(fixture.0.join("turn-processes")).unwrap();
        let processes: Vec<_> = processes.lines().collect();
        assert_eq!(processes[0], processes[1]);
        assert_eq!(provider_wire_reads(&fixture), before);
        app.close(true).unwrap();
    }
}

#[test]
fn codex_prepared_send_rejection_preserves_retry_but_disconnect_is_indeterminate_and_cleans_on_worker()
 {
    for rejected in [true, false] {
        let fixture = Fixture::codex_control(if rejected {
            "send-reject"
        } else {
            "send-disconnect"
        });
        let app = open_control_service(&fixture);
        let id = running_codex(&app, &fixture, "synthetic");
        let gate = (!rejected).then(|| {
            crate::local::execution_workers::cleanup_test_gate::register(&fixture.0.join("codex"))
        });
        let submitted = request_later(&app, "control-view", next_codex_send(&id));
        fixture.wait("send-entered");
        let error = submitted
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .err()
            .expect("submission must fail explicitly");
        assert_eq!(
            fixture
                .wire()
                .iter()
                .filter(|record| record["method"] == "turn/start")
                .count(),
            2
        );
        assert_read_before_release(&app);
        if rejected {
            assert_eq!(error.kind, FailureKind::Invalid);
            assert!(error.message.contains("输入未受理"));
            let snapshot = control_snapshot(&app, &id);
            assert!(snapshot.actions.can_send);
            assert!(
                !snapshot
                    .messages
                    .iter()
                    .any(|message| message.id.contains("user-2"))
            );
            fs::write(fixture.0.join("mode"), "send-hold").unwrap();
            fs::write(fixture.0.join("release-send"), []).unwrap();
            assert!(matches!(
                app.execute(ClientViewId("control-view".into()), next_codex_send(&id))
                    .unwrap(),
                Response::SendTurn(_)
            ));
            assert_eq!(
                fixture
                    .wire()
                    .iter()
                    .filter(|record| record["method"] == "turn/start")
                    .count(),
                3,
                "only explicit retry may submit again"
            );
            app.close(true).unwrap();
        } else {
            assert_eq!(error.code, "runtime_operation_indeterminate");
            let (entered, release) = gate.unwrap();
            entered.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(app.close(true).is_err());
            assert!(crate::repository::Repository::lock_home(&fixture.0.join("app")).is_err());
            assert_eq!(
                fixture
                    .wire()
                    .iter()
                    .filter(|record| record["method"] == "turn/start")
                    .count(),
                2,
                "unknown delivery is never replayed"
            );
            release.send(()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while app.close(true).is_err() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            drop(crate::repository::Repository::lock_home(&fixture.0.join("app")).unwrap());
        }
    }
}

#[test]
fn codex_pending_send_keeps_cancellation_flag_until_the_correct_turn_can_be_interrupted() {
    for early_identity in [true, false] {
        let fixture = Fixture::codex_control(if early_identity {
            "send-hold"
        } else {
            "send-no-id"
        });
        let app = open_control_service(&fixture);
        let id = running_codex(&app, &fixture, "synthetic");
        let flag = Arc::new(AtomicBool::new(false));
        let (reply, submitted) = mpsc::channel();
        let sending = app.clone();
        let cancellation = flag.clone();
        let command = next_codex_send(&id);
        std::thread::spawn(move || {
            reply
                .send(sending.execute_cancellable(
                    ClientViewId("control-view".into()),
                    command,
                    cancellation,
                ))
                .unwrap();
        });
        fixture.wait("send-entered");
        flag.store(true, Ordering::Release);
        assert_read_before_release(&app);
        if !early_identity {
            assert!(
                !fixture
                    .wire()
                    .iter()
                    .any(|record| record["method"] == "turn/interrupt"),
                "no identity is not a fulfilled stop intent"
            );
            assert!(matches!(
                submitted.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
            fs::write(fixture.0.join("release-send"), []).unwrap();
        }
        fixture.wait("entered-1");
        let wire = fixture.wire();
        let stops: Vec<_> = wire
            .iter()
            .filter(|record| record["method"] == "turn/interrupt")
            .collect();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0]["params"]["threadId"], "thread");
        assert_eq!(stops[0]["params"]["turnId"], "turn-2");
        fs::write(fixture.0.join("release-1"), []).unwrap();
        fs::write(fixture.0.join("release-send"), []).unwrap();
        let Response::SendTurn(result) = submitted
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap()
        else {
            panic!("send response")
        };
        assert_eq!(
            result.snapshot.unwrap().run_state,
            crate::conversation::RunState::Stopping
        );
        assert_eq!(
            fixture
                .wire()
                .iter()
                .filter(|record| record["method"] == "turn/start")
                .count(),
            2
        );
        app.close(true).unwrap();
    }
}

#[test]
fn codex_terminal_before_send_ack_never_revives_turn_and_stale_completion_preserves_acceptance() {
    for source_changed in [false, true] {
        let fixture = Fixture::codex_control("send-terminal");
        let app = open_control_service(&fixture);
        let id = running_codex(&app, &fixture, "synthetic");
        let submitted = request_later(&app, "control-view", next_codex_send(&id));
        fixture.wait("send-entered");
        let deadline = Instant::now() + Duration::from_secs(2);
        while control_snapshot(&app, &id).run_state != crate::conversation::RunState::Idle {
            assert!(Instant::now() < deadline);
        }
        if source_changed {
            let mut runtime = codex_control_runtime(&fixture, "synthetic");
            runtime.settings.insert(
                "binary".into(),
                fixture
                    .0
                    .join("different-source/codex")
                    .to_string_lossy()
                    .into_owned(),
            );
            app.execute(
                ClientViewId("editor".into()),
                Command::UpsertRuntimeWithExecution {
                    runtime,
                    execution: crate::RuntimeExecution {
                        home: crate::RuntimeHome::Managed { remote_root: None },
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        } else {
            app.execute(ClientViewId("control-view".into()), Command::ReleaseView)
                .unwrap();
        }
        fs::write(fixture.0.join("release-send"), []).unwrap();
        let error = submitted
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .err()
            .expect("stale accepted completion");
        assert_eq!(error.code, "message_delivery_accepted");
        if source_changed {
            assert!(error.message.contains("来源已改变"));
        } else {
            assert!(error.message.contains("原请求视图已关闭"));
        }
        assert_eq!(
            control_snapshot(&app, &id).run_state,
            crate::conversation::RunState::Idle
        );
        assert_eq!(
            fixture
                .wire()
                .iter()
                .filter(|record| record["method"] == "turn/start")
                .count(),
            2
        );
        assert!(!fixture.0.join("different-source").exists());
        app.close(true).unwrap();
    }
}

#[test]
fn codex_prepared_steer_keeps_snapshot_and_reply_live_before_its_ack() {
    let fixture = Fixture::codex_control("partial");
    let app = open_control_service(&fixture);
    let id = running_codex(&app, &fixture, "synthetic");
    let before = provider_wire_reads(&fixture);
    fs::write(fixture.0.join("mode"), "steer-hold").unwrap();
    let steer = request_later(
        &app,
        "control-view",
        Command::SteerTurn {
            runtime_id: "synthetic".into(),
            conversation_id: id.clone(),
            target_runtime_id: None,
            turn: control_turn("one steer input"),
        },
    );
    fixture.wait("steer-entered");
    reply_before_submission_ack(&app, &fixture, &id);
    assert!(matches!(steer.try_recv(), Err(mpsc::TryRecvError::Empty)));
    fs::write(fixture.0.join("release-steer"), []).unwrap();
    assert!(matches!(
        steer.recv_timeout(Duration::from_secs(2)).unwrap().unwrap(),
        Response::SteerTurn(_)
    ));
    assert_eq!(provider_wire_reads(&fixture), before);
    assert_eq!(
        fixture
            .wire()
            .iter()
            .filter(|record| record["method"] == "turn/steer")
            .count(),
        1
    );
    app.close(true).unwrap();
}

#[test]
fn codex_steer_preserves_the_running_execution_configuration() {
    let fixture = Fixture::codex_control("send-writer-full");
    let app = open_control_service(&fixture);
    let view = || ClientViewId("control-view".into());
    let id = created_codex(&app, &fixture, "synthetic", native_selection());
    let mut turn = control_turn("start with known settings");
    turn.effort = Some("high".into());
    turn.permission_mode = Some("workspace".into());
    app.execute(
        view(),
        Command::SendTurn {
            submission_id: None,
            source_runtime_id: "synthetic".into(),
            conversation_id: id.clone(),
            target_runtime_id: "synthetic".into(),
            selection: native_selection(),
            turn,
        },
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = control_snapshot(&app, &id);
        if snapshot.follow_up.can_steer
            && snapshot.transcript_turns.iter().any(|turn| {
                turn.settings.as_ref().is_some_and(|settings| {
                    settings.effort.as_deref() == Some("high")
                        && settings.permission_mode.as_deref() == Some("workspace")
                })
            })
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "running configuration was not observed"
        );
    }

    let steer = |target: Option<&str>, effort: Option<&str>, mode: Option<&str>| {
        Command::SteerTurn {
            runtime_id: "synthetic".into(),
            conversation_id: id.clone(),
            target_runtime_id: target.map(str::to_owned),
            turn: crate::ComposedTurn {
                effort: effort.map(str::to_owned),
                permission_mode: mode.map(str::to_owned),
                ..control_turn("steer input")
            },
        }
    };
    let other_model = Some(crate::ModelSelection::Native {
        provider_id: Some("openai".into()),
        model_id: "other-model".into(),
    });
    for command in [
        steer(Some("other-runtime"), None, None),
        steer(Some("synthetic"), None, Some("read-only")),
    ] {
        let error = app
            .execute(view(), command)
            .err()
            .expect("incompatible steer accepted");
        assert!(error.message.contains("排队"), "{error:?}");
    }

    let queue = |operation| Command::EditSendQueue {
        submission_id: None,
        runtime_id: "synthetic".into(),
        conversation_id: id.clone(),
        operation,
    };
    let mut queued_turn = control_turn("keep queued permission change");
    queued_turn.permission_mode = Some("read-only".into());
    queued_turn.effort = Some("low".into());
    app.execute(
        view(),
        queue(crate::SendQueueOperation::Add {
            source_runtime_id: "synthetic".into(),
            target_runtime_id: "synthetic".into(),
            selection: other_model.clone(),
            turn: queued_turn,
        }),
    )
    .unwrap();
    let queued = control_snapshot(&app, &id).follow_up.queue;
    assert_eq!(queued.len(), 1);
    let error = app
        .execute(
            view(),
            queue(crate::SendQueueOperation::Steer {
                id: queued[0].id.clone(),
            }),
        )
        .err()
        .expect("queued permission change was silently stripped");
    assert!(error.message.contains("排队"), "{error:?}");
    assert_eq!(control_snapshot(&app, &id).follow_up.queue, queued);
    assert!(
        !fixture
            .wire()
            .iter()
            .any(|record| record["method"] == "turn/steer")
    );

    for command in [
        steer(Some("synthetic"), Some("low"), Some("workspace")),
        steer(None, Some("not-a-current-model-effort"), None),
    ] {
        assert!(matches!(
            app.execute(view(), command).unwrap(),
            Response::SteerTurn(_)
        ));
    }
    let snapshot = control_snapshot(&app, &id);
    assert_eq!(snapshot.run_state, crate::conversation::RunState::Running);
    assert_eq!(snapshot.follow_up.queue, queued);
    let mut queued_turn = control_turn("insert despite a different next model and effort");
    queued_turn.effort = Some("low".into());
    app.execute(
        view(),
        queue(crate::SendQueueOperation::Add {
            source_runtime_id: "synthetic".into(),
            target_runtime_id: "synthetic".into(),
            selection: other_model,
            turn: queued_turn,
        }),
    )
    .unwrap();
    let queued_with_insert = control_snapshot(&app, &id).follow_up.queue;
    app.execute(
        view(),
        queue(crate::SendQueueOperation::Steer {
            id: queued_with_insert[1].id.clone(),
        }),
    )
    .unwrap();
    assert_eq!(control_snapshot(&app, &id).follow_up.queue, queued);
    let wire = fixture.wire();
    assert_eq!(
        wire.iter()
            .filter(|record| record["method"] == "turn/start")
            .count(),
        1
    );
    let steers: Vec<_> = wire
        .iter()
        .filter(|record| record["method"] == "turn/steer")
        .collect();
    assert_eq!(steers.len(), 3);
    assert!(
        steers
            .iter()
            .all(|record| record["params"]["expectedTurnId"] == "turn-1")
    );
    app.close(true).unwrap();
}

#[test]
fn codex_changed_model_and_cold_source_cannot_bypass_legacy_provider_checks() {
    let fixture = Fixture::codex_control("send-hold");
    let app = open_control_service(&fixture);
    let id = running_codex(&app, &fixture, "synthetic");
    fs::write(fixture.0.join("reject-provider-check"), []).unwrap();
    let before = fixture
        .wire()
        .iter()
        .filter(|record| record["method"] == "thread/read")
        .count();
    let mut selection = native_selection();
    let Some(crate::ModelSelection::Native { model_id, .. }) = &mut selection else {
        unreachable!()
    };
    *model_id = "different-model".into();
    let error = app
        .execute(
            ClientViewId("control-view".into()),
            Command::SendTurn {
                submission_id: None,
                source_runtime_id: "synthetic".into(),
                conversation_id: id.clone(),
                target_runtime_id: "synthetic".into(),
                selection,
                turn: control_turn("must validate different model/provider"),
            },
        )
        .err()
        .expect("provider validation must execute before switching context");
    assert!(
        error.message.contains("provider-check-required"),
        "{error:?}"
    );
    assert_eq!(
        fixture
            .wire()
            .iter()
            .filter(|record| record["method"] == "thread/read")
            .count(),
        before + 1
    );
    assert_eq!(
        fixture
            .wire()
            .iter()
            .filter(|record| record["method"] == "turn/start")
            .count(),
        1
    );
    app.close(true).unwrap();
    let cold = open_control_service(&fixture);
    let error = cold
        .execute(ClientViewId("cold".into()), next_codex_send(&id))
        .err()
        .expect("cold source cannot use an absent confirmed owner");
    assert!(
        error.message.contains("provider-check-required"),
        "{error:?}"
    );
    assert_eq!(
        fixture
            .wire()
            .iter()
            .filter(|record| record["method"] == "turn/start")
            .count(),
        1
    );
    cold.close(true).unwrap();
}

#[test]
fn codex_send_ack_before_echo_never_selects_a_historical_user_and_binds_late_settings() {
    let fixture = Fixture::codex_control("send-late-echo");
    let app = open_control_service(&fixture);
    let id = running_codex(&app, &fixture, "synthetic");
    fs::write(fixture.0.join("release-send"), []).unwrap();
    app.execute(ClientViewId("control-view".into()), next_codex_send(&id))
        .unwrap();
    let before = control_snapshot(&app, &id);
    let historical_users: Vec<_> = before
        .messages
        .iter()
        .filter(|message| message.role == crate::conversation::MessageRole::User)
        .map(|message| message.id.clone())
        .collect();
    assert_eq!(
        historical_users.len(),
        2,
        "exercise both historical U1 and U2"
    );
    let wakes = app
        .native_user_message_events()
        .expect("local owner is open");
    let wait = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let mut turn = control_turn("new input with late native echo");
    turn.effort = Some("high".into());
    let submitted = request_later(
        &app,
        "control-view",
        Command::SendTurn {
            submission_id: Some("late-echo-client-token".into()),
            source_runtime_id: "synthetic".into(),
            conversation_id: id.clone(),
            target_runtime_id: "synthetic".into(),
            selection: native_selection(),
            turn,
        },
    );
    fixture.wait("send-acked");
    let Response::SendTurn(result) = submitted
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap()
    else {
        panic!("send response")
    };
    assert_eq!(
        result.submission_receipt,
        Some(crate::SubmissionReceipt::Sending)
    );
    assert!(
        !serde_json::to_string(&fixture.wire())
            .unwrap()
            .contains("late-echo-client-token")
    );
    // The ACK is real acceptance, but it cannot identify an echo that has not
    // happened. In particular neither historical user is this submission.
    assert!(
        result.submitted_message_id.is_none(),
        "ACK without echo returned historical ID {:?}",
        result.submitted_message_id
    );
    let accepted = result.snapshot.unwrap();
    for turn in accepted
        .transcript_turns
        .iter()
        .filter(|turn| historical_users.contains(&turn.user_message_id))
    {
        assert_ne!(
            turn.settings
                .as_ref()
                .and_then(|settings| settings.effort.as_deref()),
            Some("high")
        );
    }
    // Sending the ACK and advancing the owner do not invent a transcript wake.
    // Hold the echo longer than the old client's expedited-read window.
    wait.block_on(async {
        assert!(
            tokio::time::timeout(Duration::from_millis(350), wakes.next())
                .await
                .is_err()
        );
    });
    fs::write(fixture.0.join("release-echo"), []).unwrap();
    let wake = wait.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), wakes.next())
            .await
            .expect("native input must wake the client without a snapshot read")
            .expect("application owner is still open")
    });
    assert_eq!(wake.conversation_id.as_deref(), Some(id.as_str()));
    let Response::ReadSnapshotUpdate(crate::SnapshotUpdate::Changed { snapshot }) = app
        .execute(
            ClientViewId("control-view".into()),
            Command::ReadSnapshotUpdate {
                runtime_id: "synthetic".into(),
                conversation_id: id.clone(),
                since: accepted.cursor,
            },
        )
        .unwrap()
    else {
        panic!("the wake must lead to an authoritative snapshot delta");
    };
    let new_users: Vec<_> = snapshot
        .messages
        .iter()
        .filter(|message| message.role == crate::conversation::MessageRole::User)
        .collect();
    assert_eq!(new_users.len(), 1);
    let new_user_id = &new_users[0].id;
    assert!(new_user_id.contains("user-3") && !historical_users.contains(new_user_id));
    let settings = snapshot
        .transcript_turns
        .iter()
        .find(|turn| turn.user_message_id == *new_user_id)
        .and_then(|turn| turn.settings.as_ref())
        .expect("late echo must keep accepted settings");
    assert_eq!(settings.effort.as_deref(), Some("high"));
    assert_eq!(settings.model_selection, native_selection());
    assert_eq!(
        fixture
            .wire()
            .iter()
            .filter(|record| record["method"] == "turn/start")
            .count(),
        3
    );
    app.close(true).unwrap();
    wait.block_on(async {
        assert!(
            tokio::time::timeout(Duration::from_secs(2), wakes.next())
                .await
                .expect("the owner must close the stream")
                .is_none()
        );
    });
}

#[test]
fn codex_first_admission_error_still_cleans_failed_owner_when_poll_returns_ok_and_pool_is_full() {
    use crate::local::execution_workers::cleanup_test_gate;
    let fixtures = [
        Fixture::codex_control("failure"),
        Fixture::codex_control("failure"),
        Fixture::codex_control("send-writer-full"),
    ];
    let app = open_control_service(&fixtures[0]);
    let ids: Vec<_> = fixtures
        .iter()
        .enumerate()
        .map(|(index, fixture)| {
            let runtime_id = format!("runtime-{index}");
            if index < 2 {
                return running_codex(&app, fixture, &runtime_id);
            }
            let id = created_codex(&app, fixture, &runtime_id, native_selection());
            // Keep this connection's first input on the compatibility path so
            // the later writer-full attempt really is its first typed admission.
            app.execute(
                ClientViewId("control-view".into()),
                Command::SendTurn {
                    submission_id: None,
                    source_runtime_id: runtime_id.clone(),
                    conversation_id: id.clone(),
                    target_runtime_id: runtime_id,
                    selection: native_selection(),
                    turn: crate::ComposedTurn {
                        attachments: vec![fixture.0.join("mode").to_string_lossy().into_owned()],
                        ..control_turn("compatibility bootstrap before first typed admission")
                    },
                },
            )
            .unwrap();
            id
        })
        .collect();
    let gates: Vec<_> = fixtures
        .iter()
        .map(|fixture| cleanup_test_gate::register(&fixture.0.join("codex")))
        .collect();
    for index in 0..2 {
        let cancel = request_later(
            &app,
            "control-view",
            Command::Cancel {
                runtime_id: format!("runtime-{index}"),
                conversation_id: ids[index].clone(),
            },
        );
        fixtures[index].wait("malformed-written");
        gates[index].0.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            cancel
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .err()
                .expect("failed native control")
                .code,
            "runtime_operation_indeterminate"
        );
    }
    // No typed command has run on runtime-2. Four blocked automatic replies
    // hold the physical protocol reserve; ordinary capacity is independent.
    // Its first typed cancel must fail admission without dispatching an interrupt.
    fs::write(fixtures[2].0.join("saturate-writer"), []).unwrap();
    fixtures[2].wait("writer-blocked");
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let snapshot = runtime_control_snapshot(&app, "runtime-2", &ids[2]);
        if snapshot.messages.iter().any(|message| message.blocks.iter().any(|block|
            matches!(block, crate::DeliveredBlock::Text { text } if text == "synthetic writer receipts primed"))) { break; }
        assert!(Instant::now() < deadline);
    }
    let error = app
        .execute(
            ClientViewId("control-view".into()),
            Command::Cancel {
                runtime_id: "runtime-2".into(),
                conversation_id: ids[2].clone(),
            },
        )
        .err()
        .expect("first admission must be rejected locally");
    assert!(error.message.contains("Codex 控制未派发"), "{error:?}");
    assert!(error.message.contains("budget full"), "{error:?}");
    assert_eq!(
        fixtures[2]
            .wire()
            .iter()
            .filter(|record| record["method"] == "turn/interrupt")
            .count(),
        0,
        "failed admission dispatched no interrupt"
    );
    fs::write(fixtures[2].0.join("fail-connection"), []).unwrap();
    fixtures[2].wait("failure-written");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = app.execute(
            ClientViewId("observer".into()),
            Command::ReadSnapshot {
                runtime_id: "runtime-2".into(),
                conversation_id: ids[2].clone(),
            },
        );
        match snapshot {
            Ok(Response::ReadSnapshot(result))
                if result.snapshot.as_ref().unwrap().run_state
                    == crate::conversation::RunState::Failed =>
            {
                break;
            }
            Err(error) => {
                assert_eq!(error.code, "invalid", "{error:?}");
                assert!(error.message.starts_with("运行时事件读取失败"), "{error:?}");
            }
            _ => {}
        }
        assert!(Instant::now() < deadline);
    }
    // Force later drains too: Native.poll now returns its early Ok while the
    // persistent failure fact still requires shutdown. Capacity remains held.
    for _ in 0..2 {
        let read = request_later(
            &app,
            "observer",
            Command::ReadRuntimeHomeDirectory {
                runtime_id: "runtime-2".into(),
            },
        );
        assert!(read.recv_timeout(Duration::from_secs(2)).unwrap().is_ok());
    }
    assert!(matches!(
        gates[2].0.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    assert!(app.close(true).is_err());
    assert!(crate::repository::Repository::lock_home(&fixtures[0].0.join("app")).is_err());
    gates[0].1.send(()).unwrap();
    gates[2]
        .0
        .recv_timeout(Duration::from_secs(2))
        .expect("failed first admission must retry cleanup after a slot becomes free");
    assert!(
        app.close(true).is_err(),
        "cleanup gate still owns the actual native adapter"
    );
    assert!(crate::repository::Repository::lock_home(&fixtures[0].0.join("app")).is_err());
    gates[1].1.send(()).unwrap();
    gates[2].1.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.close(true).is_err() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(crate::repository::Repository::lock_home(&fixtures[0].0.join("app")).unwrap());
}

#[test]
fn codex_terminal_without_echo_never_binds_accepted_settings_to_the_next_submission() {
    let fixture = Fixture::codex_control("send-late-echo");
    let app = open_control_service(&fixture);
    let id = running_codex(&app, &fixture, "synthetic");
    fs::write(fixture.0.join("release-send"), []).unwrap();
    app.execute(ClientViewId("control-view".into()), next_codex_send(&id))
        .unwrap();
    let historical = control_snapshot(&app, &id);
    assert_eq!(
        historical
            .messages
            .iter()
            .filter(|message| message.role == crate::conversation::MessageRole::User)
            .count(),
        2
    );
    let send = |text: &str, effort: &str| Command::SendTurn {
        submission_id: None,
        source_runtime_id: "synthetic".into(),
        conversation_id: id.clone(),
        target_runtime_id: "synthetic".into(),
        selection: native_selection(),
        turn: crate::ComposedTurn {
            effort: Some(effort.into()),
            ..control_turn(text)
        },
    };
    let Response::SendTurn(third) = request_later(
        &app,
        "control-view",
        send("S3 accepted without echo", "high"),
    )
    .recv_timeout(Duration::from_secs(2))
    .unwrap()
    .unwrap() else {
        panic!("S3 accepted response")
    };
    fixture.wait("send-acked");
    assert!(third.submitted_message_id.is_none());
    fs::write(fixture.0.join("finish-without-echo"), []).unwrap();
    fixture.wait("no-echo-terminal");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = control_snapshot(&app, &id);
        if snapshot.run_state == crate::conversation::RunState::Idle {
            assert_eq!(
                snapshot
                    .messages
                    .iter()
                    .filter(|message| message.role == crate::conversation::MessageRole::User)
                    .count(),
                2
            );
            break;
        }
        assert!(Instant::now() < deadline);
    }
    // S4 has its own early echo and deliberately different requested effort.
    // Neither an old count-only pending request nor a later bind may overwrite it.
    fs::write(fixture.0.join("mode"), "send-early-next").unwrap();
    let Response::SendTurn(fourth) =
        request_later(&app, "control-view", send("S4 owns its echo", "low"))
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap()
    else {
        panic!("S4 accepted response")
    };
    assert!(
        fourth
            .submitted_message_id
            .as_ref()
            .is_some_and(|id| id.contains("user-4"))
    );
    for snapshot in [fourth.snapshot.unwrap(), control_snapshot(&app, &id)] {
        assert!(
            !snapshot
                .messages
                .iter()
                .any(|message| message.id.contains("user-3"))
        );
        let settings = snapshot
            .transcript_turns
            .iter()
            .find(|turn| turn.user_message_id.contains("user-4"))
            .and_then(|turn| turn.settings.as_ref())
            .expect("S4 must retain its own settings");
        assert_eq!(
            settings.effort.as_deref(),
            Some("low"),
            "S3 pending settings crossed the accepted turn boundary"
        );
        assert_eq!(settings.model_selection, native_selection());
    }
    assert_eq!(
        fixture
            .wire()
            .iter()
            .filter(|record| record["method"] == "turn/start")
            .count(),
        4
    );
    app.close(true).unwrap();
}

#[test]
fn codex_created_draft_first_send_keeps_owner_reply_and_cancel_live_before_business_ack() {
    for selection in [native_selection(), None] {
        let fixture = Fixture::codex_control("draft-first-hold");
        let app = open_control_service(&fixture);
        let id = created_codex(&app, &fixture, "synthetic", selection.clone());
        let created = control_snapshot(&app, &id);
        assert!(created.messages.is_empty());
        assert_eq!(created.run_state, crate::conversation::RunState::Idle);
        let provider_reads = provider_wire_reads(&fixture);
        fs::write(fixture.0.join("changed-default"), []).unwrap();
        let flag = Arc::new(AtomicBool::new(false));
        let (reply, submitted) = mpsc::channel();
        let sending = app.clone();
        let cancellation = flag.clone();
        let command = Command::SendTurn {
            submission_id: None,
            source_runtime_id: "synthetic".into(),
            conversation_id: id.clone(),
            target_runtime_id: "synthetic".into(),
            selection,
            turn: control_turn("first input belongs to the created native context"),
        };
        std::thread::spawn(move || {
            let _ = reply.send(sending.execute_cancellable(
                ClientViewId("first-sender".into()),
                command,
                cancellation,
            ));
        });
        fixture.wait("send-entered");
        // Create has already returned its identity, but Send has no business
        // ACK yet. A different view must still read this exact native session.
        let observed = control_snapshot(&app, &id);
        assert_eq!(observed.conversation.id, created.conversation.id);
        reply_before_submission_ack(&app, &fixture, &id);
        assert!(matches!(
            submitted.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        flag.store(true, Ordering::Release);
        fixture.wait("entered-1");
        fs::write(fixture.0.join("release-1"), []).unwrap();
        assert_read_before_release(&app);
        assert!(
            matches!(submitted.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "control ACK cannot become Send acceptance"
        );
        fs::write(fixture.0.join("release-send"), []).unwrap();
        let Response::SendTurn(result) = submitted
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap()
        else {
            panic!("first Send response")
        };
        assert_eq!(result.snapshot.as_ref().unwrap().conversation.id, id);
        assert_eq!(
            result.snapshot.as_ref().unwrap().run_state,
            crate::conversation::RunState::Stopping
        );
        assert!(
            result
                .submitted_message_id
                .as_ref()
                .is_some_and(|id| id.contains("draft-user"))
        );
        let wire = fixture.wire();
        assert_eq!(
            wire.iter()
                .filter(|record| record["method"] == "thread/start")
                .count(),
            1
        );
        assert_eq!(
            wire.iter()
                .filter(|record| record["method"] == "turn/start")
                .count(),
            1
        );
        let stops: Vec<_> = wire
            .iter()
            .filter(|record| record["method"] == "turn/interrupt")
            .collect();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0]["params"]["turnId"], "draft-turn");
        assert_eq!(
            provider_wire_reads(&fixture),
            provider_reads,
            "first Send must reuse Create's captured provider/context"
        );
        assert_eq!(
            fs::read_to_string(fixture.0.join("thread-processes")).unwrap(),
            fs::read_to_string(fixture.0.join("turn-processes")).unwrap()
        );
        app.close(true).unwrap();
    }
}

#[test]
fn codex_created_draft_changed_selection_keeps_the_compatibility_retarget_path() {
    let fixture = Fixture::codex_control("send-hold");
    let app = open_control_service(&fixture);
    let id = created_codex(&app, &fixture, "synthetic", native_selection());
    let replacement = Some(crate::ModelSelection::Native {
        provider_id: Some("openai".into()),
        model_id: "different-first-model".into(),
    });
    let response = app
        .execute(
            ClientViewId("different-selector".into()),
            Command::SendTurn {
                submission_id: None,
                source_runtime_id: "synthetic".into(),
                conversation_id: id,
                target_runtime_id: "synthetic".into(),
                selection: replacement,
                turn: control_turn("explicitly retarget the unsubmitted draft"),
            },
        )
        .unwrap();
    assert!(matches!(response, Response::SendTurn(_)));
    let wire = fixture.wire();
    assert_eq!(
        wire.iter()
            .filter(|record| record["method"] == "thread/start")
            .count(),
        2,
        "different selection must not borrow the original Create proof"
    );
    let turns: Vec<_> = wire
        .iter()
        .filter(|record| record["method"] == "turn/start")
        .collect();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0]["params"]["model"], "different-first-model");
    app.close(true).unwrap();
}

#[test]
fn codex_created_draft_first_send_rejection_retries_but_unknown_retires_recreation_authority() {
    for rejected in [true, false] {
        let fixture = Fixture::codex_control(if rejected {
            "draft-first-reject"
        } else {
            "draft-first-disconnect"
        });
        let app = open_control_service(&fixture);
        let id = created_codex(&app, &fixture, "synthetic", native_selection());
        let gate = (!rejected).then(|| {
            crate::local::execution_workers::cleanup_test_gate::register(&fixture.0.join("codex"))
        });
        let error = request_later(&app, "first-sender", next_codex_send(&id))
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .err()
            .expect("first send must report its actual delivery");
        fixture.wait("send-entered");
        if rejected {
            assert_eq!(error.kind, FailureKind::Invalid);
            assert!(error.message.contains("first input explicitly rejected"));
            fs::write(fixture.0.join("mode"), "draft-first-hold").unwrap();
            let retry = request_later(&app, "retry-view", next_codex_send(&id));
            let pid = fs::read_to_string(fixture.0.join("thread-processes")).unwrap();
            fixture.wait(&format!("send-entered-{}", pid.trim()));
            reply_before_submission_ack(&app, &fixture, &id);
            fs::write(fixture.0.join("release-send"), []).unwrap();
            assert!(matches!(
                retry.recv_timeout(Duration::from_secs(2)).unwrap().unwrap(),
                Response::SendTurn(_)
            ));
            assert_eq!(
                fixture
                    .wire()
                    .iter()
                    .filter(|record| record["method"] == "thread/start")
                    .count(),
                1
            );
            assert_eq!(
                fixture
                    .wire()
                    .iter()
                    .filter(|record| record["method"] == "turn/start")
                    .count(),
                2,
                "only explicit retry may submit again"
            );
        } else {
            assert_eq!(error.code, "runtime_operation_indeterminate");
            let (entered, release) = gate.unwrap();
            entered.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(app.close(true).is_err());
            assert!(crate::repository::Repository::lock_home(&fixture.0.join("app")).is_err());
            assert_eq!(
                fixture
                    .wire()
                    .iter()
                    .filter(|record| record["method"] == "turn/start")
                    .count(),
                1
            );
            release.send(()).unwrap();
            fs::write(fixture.0.join("mode"), "send-hold").unwrap();
            let reads = fixture
                .wire()
                .iter()
                .filter(|record| record["method"] == "thread/read")
                .count();
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let reopened = app
                    .execute(
                        ClientViewId("recovery".into()),
                        Command::OpenConversation {
                            runtime_id: "synthetic".into(),
                            conversation_id: id.clone(),
                        },
                    )
                    .unwrap();
                let Response::OpenConversation(result) = reopened else {
                    panic!("recovery read response")
                };
                assert_eq!(result.snapshot.unwrap().conversation.id, id);
                if fixture
                    .wire()
                    .iter()
                    .filter(|record| record["method"] == "thread/read")
                    .count()
                    > reads
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "uncertain draft must recheck the original created identity rather than stay recreatable"
                );
            }
            // A subsequent explicit action may still need ordinary cold/control
            // validation (the helper has no real writer registry). It must never
            // use History+unsubmitted as permission to create another thread.
            let _ = app.execute(ClientViewId("recovery".into()), next_codex_send(&id));
            assert_eq!(
                fixture
                    .wire()
                    .iter()
                    .filter(|record| record["method"] == "thread/start")
                    .count(),
                1
            );
        }
        app.close(true).unwrap();
    }
}

#[test]
fn codex_created_draft_stale_view_completion_cannot_retire_a_later_creation_grant() {
    let fixture = Fixture::codex_control("draft-first-hold");
    let app = open_control_service(&fixture);
    let id = created_codex(&app, &fixture, "synthetic", native_selection());
    let first = request_later(&app, "reused-view", next_codex_send(&id));
    fixture.wait("send-entered");
    app.execute(ClientViewId("reused-view".into()), Command::ReleaseView)
        .unwrap();
    let Response::CreateConversation(second) = app
        .execute(
            ClientViewId("reused-view".into()),
            Command::CreateConversation {
                runtime_id: "synthetic".into(),
                cwd: Some(fixture.0.to_string_lossy().into_owned()),
                project: None,
                selection: None,
            },
        )
        .unwrap()
    else {
        panic!("second Create response")
    };
    // The synthetic server intentionally reuses its public/native ID across two
    // distinct connections. SessionKey plus creation token must keep them apart.
    let second_id = second.snapshot.unwrap().conversation.id;
    assert_eq!(second_id, id);
    fs::write(fixture.0.join("release-send"), []).unwrap();
    let error = first
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .err()
        .expect("closed original view");
    assert_eq!(error.code, "message_delivery_accepted");
    fs::remove_file(fixture.0.join("release-send")).unwrap();
    let second_send = request_later(
        &app,
        "reused-view",
        Command::SendTurn {
            submission_id: None,
            source_runtime_id: "synthetic".into(),
            conversation_id: second_id,
            target_runtime_id: "synthetic".into(),
            selection: None,
            turn: control_turn("first input of the second creation"),
        },
    );
    let pids = fs::read_to_string(fixture.0.join("thread-processes")).unwrap();
    let second_pid = pids.lines().nth(1).expect("two distinct created owners");
    fixture.wait(&format!("send-entered-{second_pid}"));
    let snapshot = control_snapshot(&app, &id);
    assert_eq!(snapshot.conversation.id, id);
    assert!(matches!(
        second_send.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    fs::write(fixture.0.join("release-send"), []).unwrap();
    assert!(matches!(
        second_send
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap(),
        Response::SendTurn(_)
    ));
    let wire = fixture.wire();
    assert_eq!(
        wire.iter()
            .filter(|record| record["method"] == "thread/start")
            .count(),
        2
    );
    assert_eq!(
        wire.iter()
            .filter(|record| record["method"] == "turn/start")
            .count(),
        2
    );
    let turns = fs::read_to_string(fixture.0.join("turn-processes")).unwrap();
    assert_eq!(turns.lines().last(), Some(second_pid));
    app.close(true).unwrap();
}

#[test]
fn submission_receipt_running_queue_and_poll_share_exact_client_identity() {
    let fixture = Fixture::codex_control("reject");
    let app = open_control_service(&fixture);
    let id = running_codex(&app, &fixture, "synthetic");
    let mut queue_ids = Vec::new();
    for token in ["queue-client-a", "queue-client-b"] {
        let Response::EditSendQueue(result) = app
            .execute(
                ClientViewId("control-view".into()),
                Command::EditSendQueue {
                    submission_id: Some(token.into()),
                    runtime_id: "synthetic".into(),
                    conversation_id: id.clone(),
                    operation: crate::SendQueueOperation::Add {
                        source_runtime_id: "synthetic".into(),
                        target_runtime_id: "synthetic".into(),
                        selection: native_selection(),
                        turn: control_turn("identical queued input"),
                    },
                },
            )
            .unwrap()
        else {
            panic!("queue receipt")
        };
        let Some(crate::SubmissionReceipt::Queued { queue_id }) = result.submission_receipt else {
            panic!("running submission must remain queued")
        };
        let accepted = result.snapshot.unwrap();
        assert!(accepted.messages.iter().all(|message| {
            !serde_json::to_string(message)
                .unwrap()
                .contains("identical queued input")
        }));
        assert_eq!(accepted.follow_up.queue.last().unwrap().id, queue_id);
        assert_eq!(
            accepted
                .follow_up
                .queue
                .last()
                .unwrap()
                .submission_id
                .as_deref(),
            Some(token)
        );
        queue_ids.push(queue_id);
    }
    assert_ne!(queue_ids[0], queue_ids[1]);
    let poll = control_snapshot(&app, &id);
    assert_eq!(poll.follow_up.queue.len(), 2);
    assert_eq!(
        poll.follow_up.queue[0].submission_id.as_deref(),
        Some("queue-client-a")
    );
    assert_eq!(
        poll.follow_up.queue[1].submission_id.as_deref(),
        Some("queue-client-b")
    );
    let Response::EditSendQueue(removed) = app
        .execute(
            ClientViewId("control-view".into()),
            Command::EditSendQueue {
                submission_id: None,
                runtime_id: "synthetic".into(),
                conversation_id: id.clone(),
                operation: crate::SendQueueOperation::Remove {
                    id: queue_ids[0].clone(),
                },
            },
        )
        .unwrap()
    else {
        panic!("remove receipt")
    };
    assert!(removed.submission_receipt.is_none());
    assert_eq!(
        control_snapshot(&app, &id).follow_up.queue[0].id,
        queue_ids[1]
    );
    assert_eq!(
        fixture
            .wire()
            .iter()
            .filter(|record| record["method"] == "turn/start")
            .count(),
        1
    );
    app.close(true).unwrap();
}

#[test]
fn submission_receipt_idle_queue_reports_actual_dispatch_or_retained_rejection() {
    for mode in ["send-late-echo", "send-terminal"] {
        let fixture = Fixture::codex_control(mode);
        let app = open_control_service(&fixture);
        let id = running_codex(&app, &fixture, "synthetic");
        fs::write(fixture.0.join("release-send"), []).unwrap();
        app.execute(ClientViewId("control-view".into()), next_codex_send(&id))
            .unwrap();
        let Response::EditSendQueue(result) = app
            .execute(
                ClientViewId("control-view".into()),
                Command::EditSendQueue {
                    submission_id: Some("never-model-input-token".into()),
                    runtime_id: "synthetic".into(),
                    conversation_id: id.clone(),
                    operation: crate::SendQueueOperation::Add {
                        source_runtime_id: "synthetic".into(),
                        target_runtime_id: "synthetic".into(),
                        selection: native_selection(),
                        turn: control_turn("late queue reached idle owner"),
                    },
                },
            )
            .unwrap()
        else {
            panic!("queue dispatch receipt")
        };
        let snapshot = result.snapshot.unwrap();
        if mode == "send-late-echo" {
            assert_eq!(
                result.submission_receipt,
                Some(crate::SubmissionReceipt::Sending)
            );
            assert!(snapshot.follow_up.queue.is_empty());
        } else {
            // This mode completes the parent but retains an approval. Being
            // idle does not mean the new input was actually dispatched.
            let queued = &snapshot.follow_up.queue[0];
            assert_eq!(
                result.submission_receipt,
                Some(crate::SubmissionReceipt::Queued {
                    queue_id: queued.id.clone(),
                })
            );
            assert_eq!(
                queued.submission_id.as_deref(),
                Some("never-model-input-token")
            );
            assert_eq!(
                snapshot.follow_up.paused,
                Some(crate::conversation::QueuePause::SendFailed)
            );
        }
        let wire = fixture.wire();
        assert!(
            !serde_json::to_string(&wire)
                .unwrap()
                .contains("never-model-input-token")
        );
        let starts: Vec<_> = wire
            .iter()
            .filter(|record| record["method"] == "turn/start")
            .collect();
        assert_eq!(starts.len(), if mode == "send-late-echo" { 3 } else { 2 });
        if mode == "send-late-echo" {
            assert!(
                serde_json::to_string(starts.last().unwrap())
                    .unwrap()
                    .contains("late queue reached idle owner")
            );
        }
        fs::write(fixture.0.join("release-echo"), []).unwrap();
        app.close(true).unwrap();
    }
}

#[test]
fn claude_background_send_reuses_inherited_options_and_queues_saved_configuration_change() {
    let fixture = Fixture::new();
    let app = open_control_service(&fixture);
    let view = ClientViewId("control-view".into());
    let mut runtime = crate::config::RuntimeInstance {
        enabled: true,
        id: "synthetic".into(),
        name: "Synthetic Claude".into(),
        type_id: crate::conversation::RuntimeKind::ClaudeCode,
        gateway_id: "default".into(),
        settings: BTreeMap::from([
            (
                "binary".into(),
                fixture.0.join("claude").display().to_string(),
            ),
            (
                "agentDir".into(),
                fixture.0.join("native").display().to_string(),
            ),
        ]),
    };
    let save = |runtime| {
        app.execute(
            view.clone(),
            Command::UpsertRuntimeWithExecution {
                runtime,
                execution: crate::RuntimeExecution {
                    home: crate::RuntimeHome::Managed { remote_root: None },
                    ..Default::default()
                },
            },
        )
        .unwrap()
    };
    save(runtime.clone());
    let Response::CreateConversation(created) = app
        .execute(
            view.clone(),
            Command::CreateConversation {
                runtime_id: "synthetic".into(),
                cwd: Some(fixture.0.display().to_string()),
                project: None,
                selection: None,
            },
        )
        .unwrap()
    else {
        panic!("create response");
    };
    let id = created.snapshot.unwrap().conversation.id;
    let send = |turn| {
        let Response::SendTurn(result) = app
            .execute(
                view.clone(),
                Command::SendTurn {
                    submission_id: None,
                    source_runtime_id: "synthetic".into(),
                    conversation_id: id.clone(),
                    target_runtime_id: "synthetic".into(),
                    selection: None,
                    turn,
                },
            )
            .unwrap()
        else {
            panic!("send response");
        };
        result
    };
    let wait_idle_with_work = || {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let snapshot = control_snapshot(&app, &id);
            if snapshot.run_state == crate::conversation::RunState::Idle
                && snapshot.follow_up.can_stop_sub_agents
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "background work did not hold an idle child"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let first = send(crate::ComposedTurn {
        effort: Some("medium".into()),
        permission_mode: Some("manual".into()),
        ..control_turn("start background work")
    });
    assert_eq!(
        first.submission_receipt,
        Some(crate::SubmissionReceipt::Sending)
    );
    wait_idle_with_work();
    let continued = std::thread::scope(|scope| {
        let pending = scope.spawn(|| send(control_turn("same settings follow-up")));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fixture.0.join("composed-queued").exists() {
            assert!(Instant::now() < deadline, "Claude did not queue the composed input");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!pending.is_finished(), "a queued native command cannot settle the send");
        assert!(control_snapshot(&app, &id).messages.iter().all(|message| {
            !message.blocks.iter().any(|block| matches!(block, crate::DeliveredBlock::Text { text } if text == "same settings follow-up"))
        }));
        fs::write(fixture.0.join("composed-release"), []).unwrap();
        pending.join().unwrap()
    });
    assert_eq!(
        continued.submission_receipt,
        Some(crate::SubmissionReceipt::Sending)
    );
    assert!(continued.submitted_message_id.is_some());
    wait_idle_with_work();
    let replacement = fixture.0.join("claude-replacement");
    fs::copy(fixture.0.join("claude"), &replacement).unwrap();
    runtime
        .settings
        .insert("binary".into(), replacement.display().to_string());
    save(runtime);
    let receipt = send(control_turn("follow-up after a runtime edit")).submission_receipt;
    let snapshot = control_snapshot(&app, &id);
    let [queued] = snapshot.follow_up.queue.as_slice() else {
        panic!("expected one waiting message");
    };
    assert_eq!(
        receipt,
        Some(crate::SubmissionReceipt::Queued {
            queue_id: queued.id.clone()
        })
    );
    assert_eq!(
        snapshot.follow_up.waiting,
        Some(crate::conversation::QueueWait::SubAgents)
    );
    let owners = fs::read_to_string(fixture.0.join("background-owner")).unwrap();
    let owners: Vec<_> = owners.lines().collect();
    assert_eq!(owners.len(), 2);
    assert_eq!(owners[0], owners[1]);
    assert_eq!(
        fs::read_to_string(fixture.0.join("steer-wire")).unwrap(),
        "start background work\nsame settings follow-up\n"
    );
    app.close(true).unwrap();
}

#[test]
fn managed_codex_sends_once_when_the_last_background_work_settles_during_submission() {
    use crate::config::{ExecutionBackend, Protocol, ProviderQuotaSource, RequestProfile};
    use crate::conversation::RunState;
    use velune_application_api::{ProviderDraft, ProviderModel};

    let fixture = Fixture::codex_control("managed-queue-background");
    let app = open_control_service(&fixture);
    let execute = |command| app.execute(ClientViewId("control-view".into()), command);
    let Response::SaveProvider(saved) = execute(Command::SaveProvider {
        gateway_id: "default".into(),
        provider: ProviderDraft {
            id: "synthetic".into(),
            name: "Synthetic transport".into(),
            protocol: Protocol::Responses,
            // The fixture only records native dispatch. It never calls upstream.
            endpoint: "http://127.0.0.1:9/v1".into(),
            execution_backend: ExecutionBackend::Http,
            request_profile: RequestProfile::Native,
            codex_client_version: None,
            authentication_source: crate::ProviderAuthenticationSource::ApiKey,
            quota_source: ProviderQuotaSource::None,
            quota_limits: Vec::new(),
            models: vec![ProviderModel {
                record_key: String::new(),
                provider_model_id: "synthetic-managed".into(),
                nickname: String::new(),
                icon: None,
                context_window: None,
                max_output_tokens: None,
                reasoning_levels: None,
                request_parameters: None,
            }],
        },
        authentication_edit: crate::AuthenticationEdit::SetApiKey {
            value: "synthetic-key".into(),
        },
        quota_authentication_edit: None,
    })
    .unwrap() else {
        panic!("save provider response");
    };
    let selection = Some(crate::ModelSelection::Managed {
        model_record_key: saved.gateways[0].providers[0].models[0].record_key.clone(),
    });
    let id = created_codex(&app, &fixture, "synthetic", selection.clone());
    let send = |text| {
        let Response::SendTurn(result) = execute(Command::SendTurn {
            submission_id: None,
            source_runtime_id: "synthetic".into(),
            conversation_id: id.clone(),
            target_runtime_id: "synthetic".into(),
            selection: selection.clone(),
            turn: control_turn(text),
        })
        .unwrap() else {
            panic!("send response");
        };
        assert_eq!(
            result.submission_receipt,
            Some(crate::SubmissionReceipt::Sending)
        );
    };
    let first_text = "start managed background";
    let next_text = "send during managed background settlement";
    send(first_text);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = control_snapshot(&app, &id);
        if snapshot.run_state == RunState::Idle && snapshot.sub_agents.len() == 2 {
            assert!(snapshot.actions.can_send);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "parent and background did not separate"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    fs::write(fixture.0.join("finish-background-during-provider-read"), []).unwrap();
    send(next_text);
    assert!(fixture.0.join("managed-background-finished").exists());

    let wire = fixture.wire();
    assert!(wire.iter().any(|record| {
        record["method"] == "thread/start" && record["params"]["modelProvider"] == "velune"
    }));
    let starts: Vec<_> = wire
        .iter()
        .filter(|record| record["method"] == "turn/start")
        .map(|record| &record["params"])
        .collect();
    assert_eq!(starts.len(), 2, "one native write per user input");
    for (start, text) in starts.iter().zip([first_text, next_text]) {
        assert_eq!(start["threadId"], "thread");
        assert_eq!(start["input"][0]["text"], text);
    }
    assert_eq!(starts[0]["model"], starts[1]["model"]);
    app.close(true).unwrap();
}

#[test]
fn submission_receipt_send_falls_back_to_queue_while_background_work_holds_process() {
    let fixture = Fixture::codex_control("queue-background");
    let app = open_control_service(&fixture);
    let id = running_codex(&app, &fixture, "synthetic");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = control_snapshot(&app, &id);
        if snapshot.run_state == crate::conversation::RunState::Idle
            && snapshot.sub_agents.len() == 2
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "background-only fixture did not settle"
        );
    }
    let selection = Some(crate::ModelSelection::Native {
        provider_id: Some("openai".into()),
        model_id: "new-model-requires-process".into(),
    });
    let cancelled = app
        .execute_cancellable(
            ClientViewId("control-view".into()),
            Command::SendTurn {
                submission_id: Some("withdrawn-client".into()),
                source_runtime_id: "synthetic".into(),
                conversation_id: id.clone(),
                target_runtime_id: "synthetic".into(),
                selection: selection.clone(),
                turn: control_turn("withdrawn before admission"),
            },
            Arc::new(AtomicBool::new(true)),
        )
        .err()
        .expect("cancelled request must not enter the queue");
    assert_eq!(cancelled.code, "submission_cancelled");
    assert!(control_snapshot(&app, &id).follow_up.queue.is_empty());
    let Response::SendTurn(result) = app
        .execute(
            ClientViewId("control-view".into()),
            Command::SendTurn {
                submission_id: Some("send-client-exact".into()),
                source_runtime_id: "synthetic".into(),
                conversation_id: id.clone(),
                target_runtime_id: "synthetic".into(),
                selection,
                turn: control_turn("send intent that actually waits"),
            },
        )
        .unwrap()
    else {
        panic!("send receipt")
    };
    let Some(crate::SubmissionReceipt::Queued { queue_id }) = result.submission_receipt else {
        panic!("OutstandingWork must report actual queue acceptance")
    };
    let snapshot = control_snapshot(&app, &id);
    assert_eq!(snapshot.follow_up.queue.len(), 1);
    let queued = &snapshot.follow_up.queue[0];
    assert_eq!(queued.id, queue_id);
    assert_eq!(queued.submission_id.as_deref(), Some("send-client-exact"));
    assert_eq!(queued.text, "send intent that actually waits");
    assert!(snapshot.messages.iter().all(|message| {
        !serde_json::to_string(message)
            .unwrap()
            .contains("send intent that actually waits")
    }));
    assert_eq!(
        fixture
            .wire()
            .iter()
            .filter(|record| record["method"] == "turn/start")
            .count(),
        1
    );
    app.close(true).unwrap();
}
