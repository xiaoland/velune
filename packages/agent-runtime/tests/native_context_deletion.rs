//! Synthetic native records only: no CLI authentication or model requests.
use serde_json::{Value, json};
use std::{io::Write, path::PathBuf, process::Stdio};
use velune_agent_runtime::{history, process::command, version::RuntimeKind};
use velune_test_support::TempRoot;

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
fn codex_plain_delivery_metadata_does_not_require_portable_summary() {
    let records = [
        prompt("first"),
        json!({"type":"inter_agent_communication_metadata","payload":{"trigger_turn":false}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Native communication body"}]}}),
    ];
    let probe = codex_summary_required(&records);
    assert!(
        probe.status.success(),
        "{}",
        String::from_utf8_lossy(&probe.stderr)
    );
    assert_eq!(probe.stdout, b"{\"required\": false}\n");
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
    let missing = codex(&records, Some("missing"), true);
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr)
            .contains("Codex native user message boundary was not found")
    );
    assert!(missing.stdout.is_empty());
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
    assert_assistant_first_import_roundtrip(None);
}

#[test]
#[ignore = "requires VELUNE_TEST_CODEX_BINARY pointing to an installed Codex CLI"]
fn installed_codex_reads_assistant_first_import_without_a_new_request() {
    let binary = std::env::var_os("VELUNE_TEST_CODEX_BINARY")
        .expect("enabled native test requires VELUNE_TEST_CODEX_BINARY");
    assert_assistant_first_import_roundtrip(Some(binary.into()));
}

fn assert_assistant_first_import_roundtrip(binary: Option<PathBuf>) {
    use velune_agent_runtime::{execution::ExecutionTarget, import::*};
    let root = TempRoot::new("native-context-import");
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
    let (ImportedConversation::Codex { thread_id }, _, _) =
        import_codex_conversation_owned(&execution, &home, &root, &context, "0.159.3", "openai")
            .unwrap()
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
    if let Some(binary) = binary {
        let config = history::HistoryConfig {
            kind: RuntimeKind::Codex166,
            execution,
            os_home: Some(root.to_path_buf()),
            binary,
            node_binary: root.join("unused-node"),
            resources_directory: root.to_path_buf(),
            root: root.to_path_buf(),
            home,
        };
        let history = history::read(&config, "synthetic", &thread_id);
        if let Err(error) = history {
            panic!("{}", error.full_message());
        }
    }
}

#[test]
fn imports_preserve_prompt_origins_for_repeated_and_multimodal_deletion() {
    assert_imported_prompt_origins(None);
}

#[test]
#[ignore = "requires VELUNE_TEST_CODEX_BINARY pointing to an installed Codex CLI"]
fn installed_codex_preserves_imported_display_ids_and_many_deletion() {
    let binary = std::env::var_os("VELUNE_TEST_CODEX_BINARY")
        .expect("enabled native test requires VELUNE_TEST_CODEX_BINARY");
    assert_imported_prompt_origins(Some(binary.into()));
}

