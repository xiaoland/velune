//! Synthetic native records only: no CLI authentication or model requests.
use serde_json::{Value, json};
use std::{io::Write, process::Stdio};
use velune_agent_runtime::{history, process::command, version::RuntimeKind};

fn codex(records: &[Value], id: Option<&str>, omit: bool) -> std::process::Output {
    let mut args = vec!["-c", include_str!("../resources/codex_context.py")];
    if let Some(id) = id {
        args.extend([
            if omit {
                "--without-message-id"
            } else {
                "--before-message-id"
            },
            id,
        ]);
    }
    let mut child = command("python3")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    for record in records {
        writeln!(child.stdin.as_mut().unwrap(), "{record}").unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}
fn codex_summary_required(records: &[Value]) -> std::process::Output {
    let mut child = command("python3")
        .args([
            "-c",
            include_str!("../resources/codex_context.py"),
            "--requires-portable-summary",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    for record in records {
        writeln!(child.stdin.as_mut().unwrap(), "{record}").unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}
fn codex_with_summary(records: &[Value], summary: &str) -> std::process::Output {
    let rollout = records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let request = json!({"rollout":rollout,"portableSummary":summary});
    let mut child = command("python3")
        .args([
            "-c",
            include_str!("../resources/codex_context.py"),
            "--request-envelope",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}
fn prompt(id: &str) -> Value {
    json!({"type":"response_item","payload":{"type":"message","role":"user","id":id,"content":[{"type":"input_text","text":"duplicate"}]}})
}
#[test]
fn codex_delivery_metadata_is_not_model_context_and_unknown_records_still_fail() {
    let records = vec![
        prompt("first"),
        json!({"type":"inter_agent_communication_metadata","payload":{"trigger_turn":false}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Native communication body"}]}}),
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"tool","name":"read","arguments":"{}"}}),
        json!({"type":"inter_agent_communication_metadata","payload":{"trigger_turn":true}}),
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"tool","output":"result"}}),
    ];
    let expected: Vec<_> = records
        .iter()
        .filter(|record| record["type"] != "inter_agent_communication_metadata")
        .cloned()
        .collect();
    let result = codex(&records, None, false);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, codex(&expected, None, false).stdout);
    assert_eq!(
        codex_summary_required(&records).stdout,
        br#"{"required": false}
"#,
    );
    for kind in ["unknown_context_record", "inter_agent_communication"] {
        let mut unsupported = records.clone();
        unsupported.push(json!({"type":kind,"payload":{"content":"must not disappear"}}));
        let result = codex(&unsupported, None, false);
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr)
                .contains(&format!("unsupported Codex rollout record: {kind}"))
        );
        assert!(result.stdout.is_empty());
    }
    let mut unsupported = records;
    unsupported.push(json!({"type":"response_item","payload":{"type":"unknown_context_item","content":"must not disappear"}}));
    assert!(!codex(&unsupported, None, false).status.success());
}
#[test]
fn codex_agent_message_preserves_attribution_body_and_tool_pair_after_compaction() {
    let communication = json!({"type":"agent_message","id":"amsg_1","author":"/root/worker","recipient":"/root","content":[{"type":"input_text","text":"Message Type: MESSAGE\nTask name: /root\nSender: /root/worker\nPayload:\nDone."},{"type":"input_text","text":" More details."}]});
    let records = vec![
        json!({"type":"response_item","payload":{"type":"unknown_obsolete_item"}}),
        json!({"type":"compacted","payload":{"message":"portable summary","replacement_history":[
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"compact summary"}]},
            communication,
            {"type":"function_call","call_id":"tool","name":"read","arguments":"{}"}
        ]}}),
        json!({"type":"inter_agent_communication_metadata","payload":{"trigger_turn":false}}),
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"tool","output":"tool result"}}),
        json!({"type":"response_item","payload":communication}),
    ];
    let result = codex(&records, None, false);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let context: Value = serde_json::from_slice(&result.stdout).unwrap();
    let messages = context["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 5);
    assert_eq!(messages[0]["content"][0]["text"], "compact summary");
    for index in [1, 4] {
        assert_eq!(messages[index]["role"], "assistant");
        assert_eq!(
            messages[index]["content"][0]["text"],
            "Agent message from /root/worker to /root:\n"
        );
        assert_eq!(
            messages[index]["content"][1]["text"],
            communication["content"][0]["text"]
        );
        assert_eq!(messages[index]["content"][2]["text"], " More details.");
        assert!(messages[index].get("source_message_id").is_none());
    }
    assert_eq!(messages[2]["content"][0]["id"], "tool");
    assert_eq!(messages[3]["content"][0]["id"], "tool");
    let mut encrypted = communication.clone();
    encrypted["content"] = json!([
        {"type":"input_text","text":"visible envelope"},
        {"type":"encrypted_content","encrypted_content":"opaque body"}
    ]);
    let result = codex(
        &[json!({"type":"response_item","payload":encrypted})],
        None,
        false,
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap(),
        json!({"errorCode":"portable_summary_required"})
    );

    let mut unknown = communication;
    unknown["content"] = json!([{"type":"unknown_agent_content","text":"unknown body"}]);
    let result = codex(
        &[json!({"type":"response_item","payload":unknown})],
        None,
        false,
    );
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("unsupported Codex agent message content: unknown_agent_content")
    );
}
#[test]
fn codex_omits_exact_duplicate_prompt_and_keeps_native_tools_and_replies() {
    let records = vec![
        prompt("first"),
        prompt("steer"),
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"tool","name":"read","arguments":"{}"}}),
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"tool","output":"result"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"reply"}]}}),
    ];
    let output = codex(&records, Some("steer"), true);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let context: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(context["messages"].as_array().unwrap().len(), 4);
    assert_eq!(context["messages"][1]["content"][0]["id"], "tool");
    let output = codex(&records, Some("steer"), false);
    assert!(output.status.success());
    let context: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(context["messages"].as_array().unwrap().len(), 1);
    assert!(!codex(&records, Some("missing"), true).status.success());
}
#[test]
fn codex_omits_retained_identity_after_compaction_and_keeps_summary() {
    let records = vec![
        prompt("first"),
        json!({"type":"compacted","payload":{"replacement_history":[
            {"type":"message","id":"summary","role":"user","content":[{"type":"input_text","text":"summary"}]},
            {"type":"message","id":"first","role":"user","content":[{"type":"input_text","text":"duplicate"}]}
        ]}}),
    ];
    let output = codex(&records, Some("first"), true);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let context: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(context["messages"].as_array().unwrap().len(), 1);
    assert_eq!(context["messages"][0]["content"][0]["text"], "summary");
    assert!(codex(&records, Some("first"), false).status.success());
}

