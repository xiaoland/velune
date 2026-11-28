//! Narrow regressions for the production Codex rollout translation helper.
use serde_json::{Value, json};
use std::io::Write;
use std::process::{Output, Stdio};
use velune_agent_runtime::process::command;

fn run(records: &[Value]) -> Output {
    run_input(&rollout(records), &[])
}

fn rollout(records: &[Value]) -> String {
    records.iter().map(|record| format!("{record}\n")).collect()
}

fn run_input(input: &str, args: &[&str]) -> Output {
    let mut child = command("python3")
        .arg("-c")
        .arg(include_str!("../resources/codex_context.py"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python3 must be available for Codex context export");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn export(records: &[Value]) -> Value {
    let output = run(records);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn rejected(records: &[Value], reason: &str) {
    let output = run(records);
    assert!(!output.status.success(), "unexpected successful export");
    assert!(output.stdout.is_empty(), "must not return partial context");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(reason),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn record(kind: &str, payload: Value) -> Value {
    json!({"type": kind, "payload": payload})
}

fn item(payload: Value) -> Value {
    record("response_item", payload)
}

fn metadata() -> Value {
    record(
        "inter_agent_communication_metadata",
        json!({"trigger_turn": true}),
    )
}

fn effort_update() -> Value {
    json!({"type":"configuration_update", "reasoning":{"effort":"high"}})
}

fn agent(content: Value) -> Value {
    json!({"type":"agent_message", "author":"worker-alpha", "recipient":"parent",
           "content":content})
}

#[test]
fn delivery_metadata_preserves_function_and_custom_tool_order() {
    let context = export(&[
        metadata(),
        item(
            json!({"type":"function_call", "call_id":"f", "name":"inspect", "arguments":"{\"path\":\"src\"}"}),
        ),
        item(effort_update()),
        metadata(),
        item(json!({"type":"function_call_output", "call_id":"f", "output":"found"})),
        item(json!({"type":"custom_tool_call", "call_id":"c", "name":"apply", "input":"patch"})),
        metadata(),
        item(
            json!({"type":"custom_tool_call_output", "call_id":"c", "output":{"content":[{"type":"text", "text":"done"}]}}),
        ),
        metadata(),
    ]);
    assert_eq!(
        context,
        json!({"messages":[
            {"role":"assistant", "content":[{"type":"tool_call", "id":"f", "name":"inspect", "arguments":{"path":"src"}}]},
            {"role":"user", "content":[{"type":"tool_result", "id":"f", "content":[{"type":"text", "text":"found"}], "is_error":false}]},
            {"role":"assistant", "content":[{"type":"tool_call", "id":"c", "name":"apply", "arguments":{"input":"patch"}}]},
            {"role":"user", "content":[{"type":"tool_result", "id":"c", "content":[{"type":"text", "text":"done"}], "is_error":false}]}
        ]})
    );
}

#[test]
fn plaintext_agent_body_keeps_attribution_as_assistant_context() {
    let context = export(&[
        item(json!({"type":"message", "role":"user", "content":"human request"})),
        metadata(),
        item(agent(
            json!([{"type":"input_text", "text":"first\n正文"}, {"type":"input_text", "text":"second"}]),
        )),
        item(json!({"type":"message", "role":"assistant", "content":"parent reply"})),
    ]);
    assert_eq!(
        context,
        json!({"messages":[
            {"role":"user", "content":[{"type":"text", "text":"human request"}]},
            {"role":"assistant", "content":[
                {"type":"text", "text":"Agent message from worker-alpha to parent:\n"},
                {"type":"text", "text":"first\n正文"}, {"type":"text", "text":"second"}]},
            {"role":"assistant", "content":[{"type":"text", "text":"parent reply"}]}
        ]})
    );
}

#[test]
fn newest_compaction_replacement_keeps_agent_context_and_suffix() {
    let context = export(&[
        item(json!({"type":"unknown_old_item"})),
        record(
            "compacted",
            json!({"replacement_history":[
                {"type":"message", "role":"assistant", "content":"retained summary"},
                effort_update(),
                agent(json!([{"type":"input_text", "text":"retained agent body"}]))
            ]}),
        ),
        metadata(),
        item(agent(
            json!([{"type":"input_text", "text":"new agent body"}]),
        )),
    ]);
    assert_eq!(
        context,
        json!({"messages":[
            {"role":"assistant", "content":[{"type":"text", "text":"retained summary"}]},
            {"role":"assistant", "content":[{"type":"text", "text":"Agent message from worker-alpha to parent:\n"}, {"type":"text", "text":"retained agent body"}]},
            {"role":"assistant", "content":[{"type":"text", "text":"Agent message from worker-alpha to parent:\n"}, {"type":"text", "text":"new agent body"}]}
        ]})
    );
}

#[test]
fn metadata_does_not_allow_unknown_rollout_or_response_items() {
    for kind in ["future_context", "inter_agent_communication"] {
        rejected(
            &[
                metadata(),
                record(kind, json!({"content":"must not disappear"})),
            ],
            &format!("unsupported Codex rollout record: {kind}"),
        );
    }
    rejected(
        &[metadata(), item(json!({"type":"future_item"}))],
        "unsupported Codex context item: future_item",
    );
}

#[test]
fn unknown_configuration_update_shapes_are_rejected() {
    for update in [
        json!({"type":"configuration_update"}),
        json!({"type":"configuration_update", "reasoning":{"effort":42}}),
        json!({"type":"configuration_update", "reasoning":{"effort":"high","future":true}}),
        json!({"type":"configuration_update", "reasoning":{"effort":"high"},"future":true}),
    ] {
        rejected(&[item(update)], "unsupported Codex configuration update");
    }
}

#[test]
fn encrypted_agent_payload_requires_a_source_summary() {
    let records = [
        item(effort_update()),
        item(agent(json!([
            {"type":"input_text", "text":"Routing header only"},
            {"type":"encrypted_content", "encrypted_content":"opaque fixture"}
        ]))),
        item(json!({"type":"message", "role":"assistant", "content":"later plain reply"})),
    ];
    let serialized = rollout(&records);
    let probe = run_input(&serialized, &["--requires-portable-summary"]);
    assert!(probe.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&probe.stdout).unwrap(),
        json!({"required":true})
    );
    assert_eq!(
        export(&records),
        json!({"errorCode":"portable_summary_required"})
    );
    let envelope =
        json!({"rollout":serialized, "portableSummary":"Judged source summary"}).to_string();
    let prepared = run_input(&envelope, &["--request-envelope"]);
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&prepared.stdout).unwrap(),
        json!({"messages":[{"role":"assistant","content":[{"type":"text","text":"Judged source summary"}]}]})
    );
    let mut with_unknown = records.to_vec();
    with_unknown.push(item(json!({"type":"future_item"})));
    let envelope =
        json!({"rollout":rollout(&with_unknown), "portableSummary":"Judged source summary"})
            .to_string();
    let rejected = run_input(&envelope, &["--request-envelope"]);
    assert!(!rejected.status.success() && rejected.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("unsupported Codex context item: future_item")
    );
}

#[test]
fn opaque_and_unknown_agent_content_are_rejected_even_in_replacements() {
    for content in [
        json!([{"type":"encrypted_text", "text":"opaque"}]),
        json!([{"type":"future_content", "text":"unknown"}]),
        json!([{"type":"input_image", "image_url":"data:image/png;base64,AA=="}]),
        json!([{"type":"input_text", "text":42}]),
        json!([{"type":"input_text", "text":"visible prefix"}, {"type":"encrypted_text", "text":"opaque suffix"}]),
    ] {
        rejected(
            &[metadata(), item(agent(content.clone()))],
            "unsupported Codex agent message content",
        );
        rejected(
            &[record(
                "compacted",
                json!({"replacement_history":[agent(content)]}),
            )],
            "unsupported Codex agent message content",
        );
    }
}

#[test]
fn agent_attribution_cannot_be_missing_or_empty() {
    for field in ["author", "recipient"] {
        for invalid in [json!(null), json!(""), json!(7)] {
            let mut message = agent(json!([{"type":"input_text", "text":"body"}]));
            message[field] = invalid;
            rejected(
                &[item(message)],
                "Codex agent message lacks author or recipient",
            );
        }
    }
}