fn assert_imported_prompt_origins(binary: Option<PathBuf>) {
    use velune_agent_runtime::{execution::ExecutionTarget, import::*};
    let root = TempRoot::new("native-context-import");
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
    let (ImportedConversation::Codex { thread_id }, origins, _) =
        import_codex_conversation_owned(&execution, &home, &root, &context, "0.166.0", "openai")
            .unwrap()
    else {
        panic!()
    };
    assert_eq!(origins["old-one"].len(), 2);
    assert_eq!(origins["old-two"].len(), 1);
    if let Some(binary) = binary {
        let config = history::HistoryConfig {
            kind: RuntimeKind::Codex166,
            execution,
            os_home: Some(root.to_path_buf()),
            binary,
            node_binary: root.join("unused-node"),
            resources_directory: root.to_path_buf(),
            root: root.to_path_buf(),
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
        let (ImportedConversation::Codex { thread_id: managed }, _, _) =
            import_codex_conversation_owned(
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
}

#[test]
fn empty_claude_import_is_discoverable() {
    assert_empty_claude_import(None);
}

#[test]
#[ignore = "requires VELUNE_TEST_CLAUDE_BINARY pointing to an installed Claude Code CLI"]
fn installed_claude_accepts_empty_import_identity() {
    let binary = std::env::var_os("VELUNE_TEST_CLAUDE_BINARY")
        .expect("enabled native test requires VELUNE_TEST_CLAUDE_BINARY");
    if std::env::var_os("VELUNE_TEST_CLAUDE_ISOLATED_CHILD").is_some() {
        assert_empty_claude_import(Some(binary.into()));
        return;
    }
    // NativeSession inherits its host environment. Give the production owner
    // a clean test process, rather than reproducing its CLI command here.
    let home = TempRoot::new("claude-native-owner");
    let output = command(std::env::current_exe().unwrap())
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home.as_os_str())
        .env("CLAUDE_CONFIG_DIR", home.join("claude"))
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .env("DISABLE_AUTOUPDATER", "1")
        .env("VELUNE_TEST_CLAUDE_BINARY", binary)
        .env("VELUNE_TEST_CLAUDE_ISOLATED_CHILD", "1")
        .args([
            "--ignored",
            "--exact",
            "installed_claude_accepts_empty_import_identity",
            "--nocapture",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_empty_claude_import(binary: Option<PathBuf>) {
    use velune_agent_runtime::{execution::ExecutionTarget, import::*};
    let root = TempRoot::new("native-context-import");
    let home = root.join("claude");
    let context = Context { messages: vec![] };
    let ImportedConversation::ClaudeCode { session_id } =
        import_claude_conversation(&ExecutionTarget::Local, &home, &root, &context, "synthetic")
            .unwrap()
    else {
        panic!()
    };
    let history_config = history::HistoryConfig {
        kind: velune_agent_runtime::version::RuntimeKind::ClaudeCode,
        execution: ExecutionTarget::Local,
        os_home: Some(root.to_path_buf()),
        binary: root.join("unused-cli"),
        node_binary: root.join("unused-node"),
        resources_directory: root.to_path_buf(),
        root: home.clone(),
        home: home.clone(),
    };
    let history =
        velune_agent_runtime::history::read(&history_config, "synthetic", &session_id).unwrap();
    assert!(history.messages.is_empty());
    assert_eq!(history.cwd.as_deref(), root.to_str());
    assert!(
        history::export_context(&history_config, &session_id)
            .unwrap()
            .messages
            .is_empty()
    );
    if let Some(binary) = binary {
        use velune_agent_runtime::native::{NativeConfig, NativeKind, NativeSession};
        let config = NativeConfig {
            execution: ExecutionTarget::Local,
            os_home: Some(root.to_path_buf()),
            kind: NativeKind::ClaudeCode,
            binary,
            node_binary: None,
            agent_dir: home.clone(),
            resources_directory: root.to_path_buf(),
            projection_directory: root.join("projection"),
            gateway: None,
            auto_compaction: None,
            mcp_servers: Vec::new(),
            session_agents: Vec::new(),
        };
        let assert_reopens = |session_id: &str| {
            // Both launches execute the same open → launch → initialize path as
            // the application, including an open after native metadata exists.
            for _ in 0..2 {
                let mut session = NativeSession::connect(config.clone()).unwrap();
                session
                    .open(
                        session_id,
                        Some(&root),
                        "synthetic",
                        history::read(&history_config, "synthetic", session_id)
                            .unwrap()
                            .map(|history| history.messages),
                    )
                    .unwrap();
                session.prepare_fork_turn().unwrap();
                assert!(session.snapshot().unwrap().messages.is_empty());
                session.shutdown().unwrap();
            }
        };
        assert_reopens(&session_id);
        let (ImportedConversation::ClaudeCode { session_id: legacy }, _, ownership) =
            import_claude_conversation_owned(
                &ExecutionTarget::Local,
                &home,
                &root,
                &context,
                "synthetic",
            )
            .unwrap()
        else {
            panic!()
        };
        let original = format!(
            "{}\n",
            json!({"type":"queue-operation","operation":"dequeue",
                "sessionId":legacy,"cwd":root.to_path_buf(),"timestamp":"2026-11-28T00:00:00.000Z",
            "veluneEmptyImport":true})
        );
        std::fs::write(ownership.source_path(), &original).unwrap();
        assert_reopens(&legacy);
        assert!(
            std::fs::read(ownership.source_path())
                .unwrap()
                .starts_with(original.as_bytes())
        );
        assert_eq!(
            std::fs::read(
                ownership
                    .source_path()
                    .with_extension("jsonl.velune-empty-import.bak")
            )
            .unwrap(),
            original.as_bytes()
        );
    }
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
