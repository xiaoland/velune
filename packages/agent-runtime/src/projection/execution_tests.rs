use super::*;
use serde_json::json;
#[test]
fn pi_terminal_evidence_survives_native_confirmation_and_namespace() {
    for (stop, expected) in [
        ("stop", Some(TurnOutcome::Completed)),
        ("error", Some(TurnOutcome::Failed)),
        ("aborted", Some(TurnOutcome::Cancelled)),
        ("toolUse", None),
    ] {
        let mut p = super::tests::projection();
        p.begin_turn();
        p.apply_event(&json!({"type":"agent_start"}));
        for message in [
            json!({"id":"local-user","role":"user","content":"question"}),
            json!({"id":"reply","role":"assistant","stopReason":stop,"content":"answer"}),
        ] {
            p.apply_event(&json!({"type":"message_start","message":message}));
            p.apply_event(&json!({"type":"message_end","message":message}));
        }
        p.apply_event(&json!({"type":"agent_settled"}));
        p.replace_history(&json!({"messages":[{"id":"native-user","role":"user","content":"question"},{"id":"reply","role":"assistant","stopReason":stop,"content":"answer"}], "identityConfirmations":[{"previousId":"local-user","currentId":"native-user"}]}));
        let snapshot = p.snapshot.as_mut().unwrap();
        snapshot.executions.map_ids(&|id| format!("segment-2:{id}"));
        let record = snapshot.executions.record("segment-2:native-user").unwrap();
        assert_eq!(record.outcome, expected, "{stop}");
    }
}
