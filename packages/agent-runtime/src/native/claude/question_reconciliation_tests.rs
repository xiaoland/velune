use super::*;
use crate::native::question_reconciliation_tests::{Fixture, pending};

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
