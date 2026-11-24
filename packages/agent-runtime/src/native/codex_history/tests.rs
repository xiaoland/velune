#[test]
fn archive_index_requires_current_native_coverage() {
    use super::{UNIX_EPOCH, archive_index_covers_rollouts, fs, json};
    use ruzstd::encoding::{CompressionLevel, compress_to_vec};

    let nonce = getrandom::u64().unwrap();
    let home = std::env::temp_dir().join(format!("velune-archive-index-{nonce}"));
    let archive = home.join("archived_sessions");
    fs::create_dir_all(&archive).unwrap();
    let renamed = archive.join("renamed.jsonl");
    let compressed = archive.join("migrated.jsonl.zst");
    let header = br#"{"type":"session_meta","payload":{"id":"source"}}"#;
    fs::write(&renamed, header).unwrap();
    fs::write(
        &compressed,
        compress_to_vec(header.as_slice(), CompressionLevel::Fastest),
    )
    .unwrap();
    let modified = [renamed, compressed]
        .into_iter()
        .map(|path| fs::metadata(path).unwrap().modified().unwrap())
        .max()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut threads = [json!({
        "id": "source", "historyMode": "paginated", "updatedAt": modified,
    })];
    let covered = archive_index_covers_rollouts(&home, &threads, |_, _| None);
    let unindexed = archive_index_covers_rollouts(&home, &[], |_, _| None);
    threads[0]["updatedAt"] = json!(modified - 1);
    let stale = archive_index_covers_rollouts(&home, &threads, |_, _| None);
    threads[0]["updatedAt"] = json!(modified);
    threads[0]["historyMode"] = json!("legacy");
    let legacy = archive_index_covers_rollouts(&home, &threads, |_, _| None);
    fs::remove_dir_all(home).unwrap();

    assert!(covered);
    assert!(!unindexed);
    assert!(!stale);
    assert!(!legacy);
}