#[test]
fn codex_provider_private_compaction_is_replaced_with_portable_summary() {
    let records = vec![json!({
        "type":"compacted",
        "payload":{
            "message":"Portable summary from Codex",
            "replacement_history":[
                {"type":"message","role":"user","content":[{"type":"input_text","text":"retained input"}]},
                {"type":"compaction","id":"private","encrypted_content":"synthetic-only"},
                {"type":"function_call","call_id":"tool-after-compaction","name":"read","arguments":"{}"},
                {"type":"function_call_output","call_id":"tool-after-compaction","output":"synthetic tool result"},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"reply after compaction"}]}
            ]
        }
    })];
    let output = codex(&records, None, false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let context: Value = serde_json::from_slice(&output.stdout).unwrap();
    let messages = context["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 5);
    assert_eq!(messages[0]["content"][0]["text"], "retained input");
    assert_eq!(
        messages[1]["content"][0]["text"],
        "Portable summary from Codex"
    );
    assert_eq!(messages[2]["content"][0]["id"], "tool-after-compaction");
    assert_eq!(messages[3]["content"][0]["id"], "tool-after-compaction");
    assert_eq!(messages[4]["content"][0]["text"], "reply after compaction");
    assert!(
        !output
            .stdout
            .windows(b"encrypted_content".len())
            .any(|part| part == b"encrypted_content")
    );
}

#[test]
fn generated_portable_summary_replaces_encrypted_boundary_before_tool_suffix() {
    let records = vec![json!({
        "type":"compacted",
        "payload":{
            "message":"",
            "replacement_history":[
                {"type":"message","role":"user","content":[{"type":"input_text","text":"retained prefix"}]},
                {"type":"compaction","id":"private","encrypted_content":"synthetic-only"},
                {"type":"function_call","call_id":"tail-tool","name":"read","arguments":"{}"},
                {"type":"function_call_output","call_id":"tail-tool","output":"tail result"},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"tail reply"}]}
            ]
        }
    })];
    let output = codex_with_summary(&records, "Generated portable summary");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let context: Value = serde_json::from_slice(&output.stdout).unwrap();
    let messages = context["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 5);
    assert_eq!(messages[0]["content"][0]["text"], "retained prefix");
    assert_eq!(
        messages[1]["content"][0]["text"],
        "Generated portable summary"
    );
    assert_eq!(messages[2]["content"][0]["id"], "tail-tool");
    assert_eq!(messages[3]["content"][0]["id"], "tail-tool");
    assert_eq!(messages[4]["content"][0]["text"], "tail reply");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-only"));
}

