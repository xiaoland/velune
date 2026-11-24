use super::*;

#[test]
fn claude_fork_resolves_the_selected_native_message_across_projection_drift() {
    fn view(inputs: &[(&str, &str)]) -> ConversationSnapshot {
        let messages: Vec<_> = inputs
            .iter()
            .map(|(id, text)| {
                json!({"id":id,"role":"user","completed":true,
                    "blocks":[{"kind":"text","text":text}]})
            })
            .collect();
        serde_json::from_value(json!({
                "revision":1,"conversation":{"id":"claude:source","runtimeId":"claude","title":{"source":"untitled"},"canRename":false,"canDelete":false},
                "contextRuntimeId":"claude","runState":"idle","messages":messages,
                "transcriptItems":[],"transcriptOutline":[],"transcriptMessageIdentities":[],
                "actions":{"inputAccess":"editable","canSend":true,"canCancel":false,"canSwitch":true}
            })).unwrap()
    }

    let mut current = view(&[("older", "first"), ("selected", "same text")]);
    let cold = view(&[
        ("older", "first"),
        ("lookalike", "same text"),
        ("selected", "same text"),
    ]);
    let users: Vec<_> = cold.messages.iter().collect();
    let resolve = |current: &ConversationSnapshot, users: &[&Message]| {
        claude_fork_user_index(current, users, 1, &current.messages[1])
    };
    assert_eq!(resolve(&current, &users), Some(2));
    assert_eq!(resolve(&current, &users[..2]), None);

    current.messages[1].id = "9:11:user".into();
    assert_eq!(resolve(&current, &[users[0], users[2]]), Some(1));
    assert_eq!(resolve(&current, &users), None);
    assert_eq!(resolve(&current, &users[1..]), None);

    current
        .message_identity_confirmations
        .push(crate::conversation::MessageIdentityConfirmation {
            previous_id: "9:11:user".into(),
            current_id: "selected".into(),
        });
    assert_eq!(resolve(&current, &users), Some(2));
}

