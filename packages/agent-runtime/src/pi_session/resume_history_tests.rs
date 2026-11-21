use super::*;
use crate::conversation::{MessageBlock, MessageRole};

fn projection() -> PiProjection {
    PiProjection::new(ConversationSummary {
        native_read_state: None,
        id: "pi:/synthetic/session.jsonl".into(),
        runtime_id: "pi".into(),
        title: crate::conversation::conversation_title(None, None),
        cwd: None,
        created_at_unix_ms: None,
        updated_at_unix_ms: None,
        can_rename: false,
        can_delete: false,
        parent_conversation_id: None,
    })
}

#[test]
fn resumed_turn_keeps_history_until_native_projection_replaces_it() {
    let mut cold = projection();
    cold.replace_history(&json!({"messages":[
        {"id":"pi:session:original:0","role":"user","content":"earlier prompt"},
        {"id":"pi-unconfirmed-0","role":"assistant","content":"earlier reply","stopReason":"stop"},
        {"id":"pi:session:compaction:0","role":"compactionSummary","summary":"earlier context"}
    ]}));
    let history = cold.snapshot.unwrap();

    for (aborted, stop_reason) in [(false, "stop"), (true, "aborted")] {
        let mut resumed = PiProjection::from_history(&history);
        assert!(!resumed.history_synchronized());
        assert_eq!(
            resumed.snapshot.as_ref().unwrap().messages,
            history.messages
        );

        resumed.begin_turn();
        resumed.apply_event(&json!({"type":"agent_start"}));
        for event_type in ["message_start", "message_end"] {
            resumed.apply_event(
                &json!({"type":event_type,"message":{"role":"user","content":"new prompt"}}),
            );
        }
        resumed.apply_event(
            &json!({"type":"message_start","message":{"role":"assistant","content":[]}}),
        );
        resumed.apply_event(&json!({"type":"message_update","assistantMessageEvent":{"type":"text_start","contentIndex":0}}));
        resumed.apply_event(&json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"new reply"}}));

        let running = resumed.snapshot.as_ref().unwrap();
        assert_eq!(running.run_state, RunState::Running);
        assert!(running.messages.starts_with(&history.messages));
        let [prompt, reply] = &running.messages[history.messages.len()..] else {
            panic!("expected exactly one new prompt and one streaming reply");
        };
        assert_eq!(prompt.role, MessageRole::User);
        assert_eq!(reply.role, MessageRole::Assistant);
        assert_ne!(prompt.id, reply.id);
        assert_eq!(
            prompt.blocks,
            [MessageBlock::Text {
                text: "new prompt".into()
            }]
        );
        assert_eq!(
            reply.blocks,
            [MessageBlock::Text {
                text: "new reply".into()
            }]
        );
        assert!(!reply.completed);

        resumed.apply_event(&json!({"type":"message_end","message":{"role":"assistant","content":"new reply","stopReason":stop_reason}}));
        resumed.apply_event(&json!({"type":"agent_settled","aborted":aborted}));
        let settled = resumed.snapshot.as_ref().unwrap();
        assert_eq!(settled.run_state, RunState::Idle);
        assert!(settled.messages.starts_with(&history.messages));
        assert_eq!(settled.messages.len(), history.messages.len() + 2);
        assert!(settled.messages.last().unwrap().completed);

        resumed.replace_history(&json!({"messages":[
            {"id":"pi:session:original:0","role":"user","content":"earlier prompt"},
            {"id":"pi:session:branch:0","role":"assistant","content":"current native branch","stopReason":"stop"}
        ]}));
        assert!(resumed.history_synchronized());
        let current = resumed.snapshot.as_ref().unwrap();
        assert_eq!(current.messages.len(), 2);
        assert_eq!(current.messages[0].id, "pi:session:original:0");
        assert_eq!(current.messages[1].id, "pi:session:branch:0");
    }
}

#[test]
fn resumed_startup_diagnostic_remains_after_native_rewind() {
    let mut source = projection();
    let retained = json!({"id":"retained","role":"user","content":"earlier prompt"});
    let discarded = json!({"id":"discarded","role":"user","content":"prompt being replaced"});
    source.replace_history(&json!({"messages":[retained,discarded]}));
    let mut resumed = PiProjection::from_history(&source.snapshot.unwrap());
    resumed.apply_event(&json!({
        "type":"extension_error",
        "event":"session_start",
        "error":"synthetic startup warning"
    }));
    let diagnostic_id = resumed.snapshot.as_ref().unwrap().messages[0].id.clone();
    assert!(diagnostic_id.starts_with("pi-diagnostic-"));
    resumed.replace_history(&json!({"messages":[retained,discarded]}));
    resumed.replace_history(&json!({"messages":[retained]}));
    let messages = &resumed.snapshot.as_ref().unwrap().messages;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].id, diagnostic_id);
    assert_eq!(messages[1].id, "retained");
}