#[cfg(unix)]
#[test]
fn exact_codex_child_history_reads_metadata_and_messages_in_one_process() {
    use std::{fs, os::unix::fs::PermissionsExt, time::Instant};

    let root = std::env::temp_dir().join(format!("velune-codex-child-read-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let binary = root.join("codex");
    fs::write(
            &binary,
            r#"#!/usr/bin/python3
import json, pathlib, sys
root = pathlib.Path(__file__).parent
def log(event):
    with (root / "calls").open("a") as f: f.write(event + "\n")
if "--version" in sys.argv:
    log("version")
    print("codex-cli 0.166.0")
    raise SystemExit(0)
assert sys.argv[1:] == ["app-server"]
log("app-server")
for line in sys.stdin:
    request = json.loads(line)
    method = request["method"]
    if "id" not in request:
        assert method == "initialized"
        continue
    log(method)
    if method == "initialize":
        result = {}
    elif method == "thread/read":
        params = request["params"]
        assert params["includeTurns"] is False
        result = {
            "thread": {"id": "child", "parentThreadId": "parent", "agentNickname": "Worker",
                       "historyMode": "paginated", "cwd": str(root), "createdAt": 1, "updatedAt": 2,
                       "path": str(root / "archived_sessions" / "child.jsonl")},
            "readState": {"revision": "child-receipt", "firstUnread": {"type": "threadStart"}},
        }
    elif method == "thread/items/list":
        assert request["params"]["threadId"] == "child"
        result = {"data": [{"turnId": "turn", "item": {"type": "agentMessage", "id": "message", "text": "answer"}}], "nextCursor": None}
    elif method == "thread/turns/list":
        assert request["params"]["threadId"] == "child"
        result = {"data": [{"id": "turn", "status": "completed"}], "nextCursor": None}
    else:
        raise AssertionError(method)
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#,
        )
        .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let config = crate::history::HistoryConfig {
        kind: crate::version::RuntimeKind::Codex166,
        execution: crate::execution::ExecutionTarget::Local,
        os_home: Some(root.clone()),
        provider: "codex".into(),
        binary,
        node_binary: root.join("unused"),
        resources_directory: root.clone(),
        root: root.clone(),
        home: root.clone(),
    };
    let calls = root.join("calls");
    let started = Instant::now();
    let previous = crate::history::read_summary(&config, "runtime", "child")
        .unwrap()
        .unwrap();
    let (previous_summary, archived) = previous.as_ref();
    let previous_history = crate::history::read(&config, "runtime", "child").unwrap();
    let two_reads = started.elapsed();
    let trace = fs::read_to_string(&calls).unwrap();
    assert_eq!(
        trace.lines().filter(|line| *line == "app-server").count(),
        2
    );
    assert_eq!(
        trace.lines().filter(|line| *line == "thread/read").count(),
        2
    );
    assert!(*archived);
    fs::write(&calls, "").unwrap();

    let started = Instant::now();
    let combined = crate::history::read_codex_with_summary(&config, "runtime", "child").unwrap();
    let (summary, history) = combined.as_ref();
    let combined_read = started.elapsed();
    assert_eq!(
        fs::read_to_string(&calls)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "version",
            "app-server",
            "initialize",
            "thread/read",
            "thread/items/list",
            "thread/turns/list"
        ]
    );
    assert_eq!(summary, previous_summary);
    assert_eq!(
        summary.parent_conversation_id.as_deref(),
        Some("runtime:parent")
    );
    assert_eq!(summary.title, ConversationTitle::Native("Worker".into()));
    assert_eq!(summary.native_read_state, history.native_read_state);
    assert_eq!(history.native_id, "child");
    assert_eq!(history.messages, previous_history.messages);
    assert_eq!(history.messages.len(), 1);

    fs::write(&calls, "").unwrap();
    let failure = crate::history::read_codex_with_summary(&config, "runtime", "wrong-thread")
        .err()
        .expect("a different thread must not be opened");
    assert!(failure.full_message().contains("Codex 返回不同的原生会话"));
    assert_eq!(
        fs::read_to_string(&calls)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["version", "app-server", "initialize", "thread/read"]
    );
    println!("synthetic Codex: two reads {two_reads:?}, combined {combined_read:?}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rollout_turn_context_records_the_speed_variant_as_its_model() {
    let directory = std::env::temp_dir().join(format!("velune-rollout-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("rollout.jsonl");
    std::fs::write(
            &path,
            concat!(
                "{\"type\":\"turn_context\",\"payload\":{\"turn_id\":\"t1\",\"model\":\"gpt\",\"service_tier\":\"priority\"}}\n",
                "{\"type\":\"turn_context\",\"payload\":{\"turn_id\":\"t2\",\"model\":\"gpt\"}}\n",
            ),
        )
        .unwrap();
    let turns = super::rollout_turns(&path);
    std::fs::remove_dir_all(&directory).ok();
    assert_eq!(
        turns["t1"].settings.model.as_deref(),
        Some(&velune_conversation::model_variant::codex_speed_model("gpt", "priority")[..])
    );
    assert_eq!(turns["t2"].settings.model.as_deref(), Some("gpt"));
}
#[test]
fn thread_names_preserve_native_agent_identity_without_a_preview() {
    let thread = json!({"id":"child","name":"人工名称","agentNickname":"Kepler","agentRole":"explorer","preview":"任务首句"});
    assert_eq!(
        thread_title(&thread),
        ConversationTitle::Native("人工名称".into())
    );
    let mut thread = thread;
    thread["name"] = Value::Null;
    thread["preview"] = json!("");
    assert_eq!(
        summary(&thread, "runtime").unwrap().title,
        ConversationTitle::Native("Kepler".into())
    );
    thread["agentNickname"] = json!("  ");
    assert_eq!(
        thread_title(&thread),
        ConversationTitle::Native("explorer".into())
    );
    thread["preview"] = json!("任务首句");
    assert_eq!(
        thread_title(&thread),
        ConversationTitle::FirstMessage("任务首句".into())
    );
}

#[test]
fn legacy_history_uses_loaded_native_turns_and_keeps_branch_identity() {
    let mut thread = serde_json::json!({"id":"synthetic","historyMode":"legacy","turns":[
        {"id":"turn","status":"completed","items":[{"type":"userMessage","id":"message","content":[{"type":"text","text":"synthetic"}]}]}
    ]});
    let (items, turns) = super::legacy_history(&mut thread, "synthetic").unwrap();
    assert_eq!(items[0]["turnId"], "turn");
    assert_eq!(items[0]["item"]["id"], "message");
    assert_eq!(turns.len(), 1);
    assert!(super::legacy_history(&mut thread, "another").is_err());
}

use super::*;

#[test]
fn automatic_compaction_does_not_turn_user_failure_into_maintenance() {
    let items = vec![
        json!({"turnId":"user-turn","item":{"id":"prompt","type":"userMessage","content":[{"type":"text","text":"question"}]}}),
        json!({"turnId":"user-turn","item":{"id":"compact","type":"contextCompaction"}}),
    ];
    let turns =
        vec![json!({"id":"user-turn","status":"failed","error":{"message":"answer failed"}})];
    let (messages, _executions) =
        project_history(&items, &turns, &HashMap::new(), "runtime").unwrap();
    assert!(messages.iter().any(|message| message.blocks
        == vec![MessageBlock::ContextCompaction {
            text: "上下文已压缩".into()
        }]));
    assert!(messages.iter().any(|message| message.blocks.iter().any(
            |block| matches!(block, MessageBlock::RuntimeFailure { detail: text, .. } if text.contains("answer failed"))
        )));
}

#[test]
fn input_free_turn_usage_stays_with_preceding_user_without_changing_settings() {
    let items = vec![
        json!({"turnId":"first","item":{"id":"u1","type":"userMessage","content":[{"type":"text","text":"original"}]}}),
        json!({"turnId":"resume","item":{"id":"a1","type":"agentMessage","text":"resumed"}}),
        json!({"turnId":"next","item":{"id":"u2","type":"userMessage","content":[{"type":"text","text":"next"}]}}),
        json!({"turnId":"maintenance","item":{"id":"compact","type":"contextCompaction"}}),
    ];
    let native = vec![
        json!({"id":"first","status":"interrupted"}),
        json!({"id":"resume","status":"completed"}),
        json!({"id":"next","status":"interrupted"}),
        json!({"id":"maintenance","status":"completed"}),
    ];
    let records = [
        ("first", 2, "original"),
        ("resume", 3, "resumed"),
        ("next", 7, "next"),
    ]
    .into_iter()
    .map(|(id, tokens, model)| {
        (
            id.to_owned(),
            RolloutTurn {
                settings: TurnSettings {
                    model: Some(model.into()),
                    ..Default::default()
                },
                usage: Some(TurnUsage {
                    input_tokens: Some(tokens),
                    context_tokens: Some(tokens),
                    ..Default::default()
                }),
            },
        )
    })
    .collect();
    let (messages, executions) = project_history(&items, &native, &records, "runtime").unwrap();
    assert_eq!(messages.len(), 4);
    assert_eq!(
        executions.record(&messages[0].id).unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    assert_eq!(
        executions.record(&messages[2].id).unwrap().outcome,
        Some(TurnOutcome::Cancelled)
    );
    assert_eq!(
        messages[0].turn_settings.as_ref().unwrap().model.as_deref(),
        Some("original")
    );
    assert_eq!(
        messages[0].turn_usage.as_ref().unwrap().input_tokens,
        Some(5)
    );
    assert_eq!(
        messages[0].turn_usage.as_ref().unwrap().context_tokens,
        Some(3)
    );
    assert_eq!(
        messages[2].turn_settings.as_ref().unwrap().model.as_deref(),
        Some("next")
    );
    assert_eq!(
        messages[2].turn_usage.as_ref().unwrap().input_tokens,
        Some(7)
    );
}

#[test]
fn history_keeps_failures_between_turns_even_without_visible_items() {
    let items = vec![
        json!({"turnId":"failed","startedAtMs":1000,"item":{
                "id":"user","type":"userMessage","content":[{"type":"text","text":"question"}]}}),
        json!({"turnId":"failed","item":{"id":"hidden","type":"reasoning","summary":[]}}),
        json!({"turnId":"ok","completedAtMs":3000,"item":{
                "id":"answer","type":"agentMessage","text":"answer"}}),
    ];
    let turns = vec![
        json!({"id":"empty-first","status":"failed","error":null}),
        json!({"id":"failed","status":"failed","error":{"message":"capacity"}}),
        json!({"id":"empty-middle","status":"failed","error":{"message":"network"}}),
        json!({"id":"ok","status":"completed","error":null}),
        json!({"id":"empty-last","status":"failed","error":{"message":"quota"}}),
    ];
    let (messages, _executions) =
        project_history(&items, &turns, &HashMap::new(), "runtime").unwrap();
    assert_eq!(
        messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec![
            "codex-turn-error-empty-first",
            "user",
            "codex-turn-error-failed",
            "codex-turn-error-empty-middle",
            "answer",
            "codex-turn-error-empty-last",
        ]
    );
    assert_eq!(messages[1].timestamp_unix_ms, Some(1000));
    assert_eq!(messages[1].branch_point.as_deref(), Some("failed"));
    assert_eq!(messages[4].timestamp_unix_ms, Some(3000));
    let MessageBlock::RuntimeFailure { detail: text, .. } = &messages[0].blocks[0] else {
        panic!()
    };
    assert!(text.contains("上游未提供错误详情"));
    let empty = project_history(&[], &turns[..1], &HashMap::new(), "runtime").unwrap();
    assert_eq!(empty.0.len(), 1);
}