#[test]
fn linked_edit_accepts_legacy_error_prefix_but_rejects_changed_source_content() {
    let root = std::env::temp_dir().join(format!("velune-prefix-edit-{}", crate::new_record_key()));
    let mut app = crate::Application::open(crate::Options {
        home_directory: root.clone(),
        resources_directory: root.clone(),
    })
    .unwrap();
    let core = &mut app.runtime;
    core.runtime_instances = ["source", "tail"]
        .into_iter()
        .map(|id| RuntimeInstance {
            enabled: true,
            id: id.into(),
            name: id.into(),
            type_id: RuntimeKind::Acp,
            gateway_id: String::new(),
            settings: Default::default(),
        })
        .collect();
    let snapshot = |runtime: &str, id: &str, messages: serde_json::Value| -> ConversationSnapshot {
        serde_json::from_value(json!({
                "revision":1,"conversation":{"id":id,"runtimeId":runtime,"title":{"source":"native","text":"Synthetic"},"cwd":"/synthetic","canRename":false,"canDelete":false},
                "contextRuntimeId":runtime,"runState":"idle","messages":messages,
                "transcriptItems":[],"transcriptOutline":[],"transcriptMessageIdentities":[],
                "actions":{"inputAccess":"editable","canSend":true,"canCancel":false,"canSwitch":true}
            })).unwrap()
    };
    let source = snapshot(
        "source",
        "source:native",
        json!([
            {"id":"source-user","role":"user","completed":true,"blocks":[{"kind":"text","text":"original source"}]},
            {"id":"source-error","role":"system","completed":true,"blocks":[{"kind":"runtimeFailure","failure":"other","detail":"API Error\nClaude Code 错误类型：overloaded_error","retry":null}]}
        ]),
    );
    let mut legacy = source.messages.clone();
    legacy[1].blocks = vec![
        MessageBlock::Notice {
            text: "API Error".into(),
        },
        MessageBlock::Notice {
            text: "Claude Code 错误类型：overloaded_error".into(),
        },
    ];
    let stored = super::super::continuation::prefix_digest(&legacy).unwrap();
    let tail = snapshot(
        "tail",
        "tail:native",
        json!([
            {"id":"tail-user","role":"user","completed":true,"blocks":[{"kind":"text","text":"edit this"}]}
        ]),
    );
    let link: crate::config::ConversationLink = serde_json::from_value(json!({
            "id":"source:logical","segments":[
                {"runtimeInstanceId":"source","runtimeTypeId":RuntimeKind::Acp,"nativeConversationId":"source:native","cwd":"/synthetic","cutoff":{"messageCount":2,"prefixDigest":stored},"handoff":null,"imported":null,"deleted":false},
                {"runtimeInstanceId":"tail","runtimeTypeId":RuntimeKind::Acp,"nativeConversationId":"tail:native","cwd":"/synthetic","cutoff":null,"handoff":null,"imported":null,"deleted":false}
            ]
        })).unwrap();
    core.conversation_links.push(link.clone());
    let (tail, mut projection) = core
        .read_link_with(&link, |segment| {
            Ok(if segment.runtime_instance_id == "source" {
                source.clone().into()
            } else {
                tail.clone().into()
            })
        })
        .unwrap();
    projection
        .sealed_sources
        .insert(source.conversation.id.clone(), source.clone());
    core.session.active_state = ActiveState::History(Box::new(tail));
    core.session.logical_projection = Some(projection);
    let current = core.current_snapshot().unwrap();
    assert!(
        core.read_edit_sources(&current).is_ok(),
        "a display upgrade must not block a linked edit"
    );
    let cached = core
        .session
        .logical_projection
        .as_mut()
        .unwrap()
        .sealed_sources
        .get_mut("source:native")
        .unwrap();
    cached.messages[0].blocks = vec![MessageBlock::Text {
        text: "changed source".into(),
    }];
    assert!(
        core.read_edit_sources(&current)
            .unwrap_err()
            .to_string()
            .contains("会话来源已改变")
    );
    app.close_stopping_work().unwrap();
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn edit_revision_survives_finished_activity_projection_reload() {
    let root =
        std::env::temp_dir().join(format!("velune-edit-settled-{}", crate::new_record_key()));
    std::fs::create_dir_all(&root).unwrap();
    let mut app = crate::Application::open(crate::Options {
        home_directory: root.clone(),
        resources_directory: root.clone(),
    })
    .unwrap();
    let mut view: ConversationSnapshot = serde_json::from_value(json!({
            "revision":1,
            "conversation":{"id":"pi:settled","runtimeId":"pi","title":{"source":"native","text":"Synthetic"},"canRename":false,"canDelete":false},
            "contextRuntimeId":"pi","runState":"idle",
            "messages":[
                {"id":"u1","role":"user","completed":true,"blocks":[{"kind":"text","text":"first"}]},
                {"id":"a1","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"reply"}]},
                {"id":"u2","role":"user","completed":true,"blocks":[{"kind":"text","text":"second"}]},
                {"id":"a2","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"last reply"}]}
            ],
            "transcriptItems":[],"transcriptOutline":[],"transcriptMessageIdentities":[],
            "actions":{"inputAccess":"editable","canSend":true,"canCancel":false,"canSwitch":true}
        })).unwrap();
    view.executions
        .begin(crate::conversation::ExecutionKind::UserTurn, false);
    view.executions.attach("u2");
    view.executions.attach("a2");
    view.executions
        .finish(Some(crate::conversation::TurnOutcome::Completed));
    app.runtime.session.active_state = ActiveState::History(Box::new(view.clone().into()));
    let before = app.runtime.edit_revision(&view).unwrap();
    let mut cold = view.clone();
    cold.executions.current = None;
    assert_eq!(
        before,
        app.runtime.edit_revision(&cold).unwrap(),
        "a finished activity is not a history mutation"
    );
    app.runtime
        .check_edit_revision(&cold, Some(&before))
        .unwrap();
    let mut changed = cold.clone();
    changed.messages[3].blocks = vec![MessageBlock::Text {
        text: "new native content".into(),
    }];
    assert!(
        app.runtime
            .check_edit_revision(&changed, Some(&before))
            .is_err()
    );
    cold.executions
        .begin(crate::conversation::ExecutionKind::UserTurn, false);
    cold.run_state = crate::conversation::RunState::Running;
    assert!(
        app.runtime
            .check_edit_revision(&cold, Some(&before))
            .is_err(),
        "a new active writer still needs confirmation"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn claude_message_edits_reconcile_and_recheck_native_history() {
    use std::{io::Write, os::unix::fs::PermissionsExt};

    let root = std::env::temp_dir().join(format!("velune-claude-edit-{}", crate::new_record_key()));
    let native = root.join("native");
    let project = native.join("projects/synthetic");
    std::fs::create_dir_all(&project).unwrap();
    let session_id = "22f06040-64f4-4337-aa3d-92003f66d522";
    let conversation_id = format!("claude:{session_id}");
    let source_path = project.join(format!("{session_id}.jsonl"));
    let first = "11ecfe36-0fa6-418c-802a-f4e73bc1df18";
    let preserved = "7eec1f1b-a1a0-484b-9fae-9dd67de7a3db";
    let edited = "9ea2f82e-5ebd-4b2d-a2c2-454e27f0111b";
    let discarded = "1a96a108-e251-4601-ab9a-88db2cc4eebc";
    let record = |role: &str, id: &str, parent: Option<&str>, text: &str| {
        json!({"type":role,"uuid":id,"parentUuid":parent,"sessionId":session_id,
                "cwd":root,"timestamp":"2026-11-01T00:00:00Z",
                "message":{"role":role,"id":"api-message-id","stop_reason":"end_turn",
                    "content":[{"type":"text","text":text}]}})
    };
    let mut file = std::fs::File::create(&source_path).unwrap();
    for entry in [
        record("user", first, None, "first prompt"),
        record("assistant", preserved, Some(first), "first reply"),
        record("user", edited, Some(preserved), "selected prompt"),
        record("assistant", discarded, Some(edited), "reply later replaced"),
    ] {
        writeln!(file, "{entry}").unwrap();
    }
    let cli = root.join("claude");
    std::fs::write(&cli, r#"#!/usr/bin/python3
import json,pathlib,sys
root=pathlib.Path(__file__).parent
if '--version' in sys.argv:
    print('2.1.286 (Claude Code)');sys.exit(0)
for line in sys.stdin:
    req=json.loads(line)
    if req['type']=='control_request':
        print(json.dumps({'type':'control_response','response':{'subtype':'success','request_id':req['request_id'],'response':{'models':[],'mcpServers':[]}}}),flush=True)
        if req['request']['subtype']=='interrupt':
            root.joinpath('interrupted').touch()
            print(json.dumps({'type':'result','subtype':'success','is_error':False,'terminal_reason':'aborted_tools'}),flush=True)
            print(json.dumps({'type':'system','subtype':'session_state_changed','state':'idle'}),flush=True)
    elif req['type']=='user':
        if '--resume-session-at' not in sys.argv:
            continue
        position=sys.argv[sys.argv.index('--resume-session-at')+1]
        root.joinpath('sent').write_text(position+'\n'+req['message']['content'][-1]['text'])
        print(json.dumps({'type':'result','subtype':'success','is_error':False}),flush=True)
        print(json.dumps({'type':'system','subtype':'session_state_changed','state':'idle'}),flush=True)
"#).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut app = crate::Application::open(crate::Options {
        home_directory: root.join("app"),
        resources_directory: root.clone(),
    })
    .unwrap();
    let core = &mut app.runtime;
    core.runtime_instances = vec![RuntimeInstance {
        enabled: true,
        id: "claude".into(),
        name: "Synthetic Claude".into(),
        type_id: RuntimeKind::ClaudeCode,
        gateway_id: String::new(),
        settings: serde_json::from_value(json!({"binary":cli,"agentDir":native})).unwrap(),
    }];
    let history = core.read_history("claude", &conversation_id).unwrap();
    core.present_history(history, None);
    let before = core.current_snapshot().unwrap();
    let revision = core.edit_revision(&before).unwrap();
    core.replace_handoff_compaction_events(vec![
        super::super::handoff_events::HandoffCompactionEvent {
            work_id: "local-receipt".into(),
            conversation_id: conversation_id.clone(),
            physical_runtime_id: "claude".into(),
            physical_native_id: session_id.into(),
            source_anchor: Some(Some(discarded.into())),
            state: super::super::handoff_events::HandoffCompactionState::Ready,
        },
    ]);
    let decorated = core.current_snapshot().unwrap();
    assert_eq!(decorated.messages.len(), before.messages.len() + 1);
    assert_eq!(core.edit_revision(&decorated).unwrap(), revision);
    core.read_edit_sources(&decorated).unwrap();

    let edit = |core: &mut CoreRuntime, revision: &str, placement| {
        core.edit_message_with_revision(
            "claude",
            &conversation_id,
            edited,
            placement,
            None,
            Some(revision),
            None,
            crate::ComposedTurn {
                text: "replacement".into(),
                attachments: vec![],
                effort: None,
                permission_mode: None,
            },
        )
    };
    let wait_for_turn = |core: &mut CoreRuntime| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while core.turn_running() && std::time::Instant::now() < deadline {
            core.drain_sessions().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!core.turn_running());
    };

    edit(core, &revision, EditPlacement::InPlace).unwrap();
    wait_for_turn(core);
    assert_eq!(
        std::fs::read_to_string(root.join("sent")).unwrap(),
        format!("{preserved}\nreplacement")
    );
    std::fs::remove_file(root.join("sent")).unwrap();

    // While a turn is live, verification must precede interruption. This
    // fixture deliberately leaves the active prompt outside the JSONL.
    core.prepare_with_selection(before.clone(), None, false)
        .unwrap();
    core.send_action(super::super::conversations::Prompt {
        runtime_id: "claude",
        message: Some(("still processing", &[])),
        effort: None,
        permission_mode: None,
    })
    .unwrap();
    let running = core.current_snapshot().unwrap();
    let running_revision = core.edit_revision(&running).unwrap();
    let failure =
        crate::Failure::from(edit(core, &running_revision, EditPlacement::InPlace).unwrap_err());
    assert_eq!(failure.phase, "source_read");
    assert!(core.turn_running());
    assert!(!root.join("interrupted").exists());
    core.cancel("claude").unwrap();
    wait_for_turn(core);
    assert!(root.join("interrupted").exists());

    // SDK streaming uses API message IDs and can combine rows written under
    // several different UUIDs. The selected native user identity survives.
    let mut old_projection = before.clone();
    old_projection.messages.pop();
    old_projection.messages[1].id = "api-message-id".into();
    core.prepare_with_selection(old_projection, None, false)
        .unwrap();
    assert!(matches!(core.session.active_state, ActiveState::Native(_)));
    let stale = core.current_snapshot().unwrap();
    let stale_revision = core.edit_revision(&stale).unwrap();
    let failure =
        crate::Failure::from(edit(core, &stale_revision, EditPlacement::InPlace).unwrap_err());
    assert_eq!(failure.code, "edit_history_changed");
    assert_eq!(failure.phase, "source_read");
    let native_baseline = core.current_snapshot().unwrap();
    assert_eq!(core.edit_revision(&native_baseline).unwrap(), revision);
    assert!(matches!(core.session.active_state, ActiveState::History(_)));
    assert!(!root.join("sent").exists());

    let new_id = "9f8213d8-587d-46c0-a016-f662e5296a46";
    writeln!(
        file,
        "{}",
        record("user", new_id, Some(discarded), "external prompt")
    )
    .unwrap();
    let failure = crate::Failure::from(edit(core, &revision, EditPlacement::Fork).unwrap_err());
    assert_eq!(failure.code, "edit_history_changed");
    assert_eq!(failure.phase, "source_read");
    assert!(!root.join("sent").exists());

    let refreshed = core.current_snapshot().unwrap();
    let confirmed = core.edit_revision(&refreshed).unwrap();
    assert_ne!(revision, confirmed);
    let source = std::fs::read(&source_path).unwrap();
    for placement in [EditPlacement::InPlace, EditPlacement::Fork] {
        edit(core, &confirmed, placement).unwrap();
        wait_for_turn(core);
        assert_eq!(
            std::fs::read_to_string(root.join("sent")).unwrap(),
            format!("{preserved}\nreplacement")
        );
        let result = core.current_snapshot().unwrap();
        assert_eq!(
            result.conversation.id == conversation_id,
            placement == EditPlacement::InPlace
        );
        assert!(result.messages.iter().all(|message| message.id != edited
            && message.id != discarded
            && message.id != new_id));
        assert_eq!(std::fs::read(&source_path).unwrap(), source);
        if placement == EditPlacement::InPlace {
            // The synthetic child records wire input only, leaving the
            // fixture's native source available for the independent fork.
            core.open_conversation("claude", &conversation_id).unwrap();
            std::fs::remove_file(root.join("sent")).unwrap();
        }
    }
    app.close_stopping_work().unwrap();
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn deletion_links_keep_new_schema_and_refuse_unknown_or_corrupt_state() {
    let root =
        std::env::temp_dir().join(format!("velune-delete-schema-{}", crate::new_record_key()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("conversation-links.json");
    std::fs::write(&path, br#"{"schemaVersion":2,"links":[]}"#).unwrap();
    let application = crate::Application::open(crate::Options {
        home_directory: root.clone(),
        resources_directory: root.clone(),
    })
    .unwrap();
    let repository = &application.runtime.repository;
    repository.store_conversation_links(&[]).unwrap();
    let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["schemaVersion"], 3);
    for bytes in [
        br#"{"schemaVersion":99,"links":[]}"#.as_slice(),
        br#"{"schemaVersion":2,"links":[],"unknown":true}"#.as_slice(),
        b"{corrupt".as_slice(),
    ] {
        std::fs::write(&path, bytes).unwrap();
        assert!(repository.store_conversation_links(&[]).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    drop(application);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn claude_delete_rebuilds_without_send_and_preserves_repeated_identity_after_reopen() {
    for (truncate_to_empty, external_change) in [(false, false), (true, false), (false, true)] {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("velune-claude-delete-{}", crate::new_record_key()));
        std::fs::create_dir_all(&root).unwrap();
        let cli = root.join("claude");
        std::fs::write(&cli, r#"#!/usr/bin/python3
import sys,json,pathlib
root=pathlib.Path(__file__).parent
if '--version' in sys.argv:
    count=root.joinpath('version-count')
    nth=int(count.read_text())+1 if count.exists() else 1
    count.write_text(str(nth))
    marker=root.joinpath('mutate-during-import')
    if marker.exists() and nth==3:
        original=pathlib.Path(marker.read_text())
        records=[json.loads(row) for row in original.read_text().splitlines()]
        records[-1]['message']['content']=[{'type':'text','text':'external source change'}]
        original.write_text(''.join(json.dumps(row)+'\n' for row in records))
    print('2.1.286 (Claude Code)');sys.exit(0)
for line in sys.stdin:
    request=json.loads(line)
    if request.get('type')=='user':
        if not root.joinpath('allow-send').exists():
            root.joinpath('unexpected-send').write_text(line);sys.exit(1)
        import uuid,datetime
        native_id=sys.argv[sys.argv.index('--resume')+1] if '--resume' in sys.argv else sys.argv[sys.argv.index('--session-id')+1]
        path=next(root.joinpath('native','projects').glob('*/'+native_id+'.jsonl'))
        records=[json.loads(row) for row in path.read_text().splitlines()]
        parent=records[-1].get('uuid') if records else None
        now=datetime.datetime.now(datetime.timezone.utc).isoformat()
        user={'type':'user','uuid':str(uuid.uuid4()),'parentUuid':parent,'timestamp':now,'cwd':str(root),'sessionId':native_id,'message':request['message']}
        reply={'type':'assistant','uuid':str(uuid.uuid4()),'parentUuid':user['uuid'],'timestamp':now,'cwd':str(root),'sessionId':native_id,'message':{'role':'assistant','content':[{'type':'text','text':'explicit reply'}]}}
        with path.open('a') as stream: stream.write(json.dumps(user)+'\n'+json.dumps(reply)+'\n')
        root.joinpath('explicit-send').write_text(line)
        for record in [reply,{'type':'result','subtype':'success','is_error':False,'session_id':native_id},{'type':'system','subtype':'session_state_changed','state':'idle'}]: print(json.dumps(record),flush=True)
        continue
    if request.get('type')=='control_request':
        response={'models':[{'value':'synthetic','displayName':'Synthetic'}],'mcpServers':[],'errors':{}}
        print(json.dumps({'type':'control_response','response':{'subtype':'success','request_id':request['request_id'],'response':response}}),flush=True)
"#).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = root.join("native");
        std::fs::create_dir_all(&config).unwrap();
        let context = serde_json::from_value(json!({"messages":[
            {"role":"user","content":[{"type":"text","text":"duplicate"}]},
            {"role":"assistant","content":[{"type":"text","text":"first reply"}]},
            {"role":"user","content":[{"type":"text","text":"duplicate"}]},
            {"role":"assistant","content":[{"type":"text","text":"second reply"}]},
            {"role":"user","content":[{"type":"text","text":"last user"}]},
            {"role":"assistant","content":[{"type":"text","text":"last reply"}]}
        ]}))
        .unwrap();
        let imported = velune_agent_runtime::import::import_claude_conversation(
            &velune_agent_runtime::execution::ExecutionTarget::Local,
            &config,
            &root,
            &context,
            "2.1.286",
        )
        .unwrap();
        let velune_agent_runtime::import::ImportedConversation::ClaudeCode { session_id } =
            imported
        else {
            panic!("Claude import")
        };
        let mut application = crate::Application::open(crate::Options {
            home_directory: root.join("app"),
            resources_directory: root.clone(),
        })
        .unwrap();
        let core = &mut application.runtime;
        core.runtime_instances = vec![RuntimeInstance {
            enabled: true,
            id: "claude".into(),
            name: "Synthetic".into(),
            type_id: RuntimeKind::ClaudeCode,
            gateway_id: String::new(),
            settings: serde_json::from_value(json!({"binary":cli,"agentDir":config})).unwrap(),
        }];
        let id = format!("claude:{session_id}");
        core.index_history(&core.runtime_instances[0].clone())
            .unwrap();
        let source = core.read_history("claude", &id).unwrap();
        let original_users: Vec<String> = source
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .map(|m| m.id.clone())
            .collect();
        assert_eq!(original_users.len(), 3);
        core.session.active_state = ActiveState::History(Box::new(source));
        let view = core.current_snapshot().unwrap();
        let revision = core.edit_revision(&view).unwrap();
        if external_change {
            let project = std::fs::read_dir(config.join("projects"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let original = project.join(format!("{session_id}.jsonl"));
            std::fs::write(
                root.join("mutate-during-import"),
                original.to_string_lossy().as_bytes(),
            )
            .unwrap();
            let result =
                core.delete_user_message("claude", &id, &original_users[1], false, &revision);
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("会话已在编辑期间改变")
            );
            assert_eq!(
                std::fs::read_to_string(root.join("version-count")).unwrap(),
                "3"
            );
            let sources: Vec<_> = std::fs::read_dir(project)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
                .collect();
            assert_eq!(
                sources.len(),
                1,
                "failed deletion must discard only its newly owned native import"
            );
            assert_eq!(sources[0].path(), original);
            assert!(
                std::fs::read_to_string(&original)
                    .unwrap()
                    .contains("external source change")
            );
            assert!(
                core.repository
                    .load_conversation_links()
                    .unwrap()
                    .is_empty()
            );
            assert!(!root.join("unexpected-send").exists());
            application.close_stopping_work().unwrap();
            drop(application);
            std::fs::remove_dir_all(root).unwrap();
            continue;
        }
        if truncate_to_empty {
            core.delete_user_message("claude", &id, &original_users[0], true, &revision)
                .unwrap();
            let view = core.current_snapshot().unwrap();
            assert_eq!(view.conversation.id, id);
            assert_eq!(
                view.messages
                    .iter()
                    .filter(|m| matches!(m.role, MessageRole::User | MessageRole::Assistant))
                    .count(),
                0
            );
            let link = core.repository.load_conversation_links().unwrap().remove(0);
            let (fresh, projection) = core.read_link(&link).unwrap();
            core.session.active_state = ActiveState::History(Box::new(fresh));
            core.session.logical_projection = Some(projection);
            assert_eq!(core.current_snapshot().unwrap().conversation.id, id);
            assert!(!root.join("unexpected-send").exists());
            application.close_stopping_work().unwrap();
            drop(application);
            std::fs::remove_dir_all(root).unwrap();
            continue;
        }
        core.delete_user_message("claude", &id, &original_users[1], false, &revision)
            .unwrap();
        assert!(!root.join("unexpected-send").exists());
        let view = core.current_snapshot().unwrap();
        assert_eq!(view.conversation.id, id);
        assert_eq!(
            view.messages
                .iter()
                .filter(|m| m.role == MessageRole::User)
                .count(),
            2
        );
        assert_eq!(
            view.messages
                .iter()
                .filter(|m| m.role == MessageRole::Assistant)
                .count(),
            3
        );
        for reply in view
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::Assistant)
        {
            assert!(view.transcript_items.iter().any(|item| matches!(item,
                    crate::conversation::TranscriptItem::Message { message_id, .. } if message_id == &reply.id
                )), "retained reply must remain directly visible: {}", reply.id);
        }
        assert!(
            !view
                .messages
                .iter()
                .any(|message| message.id.ends_with(":boundary")),
            "same-runtime rebuild must not invent a handoff"
        );

        assert!(
            core.delete_user_message("claude", &id, &original_users[0], false, &revision)
                .is_err()
        );
        let links = core.repository.load_conversation_links().unwrap();
        assert_eq!(links, core.conversation_links);
        // Drop all transient provenance and recover solely from the persisted link.
        core.session.logical_projection = None;
        let (fresh, projection) = core.read_link(&links[0]).unwrap();
        core.session.active_state = ActiveState::History(Box::new(fresh));
        core.session.logical_projection = Some(projection);
        let before_second_delete = std::fs::read(root.join("app/conversation-links.json")).unwrap();
        let first = super::super::continuation::display_id(0, &original_users[0]);
        let view = core.current_snapshot().unwrap();
        assert!(core.deletion_source(&view, &first).is_some());
        core.delete_user_message(
            "claude",
            &id,
            &first,
            false,
            &core.edit_revision(&view).unwrap(),
        )
        .unwrap();
        let view = core.current_snapshot().unwrap();
        assert_eq!(
            view.messages
                .iter()
                .filter(|m| m.role == MessageRole::User)
                .count(),
            1
        );
        assert_eq!(
            view.messages
                .iter()
                .filter(|m| m.role == MessageRole::Assistant)
                .count(),
            3
        );
        assert!(
            std::fs::read_dir(root.join("app/conversation-edits"))
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| std::fs::read(entry.path())
                    .is_ok_and(|bytes| bytes == before_second_delete)),
            "exact pre-delete association bytes must be recoverable"
        );
        let last = super::super::continuation::display_id(0, &original_users[2]);
        core.delete_user_message(
            "claude",
            &id,
            &last,
            true,
            &core.edit_revision(&view).unwrap(),
        )
        .unwrap();
        let view = core.current_snapshot().unwrap();
        assert_eq!(view.conversation.id, id);
        assert_eq!(
            view.messages
                .iter()
                .filter(|m| m.role == MessageRole::User)
                .count(),
            0
        );
        assert_eq!(
            view.messages
                .iter()
                .filter(|m| m.role == MessageRole::Assistant)
                .count(),
            2
        );
        assert!(
            !view
                .messages
                .iter()
                .any(|message| message.id.ends_with(":boundary")),
            "truncation must not leave recovery segment boundaries"
        );
        assert!(!root.join("unexpected-send").exists());
        std::fs::write(root.join("allow-send"), "authorized synthetic turn").unwrap();
        core.send_turn(super::super::conversations::TurnRequest {
            submission_id: None,
            source_runtime_id: "claude".into(),
            conversation_id: id.clone(),
            runtime_id: "claude".into(),
            selection: Some(crate::ModelSelection::Native {
                provider_id: None,
                model_id: "synthetic".into(),
            }),
            turn: crate::ComposedTurn {
                text: "explicit new user".into(),
                attachments: vec![],
                effort: None,
                permission_mode: None,
            },
        })
        .unwrap();
        for _ in 0..100 {
            core.sync_projection().unwrap();
            if !core.turn_running() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!core.turn_running());
        assert!(root.join("explicit-send").exists());
        let view = core.current_snapshot().unwrap();
        assert_eq!(view.conversation.id, id);
        assert_eq!(
            view.messages
                .iter()
                .filter(|m| m.role == MessageRole::User)
                .count(),
            1
        );
        assert_eq!(
            view.messages
                .iter()
                .filter(|m| m.role == MessageRole::Assistant)
                .count(),
            3
        );
        core.invalidate_execution().unwrap();
        let link = core.repository.load_conversation_links().unwrap().remove(0);
        let (fresh, projection) = core.read_link(&link).unwrap();
        core.session.active_state = ActiveState::History(Box::new(fresh));
        core.session.logical_projection = Some(projection);
        let view = core.current_snapshot().unwrap();
        let newest = view
            .messages
            .iter()
            .find(|m| m.role == MessageRole::User)
            .unwrap()
            .id
            .clone();
        core.delete_user_message(
            "claude",
            &id,
            &newest,
            false,
            &core.edit_revision(&view).unwrap(),
        )
        .unwrap();
        let view = core.current_snapshot().unwrap();
        assert_eq!(
            view.messages
                .iter()
                .filter(|m| m.role == MessageRole::User)
                .count(),
            0
        );
        assert_eq!(
            view.messages
                .iter()
                .filter(|m| m.role == MessageRole::Assistant)
                .count(),
            3
        );
        core.shutdown_active().unwrap();
        application.close_stopping_work().unwrap();
        drop(application);
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// Opt-in real-SDK acceptance. Only an explicitly supplied synthetic
/// fixture is cloned; no original session or ambient HOME is opened.
#[test]
fn real_pi_sdk_cold_core_deletion_preserves_replies_without_model_turns() {
    let (Some(fixture), Some(cli), Some(node)) = (
        std::env::var_os("VELUNE_TEST_PI_SYNTHETIC_FIXTURE"),
        std::env::var_os("VELUNE_TEST_PI_BINARY"),
        std::env::var_os("VELUNE_TEST_NODE_BINARY"),
    ) else {
        return;
    };
    fn clone_private(source: &Path, target: &Path) {
        std::fs::create_dir_all(target).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(
                !kind.is_symlink(),
                "synthetic fixture must not escape its root"
            );
            let target = target.join(entry.file_name());
            if kind.is_dir() {
                clone_private(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let fixture = PathBuf::from(fixture);
    let cli = PathBuf::from(cli);
    let node = PathBuf::from(node);
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../agent-runtime/resources")
        .canonicalize()
        .unwrap();
    for truncate in [false, true] {
        let root = std::env::temp_dir().join(format!(
            "velune-core-pi-sdk-delete-{}",
            crate::new_record_key()
        ));
        let agent = root.join("agent");
        clone_private(&fixture.join("pi"), &agent);
        let models: Value =
            serde_json::from_slice(&std::fs::read(agent.join("models.json")).unwrap()).unwrap();
        assert!(
            models["providers"]
                .as_object()
                .unwrap()
                .values()
                .all(|provider| provider["baseUrl"] == "http://127.0.0.1:1/v1"
                    && provider["apiKey"] == "synthetic"),
            "real SDK acceptance must have only the closed synthetic endpoint"
        );
        let workspace = root.join("workspace");
        let os_home = root.join("os-home");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&os_home).unwrap();
        // The visible fixture may already have been edited by GUI
        // acceptance. Start from an immutable, complete synthetic backup,
        // never from an already truncated active branch.
        let candidates = [
            fixture.join("state/conversation-edits"),
            fixture.join("pi/sessions"),
        ];
        let mut records: Vec<Value> = candidates
            .iter()
            .flat_map(|directory| {
                std::fs::read_dir(directory)
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
            })
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .filter_map(|text| {
                text.lines()
                    .map(serde_json::from_str)
                    .collect::<Result<Vec<Value>, _>>()
                    .ok()
            })
            .find(|records| {
                records
                    .first()
                    .is_some_and(|record| record["type"] == "session")
                    && records
                        .iter()
                        .filter(|record| {
                            record["type"] == "message" && record["message"]["role"] == "user"
                        })
                        .count()
                        == 3
                    && records
                        .iter()
                        .filter(|record| {
                            record["type"] == "message" && record["message"]["role"] == "assistant"
                        })
                        .count()
                        == 3
                    && !records.iter().any(|record| {
                        matches!(record["type"].as_str(), Some("context_edit" | "compaction"))
                    })
            })
            .expect("authorized fixture needs a complete, unedited six-message source or backup");
        std::fs::remove_dir_all(agent.join("sessions")).unwrap();
        std::fs::create_dir_all(agent.join("sessions")).unwrap();
        let source = agent.join("sessions/core-deletion-baseline.jsonl");
        records[0]["cwd"] = json!(workspace);
        let before = records
            .iter()
            .filter(|record| record["type"] == "message")
            .count();
        std::fs::write(
            &source,
            records
                .iter()
                .map(|record| format!("{record}\n"))
                .collect::<String>(),
        )
        .unwrap();
        if !truncate {
            // Seed a real official-SDK replacement at the old tip. A later
            // rewind must preserve it together with the deletion tombstone.
            let target = records
                .iter()
                .find(|record| record["type"] == "message" && record["message"]["role"] == "user")
                .unwrap()["id"]
                .as_str()
                .unwrap();
            let mut command = velune_agent_runtime::process::command(&node);
            command.args(["--input-type=module","--eval",r#"
import {pathToFileURL} from 'node:url';
const [sdk,binary,source,target] = process.argv.slice(1);
const {resolvePiSdk} = await import(pathToFileURL(sdk).href);
const {SessionManager} = await resolvePiSdk({binary,runtimeTypeId:'pi-1.0'}).importModule('dist/core/session-manager.js');
SessionManager.open(source).appendContextEdit(target,{content:'synthetic earlier replacement'});
"#]).arg(resources.join("pi_sdk.mjs")).arg(&cli).arg(&source).arg(target);
            velune_agent_runtime::execution::ExecutionTarget::Local
                .output(command, None, Some(&os_home))
                .unwrap();
        }
        let mut application = crate::Application::open(crate::Options {
            home_directory: root.join("state"),
            resources_directory: resources.clone(),
        })
        .unwrap();
        let core = &mut application.runtime;
        let mut runtime = RuntimeInstance { enabled:true,id:"pi".into(),name:"Synthetic Pi SDK".into(),type_id:RuntimeKind::Pi,gateway_id:String::new(),
                settings:serde_json::from_value(json!({"binary":cli,"nodeBinary":node,"agentDir":agent,"sessionDir":agent.join("sessions")})).unwrap() };
        runtime
            .set_execution(crate::RuntimeExecution {
                transport: crate::RuntimeTransport::Local,
                home: crate::RuntimeHome::Existing {
                    path: os_home.to_string_lossy().into_owned(),
                },
                remote_resources_directory: None,
            })
            .unwrap();
        core.runtime_instances = vec![runtime.clone()];
        core.index_history(&runtime).unwrap();
        let id = format!("pi:{}", source.display());
        let snapshot = core.read_history("pi", &id).unwrap();
        core.session.active_state = ActiveState::History(Box::new(snapshot));
        let current = core.current_snapshot().unwrap();
        let users: Vec<_> = current
            .messages
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .map(|message| message.id.clone())
            .collect();
        assert!(
            users.len() >= 3,
            "synthetic session needs several native user turns"
        );
        let selected = &users[1];
        let boundary = current
            .messages
            .iter()
            .position(|message| &message.id == selected)
            .unwrap();
        let expected: Vec<_> = current
            .messages
            .iter()
            .enumerate()
            .filter(|(index, message)| {
                if truncate {
                    *index < boundary
                } else {
                    &message.id != selected
                }
            })
            .map(|(_, message)| message.id.clone())
            .collect();
        let revision = core.edit_revision(&current).unwrap();
        let delivered = core
            .delete_user_message("pi", &id, selected, truncate, &revision)
            .unwrap()
            .snapshot
            .expect("command must return its authoritative snapshot");
        assert_eq!(
            delivered.message_ids, expected,
            "SnapshotResult must deliver the same native order"
        );
        assert_eq!(
            delivered
                .transcript_items
                .iter()
                .filter_map(|item| match item {
                    crate::conversation::TranscriptItem::Message { message_id, .. } =>
                        Some(message_id.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            expected,
            "delivered transcript rows must retain native order"
        );
        let visible = core.current_snapshot().unwrap();
        assert_eq!(visible.conversation.id, id);
        assert_eq!(
            visible
                .messages
                .iter()
                .map(|message| message.id.clone())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            visible
                .transcript_items
                .iter()
                .filter_map(|item| match item {
                    crate::conversation::TranscriptItem::Message { message_id, .. } =>
                        Some(message_id.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            expected,
            "actual Core transcript rows must retain native order"
        );
        for reply in visible
            .messages
            .iter()
            .filter(|message| message.role == MessageRole::Assistant)
        {
            assert!(visible.transcript_items.iter().any(|item| matches!(item,
                    crate::conversation::TranscriptItem::Message {message_id,..} if message_id==&reply.id
                )),"retained original closing reply must remain directly visible");
        }
        // Exercise the cold official SDK reader after the production Core
        // command closed its actual Pi process and mutated native history.
        let cold = core.read_history("pi", &id).unwrap();
        assert_eq!(
            cold.messages
                .iter()
                .map(|message| message.id.clone())
                .collect::<Vec<_>>(),
            expected
        );
        if !truncate {
            let preserved_replacement = |snapshot: &ConversationSnapshot| {
                assert!(snapshot.messages.iter().find(|message| message.id==users[0]).unwrap().blocks.iter()
                        .any(|block| matches!(block,MessageBlock::Text{text} if text=="synthetic earlier replacement")),
                        "branch-local earlier replacement must survive later deletion and truncation");
            };
            preserved_replacement(&cold);
            core.session.active_state = ActiveState::History(Box::new(cold));
            let reopened = core.current_snapshot().unwrap();
            let third = &users[2];
            let third_at = reopened
                .messages
                .iter()
                .position(|message| &message.id == third)
                .unwrap();
            let chained_expected: Vec<_> = reopened.messages[..third_at]
                .iter()
                .map(|message| message.id.clone())
                .collect();
            let revision = core.edit_revision(&reopened).unwrap();
            let delivered = core
                .delete_user_message("pi", &id, third, true, &revision)
                .unwrap()
                .snapshot
                .expect("chained command snapshot");
            assert_eq!(
                delivered.message_ids, chained_expected,
                "chained SnapshotResult must never resurrect User2"
            );
            assert_eq!(
                delivered
                    .transcript_items
                    .iter()
                    .filter_map(|item| match item {
                        crate::conversation::TranscriptItem::Message { message_id, .. } =>
                            Some(message_id.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                chained_expected
            );
            let chained = core.current_snapshot().unwrap();
            assert_eq!(chained.conversation.id, id);
            assert_eq!(
                chained
                    .messages
                    .iter()
                    .map(|message| message.id.clone())
                    .collect::<Vec<_>>(),
                chained_expected
            );
            assert!(
                !chained
                    .messages
                    .iter()
                    .any(|message| message.id == users[1] || message.id == users[2]),
                "deleted User2 must never resurrect when truncating User3"
            );
            assert_eq!(
                chained
                    .transcript_items
                    .iter()
                    .filter_map(|item| match item {
                        crate::conversation::TranscriptItem::Message { message_id, .. } =>
                            Some(message_id.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                chained_expected,
                "chained truncation rows must retain native order"
            );
            preserved_replacement(&chained);
            let chained_cold = core.read_history("pi", &id).unwrap();
            assert_eq!(
                chained_cold
                    .messages
                    .iter()
                    .map(|message| message.id.clone())
                    .collect::<Vec<_>>(),
                chained_expected
            );
            preserved_replacement(&chained_cold);
            for reply in chained
                .messages
                .iter()
                .filter(|message| message.role == MessageRole::Assistant)
            {
                assert!(chained.transcript_items.iter().any(|item| matches!(item,
                        crate::conversation::TranscriptItem::Message {message_id,..} if message_id==&reply.id)));
            }
        }
        let after = std::fs::read_to_string(&source)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|record| record["type"] == "message")
            .count();
        assert_eq!(
            after, before,
            "deletion must not send a model prompt or create a native reply"
        );
        assert!(
            std::fs::read_dir(root.join("state/conversation-edits"))
                .unwrap()
                .next()
                .is_some()
        );
        application.close_stopping_work().unwrap();
        drop(application);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn pi_edit_command_prepares_diagnostics_and_rechecks_storage_before_branch() {
    pi_message_mutation_preconditions(false);
}

#[test]
fn pi_delete_command_prepares_diagnostics_and_rechecks_storage_before_branch() {
    pi_message_mutation_preconditions(true);
}

fn pi_message_mutation_preconditions(test_deletion: bool) {
    use std::os::unix::fs::PermissionsExt;
    for (mode, linked, deletion) in [
        ("wrong-token", false, None),
        ("missing-frame", false, None),
        ("wrong-identity", false, None),
        ("unchanged", false, None),
        ("user", false, None),
        ("stable", false, None),
        ("unchanged", true, None),
        ("user", true, None),
        ("stable", true, None),
        ("sync-failure", false, None),
        ("sync-failure", true, None),
        ("unchanged", false, Some(false)),
        ("unchanged", false, Some(true)),
        ("unchanged", true, Some(false)),
        ("unchanged", true, Some(true)),
        ("user", false, Some(false)),
        ("stable", true, Some(true)),
    ] {
        if deletion.is_some() != test_deletion {
            continue;
        }
        let root = std::env::temp_dir().join(format!(
            "velune-pi-edit-command-{}",
            crate::new_record_key()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut application = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        let cli = root.join("pi");
        std::fs::write(&cli, "#!/bin/sh\nprintf '1.4.0\\n'\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let node = root.join("node");
        std::fs::write(&node, r#"#!/usr/bin/python3
import json,os,pathlib,sys
if '--input-type=module' in sys.argv: print(json.dumps({'binary':str(pathlib.Path(__file__).with_name('pi'))}))
elif '--version' in sys.argv: print('1.4.0')
else: os.execv('/usr/bin/python3',['/usr/bin/python3']+sys.argv[1:])
"#).unwrap();
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o700)).unwrap();
        let session_path = root.join("session.jsonl");
        std::fs::write(&session_path, "synthetic membership marker").unwrap();
        let sealed_path = root.join("sealed.jsonl");
        std::fs::write(&sealed_path, "synthetic sealed membership marker").unwrap();
        std::fs::write(root.join("sealed-history.json"), json!({"cwd":root,"messages":[
                {"id":"pi:sealed:user:0","role":"user","content":"sealed prompt"},
                {"id":"pi:sealed:reply:0","role":"assistant","content":"sealed reply","stopReason":"stop"}
            ]}).to_string()).unwrap();
        let history = json!({"cwd":root,"messages":[
            {"id":"pi:synthetic:prefix-user:0","role":"user","content":"retained prompt"},
            {"id":"pi:synthetic:prefix-reply:0","role":"assistant","content":"retained reply","stopReason":"stop"},
            {"id":"pi:synthetic:user:0","role":"user","content":"original"},
            {"id":"pi:synthetic:reply:0","role":"assistant","provider":"synthetic","model":"alpha","content":"stable reply","stopReason":"stop"},
            {"id":"pi:synthetic:future-user:0","role":"user","content":"discarded future prompt"},
            {"id":"pi:synthetic:future-reply:0","role":"assistant","content":"discarded future reply","stopReason":"stop"}
        ]});
        std::fs::write(root.join("history.json"), history.to_string()).unwrap();
        std::fs::write(root.join("mode"), mode).unwrap();
        std::fs::write(root.join("pi_sessions.mjs"), r#"
import json,pathlib,sys
root=pathlib.Path(__file__).parent
if '--inspect-session' in sys.argv:
    source=sys.argv[sys.argv.index('--inspect-session')+1]
    history=json.loads(root.joinpath('sealed-history.json' if source==str(root/'sealed.jsonl') else 'history.json').read_text())
    if '--with-summary' in sys.argv:
        history['summary']={'path':source,'cwd':str(root),'name':'Synthetic'}
    print(json.dumps(history))
elif '--delete-message' in sys.argv:
    request=json.load(sys.stdin)
    history=json.loads(root.joinpath('history.json').read_text())
    boundary=next(i for i,m in enumerate(history['messages']) if m['id']==request['messageId'])
    history['messages']=history['messages'][:boundary] if request['truncate'] else [m for i,m in enumerate(history['messages']) if i!=boundary]
    history['retainedReplyIds']=[m['id'] for m in history['messages'] if m['role']=='assistant']
    root.joinpath('history.json').write_text(json.dumps(history))
    root.joinpath('control.log').write_text('delete\n')
    print(json.dumps({'ok':True}))
else:
    json.load(sys.stdin)
    files=[{'path':str(root/name),'sessionId':identity,'size':'1','mtimeNs':'1','cwd':str(root)} for name,identity in [('session.jsonl','synthetic'),('sealed.jsonl','sealed')]]
    print(json.dumps({'contractVersion':3,'files':files,'sessions':[{'path':f['path'],'cwd':str(root),'name':'Synthetic'} for f in files]}))
"#).unwrap();
        std::fs::write(root.join("pi_rpc.mjs"), r#"
import json,pathlib,sys
root=pathlib.Path(__file__).parent
if '--validate' in sys.argv: sys.exit(0)
history=json.loads((root/'history.json').read_text())
mode=(root/'mode').read_text()
if mode in ('user','stable'):
    history['messages'][2 if mode=='user' else 3]['content']='external change'
    (root/'history.json').write_text(json.dumps(history))
def emit(value): print(json.dumps(value),flush=True)
def status(key,value): emit({'type':'extension_ui_request','method':'setStatus','statusKey':key,'statusText':json.dumps(value)})
emit({'type':'extension_error','extensionPath':'synthetic.ts','event':'session_start','error':'synthetic startup diagnostic'})
for line in sys.stdin:
    request=json.loads(line); kind=request['type']; data={}
    if kind=='prompt':
        text=request.get('message','')
        if text.startswith('/velune-projection-sync '):
            token=text.split(' ',1)[1]
            assert token==request['id']
            if mode=='sync-failure' and (root/'control.log').exists():
                (root/'snapshot-sync-failure').write_text(token)
                emit({'type':'response','id':request['id'],'command':kind,'success':False,'error':'synthetic post-rewind snapshot failure'})
                continue
            messages=history['messages']
            frame={'requestId':token,'snapshotVersion':1,'messages':messages,
                   'messageIds':[m['id'] for m in messages],'identityConfirmations':[],
                   'sessionFile':str(root/'session.jsonl'),'name':'Synthetic','cwd':history['cwd']}
            if mode=='wrong-token': frame['requestId']='stale-'+token
            if mode=='wrong-identity': frame['messageIds'][0]='foreign-user'
            if mode!='missing-frame': status('velune.session-metadata',frame)
            data={'disposition':'handled'}
        elif text.startswith('/velune-background-work '):
            status('velune.background-work-v1',{'version':1,'kind':'unsupported','supported':False,'tasks':[]})
            data={'disposition':'handled'}
        elif text.startswith('/velune-rewind '):
            with (root/'control.log').open('a') as f: f.write('branch\n')
            boundary=next(i for i,m in enumerate(history['messages']) if m['id'].split(':')[-2]==text.split(' ',1)[1])
            history['messages']=history['messages'][:boundary]
            (root/'history.json').write_text(json.dumps(history))
            status('velune.rewind',{'entryId':text.split(' ',1)[1]})
            data={'disposition':'handled'}
        else:
            with (root/'control.log').open('a') as f: f.write('send\n')
            message={'id':'pi:synthetic:replacement:0','role':'user','content':text}
            history['messages'].append(message)
            (root/'history.json').write_text(json.dumps(history))
            emit({'type':'agent_start'})
            emit({'type':'message_start','message':message})
            emit({'type':'message_end','message':message})
            data={'requestId':'synthetic-request'}
    elif kind=='get_state': data={'sessionFile':str(root/'session.jsonl'),'sessionName':'Synthetic'}
    elif kind=='get_messages': data={'messages':history['messages']}
    emit({'type':'response','id':request['id'],'command':kind,'success':True,'data':data})
"#).unwrap();
        let core = &mut application.runtime;
        core.runtime_instances = vec![RuntimeInstance {
            enabled: true,
            id: "pi".into(),
            name: "Synthetic Pi".into(),
            type_id: RuntimeKind::Pi,
            gateway_id: String::new(),
            settings: serde_json::from_value(
                json!({"binary":cli,"nodeBinary":node,"agentDir":root,"sessionDir":root}),
            )
            .unwrap(),
        }];
        let id = format!("pi:{}", session_path.display());
        core.index_history(&core.runtime_instances[0].clone())
            .unwrap();
        let original = core.read_history("pi", &id).unwrap();
        core.session.active_state = ActiveState::History(Box::new(original));
        let addressed = if linked {
            let sealed = core
                .read_history("pi", &format!("pi:{}", sealed_path.display()))
                .unwrap();
            let link = crate::config::ConversationLink {
                id: "logical:synthetic".into(),
                segments: vec![
                    crate::config::ConversationSegmentReference {
                        excluded_message_ids: Vec::new(),
                        message_origins: BTreeMap::new(),
                        runtime_instance_id: "pi".into(),
                        runtime_type_id: RuntimeKind::Pi,
                        native_conversation_id: format!("pi:{}", sealed_path.display()),
                        cwd: Some(root.to_string_lossy().into_owned()),
                        cutoff: Some(crate::config::ConversationCutoff {
                            message_count: sealed.messages.len() as u64,
                            prefix_digest: super::super::continuation::prefix_digest(
                                &sealed.messages,
                            )
                            .unwrap(),
                        }),
                        handoff: None,
                        imported: None,
                        deleted: false,
                    },
                    crate::config::ConversationSegmentReference {
                        excluded_message_ids: Vec::new(),
                        message_origins: BTreeMap::new(),
                        runtime_instance_id: "pi".into(),
                        runtime_type_id: RuntimeKind::Pi,
                        native_conversation_id: id.clone(),
                        cwd: Some(root.to_string_lossy().into_owned()),
                        cutoff: None,
                        handoff: None,
                        imported: None,
                        deleted: false,
                    },
                ],
            };
            core.conversation_links = vec![link.clone()];
            core.repository
                .store_conversation_links(&core.conversation_links)
                .unwrap();
            let (tail, projection) = core.read_link(&link).unwrap();
            core.session.active_state = ActiveState::History(Box::new(tail));
            core.session.logical_projection = Some(projection);
            if mode == "unchanged" && deletion.is_none() {
                let choice = crate::config::RuntimeTurnChoice {
                    runtime_id: "pi".into(),
                    choice: crate::config::TurnChoice {
                        model: Some(crate::ModelSelection::Native {
                            provider_id: Some("choice-provider".into()),
                            model_id: "choice-model".into(),
                        }),
                        effort: Some("high".into()),
                        permission_mode: None,
                    },
                };
                core.composer_choices.conversations = vec![crate::config::NextTurnIntent {
                    conversation_id: link.id.clone(),
                    runtime_id: Some("pi".into()),
                    choices: vec![choice],
                }];
                let choices_before = core.composer_choices.clone();
                let history_path = root.join("history.json");
                let original_history = std::fs::read_to_string(&history_path).unwrap();
                let crate::local::conversation_history::ConversationRead::Load(request) =
                    core.prepare_conversation_refresh("pi", &link.id).unwrap()
                else {
                    panic!("linked explicit refresh must reload source history")
                };
                let mut changed: Value = serde_json::from_str(&original_history).unwrap();
                changed["messages"][3]["content"] = json!("externally refreshed linked reply");
                std::fs::write(&history_path, changed.to_string()).unwrap();
                let loaded = std::thread::spawn(move || request.load()).join().unwrap();
                core.finish_conversation_refresh(loaded).unwrap();
                let refreshed = core.current_snapshot().unwrap();
                assert_eq!(refreshed.conversation.id, link.id);
                assert!(refreshed.messages.iter().any(|message| {
                    message.blocks.iter().any(|block| {
                        matches!(
                            block,
                            crate::conversation::MessageBlock::Text { text }
                                if text == "externally refreshed linked reply"
                        )
                    })
                }));
                assert_eq!(
                    core.session
                        .logical_projection
                        .as_ref()
                        .map(|projection| projection.link.id.as_str()),
                    Some(link.id.as_str())
                );
                assert_eq!(core.composer_choices, choices_before);
                std::fs::write(&history_path, original_history).unwrap();
                let (restored, projection) = core.read_link(&link).unwrap();
                core.session.active_state = ActiveState::History(Box::new(restored));
                core.session.logical_projection = Some(projection);
            }
            link.id
        } else {
            id.clone()
        };
        let user_id = if linked {
            super::super::continuation::display_id(1, "pi:synthetic:user:0")
        } else {
            "pi:synthetic:user:0".into()
        };
        let view = core.current_snapshot().unwrap();
        let recorded = view
            .transcript_turns
            .iter()
            .find(|turn| turn.user_message_id == user_id)
            .and_then(|turn| turn.settings.clone())
            .unwrap();
        let native_model = crate::ModelSelection::Native {
            provider_id: Some("synthetic".into()),
            model_id: "alpha".into(),
        };
        assert_eq!(recorded.model_selection.as_ref(), Some(&native_model));
        let next =
            velune_application_api::resolve_next_turn(velune_application_api::NextTurnInput {
                runtime_id: "pi".into(),
                runtime_type: None,
                providers: Vec::new(),
                catalog: Some(vec![
                    velune_application_api::RuntimeModel {
                        selection: native_model.clone(),
                        name: "Synthetic Alpha".into(),
                        reasoning_efforts: Vec::new(),
                    },
                    velune_application_api::RuntimeModel {
                        selection: crate::ModelSelection::Native {
                            provider_id: Some("other".into()),
                            model_id: "alpha".into(),
                        },
                        name: "Other Alpha".into(),
                        reasoning_efforts: Vec::new(),
                    },
                ]),
                turn_settings: vec![recorded.clone()],
                conversation_runtime_id: Some("pi".into()),
                model_record_key: None,
                kept: Some(velune_application_api::TurnChoice {
                    model: recorded.model_selection,
                    ..Default::default()
                }),
                runtime_choices: None,
                new_conversation: Default::default(),
            });
        assert!(
            !next.chosen_model_unavailable,
            "a known provider-qualified edit model must stay available"
        );
        assert_eq!(next.model, Some(native_model));
        let revision = core.edit_revision(&view).unwrap();
        let result = if let Some(truncate) = deletion {
            // Exercise the service's delayed business receipt route. The
            // test, not the application owner, waits for final history.
            core.begin_execution_command(
                super::super::execution_workers::ExecutionCommandKind::Delete,
            )
            .unwrap();
            let admitted = core
                .serve_live_control(Focus::Conversation(addressed.clone()), |core| {
                    core.delete_user_message("pi", &addressed, &user_id, truncate, &revision)
                });
            let ticket = core.finish_execution_command();
            if let Some(ticket) = ticket {
                admitted.unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
                loop {
                    core.drain_runtime().unwrap();
                    if let Some(result) = core.drain_execution_results().remove(&ticket) {
                        break result.and_then(|_| core.snapshot_result());
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "Pi delete business receipt did not settle"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            } else {
                admitted
            }
        } else {
            core.edit_message_with_revision(
                "pi",
                &addressed,
                &user_id,
                EditPlacement::InPlace,
                None,
                Some(&revision),
                None,
                crate::ComposedTurn {
                    text: "replacement".into(),
                    attachments: vec![],
                    effort: None,
                    permission_mode: None,
                },
            )
        };
        if matches!(mode, "wrong-token" | "missing-frame" | "wrong-identity") {
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains(match mode {
                    "wrong-token" | "missing-frame" => "缺少同源快照",
                    "wrong-identity" => "消息身份不属于同一快照",
                    _ => unreachable!(),
                }),
                "{mode}: {error}"
            );
            assert!(
                !root.join("control.log").exists(),
                "invalid snapshot authorized branch or send"
            );
        } else if mode == "unchanged" {
            assert!(
                result.is_ok(),
                "unchanged history edit failed (linked={linked}, deletion={deletion:?}): {:?}",
                result.err()
            );
            if let Some(truncate) = deletion {
                assert_eq!(
                    std::fs::read_to_string(root.join("control.log")).unwrap(),
                    "delete\n"
                );
                let fresh = core.current_snapshot().unwrap();
                assert_eq!(fresh.conversation.id, addressed);
                assert!(!fresh.messages.iter().any(|m| m.id == user_id));
                assert_eq!(
                    fresh
                        .messages
                        .iter()
                        .filter(|m| m.role == MessageRole::Assistant)
                        .count(),
                    usize::from(linked) + if truncate { 1 } else { 3 }
                );
                assert!(root.join("conversation-edits").exists());
            } else {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
                loop {
                    let pending = core.current_snapshot().unwrap();
                    if pending.run_state == RunState::Running {
                        assert!(
                            pending.messages.iter().all(|message| {
                                message.id != user_id
                                    && !message.id.contains("future-user")
                                    && !message.id.contains("future-reply")
                            }),
                            "every intermediate Running snapshot must exclude abandoned native messages"
                        );
                    }
                    assert!(pending.transcript_turns.iter().all(|turn| {
                        turn.user_message_id != user_id
                            && !turn.user_message_id.contains("future-user")
                    }));
                    if pending.messages.iter().any(|message| {
                        message.role == MessageRole::User && message_text(message) == "replacement"
                    }) {
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "the runtime did not accept the replacement prompt"
                    );
                    core.drain_runtime().unwrap();
                    std::thread::yield_now();
                }
                assert_eq!(
                    std::fs::read_to_string(root.join("control.log")).unwrap(),
                    "branch\nsend\n"
                );
                let fresh = core.current_snapshot().unwrap();
                assert_eq!(fresh.conversation.id, addressed);
                assert_eq!(fresh.run_state, RunState::Running);
                let native = fresh
                    .messages
                    .iter()
                    .filter(|message| {
                        matches!(message.role, MessageRole::User | MessageRole::Assistant)
                    })
                    .map(message_text)
                    .collect::<Vec<_>>();
                let mut expected = if linked {
                    vec!["sealed prompt", "sealed reply"]
                } else {
                    Vec::new()
                };
                expected.extend(["retained prompt", "retained reply", "replacement"]);
                assert_eq!(
                    native, expected,
                    "the accepted running projection must replace the selected message and its future"
                );
                assert!(fresh.transcript_turns.iter().all(|turn| {
                    turn.user_message_id != user_id && !turn.user_message_id.contains("future-user")
                }));
                assert!(
                    core.native_snapshot()
                        .unwrap()
                        .messages
                        .iter()
                        .any(|message| message.id.starts_with("pi-diagnostic-"))
                );
            }
        } else if mode == "sync-failure" {
            assert!(
                root.join("snapshot-sync-failure").exists(),
                "failure must hit the atomic projection snapshot, not legacy get_messages"
            );
            assert!(result.unwrap_err().to_string().contains("尚未发送替换内容"));
            assert_eq!(
                std::fs::read_to_string(root.join("control.log")).unwrap(),
                "branch\n"
            );
            assert!(matches!(core.session.active_state, ActiveState::Empty));
            assert!(
                core.current_snapshot().is_none(),
                "failed synchronization must discard the unverified live projection"
            );
        } else {
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains("会话已在编辑期间改变"),
                "{mode} (linked={linked}, deletion={deletion:?}): {error}"
            );
            assert!(
                !root.join("control.log").exists(),
                "changed history was branched or sent"
            );
        }
        // A known edit result can precede actual terminal cleanup. Only
        // the synthetic fixture waits here; production owner progress stays
        // nonblocking, and successful Close must still release real IO.
        let closed = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            application.runtime.drain_sessions().unwrap();
            match application.close_stopping_work() {
                Ok(()) => break,
                Err(crate::Error::Busy) if std::time::Instant::now() < closed => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => panic!(
                    "{mode} cleanup (linked={linked}, deletion={deletion:?}) failed: {error}"
                ),
            }
        }
        drop(application);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn pi_preparation_diagnostics_are_not_native_history_changes() {
    let mut projection = PiProjection::default();
    let mut view: ConversationSnapshot = serde_json::from_value(json!({
            "revision":1,"conversation":{"id":"pi:synthetic","runtimeId":"pi","title":{"source":"native","text":"Synthetic"},"canRename":false,"canDelete":false},
            "contextRuntimeId":"pi","runState":"idle","messages":[],
            "transcriptItems":[],"transcriptOutline":[],"transcriptMessageIdentities":[],
            "actions":{"inputAccess":"editable","canSend":true,"canCancel":false,"canSwitch":true}
        })).unwrap();
    projection.snapshot = Some(view.clone());
    let history = json!({"messages":[
        {"id":"pi:session:user:0","role":"user","content":"original"},
        {"id":"pi:session:reply:0","role":"assistant","content":"reply","stopReason":"stop"}
    ]});
    projection.replace_history(&history);
    view = projection.snapshot.clone().unwrap();
    projection.apply_event(&json!({"type":"extension_error","extensionPath":"synthetic.ts","event":"session_start","error":"synthetic startup failure"}));
    projection.replace_history(&history);
    let prepared = projection.snapshot.unwrap();
    assert_eq!(prepared.messages.len(), view.messages.len() + 1);
    assert!(same_edit_history(&view, &prepared.messages));
    assert!(same_edit_history(&prepared, &view.messages));
    for index in 0..view.messages.len() {
        let mut changed = prepared.messages.clone();
        let id = &view.messages[index].id;
        changed
            .iter_mut()
            .find(|message| &message.id == id)
            .unwrap()
            .blocks
            .push(MessageBlock::Text {
                text: "external change".into(),
            });
        assert!(!same_edit_history(&view, &changed));
    }
    let mut changed = view.messages.clone();
    changed[0].branch_point = Some("another-branch".into());
    assert!(!same_edit_history(&view, &changed));
    changed = view.messages.clone();
    changed[0].id = "pi-diagnostic-user-controlled".into();
    assert!(!same_edit_history(&view, &changed));
    changed = view.messages.clone();
    changed.push(view.messages[0].clone());
    assert!(!same_edit_history(&view, &changed));
}

#[test]
fn edit_revision_cache_reuses_idle_and_streaming_history_but_commands_recheck() {
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce).unwrap();
    let root =
        std::env::temp_dir().join(format!("velune-edit-cache-{:x}", u64::from_le_bytes(nonce)));
    std::fs::create_dir(&root).unwrap();
    let mut application = crate::Application::open(crate::Options {
        home_directory: root.clone(),
        resources_directory: root.clone(),
    })
    .unwrap();
    let core = &mut application.runtime;
    let mut view: ConversationSnapshot = serde_json::from_value(json!({
            "revision":1,
            "conversation":{"id":"pi:cache","runtimeId":"pi","title":{"source":"native","text":"Cache"},
                "canRename":false,"canDelete":false},
            "contextRuntimeId":"pi","runState":"idle",
            "messages":[
                {"id":"u1","role":"user","completed":true,"blocks":[{"kind":"text","text":"first"}]},
                {"id":"a1","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"history"}]},
                {"id":"u2","role":"user","completed":true,"blocks":[{"kind":"text","text":"next"}]},
                {"id":"a2","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"reply"}]}
            ],
            "transcriptItems":[],"transcriptOutline":[],"transcriptMessageIdentities":[],
            "actions":{"inputAccess":"editable","canSend":true,"canCancel":false,"canSwitch":true}
        })).unwrap();
    let history_bytes = std::env::var("VELUNE_EDIT_CACHE_BENCH_BYTES")
        .ok()
        .and_then(|bytes| bytes.parse().ok())
        .unwrap_or(256 * 1024);
    view.messages[1].blocks = vec![MessageBlock::Text {
        text: "x".repeat(history_bytes),
    }];
    core.session.active_state = ActiveState::History(Box::new(view.clone().into()));
    let started = std::time::Instant::now();
    let token = core.edit_revision(&view).unwrap();
    let initial = started.elapsed();
    let mut with_diagnostic = view.clone();
    with_diagnostic.messages.push(Message {
        id: "pi-diagnostic-local".into(),
        role: MessageRole::System,
        timestamp_unix_ms: None,
        completed: true,
        turn_settings: None,
        turn_usage: None,
        branch_point: None,
        blocks: vec![MessageBlock::Notice {
            text: "synthetic extension failure".into(),
        }],
    });
    assert_eq!(
        core.edit_revision(&with_diagnostic).as_deref(),
        Some(token.as_str())
    );
    core.check_edit_revision(&with_diagnostic, Some(&token))
        .unwrap();
    let buffer = core
        .session
        .edit_revision_cache
        .borrow()
        .as_ref()
        .unwrap()
        .token
        .as_ptr();
    let started = std::time::Instant::now();
    for _ in 0..40 {
        assert_eq!(core.edit_revision(&view).as_deref(), Some(token.as_str()));
        assert_eq!(
            core.session
                .edit_revision_cache
                .borrow()
                .as_ref()
                .unwrap()
                .token
                .as_ptr(),
            buffer
        );
    }
    let idle = started.elapsed();
    if let ActiveState::History(raw) = &mut core.session.active_state {
        raw.revision += 1;
    }
    assert_eq!(core.edit_revision(&view).as_deref(), Some(token.as_str()));
    let refreshed = core
        .session
        .edit_revision_cache
        .borrow()
        .as_ref()
        .unwrap()
        .token
        .as_ptr();
    assert_ne!(refreshed, buffer);
    core.session.key = session::SessionKey::default();
    assert_eq!(core.edit_revision(&view).as_deref(), Some(token.as_str()));
    assert_ne!(
        core.session
            .edit_revision_cache
            .borrow()
            .as_ref()
            .unwrap()
            .token
            .as_ptr(),
        refreshed
    );
    view.run_state = RunState::Running;
    view.revision += 1;
    let token = core.edit_revision(&view).unwrap();
    let buffer = core
        .session
        .edit_revision_cache
        .borrow()
        .as_ref()
        .unwrap()
        .token
        .as_ptr();
    let started = std::time::Instant::now();
    for chunk in 0..40 {
        view.messages[3].blocks = vec![MessageBlock::Text {
            text: format!("chunk {chunk}"),
        }];
        if let ActiveState::History(raw) = &mut core.session.active_state {
            raw.revision += 1;
        }
        assert_eq!(core.edit_revision(&view).as_deref(), Some(token.as_str()));
        assert_eq!(
            core.session
                .edit_revision_cache
                .borrow()
                .as_ref()
                .unwrap()
                .token
                .as_ptr(),
            buffer
        );
    }
    let streaming = started.elapsed();
    let mut other_conversation = view.clone();
    other_conversation.conversation.id = "pi:another-conversation".into();
    assert!(
        core.check_edit_revision(&other_conversation, Some(&token))
            .is_err()
    );
    // The UI cache is never trusted by the writer, including accidental
    // stale keys from a projection that reused its native revision.
    view.messages[1].blocks = vec![MessageBlock::Text {
        text: "changed settled history".into(),
    }];
    assert!(core.check_edit_revision(&view, Some(&token)).is_err());
    assert!(core.session.edit_revision_cache.borrow().is_none());
    assert_ne!(core.edit_revision(&view).as_deref(), Some(token.as_str()));
    eprintln!(
        "edit cache bytes={history_bytes}, first={initial:?}, 40 idle hits={idle:?}, 40 streaming hits={streaming:?}"
    );
    let segment = crate::config::ConversationSegmentReference {
        excluded_message_ids: Vec::new(),
        message_origins: BTreeMap::new(),
        runtime_instance_id: "pi".into(),
        runtime_type_id: RuntimeKind::Pi,
        native_conversation_id: "pi:cache".into(),
        cwd: None,
        cutoff: None,
        handoff: None,
        imported: None,
        deleted: false,
    };
    let mut linked = view.clone();
    linked.run_state = RunState::Idle;
    let mut native_system = with_diagnostic.messages.last().unwrap().clone();
    native_system.id = "pi:cache:system:0".into();
    linked.messages.push(native_system);
    let mut linked_diagnostic = linked.clone();
    linked_diagnostic
        .messages
        .push(with_diagnostic.messages.last().unwrap().clone());
    for snapshot in [&mut linked, &mut linked_diagnostic] {
        super::super::continuation::namespace_associations(snapshot, 2);
        snapshot.messages =
            super::super::continuation::display_segment(&segment, &snapshot.messages, 2).unwrap();
    }
    let linked_token = core.edit_revision(&linked).unwrap();
    assert_eq!(
        core.edit_revision(&linked_diagnostic).as_deref(),
        Some(linked_token.as_str())
    );
    core.check_edit_revision(&linked_diagnostic, Some(&linked_token))
        .unwrap();
    assert!(same_edit_history(&linked, &linked_diagnostic.messages));
    let mut changed = linked_diagnostic.clone();
    changed
        .messages
        .iter_mut()
        .find(|message| message.id == "segment-2:pi:cache:system:0")
        .unwrap()
        .blocks
        .clear();
    assert!(
        core.check_edit_revision(&changed, Some(&linked_token))
            .is_err()
    );
    assert!(!same_edit_history(&linked, &changed.messages));
    changed = linked_diagnostic.clone();
    changed.messages[0].branch_point = Some("external branch".into());
    assert!(
        core.check_edit_revision(&changed, Some(&linked_token))
            .is_err()
    );
    assert!(!same_edit_history(&linked, &changed.messages));
    changed = linked_diagnostic;
    changed.messages[0].id = "segment-2:pi-diagnostic-forged-user".into();
    assert!(
        core.check_edit_revision(&changed, Some(&linked_token))
            .is_err()
    );
    assert!(!same_edit_history(&linked, &changed.messages));
    drop(application);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn codex_fork_boundaries_distinguish_copying_a_turn_from_editing_it() {
    let messages: Vec<Message> = serde_json::from_value(json!([
        {"id":"first","role":"user","branch_point":"turn-1","completed":true,"blocks":[]},
        {"id":"steer","role":"user","completed":true,"blocks":[]},
        {"id":"second","role":"user","branch_point":"turn-2","completed":true,"blocks":[]}
    ]))
    .unwrap();
    assert_eq!(
        codex_fork_point(&messages, "first", ForkBoundary::ThroughTurn).unwrap(),
        "turn-1"
    );
    assert_eq!(
        codex_fork_point(&messages, "steer", ForkBoundary::ThroughTurn).unwrap(),
        "turn-1"
    );
    assert_eq!(
        codex_fork_point(&messages, "second", ForkBoundary::BeforeTurn).unwrap(),
        "turn-1"
    );
    assert!(codex_fork_point(&messages, "first", ForkBoundary::BeforeTurn).is_err());
}

#[test]
fn fork_names_are_distinct_without_stacking_suffixes() {
    let titles = vec!["Work".into(), "Work (1)".into(), "Work (3)".into()];
    assert_eq!(fork_title("Work", &titles), "Work (2)");
    assert_eq!(fork_title("Work (3)", &titles), "Work (4)");
    assert_eq!(fork_title("Work (2)", &titles), "Work (4)");
    assert_eq!(fork_title("Work (draft)", &[]), "Work (draft) (1)");
}

#[test]
fn invalid_edits_and_unavailable_forks_leave_running_source_untouched() {
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce).unwrap();
    let root = std::env::temp_dir().join(format!(
        "velune-edit-admission-{:x}",
        u64::from_le_bytes(nonce)
    ));
    std::fs::create_dir(&root).unwrap();
    let helper = root.join("rpc.py");
    std::fs::write(&helper, r#"
import json, pathlib, sys
for line in sys.stdin:
    request = json.loads(line)
    mutating = request['type'] in ('abort', 'fork', 'clone', 'new_session', 'switch_session') or (
        request['type'] == 'prompt' and request.get('message', '').startswith('/velune-rewind ')
    )
    if mutating:
        with pathlib.Path(__file__).with_name('control.log').open('a') as log:
            log.write(request['type'] + '\n')
    print(json.dumps({'type':'response','id':request['id'],'command':request['type'],'success':True,'data':{}}), flush=True)
"#).unwrap();
    let mut application = crate::Application::open(crate::Options {
        home_directory: root.clone(),
        resources_directory: root.clone(),
    })
    .unwrap();
    let core = &mut application.runtime;
    core.runtime_instances = vec![RuntimeInstance {
        enabled: true,
        id: "pi".into(),
        name: "Synthetic Pi".into(),
        type_id: RuntimeKind::Pi,
        gateway_id: String::new(),
        settings: Default::default(),
    }];
    let config: PiConfig = serde_json::from_value(json!({
        "execution":{"kind":"local"}, "os_home":root,
        "binary":"/usr/bin/python3", "node_binary":"/usr/bin/python3", "rpc_entry":helper,
    }))
    .unwrap();
    let client = pi::Client::spawn(config.clone()).unwrap();
    let mut projection = PiProjection::new(ConversationSummary {
        native_read_state: None,
        id: "pi:synthetic".into(),
        runtime_id: "pi".into(),
        title: crate::conversation::ConversationTitle::Untitled,
        cwd: Some(root.to_string_lossy().into_owned()),
        created_at_unix_ms: None,
        updated_at_unix_ms: None,
        can_rename: false,
        can_delete: false,
        parent_conversation_id: None,
    });
    let snapshot = projection.snapshot.as_mut().unwrap();
    snapshot.run_state = RunState::Running;
    snapshot.messages = serde_json::from_value(json!([
        {"id":"u1","role":"user","completed":true,"blocks":[{"kind":"text","text":"first"}]},
        {"id":"a1","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"answer"}]},
        {"id":"u2","role":"user","completed":true,"blocks":[{"kind":"text","text":"running"}]}
    ]))
    .unwrap();
    let mut session = pi::PiSession::new(config);
    session.start(client, projection);
    core.session.active_state = ActiveState::Pi(Box::new(session));
    core.session.execution_runtime_id = Some("pi".into());
    let source_key = core.session.key;
    let mut view = core.current_snapshot().unwrap();
    assert!(!view.actions.can_send);
    assert!(
        core.editable_messages(&view)
            .iter()
            .all(|edit| edit.in_place)
    );
    for access in [
        crate::conversation::InputAccess::ExternalController,
        crate::conversation::InputAccess::ReadOnly,
    ] {
        view.actions.input_access = access;
        let editable = core.editable_messages(&view);
        assert_eq!(editable.len(), 2);
        assert!(editable.iter().all(|edit| !edit.in_place));
    }
    view.conversation.parent_conversation_id = Some("parent".into());
    assert!(core.editable_messages(&view).is_empty());
    let mut target = core.runtime_instances[0].clone();
    target.id = "other-pi".into();
    target.enabled = false;
    core.runtime_instances.push(target);
    let error = core
        .edit_message_with_runtime(
            "pi",
            "pi:synthetic",
            "u1",
            EditPlacement::InPlace,
            Some("other-pi"),
            None,
            crate::ComposedTurn {
                text: "replacement".into(),
                attachments: vec![],
                effort: None,
                permission_mode: None,
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("此运行时已停用"));
    let missing = root.join("missing.txt").to_string_lossy().into_owned();
    let error = core
        .edit_message_with_runtime(
            "pi",
            "pi:synthetic",
            "u1",
            EditPlacement::InPlace,
            None,
            None,
            crate::ComposedTurn {
                text: "replacement".into(),
                attachments: vec![missing],
                effort: None,
                permission_mode: None,
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("无法读取附件"));
    let error = core
        .edit_message_with_runtime(
            "pi",
            "pi:synthetic",
            "u1",
            EditPlacement::Fork,
            None,
            None,
            crate::ComposedTurn {
                text: "replacement".into(),
                attachments: vec![],
                effort: None,
                permission_mode: None,
            },
        )
        .unwrap_err();
    assert!(!matches!(error, RuntimeError::Busy), "{error:?}");
    let fork_error = core.fork_message("pi", "pi:synthetic", "u1").unwrap_err();
    assert!(!matches!(fork_error, RuntimeError::Busy), "{fork_error:?}");
    assert!(core.turn_running());
    assert!(core.session.key == source_key);
    assert!(
        !root.join("control.log").exists(),
        "rejected edits sent runtime controls"
    );
    core.shutdown_active().unwrap();
    application.close_stopping_work().unwrap();
    drop(application);
    std::fs::remove_dir_all(root).unwrap();
}
