//! Exercise actual native request lifetimes without a model, account or user data.
use super::*;
use std::os::unix::fs::PermissionsExt;

struct Fixture {
    root: PathBuf,
    session: NativeSession,
}
impl Fixture {
    fn new() -> Self {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-question-lifecycle-{:x}",
            u64::from_le_bytes(nonce)
        ));
        std::fs::create_dir(&root).unwrap();
        let binary = root.join("codex");
        std::fs::write(
            &binary,
            r#"#!/usr/bin/python3
import json,sys
if '--version' in sys.argv:
 print('codex-cli 0.166.0');sys.exit(0)
for line in sys.stdin:
 request=json.loads(line)
 if 'id' in request and 'method' in request:
  print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':{}}),flush=True)
"#,
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut session = NativeSession::connect(NativeConfig {
            execution: crate::execution::ExecutionTarget::Local,
            os_home: Some(root.clone()),
            kind: NativeKind::Codex166,
            binary,
            node_binary: None,
            agent_dir: root.clone(),
            resources_directory: root.clone(),
            projection_directory: root.clone(),
            gateway: None,
            auto_compaction: None,
            mcp_servers: vec![],
            session_agents: vec![],
        })
        .unwrap();
        session.replace_snapshot("parent", Some(&root), "runtime", vec![]);
        session.turn_id = Some("turn".into());
        session.running();
        Self { root, session }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.session.shutdown().unwrap();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
fn async_question(id: &str) -> Value {
    json!({"type":"agentMessage","id":id,"delivery":"async","text":"Synthetic question?","questions":[{"title":"Synthetic question?"}]})
}
fn pending(session: &NativeSession) -> Vec<RuntimeInteraction> {
    session
        .snapshot
        .as_ref()
        .unwrap()
        .pending_interactions
        .clone()
}
fn native_codex_question(session: &mut NativeSession, id: i32, thread: &str) -> String {
    session.codex_handle_record(json!({"id":id,"method":"item/tool/requestUserInput","params":{
        "threadId":thread,"isBlocking":false,"questions":[{"id":"choice","question":"Native question?","isOther":true}]
    }})).unwrap();
    pending(session).last().unwrap().id.clone()
}

#[test]
fn actual_codex_projection_reconciles_native_answers_and_keeps_unanswered_async_after_turn_end() {
    let mut f = Fixture::new();
    let session = &mut f.session;
    let question = async_question("ask");
    session.codex_item(&question, Some("turn"));
    assert_eq!(pending(session).len(), 1);
    session.codex_item(&json!({"type":"userMessage","id":"ordinary","content":[{"type":"text","text":"An unrelated later input"}]}),Some("later"));
    assert_eq!(pending(session).len(), 1);
    let payload = json!({"questionItemId":json!(["request_user_input_async","ask",0]).to_string(),"question":"Synthetic question?","answer":"Yes"});
    let text = format!(
        "<send_user_message_question_reply>\n{payload}\n</send_user_message_question_reply>"
    );
    session.codex_item(
        &json!({"type":"userMessage","id":"reply","content":[{"type":"text","text":text}]}),
        Some("later"),
    );
    assert!(pending(session).is_empty());
    session.codex_item(&question, Some("turn"));
    assert!(
        pending(session).is_empty(),
        "repeated completed item must not revive the card"
    );
    let steered = async_question("steered");
    session.codex_item(&steered, Some("turn"));
    let payload = json!({"questionItemId":json!(["request_user_input_async","steered",0]).to_string(),"question":"Synthetic question?","answer":"Yes"});
    let text = format!(
        "<send_user_message_question_reply>\n{payload}\n</send_user_message_question_reply>"
    );
    session.codex_item(&json!({"type":"steeringUserMessage","id":"accepted-steer","status":"accepted","input":[{"type":"text","text":text}]}), Some("turn"));
    assert!(pending(session).is_empty());
    session.codex_item(&steered, Some("turn"));
    assert!(
        pending(session).is_empty(),
        "accepted steering resolves without waiting for a user-message echo"
    );
    session.codex_item(&async_question("still-open"), Some("turn"));
    session.settle(RunState::Idle);
    assert_eq!(
        pending(session).len(),
        1,
        "unanswered ConversationInput outlives the turn"
    );
}

#[test]
fn codex_resolved_requests_are_settled_before_parent_child_and_old_turn_filters() {
    let mut f = Fixture::new();
    let session = &mut f.session;
    session.sub_agents.insert("runtime:child".into());
    let child = native_codex_question(session, 77, "child");
    let own = native_codex_question(session, 78, "parent");
    session.codex_item(&async_question("unrelated-async"), Some("turn"));
    session.codex_handle_record(json!({"method":"serverRequest/resolved","params":{"threadId":"child","requestId":999}})).unwrap();
    assert_eq!(pending(session).len(), 3);
    session
        .codex_handle_record(
            json!({"method":"serverRequest/resolved","params":{"threadId":"child","requestId":77}}),
        )
        .unwrap();
    assert!(
        !pending(session)
            .iter()
            .any(|interaction| interaction.id == child)
    );
    assert!(
        pending(session)
            .iter()
            .any(|interaction| interaction.id == own)
    );
    session.codex_handle_record(json!({"method":"serverRequest/resolved","params":{"threadId":"parent","turnId":"older-turn","requestId":78}})).unwrap();
    assert_eq!(pending(session).len(), 1);
    assert!(
        session
            .reply(&own, RuntimeInteractionReply::Cancel)
            .is_err()
    );
    native_codex_question(session, 79, "parent");
    session.fail_connection();
    assert!(!session.has_pending_requests());
    assert_eq!(
        pending(session).len(),
        1,
        "RPC loss clears NativeReply, not independent async output"
    );
    assert_eq!(pending(session)[0].id, "codex-async:unrelated-async");
}

fn claude_question(session: &mut NativeSession, id: &str) -> String {
    session.claude_handle_record(json!({"type":"control_request","request_id":id,
        "request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Native question?","options":[{"label":"Yes"}]}]}}})).unwrap();
    pending(session).last().unwrap().id.clone()
}
#[test]
fn claude_actual_control_handler_removes_answered_cancelled_and_disconnected_native_requests() {
    let mut f = Fixture::new();
    let session = &mut f.session;
    session.config.kind = NativeKind::ClaudeCode;
    let cancelled = claude_question(session, "cancel-me");
    let answered = claude_question(session, "answer-me");
    session
        .claude_handle_record(json!({"type":"control_cancel_request","request_id":"unknown"}))
        .unwrap();
    assert_eq!(pending(session).len(), 2);
    session
        .claude_handle_record(json!({"type":"control_cancel_request","request_id":"cancel-me"}))
        .unwrap();
    assert!(
        !pending(session)
            .iter()
            .any(|interaction| interaction.id == cancelled)
    );
    assert!(
        session
            .reply(&cancelled, RuntimeInteractionReply::Cancel)
            .is_err()
    );
    assert!(
        session
            .reply(
                &answered,
                RuntimeInteractionReply::Answers { answers: vec![] }
            )
            .is_err()
    );
    assert_eq!(
        pending(session).len(),
        1,
        "invalid answers keep their native request"
    );
    session
        .reply(
            &answered,
            RuntimeInteractionReply::Answers {
                answers: vec![InteractionAnswer {
                    question_id: "Native question?".into(),
                    values: vec!["Yes".into()],
                }],
            },
        )
        .unwrap();
    assert!(pending(session).is_empty());
    let disconnected = claude_question(session, "disconnect");
    session.fail_connection();
    assert!(pending(session).is_empty());
    assert!(
        session
            .reply(&disconnected, RuntimeInteractionReply::Cancel)
            .is_err()
    );
}

#[test]
fn deepseek_does_not_fabricate_question_forms_and_native_approval_loses_its_channel_on_disconnect()
{
    let mut f = Fixture::new();
    let session = &mut f.session;
    session.config.kind = NativeKind::DeepSeek;
    session.dsh_handle_record(json!({"jsonrpc":"2.0","id":31,"method":"elicitation/create","params":{"sessionId":"parent","mode":"form","requestedSchema":{"type":"object","properties":{"answer":{"type":"string"}}}}})).unwrap();
    assert!(
        pending(session).is_empty(),
        "unsupported capability is rejected, never projected as an answerable question"
    );
    session.dsh_handle_record(json!({"jsonrpc":"2.0","id":32,"method":"session/request_permission","params":{"sessionId":"parent","toolCall":{"title":"Synthetic tool"},"options":[{"optionId":"once","name":"Allow once","kind":"allow_once"}]}})).unwrap();
    let interaction = pending(session)[0].id.clone();
    assert!(matches!(
        pending(session)[0].kind,
        InteractionKind::Approval { .. }
    ));
    session.fail_connection();
    assert!(pending(session).is_empty());
    assert!(
        session
            .reply(&interaction, RuntimeInteractionReply::Cancel)
            .is_err()
    );
}
