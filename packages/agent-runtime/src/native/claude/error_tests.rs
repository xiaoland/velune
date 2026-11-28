use super::*;

#[test]
fn named_fork_launch_keeps_its_source_and_boundary() {
    let config = NativeConfig {
        execution: crate::execution::ExecutionTarget::Local,
        os_home: None,
        kind: NativeKind::ClaudeCode,
        binary: "/synthetic/claude".into(),
        node_binary: None,
        agent_dir: "/synthetic/home".into(),
        resources_directory: "/synthetic/resources".into(),
        projection_directory: "/synthetic/projection".into(),
        gateway: None,
        auto_compaction: None,
        mcp_servers: Vec::new(),
        session_agents: Vec::new(),
    };
    let pending = PendingFork {
        conversation_id: "claude:copy".into(),
        from: "source".into(),
        at: Some("boundary".into()),
        name: Some("Work (2)".into()),
    };
    let start = ClaudeStart::Fork {
        from: pending.from,
        at: pending.at,
        name: pending.name,
    };
    let command =
        NativeSession::claude_command(&config, None, Some(("copy", None, &start)), None, None)
            .unwrap();
    let args: Vec<_> = command
        .get_args()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    for pair in [
        ["--name", "Work (2)"],
        ["--resume", "source"],
        ["--resume-session-at", "boundary"],
        ["--session-id", "copy"],
    ] {
        assert!(args.windows(2).any(|args| args == pair));
    }
    assert!(args.iter().any(|arg| arg == "--fork-session"));
    let fresh = ClaudeStart::Fresh {
        name: Some("Work (1)".into()),
    };
    let command =
        NativeSession::claude_command(&config, None, Some(("fresh", None, &fresh)), None, None)
            .unwrap();
    let args: Vec<_> = command
        .get_args()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    assert!(args.windows(2).any(|args| args == ["--name", "Work (1)"]));
    assert!(
        args.windows(2)
            .any(|args| args == ["--session-id", "fresh"])
    );
    assert!(!args.iter().any(|arg| arg == "--resume"));
}

#[test]
fn result_and_assistant_errors_preserve_native_details() {
    let assistant =
        claude_error_blocks(&json!({"type":"assistant","error":"authentication_failed",
            "message":{"content":[{"type":"text","text":"synthetic credential rejected"}]}}))
        .unwrap();
    assert!(assistant.contains(&MessageBlock::Notice {
        text: "synthetic credential rejected".into()
    }));
    assert!(assistant.contains(&MessageBlock::Notice {
        text: "Claude Code 错误类型：authentication_failed".into()
    }));
    let result = claude_error_blocks(&json!({"type":"result","subtype":"success","is_error":true,
            "api_error_status":401,"result":"synthetic credential rejected"}))
    .unwrap();
    assert!(result.contains(&assistant[0]));
    assert!(result.contains(&MessageBlock::Notice {
        text: "HTTP 401".into()
    }));
    let loop_error = claude_error_blocks(&json!({"type":"result","subtype":"error_max_turns",
            "errors":["maximum turns reached"]}))
    .unwrap();
    assert!(loop_error.contains(&MessageBlock::Notice {
        text: "maximum turns reached".into()
    }));
    assert!(claude_error_blocks(&json!({"type":"result","errors":[42]})).is_err());
}
#[test]
fn empty_import_detection_requires_exact_native_metadata() {
    let marker = json!({"type":"queue-operation","operation":"dequeue","veluneEmptyImport":true,"sessionId":"empty","cwd":"/synthetic","timestamp":"2026-11-28T00:00:00.000Z"});
    let bytes = format!("{marker}\n").into_bytes();
    assert!(super::claude_empty_import(
        &bytes,
        "empty",
        Path::new("/synthetic")
    ));
    assert!(!super::claude_empty_import(
        &bytes,
        "other",
        Path::new("/synthetic")
    ));
    assert!(!super::claude_empty_import(
        b"",
        "empty",
        Path::new("/synthetic")
    ));
    let with_other = format!("{marker}\n{{\"type\":\"user\"}}\n");
    assert!(!super::claude_empty_import(
        with_other.as_bytes(),
        "empty",
        Path::new("/synthetic")
    ));
    for (key, value) in [("version", json!(2)), ("veluneEmptyImport", json!(2))] {
        let mut unknown = marker.clone();
        unknown[key] = value;
        assert!(!super::claude_empty_import(
            format!("{unknown}\n").as_bytes(),
            "empty",
            Path::new("/synthetic")
        ));
    }
}

