use serde_json::json;
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use velune_agent_runtime::{
    execution::ExecutionTarget, history::HistoryConfig, import::*, version::RuntimeKind,
};
fn root() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "velune-owned-import-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&path).unwrap();
    path
}
fn config(root: &Path, provider: &str, binary: PathBuf) -> HistoryConfig {
    HistoryConfig {
        kind: if provider == "codex" {
            RuntimeKind::Codex166
        } else {
            RuntimeKind::ClaudeCode
        },
        execution: ExecutionTarget::Local,
        os_home: Some(root.into()),
        provider: provider.into(),
        binary,
        node_binary: root.join("unused-node"),
        resources_directory: root.into(),
        root: root.into(),
        home: root.join(provider),
    }
}
#[test]
fn claude_owned_copy_cleanup_preserves_an_external_change() {
    let root = root();
    let config = config(&root, "claude-code", root.join("unused-cli"));
    for changed in [false, true] {
        let (_, _, token) = import_claude_conversation_owned(
            &config.execution,
            &config.home,
            &root,
            &Context { messages: vec![] },
            "synthetic",
        )
        .unwrap();
        let path = token.source_path().to_owned();
        if changed {
            std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(b"external change\n")
                .unwrap();
            let before = std::fs::read(&path).unwrap();
            assert!(cleanup_owned_import(&config, &token).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), before);
        } else {
            cleanup_owned_import(&config, &token).unwrap();
            assert!(!path.exists());
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn codex_owned_materialization_and_failed_materialization_leave_no_list_orphan() {
    let Some(binary) = std::env::var_os("VELUNE_TEST_CODEX_BINARY") else {
        return;
    };
    let root = root();
    let config = config(&root, "codex", binary.into());
    let context: Context = serde_json::from_value(
        json!({"messages":[{"role":"user","content":[{"type":"text","text":"synthetic"}]}]}),
    )
    .unwrap();
    for provider in ["openai", "velune", "missing-provider"] {
        let saved = if provider == "velune" {
            "velune"
        } else {
            "openai"
        };
        let (ImportedConversation::Codex { thread_id }, _, mut token) =
            import_codex_conversation_owned(
                &config.execution,
                &config.home,
                &root,
                &context,
                "0.166.0",
                saved,
            )
            .unwrap()
        else {
            panic!()
        };
        let materialized = velune_agent_runtime::native::materialize_owned_codex_import(
            &config, &thread_id, provider, &mut token,
        );
        assert_eq!(materialized.is_ok(), provider != "missing-provider");
        let path = token.source_path().to_owned();
        cleanup_owned_import(&config, &token).unwrap();
        assert!(!path.exists());
        let listed = velune_agent_runtime::native::list_codex_history_at(
            config.kind,
            &config.binary,
            &config.home,
            "synthetic",
            false,
            &config.execution,
            config.os_home.as_deref(),
        )
        .unwrap();
        assert!(
            !listed
                .iter()
                .any(|session| session.id.ends_with(&thread_id))
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn codex_native_to_managed_import_preserves_inter_agent_context_and_source_bytes() {
    let Some(binary) = std::env::var_os("VELUNE_TEST_CODEX_BINARY") else {
        return;
    };
    let root = root();
    let config = config(&root, "codex", binary.into());
    let seed: Context = serde_json::from_value(json!({"messages":[
        {"role":"user","content":[{"type":"text","text":"Synthetic native request"}]},
        {"role":"assistant","content":[{"type":"tool_call","id":"tool","name":"read","arguments":{}}]},
        {"role":"user","content":[{"type":"tool_result","id":"tool","content":[{"type":"text","text":"Synthetic result"}],"is_error":false}]}
    ]})).unwrap();
    let (
        ImportedConversation::Codex {
            thread_id: source_id,
        },
        _,
        source,
    ) = import_codex_conversation_owned(
        &config.execution,
        &config.home,
        &root,
        &seed,
        "0.167.0",
        "openai",
    )
    .unwrap()
    else {
        panic!()
    };
    let mut append = std::fs::OpenOptions::new()
        .append(true)
        .open(source.source_path())
        .unwrap();
    let ordinal = std::fs::read_to_string(source.source_path())
        .unwrap()
        .lines()
        .count();
    for (index, mut record) in [
        json!({"type":"inter_agent_communication_metadata","payload":{"trigger_turn":false}}),
        json!({"type":"response_item","payload":{"type":"agent_message","id":"amsg_synthetic","author":"/root/worker","recipient":"/root","content":[{"type":"input_text","text":"Synthetic child result"}]}}),
    ].into_iter().enumerate() {
        record["timestamp"] = json!("2026-11-13T00:00:00.000Z");
        record["ordinal"] = json!(ordinal + index);
        writeln!(append, "{record}").unwrap();
    }
    drop(append);
    let original = std::fs::read(source.source_path()).unwrap();
    let exported = velune_agent_runtime::history::export_context(&config, &source_id).unwrap();
    assert_eq!(exported.messages.last().unwrap().role, Role::Assistant);
    assert!(
        matches!(&exported.messages.last().unwrap().content[0], Block::Text {text} if text.contains("/root/worker") && text.contains("/root"))
    );
    let (
        ImportedConversation::Codex {
            thread_id: target_id,
        },
        _,
        mut target,
    ) = import_codex_conversation_owned(
        &config.execution,
        &config.home,
        &root,
        &exported,
        "0.167.0",
        "velune",
    )
    .unwrap()
    else {
        panic!()
    };
    velune_agent_runtime::native::materialize_owned_codex_import(
        &config,
        &target_id,
        "velune",
        &mut target,
    )
    .unwrap();
    let target_records = std::fs::read_to_string(target.source_path()).unwrap();
    assert!(
        target_records
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .any(|record| record["type"] == "session_meta"
                && record["payload"]["model_provider"] == "velune")
    );
    let reexported = velune_agent_runtime::history::export_context(&config, &target_id).unwrap();
    // Import remaps native item/tool identities. Ordered role/content and pairing
    // remain equivalent, including the attributed communication body.
    assert_eq!(reexported.messages.len(), exported.messages.len());
    for (before, after) in exported.messages.iter().zip(&reexported.messages) {
        assert_eq!(before.role, after.role);
        assert_eq!(before.content.len(), after.content.len());
        for (before, after) in before.content.iter().zip(&after.content) {
            match (before, after) {
                (
                    Block::ToolCall {
                        name: a,
                        arguments: x,
                        ..
                    },
                    Block::ToolCall {
                        name: b,
                        arguments: y,
                        ..
                    },
                ) => assert_eq!((a, x), (b, y)),
                (
                    Block::ToolResult {
                        content: a,
                        is_error: x,
                        ..
                    },
                    Block::ToolResult {
                        content: b,
                        is_error: y,
                        ..
                    },
                ) => assert_eq!((a, x), (b, y)),
                _ => assert_eq!(before, after),
            }
        }
    }
    assert_eq!(std::fs::read(source.source_path()).unwrap(), original);
    cleanup_owned_import(&config, &target).unwrap();
    // The source token predates the appended native records, so cleanup must
    // reject it instead of removing changed history.
    assert!(cleanup_owned_import(&config, &source).is_err());
    assert_eq!(std::fs::read(source.source_path()).unwrap(), original);
    std::fs::remove_dir_all(root).unwrap();
}