#[test]
fn codex_compaction_without_plaintext_summary_is_rejected() {
    let records = vec![json!({
        "type":"compacted",
        "payload":{
            "message":"",
            "replacement_history":[
                {"type":"message","role":"user","content":[{"type":"input_text","text":"retained input"}]},
                {"type":"compaction","id":"private","encrypted_content":"synthetic-only"}
            ]
        }
    })];
    let output = codex(&records, None, false);
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"errorCode":"portable_summary_required"})
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn codex_compaction_preflight_reports_only_boolean_and_respects_plaintext_summary() {
    let encrypted_only = vec![json!({
        "type":"compacted",
        "payload":{"message":"","replacement_history":[
            {"type":"compaction","encrypted_content":"synthetic-secret-marker"}
        ]}
    })];
    let output = codex_summary_required(&encrypted_only);
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"required":true})
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-secret-marker"));

    let portable = vec![json!({
        "type":"compacted",
        "payload":{"message":"portable summary","replacement_history":[
            {"type":"compaction","encrypted_content":"synthetic-secret-marker"}
        ]}
    })];
    let output = codex_summary_required(&portable);
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"required":false})
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("portable summary"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-secret-marker"));
}
#[test]
fn claude_omission_and_truncation_preserve_co_located_tool_results() {
    let script = concat!(
        include_str!("../resources/claude_context.py"),
        r#"
import json, tempfile, os
def local_command(content): return None
records = [
    {'type':'user','uuid':'first','parentUuid':None,'message':{'content':'duplicate'}},
    {'type':'assistant','uuid':'call','parentUuid':'first','message':{'content':[{'type':'tool_use','id':'tool','name':'read','input':{}}]}},
    {'type':'user','uuid':'selected','parentUuid':'call','message':{'content':[{'type':'tool_result','tool_use_id':'tool','content':'result'},{'type':'text','text':'duplicate'}]}},
    {'type':'assistant','uuid':'reply','parentUuid':'selected','message':{'content':'reply'}}
]
with tempfile.TemporaryDirectory(prefix='velune-context-delete-') as root:
    path = os.path.join(root, 'synthetic.jsonl')
    def save():
        with open(path, 'w') as f:
            for record in records: f.write(json.dumps(record)+'\n')
    save()
    omitted = export_context(path, without_message_id='selected')['messages']
    assert len(omitted) == 4 and omitted[2]['content'] == [{'type':'tool_result','id':'tool','content':[{'type':'text','text':'result'}],'is_error':False}]
    assert omitted[3]['content'][0]['text'] == 'reply'
    before = export_context(path, before_message_id='selected')['messages']
    assert len(before) == 3 and before[-1] == omitted[2]
    assert export_context(path, before_message_id='first')['messages'] == []
    assert export_context(path, without_message_id='first')['messages'][0]['role'] == 'assistant'
    records.append({'type':'system','subtype':'compact_boundary','uuid':'compact','parentUuid':'reply'})
    save()
    assert export_context(path, without_message_id='selected')['messages'] == omitted
    # Compaction severs the physical branch; the original record remains in
    # display history, but no longer belongs to effective native model input.
    records[-1]['parentUuid'] = None
    records.append({'type':'user','uuid':'summary','parentUuid':'compact','isCompactSummary':True,'message':{'content':'summary'}})
    save()
    assert export_context(path, without_message_id='selected')['messages'] == [{'role':'user','content':[{'type':'text','text':'summary'}]}]
"#
    );
    let output = command("python3").args(["-c", script]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn native_imports_roundtrip_assistant_first_context_without_a_new_request() {
    use velune_agent_runtime::{execution::ExecutionTarget, import::*};
    let root = std::env::temp_dir().join(format!(
        "velune-delete-import-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let context: Context = serde_json::from_value(json!({"messages":[
        {"role":"assistant","content":[{"type":"tool_call","id":"original","name":"read","arguments":{}}]},
        {"role":"user","content":[{"type":"tool_result","id":"original","content":[{"type":"text","text":"result"}],"is_error":false}]},
        {"role":"assistant","content":[{"type":"text","text":"reply"}]}
    ]})).unwrap();
    let execution = ExecutionTarget::Local;
    let home = root.join("claude");
    let ImportedConversation::ClaudeCode { session_id } =
        import_claude_conversation(&execution, &home, &root, &context, "synthetic").unwrap()
    else {
        panic!()
    };
    let project = home
        .join("projects")
        .read_dir()
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let path = project.join(format!("{session_id}.jsonl"));
    let script = concat!(
        include_str!("../resources/claude_context.py"),
        "\nimport json,sys\ndef local_command(content): return None\nprint(json.dumps(export_context(sys.argv[1])))\n"
    );
    let output = command("python3")
        .args(["-c", script])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let restored: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(restored["messages"].as_array().unwrap().len(), 3);
    assert_eq!(restored["messages"][0]["role"], "assistant");
    assert_eq!(
        restored["messages"][0]["content"][0]["id"],
        restored["messages"][1]["content"][0]["id"]
    );
    assert_eq!(restored["messages"][2]["content"][0]["text"], "reply");
    let home = root.join("codex");
    let ImportedConversation::Codex { thread_id } =
        import_codex_conversation(&execution, &home, &root, &context, "0.159.3", "openai").unwrap()
    else {
        panic!()
    };
    fn rollout(path: &std::path::Path) -> Option<std::path::PathBuf> {
        for entry in path.read_dir().unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if let Some(found) = rollout(&path) {
                    return Some(found);
                }
            } else if path.extension().is_some_and(|ext| ext == "jsonl") {
                return Some(path);
            }
        }
        None
    }
    let path = rollout(&home.join("sessions")).unwrap();
    let mut child = command("python3")
        .args(["-c", include_str!("../resources/codex_context.py")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&std::fs::read(&path).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let restored: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(restored["messages"].as_array().unwrap().len(), 3);
    assert_eq!(restored["messages"][0]["role"], "assistant");
    assert_eq!(
        restored["messages"][0]["content"][0]["id"],
        restored["messages"][1]["content"][0]["id"]
    );
    assert_eq!(restored["messages"][2]["content"][0]["text"], "reply");
    // Optional installed official CLI read, never turn/start or model access.
    if let Some(binary) = std::env::var_os("VELUNE_TEST_CODEX_BINARY") {
        let config = history::HistoryConfig {
            kind: RuntimeKind::Codex166,
            execution,
            os_home: Some(root.clone()),
            provider: "codex".into(),
            binary: binary.into(),
            node_binary: root.join("unused-node"),
            resources_directory: root.clone(),
            root: root.clone(),
            home,
        };
        let history = history::read(&config, "synthetic", &thread_id);
        if let Err(error) = history {
            panic!("{}", error.full_message());
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn imports_preserve_prompt_origins_for_repeated_and_multimodal_deletion() {
    use velune_agent_runtime::{execution::ExecutionTarget, import::*};
    let root = std::env::temp_dir().join(format!(
        "velune-delete-origins-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let context: Context = serde_json::from_value(json!({"messages":[
        {"source_message_id":"old-one","role":"user","content":[{"type":"text","text":"duplicate"},{"type":"image","media_type":"image/png","data":"AA=="}]},
        {"source_message_id":"old-two","role":"user","content":[{"type":"text","text":"duplicate"}]},
        {"role":"assistant","content":[{"type":"text","text":"reply"}]}
    ]})).unwrap();
    let execution = ExecutionTarget::Local;
    let home = root.join("claude");
    let (ImportedConversation::ClaudeCode { session_id }, origins) =
        import_claude_conversation_with_origins(&execution, &home, &root, &context, "synthetic")
            .unwrap()
    else {
        panic!()
    };
    assert_eq!(origins["old-one"].len(), 1);
    assert_ne!(origins["old-one"], origins["old-two"]);
    let project = home
        .join("projects")
        .read_dir()
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let path = project.join(format!("{session_id}.jsonl"));
    let ids: Vec<String> = origins.values().flatten().cloned().collect();
    let script = concat!(
        include_str!("../resources/claude_context.py"),
        "\nimport json,sys\ndef local_command(content): return None\nprint(json.dumps(export_context(sys.argv[1], without_message_ids=json.loads(sys.argv[2]))))\n"
    );
    let output = command("python3")
        .args(["-c", script])
        .arg(path)
        .arg(serde_json::to_string(&ids).unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let restored: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(restored["messages"].as_array().unwrap().len(), 1);
    assert_eq!(restored["messages"][0]["role"], "assistant");
    let home = root.join("codex");
    let (ImportedConversation::Codex { thread_id }, origins) =
        import_codex_conversation_with_origins(
            &execution, &home, &root, &context, "0.166.0", "openai",
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(origins["old-one"].len(), 2);
    assert_eq!(origins["old-two"].len(), 1);
    if let Some(binary) = std::env::var_os("VELUNE_TEST_CODEX_BINARY") {
        let config = history::HistoryConfig {
            kind: RuntimeKind::Codex166,
            execution,
            os_home: Some(root.clone()),
            provider: "codex".into(),
            binary: binary.into(),
            node_binary: root.join("unused-node"),
            resources_directory: root.clone(),
            root: root.clone(),
            home,
        };
        let ids: Vec<String> = origins.values().flatten().cloned().collect();
        history::materialize_codex_import(&config, &thread_id, "openai").unwrap();
        let history = history::read(&config, "synthetic", &thread_id).unwrap();
        for id in &ids {
            assert!(
                history.messages.iter().any(|message| &message.id == id),
                "import mapping lacks native display identity {id}; native={:?}",
                history
                    .messages
                    .iter()
                    .map(|message| (&message.id, &message.role))
                    .collect::<Vec<_>>()
            );
        }
        let exported = history::export_context(&config, &thread_id).unwrap();
        let exported_ids: Vec<&str> = exported
            .messages
            .iter()
            .filter_map(|message| message.source_message_id.as_deref())
            .collect();
        for id in &ids {
            assert!(exported_ids.contains(&id.as_str()));
        }
        let restored = history::export_context_without_many(&config, &thread_id, &ids).unwrap();
        assert_eq!(restored.messages.len(), 1);
        assert_eq!(restored.messages[0].role, Role::Assistant);
        let (ImportedConversation::Codex { thread_id: managed }, _) =
            import_codex_conversation_with_origins(
                &config.execution,
                &config.home,
                &root,
                &context,
                "0.166.0",
                "velune",
            )
            .unwrap()
        else {
            panic!()
        };
        history::materialize_codex_import(&config, &managed, "velune").unwrap();
        let managed_history = history::read(&config, "synthetic", &managed).unwrap();
        assert!(!managed_history.messages.is_empty());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn empty_claude_import_is_discoverable_and_fresh_cli_accepts_its_identity() {
    use velune_agent_runtime::{execution::ExecutionTarget, import::*};
    let root = std::env::temp_dir().join(format!(
        "velune-empty-import-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let home = root.join("claude");
    let context = Context { messages: vec![] };
    let ImportedConversation::ClaudeCode { session_id } =
        import_claude_conversation(&ExecutionTarget::Local, &home, &root, &context, "synthetic")
            .unwrap()
    else {
        panic!()
    };
    let config = history::HistoryConfig {
        kind: velune_agent_runtime::version::RuntimeKind::ClaudeCode,
        execution: ExecutionTarget::Local,
        os_home: Some(root.clone()),
        provider: "claude-code".into(),
        binary: root.join("unused-cli"),
        node_binary: root.join("unused-node"),
        resources_directory: root.clone(),
        root: home.clone(),
        home: home.clone(),
    };
    let history = velune_agent_runtime::history::read(&config, "synthetic", &session_id).unwrap();
    assert!(history.messages.is_empty());
    assert_eq!(history.cwd.as_deref(), root.to_str());
    if let Some(binary) = std::env::var_os("VELUNE_TEST_CLAUDE_BINARY") {
        let mut child = command(binary)
            .env_clear()
            .env("PATH", "/opt/homebrew/bin:/usr/bin:/bin")
            .env("HOME", &root)
            .env("CLAUDE_CONFIG_DIR", &home)
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
            .env("DISABLE_AUTOUPDATER", "1")
            .env("CLAUDE_CODE_ENTRYPOINT", "velune")
            .env("CLAUDE_CODE_SDK_READS_SESSION_STATE", "1")
            .current_dir(&root)
            .args([
                "--print",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--session-id",
                &session_id,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"{\"type\":\"control_request\",\"request_id\":\"empty-import-check\",\"request\":{\"subtype\":\"initialize\"}}\n").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .any(|message| message["type"] == "control_response"
                    && message["response"]["request_id"] == "empty-import-check"
                    && message["response"]["subtype"] == "success")
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_steer_duplicates_use_canonical_turn_ordinals_for_many_deletion() {
    let users = vec![
        json!({"id":"legacy-first","type":"userMessage","content":[{"type":"text","text":"duplicate"}]}),
        json!({"id":"legacy-steer-one","type":"userMessage","content":[{"type":"text","text":"duplicate"}]}),
        json!({"id":"legacy-steer-two","type":"userMessage","content":[{"type":"text","text":"duplicate"}]}),
    ];
    let records = [
        json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"turn"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"duplicate"}]}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"duplicate"}]}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"duplicate"}]}}),
    ];
    let rollout = records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let request = json!({"rollout":rollout,"legacyTurns":{"turn":users},"legacyBoundaries":{
        "legacy-steer-one":{"turnId":"turn","ordinal":1,"users":users},
        "legacy-steer-two":{"turnId":"turn","ordinal":2,"users":users}
    }});
    let mut child = command("python3")
        .args([
            "-c",
            include_str!("../resources/codex_context.py"),
            "--request-envelope",
            "--without-message-ids",
            "[\"legacy-steer-one\",\"legacy-steer-two\"]",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let context: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(context["messages"].as_array().unwrap().len(), 1);
    assert_eq!(context["messages"][0]["source_message_id"], "legacy-first");
}

#[test]
fn codex_separate_display_event_identity_is_used_for_omission_and_origins() {
    let records = vec![
        prompt("input-one"),
        json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","id":"display-one"}}}),
        prompt("input-two"),
        json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","id":"display-two"}}}),
    ];
    let output = codex(&records, Some("display-one"), true);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let context: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(context["messages"].as_array().unwrap().len(), 1);
    assert_eq!(context["messages"][0]["source_message_id"], "display-two");
}
