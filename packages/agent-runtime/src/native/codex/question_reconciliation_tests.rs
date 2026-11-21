use super::*;
use crate::native::question_reconciliation_tests::{Fixture, pending};

fn async_question(id: &str) -> Value {
    json!({"type":"agentMessage","id":id,"delivery":"async","text":"Synthetic question?","questions":[{"title":"Synthetic question?"}]})
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
