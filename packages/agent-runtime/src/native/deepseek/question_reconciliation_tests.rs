use super::*;
use crate::native::question_reconciliation_tests::{Fixture, pending};

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