fn empty_import_config(root: &Path) -> NativeConfig {
    NativeConfig {
        execution: crate::execution::ExecutionTarget::Local,
        os_home: Some(root.into()),
        kind: NativeKind::ClaudeCode,
        binary: root.join("unused-cli"),
        node_binary: None,
        agent_dir: root.join("claude"),
        resources_directory: root.into(),
        projection_directory: root.join("projection"),
        gateway: None,
        auto_compaction: None,
        mcp_servers: Vec::new(),
        session_agents: Vec::new(),
    }
}

fn legacy_empty_import(config: &NativeConfig, cwd: &Path) -> (PathBuf, Vec<u8>) {
    let path = crate::import::claude_project_directory(&config.agent_dir, cwd).join("empty.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let marker = json!({"type":"queue-operation","operation":"dequeue","veluneEmptyImport":true,
        "sessionId":"empty","cwd":cwd,"timestamp":"2026-11-28T00:00:00.000Z"});
    let bytes = format!("{marker}\n").into_bytes();
    std::fs::write(&path, &bytes).unwrap();
    (path, bytes)
}

#[test]
fn legacy_empty_import_is_backed_up_resumable_and_idempotent() {
    let root = velune_test_support::TempRoot::new("claude-empty-upgrade");
    let config = empty_import_config(&root);
    let (path, before) = legacy_empty_import(&config, &root);
    // Both opens observed the same legacy bytes before either helper ran.
    std::thread::scope(|scope| {
        let one = scope
            .spawn(|| super::claude_upgrade_empty_import(&config, &path, &before, "empty", &root));
        let two = scope
            .spawn(|| super::claude_upgrade_empty_import(&config, &path, &before, "empty", &root));
        one.join().unwrap().unwrap();
        two.join().unwrap().unwrap();
    });
    let after = std::fs::read(&path).unwrap();
    assert!(after.starts_with(&before));
    assert_eq!(
        std::fs::read(path.with_extension("jsonl.velune-empty-import.bak")).unwrap(),
        before
    );
    let added: Value = serde_json::from_slice(&after[before.len()..]).unwrap();
    assert_eq!(added["type"], "system");
    assert_eq!(added["subtype"], "turn_duration");
    assert_eq!(added["durationMs"], 0);
    assert_eq!(added["sessionId"], "empty");
    assert_eq!(added["cwd"], root.to_str().unwrap());
    assert!(added["uuid"].is_string());
    assert!(added.get("message").is_none());
    super::claude_prepare_empty_import(&config, "empty", &root).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), after);
}

#[test]
fn empty_import_upgrade_keeps_new_messages_unknown_records_and_failed_backup() {
    let root = velune_test_support::TempRoot::new("claude-empty-boundary");
    let config = empty_import_config(&root);
    let (path, before) = legacy_empty_import(&config, &root);
    let backup = path.with_extension("jsonl.velune-empty-import.bak");
    for addition in [
        "{\"type\":\"user\",\"uuid\":\"new\",\"message\":{\"content\":\"external write\"}}\n",
        "{\"type\":\"unknown\"}\n",
        "{broken",
    ] {
        let changed = [before.as_slice(), addition.as_bytes()].concat();
        std::fs::write(&path, &changed).unwrap();
        super::claude_prepare_empty_import(&config, "empty", &root).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), changed);
        assert!(!backup.exists());
        // The file can change after the Rust read but before the helper opens it.
        let error = super::claude_upgrade_empty_import(&config, &path, &before, "empty", &root)
            .unwrap_err();
        assert!(
            error.to_string().contains("empty import changed"),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), changed);
        assert!(!backup.exists());
    }
    std::fs::write(&path, &before).unwrap();
    std::fs::write(&backup, b"existing recovery data").unwrap();
    let error = super::claude_prepare_empty_import(&config, "empty", &root).unwrap_err();
    assert!(
        error.to_string().contains("backup verification failed"),
        "{error}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read(&backup).unwrap(), b"existing recovery data");
    // A crash after publishing the durable backup but before unlinking its
    // temporary name leaves two links to valid recovery bytes. Reopening works.
    std::fs::write(&backup, &before).unwrap();
    std::fs::hard_link(&backup, root.join("interrupted-backup-temporary")).unwrap();
    super::claude_prepare_empty_import(&config, "empty", &root).unwrap();
    assert!(std::fs::read(&path).unwrap().starts_with(&before));
    assert_eq!(std::fs::read(&backup).unwrap(), before);
}
