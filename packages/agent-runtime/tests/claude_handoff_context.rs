//! Claude source export through the production validator and native writers.
//! Everything lives in an owned temporary directory; no runtime or model runs.
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf};
use velune_agent_runtime::{
    execution::ExecutionTarget,
    history::{HistoryConfig, export_context},
    import::*,
    version::RuntimeKind,
};

const SESSION: &str = "00000000-0000-4000-8000-000000000001";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "velune-claude-handoff-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn config(&self) -> HistoryConfig {
        HistoryConfig {
            kind: RuntimeKind::ClaudeCode,
            execution: ExecutionTarget::Local,
            os_home: Some(self.0.clone()),
            provider: "claude-code".into(),
            binary: self.0.join("unused-cli"),
            node_binary: self.0.join("unused-node"),
            resources_directory: self.0.clone(),
            root: self.0.clone(),
            home: self.0.clone(),
        }
    }
    fn save(&self, records: &[Value]) {
        std::fs::write(
            self.0.join(format!("{SESSION}.jsonl")),
            records.iter().map(|r| format!("{r}\n")).collect::<String>(),
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn completed_parallel_calls() -> Vec<Value> {
    vec![
        json!({"type":"user","uuid":"u","parentUuid":null,"message":{"content":"question"}}),
        json!({"type":"assistant","uuid":"a1","parentUuid":"u","message":{"id":"api-1","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"path":"synthetic-a"}}]}}),
        json!({"type":"assistant","uuid":"a2","parentUuid":"a1","message":{"id":"api-1","content":[{"type":"tool_use","id":"t2","name":"Read","input":{"path":"synthetic-b"}}]}}),
        json!({"type":"user","uuid":"r1","parentUuid":"a1","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"result-a-preserved"}]}}),
        json!({"type":"user","uuid":"r2","parentUuid":"a2","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"result-b-preserved","is_error":true},{"type":"text","text":"follow-up-preserved"}]}}),
        json!({"type":"assistant","uuid":"answer","parentUuid":"r2","message":{"id":"api-2","content":"answer-preserved"}}),
    ]
}

fn native_tool_ids(value: &Value, calls: &mut BTreeSet<String>, results: &mut BTreeSet<String>) {
    let kind = value["type"].as_str();
    if matches!(kind, Some("tool_use" | "toolCall" | "function_call")) {
        calls.insert(
            value["call_id"]
                .as_str()
                .or_else(|| value["id"].as_str())
                .unwrap()
                .into(),
        );
    }
    if kind == Some("tool_result")
        || kind == Some("function_call_output")
        || value["role"] == "toolResult"
    {
        results.insert(
            value["tool_use_id"]
                .as_str()
                .or_else(|| value["call_id"].as_str())
                .or_else(|| value["toolCallId"].as_str())
                .unwrap()
                .into(),
        );
    }
    match value {
        Value::Array(values) => values
            .iter()
            .for_each(|v| native_tool_ids(v, calls, results)),
        Value::Object(values) => values
            .values()
            .for_each(|v| native_tool_ids(v, calls, results)),
        _ => {}
    }
}

#[test]
fn sibling_claude_results_export_and_write_all_native_targets() {
    let f = Fixture::new();
    f.save(&completed_parallel_calls());
    let context = export_context(&f.config(), SESSION).unwrap();
    let results: Vec<_> = context
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .filter_map(|b| {
            if let Block::ToolResult { id, is_error, .. } = b {
                Some((id.as_str(), *is_error))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(results, [("t1", false), ("t2", true)]);
    let execution = ExecutionTarget::Local;
    let (_, _, claude) = import_claude_conversation_owned(
        &execution,
        &f.0.join("claude-target"),
        &f.0,
        &context,
        "synthetic",
    )
    .unwrap();
    let (_, _, codex) = import_codex_conversation_owned(
        &execution,
        &f.0.join("codex-target"),
        &f.0,
        &context,
        "0.159.3",
        "synthetic",
    )
    .unwrap();
    let ImportedConversation::Pi { path: pi } =
        import_pi_conversation(&execution, &f.0.join("pi-target"), None, &f.0, &context).unwrap()
    else {
        panic!()
    };
    for path in [claude.source_path(), codex.source_path(), pi.as_path()] {
        let text = std::fs::read_to_string(path).unwrap();
        for preserved in [
            "synthetic-a",
            "synthetic-b",
            "result-a-preserved",
            "result-b-preserved",
            "follow-up-preserved",
            "answer-preserved",
        ] {
            assert!(text.contains(preserved), "missing {preserved} in {path:?}");
        }
        let mut calls = BTreeSet::new();
        let mut results = BTreeSet::new();
        for line in text.lines() {
            native_tool_ids(
                &serde_json::from_str::<Value>(line).unwrap(),
                &mut calls,
                &mut results,
            );
        }
        assert_eq!(calls.len(), 2);
        assert_eq!(calls, results);
        assert!(
            !calls.contains("t1") && !calls.contains("t2"),
            "target must own its tool IDs"
        );
    }
}

#[test]
fn genuinely_missing_claude_result_is_rejected_at_source_export() {
    let f = Fixture::new();
    let mut records = completed_parallel_calls();
    records.remove(4);
    records[3]["parentUuid"] = json!("a2");
    records.last_mut().unwrap()["parentUuid"] = json!("r1");
    f.save(&records);
    let error = export_context(&f.config(), SESSION).unwrap_err();
    assert!(error.to_string().contains("工具结果不完整"));
}

#[test]
fn abandoned_tool_results_do_not_repair_a_rewound_branch() {
    let f = Fixture::new();
    let mut records = completed_parallel_calls();
    records.push(json!({"type":"user","uuid":"replacement","parentUuid":"a2","message":{"content":"different branch"}}));
    records.push(json!({"type":"assistant","uuid":"replacement-answer","parentUuid":"replacement","message":{"id":"api-replacement","content":"new answer"}}));
    f.save(&records);
    assert!(export_context(&f.config(), SESSION).is_err());
}

#[test]
fn compaction_does_not_resurrect_summarized_away_sibling_results() {
    let f = Fixture::new();
    let mut records = completed_parallel_calls();
    records.extend([
        json!({"type":"system","subtype":"compact_boundary","uuid":"compact","parentUuid":null,"logicalParentUuid":"answer","compactMetadata":{"preservedMessages":{"uuids":["a1","a2","r2"],"anchorUuid":"summary"}}}),
        json!({"type":"user","uuid":"summary","parentUuid":"compact","isCompactSummary":true,"message":{"content":"summary"}}),
        json!({"type":"assistant","uuid":"after-compact","parentUuid":"summary","message":{"id":"api-after-compact","content":"answer"}}),
    ]);
    f.save(&records);
    assert!(export_context(&f.config(), SESSION).is_err());
    records[6]["compactMetadata"]["preservedMessages"]["uuids"] = json!(["a1", "a2", "r1", "r2"]);
    f.save(&records);
    export_context(&f.config(), SESSION).unwrap();
}

#[test]
fn foreign_late_and_ambiguous_siblings_remain_excluded() {
    for mode in ["sidechain", "late", "ambiguous"] {
        let f = Fixture::new();
        let mut records = completed_parallel_calls();
        match mode {
            "sidechain" => records[3]["isSidechain"] = json!(true),
            "late" => {
                let result = records.remove(3);
                records.insert(4, result);
            }
            "ambiguous" => {
                let mut duplicate = records[3].clone();
                duplicate["uuid"] = json!("ambiguous-result");
                records.insert(4, duplicate);
            }
            _ => unreachable!(),
        }
        f.save(&records);
        assert!(export_context(&f.config(), SESSION).is_err(), "{mode}");
    }
}

#[test]
fn recovered_results_do_not_revive_off_branch_human_text() {
    let f = Fixture::new();
    let mut records = completed_parallel_calls();
    records[3]["message"]["content"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"text","text":"abandoned-human-prompt"}));
    f.save(&records);
    let context = export_context(&f.config(), SESSION).unwrap();
    assert!(
        !context
            .messages
            .iter()
            .flat_map(|m| &m.content)
            .any(|b| { matches!(b, Block::Text { text } if text == "abandoned-human-prompt") })
    );
    assert!(
        !context
            .messages
            .iter()
            .any(|m| m.source_message_id.as_deref() == Some("r1"))
    );
}
