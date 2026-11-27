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
    let marker = json!({"type":"queue-operation","operation":"dequeue","veluneEmptyImport":true,"sessionId":"empty","cwd":"/synthetic"});
    let bytes = serde_json::to_vec(&marker).unwrap();
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
}
