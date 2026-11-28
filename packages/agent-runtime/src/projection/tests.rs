use super::*;
use crate::conversation::TranscriptItem;
use serde_json::json;

pub(super) fn projection() -> PiProjection {
    PiProjection::new(ConversationSummary {
        native_read_state: None,
        id: "pi:synthetic".into(),
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
fn pi_subagent_descriptors_survive_cold_and_live_history_without_changing_session_names() {
    let mut p = projection();
    let original_title = p.snapshot.as_ref().unwrap().conversation.title.clone();
    p.replace_history(&json!({"messages":[],"subAgents":[
        {"sessionFile":"/synthetic/child.jsonl","title":"worker：检查缓存归属","failed":false},
        {"sessionFile":"relative.jsonl","title":"untrusted"},
        {"sessionFile":"/synthetic/empty.jsonl","title":" "}
    ]}));
    let view = p.snapshot.as_ref().unwrap();
    assert_eq!(view.sub_agents.len(), 1);
    assert_eq!(
        view.sub_agents[0].conversation_id,
        "pi:/synthetic/child.jsonl"
    );
    assert_eq!(view.conversation.title, original_title);
    p.replace_history(&json!({"data":{"messages":[]}}));
    assert_eq!(
        p.snapshot.as_ref().unwrap().sub_agents[0].title,
        "worker：检查缓存归属"
    );
    let revision = p.snapshot.as_ref().unwrap().revision;
    let terminal = json!([{"sessionFile":"/synthetic/child.jsonl","title":"reviewer：复核缓存","failed":true}]);
    p.update_sub_agents(&terminal, true);
    assert!(p.snapshot.as_ref().unwrap().revision > revision);
    p.update_sub_agents(
        &json!([{"sessionFile":"/synthetic/child.jsonl","title":"child fallback"}]),
        false,
    );
    let view = p.snapshot.as_ref().unwrap();
    assert_eq!(view.sub_agents[0].title, "reviewer：复核缓存");
    assert!(view.sub_agents[0].failed);
    let revision = view.revision;
    p.update_sub_agents(&terminal, true);
    assert_eq!(p.snapshot.as_ref().unwrap().revision, revision);
    p.replace_history(&json!({"messages":[],"subAgents":[]}));
    assert_eq!(p.snapshot.as_ref().unwrap().sub_agents.len(), 1);
    let mut other = p.snapshot.as_ref().unwrap().conversation.clone();
    other.id = "pi:/synthetic/other-parent.jsonl".into();
    p.set_conversation(other);
    assert!(p.snapshot.as_ref().unwrap().sub_agents.is_empty());
}

fn message(projection: &mut PiProjection, value: Value) {
    projection.apply_event(&json!({"type":"message_start","message":value}));
    projection.apply_event(&json!({"type":"message_end","message":value}));
}

#[test]
fn native_model_identity_survives_history_and_transcript_without_claiming_gateway_routes() {
    use crate::conversation::{ModelSelection, TurnSettings};
    use crate::transcript::{TranscriptEvent, TranscriptProjection};
    for provider in [
        Some("synthetic"),
        Some("velune"),
        Some("velune-gateway"),
        Some(""),
        None,
    ] {
        let mut p = projection();
        p.replace_history(&json!({"messages":[
                {"id":"user","role":"user","content":"synthetic prompt"},
                {"id":"reply","role":"assistant","provider":provider,"model":"alpha","content":"first reply","stopReason":"toolUse"},
                {"id":"reply2","role":"assistant","provider":"other","model":"beta","content":"later reply","stopReason":"stop"}
            ]}));
        let snapshot = p.snapshot.as_ref().unwrap();
        let expected = (provider == Some("synthetic")).then(|| ModelSelection::Native {
            provider_id: Some("synthetic".into()),
            model_id: "alpha".into(),
        });
        let settings = snapshot.messages[0].turn_settings.as_ref().unwrap();
        assert_eq!(settings.model.as_deref(), Some("alpha"));
        assert_eq!(settings.model_selection, expected);
        let mut transcript = TranscriptProjection::default();
        transcript.apply(TranscriptEvent::Replace {
            messages: &snapshot.messages,
            executions: &snapshot.executions,
        });
        assert_eq!(
            transcript.turns()[0]
                .settings
                .as_ref()
                .unwrap()
                .model_selection,
            expected
        );
        let managed = ModelSelection::Managed {
            model_record_key: "synthetic-managed".into(),
        };
        transcript.recall_request(
            "user",
            TurnSettings {
                model_selection: Some(managed.clone()),
                ..Default::default()
            },
        );
        assert_eq!(
            transcript.turns()[0]
                .settings
                .as_ref()
                .unwrap()
                .model_selection,
            Some(managed)
        );
    }
}

#[test]
fn extension_notifications_are_visible_client_events_without_a_user_turn() {
    let mut p = projection();
    message(
        &mut p,
        json!({"role":"custom","customType":"background-task","content":"Task completed","display":true,"timestamp":1234}),
    );
    let snapshot = p.snapshot.as_ref().unwrap();
    assert_eq!(snapshot.messages.last().unwrap().role, MessageRole::Client);
    assert_eq!(
        snapshot.messages.last().unwrap().timestamp_unix_ms,
        Some(1234)
    );
    assert!(
        !snapshot
            .messages
            .iter()
            .any(|message| message.role == MessageRole::User)
    );
    assert!(
        message_from_pi(
            &json!({"role":"custom","content":"hidden context","display":false}),
            "hidden".into()
        )
        .is_none()
    );
}

#[test]
fn background_notification_keeps_live_and_resynchronized_position() {
    let mut p = projection();
    let before = json!({
        "id":"before",
        "role":"assistant",
        "content":[{"type":"text","text":"synthetic"}]
    });
    let notice = json!({
        "id":"notice",
        "role":"custom",
        "customType":"background-task-notification",
        "content":[{"type":"text","text":"synthetic"}],
        "display":true,
        "timestamp":1234
    });
    let reply = json!({
        "id":"reply",
        "role":"assistant",
        "stopReason":"stop",
        "content":[{"type":"text","text":"synthetic"}]
    });

    p.replace_history(&json!({"messages":[before.clone()]}));
    message(&mut p, notice.clone());
    message(&mut p, reply.clone());
    let expected = ["before", "notice", "reply"];
    let snapshot = p.snapshot.as_ref().unwrap();
    assert_eq!(
        snapshot
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        expected
    );

    // Pi's authoritative context contains the same tree order. Replacing
    // the live projection must not move the notification behind the reply.
    p.replace_history(&json!({"messages":[before, notice, reply]}));
    let snapshot = p.snapshot.as_ref().unwrap();
    assert_eq!(
        snapshot
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        expected
    );

    let mut transcript = crate::transcript::TranscriptProjection::default();
    transcript.apply(crate::transcript::TranscriptEvent::Replace {
        messages: &snapshot.messages,
        executions: &snapshot.executions,
    });
    let transcript_ids = transcript
        .items()
        .into_iter()
        .filter_map(|item| match item {
            crate::conversation::TranscriptItem::Message { message_id, .. } => Some(message_id),
            crate::conversation::TranscriptItem::Work { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(transcript_ids, expected);
}

#[test]
fn compacted_snapshot_replaces_previous_notification_and_keeps_native_order() {
    let mut p = projection();
    message(&mut p, notice("notice-old"));
    assert!(
        p.snapshot
            .as_ref()
            .unwrap()
            .messages
            .iter()
            .any(|m| m.id == "notice-old")
    );
    // This is the sanitized effective-message shape produced by Pi 1.4's
    // pure buildSessionProjection for a branch with a prior notification
    // before firstKeptEntryId, then a retained notification followed by a
    // later reply. The old notification is absent from the SDK context;
    // the retained one stays between its neighboring entries.
    p.replace_history(&json!({"messages":[
            {"id":"compaction","role":"compactionSummary","content":[{"type":"text","text":"synthetic"}]},
            {"id":"kept","role":"assistant","content":[{"type":"text","text":"synthetic"}]},
            {"id":"notice-new","role":"custom","customType":"background-task-notification","content":[{"type":"text","text":"synthetic"}],"display":true,"timestamp":2345},
            {"id":"after","role":"assistant","content":[{"type":"text","text":"synthetic"}]}
        ]}));

    let snapshot = p.snapshot.as_ref().unwrap();
    let ids = snapshot
        .messages
        .iter()
        .map(|message| message.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["compaction", "kept", "notice-new", "after"]);

    let mut transcript = crate::transcript::TranscriptProjection::default();
    transcript.apply(crate::transcript::TranscriptEvent::Replace {
        messages: &snapshot.messages,
        executions: &snapshot.executions,
    });
    let transcript_ids = transcript
        .items()
        .into_iter()
        .filter_map(|item| match item {
            crate::conversation::TranscriptItem::Message { message_id, .. } => Some(message_id),
            crate::conversation::TranscriptItem::Work { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(transcript_ids, ids);
}

#[test]
fn compaction_disconnect_keeps_the_previous_reply_visible() {
    for started in [false, true] {
        let mut p = projection();
        p.replace_history(&json!({"messages":[
            {"id":"user","role":"user","content":[{"type":"text","text":"hello"}]},
            {"id":"answer","role":"assistant","content":[{"type":"text","text":"done"}]}
        ]}));
        p.begin_compaction("compact-1".into());
        if started {
            p.apply_event(&json!({"type":"compaction_start","reason":"manual"}));
        }
        p.apply_event(&json!({"type":"velune_closed","error":"synthetic EOF"}));
        let snapshot = p.snapshot.as_ref().unwrap();
        assert_eq!(snapshot.run_state, RunState::Failed);
        assert!(snapshot.messages.iter().all(|message| message.completed));
        assert!(
            matches!(&snapshot.messages.last().unwrap().blocks[0], MessageBlock::ContextCompaction { text } if text.contains("synthetic EOF"))
        );
        let mut transcript = crate::transcript::TranscriptProjection::default();
        transcript.apply(crate::transcript::TranscriptEvent::Replace {
            messages: &snapshot.messages,
            executions: &snapshot.executions,
        });
        assert_eq!(
            transcript.turns()[0].last_message_id.as_deref(),
            Some("answer")
        );
    }
}

#[test]
fn manual_compaction_is_pollable_survives_refresh_and_cancels() {
    let mut p = projection();
    p.begin_compaction("compact-1".into());
    p.apply_event(&json!({"type":"compaction_start","reason":"manual"}));
    let id = p
        .snapshot
        .as_ref()
        .unwrap()
        .messages
        .last()
        .unwrap()
        .id
        .clone();
    p.replace_history(&json!({"messages":[]}));
    assert_eq!(p.snapshot.as_ref().unwrap().messages[0].id, id);
    assert_eq!(p.snapshot.as_ref().unwrap().run_state, RunState::Running);
    p.apply_event(&json!({"type":"compaction_end","reason":"manual","aborted":true}));
    p.apply_event(&json!({"type":"response","command":"compact","id":"compact-1","success":false,"error":"cancelled"}));
    let snapshot = p.snapshot.as_ref().unwrap();
    assert_eq!(snapshot.run_state, RunState::Idle);
    assert!(p.turn_ended() && !p.turn_failed());
    assert_eq!(snapshot.messages[0].id, id);
    assert_eq!(
        snapshot.messages[0].blocks,
        vec![MessageBlock::ContextCompaction {
            text: "上下文压缩已取消".into()
        }]
    );
    p.apply_event(&json!({"type":"velune_closed"}));
    assert!(
        !p.snapshot
            .as_ref()
            .unwrap()
            .messages
            .iter()
            .any(|message| !message.completed)
    );
}

#[test]
fn extension_diagnostic_keeps_its_position_when_native_history_is_replaced() {
    let mut p = projection();
    p.begin_turn();
    p.apply_event(&json!({"type":"agent_start"}));
    let user = json!({"id":"prompt","role":"user","content":"question"});
    let answer = json!({"id":"answer","role":"assistant","stopReason":"stop","content":"answer"});
    message(&mut p, user.clone());
    p.apply_event(
        &json!({"type":"extension_error","extensionPath":"synthetic.ts",
            "event":"before_agent_start","error":"synthetic extension failure"}),
    );
    assert_eq!(p.snapshot.as_ref().unwrap().run_state, RunState::Running);
    message(&mut p, answer.clone());
    p.apply_event(&json!({"type":"agent_settled"}));
    assert!(p.turn_ended());
    assert!(!p.turn_failed());
    let newer = json!({"id":"newer","role":"user","content":"next question"});
    for _ in 0..2 {
        p.replace_history(&json!({"messages":[user,answer,newer]}));
        let snapshot = p.snapshot.as_ref().unwrap();
        assert_eq!(snapshot.run_state, RunState::Idle);
        assert!(snapshot.actions.can_send);
        assert_eq!(snapshot.messages.len(), 4);
        assert!(snapshot.messages[1].id.starts_with("pi-diagnostic-"));
        let MessageBlock::Notice { text } = &snapshot.messages[1].blocks[0] else {
            panic!()
        };
        assert!(text.contains("synthetic extension failure") && text.contains("synthetic.ts"));
        assert_eq!(snapshot.messages[2].id, "answer");
    }
}

#[test]
fn retry_outcome_comes_from_the_last_native_message_not_retry_end() {
    for stop in ["stop", "error", "aborted"] {
        let mut p = projection();
        p.begin_turn();
        p.apply_event(&json!({"type":"agent_start"}));
        message(
            &mut p,
            json!({"id":"omitted","role":"assistant","stopReason":"error",
                "errorMessage":"first failed attempt","content":[]}),
        );
        p.apply_event(
            &json!({"type":"auto_retry_start","attempt":1,"maxAttempts":3,
                "delayMs":10,"errorMessage":"first failed attempt"}),
        );
        assert!(!p.turn_ended());
        assert!(
            !p.snapshot
                .as_ref()
                .unwrap()
                .messages
                .iter()
                .any(|m| m.id == "omitted")
        );
        p.apply_event(&json!({"type":"agent_start"}));
        let last = json!({"id":"last","role":"assistant","stopReason":stop,
                "errorMessage":if stop=="error" {"final native failure"} else {""},
                "content":[{"type":"text","text":"last response"}]});
        message(&mut p, last.clone());
        p.apply_event(
            &json!({"type":"auto_retry_end","success":stop=="stop","finalError":"Retry cancelled"}),
        );
        p.apply_event(&json!({"type":"agent_settled"}));
        assert!(p.turn_ended());
        assert_eq!(p.turn_failed(), stop == "error");
        p.replace_history(&json!({"messages":[last]}));
        let snapshot = p.snapshot.as_ref().unwrap();
        assert_eq!(snapshot.messages.len(), 1);
        assert_eq!(snapshot.messages[0].id, "last");
        assert_eq!(
            snapshot.run_state,
            if stop == "error" {
                RunState::Failed
            } else {
                RunState::Idle
            }
        );
        assert!(snapshot.actions.can_send);
    }
}

#[test]
fn settled_abort_without_assistant_is_cancelled_but_idle_does_not_imply_success() {
    for aborted in [false, true] {
        let mut p = projection();
        p.begin_turn();
        p.apply_event(&json!({"type":"agent_start"}));
        message(
            &mut p,
            json!({"id":"user","role":"user","content":"question"}),
        );
        p.apply_event(&json!({"type":"agent_settled","aborted":aborted}));
        let snapshot = p.snapshot.as_ref().unwrap();
        assert_eq!(
            snapshot.executions.record("user").unwrap().outcome,
            aborted.then_some(TurnOutcome::Cancelled)
        );
        assert!(p.turn_ended());
        assert!(!p.turn_failed());
    }
}

#[test]
fn terminal_connection_failure_survives_late_lifecycle_and_history_records() {
    let mut p = projection();
    p.apply_event(&json!({"type":"agent_settled"}));
    assert!(!p.turn_ended());
    p.begin_turn();
    p.apply_event(&json!({"type":"velune_error","error":"invalid JSON; synthetic stderr"}));
    for value in [
        json!({"type":"agent_start"}),
        json!({"type":"agent_settled"}),
    ] {
        p.apply_event(&value);
    }
    p.replace_history(&json!({"messages":[]}));
    p.begin_turn();
    let snapshot = p.snapshot.as_ref().unwrap();
    assert!(p.turn_ended());
    assert!(!p.can_sync_history());
    assert_eq!(snapshot.run_state, RunState::Failed);
    assert!(!snapshot.actions.can_send && !snapshot.actions.can_cancel);
    assert_eq!(snapshot.messages.len(), 1);
    let MessageBlock::Notice { text } = &snapshot.messages[0].blocks[0] else {
        panic!()
    };
    assert_eq!(text, "invalid JSON; synthetic stderr");
}

#[test]
fn connection_failure_ends_a_visible_retry() {
    let mut p = projection();
    p.begin_turn();
    p.apply_event(&json!({"type":"agent_start"}));
    p.apply_event(
        &json!({"type":"auto_retry_start","attempt":1,"maxAttempts":3,
            "errorMessage":"temporary failure"}),
    );
    p.apply_event(&json!({"type":"velune_error","error":"synthetic connection lost"}));
    assert!(
        !p.snapshot
            .as_ref()
            .unwrap()
            .messages
            .iter()
            .any(|m| m.id == "pi-auto-retry")
    );
    assert!(p.turn_ended() && !p.can_sync_history());
}

#[test]
fn tool_result_images_attach_to_the_tool_call_message() {
    let mut p = projection();
    p.replace_history(&json!({"messages":[
        {"id":"prompt","role":"user","content":"看看这张图"},
        {"id":"caller","role":"assistant","content":[
            {"type":"toolCall","id":"call-1","name":"read","arguments":{"path":"/tmp/photo.jpg"}}]},
        {"id":"result","role":"toolResult","toolCallId":"call-1","toolName":"read",
         "content":[
            {"type":"text","text":"Read image file [image/jpeg]"},
            {"type":"image","mimeType":"image/jpeg","data":"aGVsbG8="}]}
    ]}));
    let snapshot = p.snapshot.as_ref().unwrap();
    assert_eq!(snapshot.messages.len(), 2);
    let caller = &snapshot.messages[1];
    assert_eq!(caller.role, MessageRole::Assistant);
    assert!(matches!(
        &caller.blocks[0],
        MessageBlock::Tool { tool_id, state: ToolState::Completed, .. } if tool_id.as_deref() == Some("call-1")
    ));
    assert_eq!(
        caller.blocks[1..],
        [MessageBlock::Image {
            media_type: Some("image/jpeg".into()),
            source: crate::conversation::ImageSource::Data {
                data: "aGVsbG8=".into()
            },
        }]
    );
}

#[test]
fn tool_result_images_do_not_duplicate_across_start_and_end() {
    let mut p = projection();
    let call = json!({"id":"caller","role":"assistant","content":[
            {"type":"toolCall","id":"call-1","name":"read","arguments":{"path":"/tmp/photo.jpg"}}]});
    let result = json!({"id":"result","role":"toolResult","toolCallId":"call-1","toolName":"read",
            "content":[
                {"type":"text","text":"Read image file [image/jpeg]"},
                {"type":"image","mimeType":"image/jpeg","data":"aGVsbG8="}]});
    for event in ["message_start", "message_end"] {
        p.apply_event(&json!({"type":event,"message":call}));
        p.apply_event(&json!({"type":event,"message":result}));
    }
    let snapshot = p.snapshot.as_ref().unwrap();
    let caller = snapshot
        .messages
        .iter()
        .find(|message| message.id == "caller")
        .unwrap();
    assert_eq!(
        caller
            .blocks
            .iter()
            .filter(|block| matches!(block, MessageBlock::Image { .. }))
            .count(),
        1
    );
}

#[test]
fn tool_result_images_standalone_when_the_call_is_not_projected() {
    let mut p = projection();
    p.apply_event(&json!({"type":"message_start","message":{"id":"result","role":"toolResult","toolCallId":"call-1","toolName":"read","content":[{"type":"image","mimeType":"image/png","data":"aGVsbG8="}]}}));
    p.apply_event(&json!({"type":"message_end","message":{"id":"result","role":"toolResult","toolCallId":"call-1","toolName":"read","content":[{"type":"image","mimeType":"image/png","data":"aGVsbG8="}]}}));
    let snapshot = p.snapshot.as_ref().unwrap();
    let message = snapshot
        .messages
        .iter()
        .find(|message| message.role == MessageRole::Tool)
        .unwrap();
    assert_eq!(message.role, MessageRole::Tool);
    assert!(matches!(message.blocks[0], MessageBlock::Tool { .. }));
    assert_eq!(
        message.blocks[1..],
        [MessageBlock::Image {
            media_type: Some("image/png".into()),
            source: crate::conversation::ImageSource::Data {
                data: "aGVsbG8=".into()
            },
        }]
    );
}
fn native(messages: Value) -> ConversationSnapshot {
    let mut p = projection();
    p.replace_history(&json!({"messages":messages}));
    p.snapshot.unwrap()
}
fn transcript(s: &ConversationSnapshot) -> crate::transcript::TranscriptProjection {
    let mut p = crate::transcript::TranscriptProjection::default();
    p.apply(crate::transcript::TranscriptEvent::ReplaceSnapshot(s));
    p
}
fn ids(p: &crate::transcript::TranscriptProjection) -> Vec<String> {
    p.items()
        .into_iter()
        .map(|i| match i {
            TranscriptItem::Message { message_id, .. } => message_id,
            TranscriptItem::Work { .. } => "work".into(),
        })
        .collect()
}
fn user() -> Value {
    json!({"id":"user","role":"user","content":"request"})
}
fn tool(id: &str) -> Value {
    json!({"id":id,"role":"assistant","content":[{"type":"toolCall","id":format!("call-{id}"),"name":"shell","arguments":{"command":"synthetic"}}]})
}
fn notice(id: &str) -> Value {
    json!({"id":id,"role":"custom","customType":"background-task-notification","content":"arbitrary body"})
}
const SUBAGENT_NOTIFICATIONS: [&str; 2] =
    ["subagent-future-notification", "subagent_future_notice"];
fn subagent_return(id: &str, custom_type: &str) -> Value {
    json!({"id":id,"role":"custom","customType":custom_type,"display":true,
            "content":"**结果**\n\n[报告](/synthetic/report.md:20)\n\n```text\nresult\n```"})
}
#[test]
fn subagent_notification_types_do_not_reclassify_user_directives() {
    let mut directive = subagent_return("directive", "subagent-notify");
    directive["role"] = json!("user");
    let s = native(json!([user(), directive]));
    assert!(s.work_notification_ids.is_empty());
    assert_eq!(s.messages.last().unwrap().role, MessageRole::User);
}

#[test]
fn tools_and_work_notices_preserve_native_work_order_without_closing() {
    let s = native(json!([
        user(),
        tool("before"),
        notice("notice1"),
        tool("after"),
        notice("notice2")
    ]));
    let p = transcript(&s);
    let turns = p.turns();
    assert_eq!(turns[0].last_message_id, None);
    assert_eq!(
        turns[0].work_message_ids,
        ["before", "notice1", "after", "notice2"]
    );
    assert_eq!(ids(&p), ["user", "work"]);
}
#[test]
fn closing_stays_visible_late_notifications_stay_after_and_mixed_identity_is_stable() {
    for mixed in [false, true] {
        let close = if mixed {
            json!([{"type":"thinking","thinking":"reason"},{"type":"text","text":"answer"}])
        } else {
            json!([{"type":"text","text":"answer"}])
        };
        let s = native(json!([
            subagent_return("orphan", "subagent-notify"), user(), tool("before"),
            notice("early"), {"id":"closing","role":"assistant","content":close}, notice("late")
        ]));
        let p = transcript(&s);
        let t = &p.turns()[0];
        assert_eq!(t.last_message_id.as_deref(), Some("closing"));
        assert_eq!(
            t.work_message_ids,
            if mixed {
                vec!["before", "early", "closing"]
            } else {
                vec!["before", "early"]
            }
        );
        assert_eq!(ids(&p), ["orphan", "user", "work", "closing", "late"]);
        assert!(
            matches!(&p.items()[3],TranscriptItem::Message{id,..} if id == &format!("{}message:closing",if mixed{"reply:"}else{""}))
        );
        let before = p.items();
        let mut p = p;
        p.apply(crate::transcript::TranscriptEvent::ReplaceSnapshot(&s));
        assert_eq!(p.items(), before);
    }
}
#[test]
fn unknown_custom_text_error_and_image_remain_standalone_without_text_guessing() {
    let s = native(
        json!([user(),tool("before"),{"id":"unknown","role":"custom","customType":"unrelated","content":"background-task-notification"},{"id":"error","role":"custom","customType":"unrelated","errorMessage":"error"},{"id":"image","role":"custom","customType":"unrelated","content":[{"type":"image"}]},tool("after")]),
    );
    let p = transcript(&s);
    assert_eq!(p.turns()[0].work_message_ids, ["before", "after"]);
    assert_eq!(ids(&p), ["user", "work", "unknown", "error", "image"]);
    assert!(
        s.messages
            .iter()
            .filter(|m| ["error", "image"].contains(&m.id.as_str()))
            .all(|m| m
                .blocks
                .iter()
                .all(|b| matches!(b, MessageBlock::Notice { .. })))
    );
}
#[test]
fn orphan_notices_keep_order_and_standalone_compaction_blocks_backwards_attachment() {
    let s = native(
        json!([notice("orphan1"),notice("orphan2"),user(),tool("before"),{"id":"compact","role":"compactionSummary"},notice("after_compact")]),
    );
    let p = transcript(&s);
    assert_eq!(p.turns()[0].work_message_ids, ["before"]);
    assert_eq!(
        ids(&p),
        [
            "orphan1",
            "orphan2",
            "user",
            "work",
            "compact",
            "after_compact"
        ]
    );
}

#[test]
fn notifications_inside_automatic_compaction_work_remain_in_the_owning_range() {
    let mut snapshot = native(json!([
        user(), tool("before"), {"id":"compact","role":"compactionSummary"},
        notice("notice"), tool("after")
    ]));
    snapshot.executions.begin(ExecutionKind::UserTurn, false);
    for message in &snapshot.messages {
        snapshot.executions.attach(&message.id);
    }
    snapshot.executions.finish(Some(TurnOutcome::Completed));
    let p = transcript(&snapshot);
    assert_eq!(
        p.turns()[0].work_message_ids,
        ["before", "compact", "notice", "after"]
    );
    assert_eq!(ids(&p), ["user", "work"]);
}

#[test]
fn supervisor_requests_remain_history_and_reply_results_do_not_invent_request_state() {
    for replied in [None, Some(false), Some(true)] {
        let mut messages = vec![
            user(),
            tool("before"),
            json!({"id":"request","role":"custom","customType":"subagent_supervisor_request","display":true,"content":"Subagent needs a supervisor decision.","details":{"requestId":"R","expectsReply":true,"reason":"need_decision"}}),
        ];
        if let Some(is_error) = replied {
            messages.push(json!({"id":"reply","role":"assistant","content":[{"type":"toolCall","id":"reply-call","name":"subagent_supervisor","arguments":{"action":"reply","replyTo":"R","message":"synthetic"}}]}));
            messages.push(json!({"id":"reply-result","role":"toolResult","toolCallId":"reply-call","toolName":"subagent_supervisor","isError":is_error,"content":[{"type":"text","text":"native reply outcome"}],"details":if is_error { json!({}) } else { json!({"replyTo":"R","runId":"synthetic-run","agent":"synthetic-agent"}) }}));
        }
        messages.push(json!({"id":"closing","role":"assistant","content":"answer"}));
        let snapshot = native(json!(messages));
        let t = transcript(&snapshot);
        assert!(snapshot.work_notification_ids.contains(&"request".into()));
        assert!(t.turns()[0].work_message_ids.contains(&"request".into()));
        assert_eq!(ids(&t), ["user", "work", "closing"]);
        assert!(
            snapshot.pending_interactions.is_empty(),
            "custom history does not prove current reply availability or resolution"
        );
        if let Some(is_error) = replied {
            let reply = snapshot.messages.iter().find(|m| m.id == "reply").unwrap();
            assert!(
                matches!(&reply.blocks[0], MessageBlock::Tool { state, output: Some(output), .. }
                    if state == &(if is_error { ToolState::Failed } else { ToolState::Completed }) && output == "native reply outcome")
            );
        }
    }
}

#[test]
fn subagent_notices_join_work_while_other_extensions_stay_independent() {
    let mut messages = vec![user(), tool("before")];
    for kind in SUBAGENT_NOTIFICATIONS {
        messages.push(subagent_return(kind, kind));
    }
    for kind in ["subagents-admin", "unrecognized"] {
        messages.push(json!({"id":kind,"role":"custom","customType":kind,"content":"independent extension output","display":true}));
    }
    messages.push(tool("after"));
    let snapshot = native(json!(messages));
    let t = transcript(&snapshot);
    assert_eq!(snapshot.work_notification_ids, SUBAGENT_NOTIFICATIONS);
    let mut expected_work = vec!["before".to_owned()];
    expected_work.extend(SUBAGENT_NOTIFICATIONS.into_iter().map(str::to_owned));
    expected_work.push("after".into());
    assert_eq!(t.turns()[0].work_message_ids, expected_work);
    assert_eq!(ids(&t), ["user", "work", "subagents-admin", "unrecognized"]);
}

#[test]
fn sdk_injected_context_and_shell_messages_preserve_content_without_inventing_users() {
    for live in [false, true] {
        let messages = vec![
            user(),
            json!({"id":"answer","role":"assistant","content":"answer"}),
            json!({"id":"branch","role":"branchSummary","summary":"retained branch summary","fromId":"source"}),
            json!({"id":"bash","role":"bashExecution","command":"synthetic-command","output":"synthetic output","exitCode":7,"cancelled":false,"truncated":true,"fullOutputPath":"/synthetic/full.txt","excludeFromContext":true}),
            json!({"id":"compact","role":"compactionSummary","summary":"retained compact summary","tokensBefore":123}),
        ];
        let mut p = projection();
        if live {
            for m in messages {
                message(&mut p, m);
            }
        } else {
            p.replace_history(&json!({"messages":messages}));
        }
        let snapshot = p.snapshot.as_ref().unwrap();
        let branch = snapshot
            .messages
            .iter()
            .find(|m| m.id == "branch")
            .expect("SDK branch summary is visible");
        assert_eq!(branch.role, MessageRole::System);
        assert_eq!(
            branch.blocks,
            [MessageBlock::ContextCompaction {
                text: "分支摘要\n\nretained branch summary".into()
            }]
        );
        let bash = snapshot
            .messages
            .iter()
            .find(|m| m.id == "bash")
            .expect("SDK shell result is visible, including context-excluded !! commands");
        assert_eq!(bash.role, MessageRole::Client);
        assert!(
            matches!(&bash.blocks[0], MessageBlock::Tool { tool_id: None, title, state: ToolState::Failed, output: Some(output), conversation_id: None }
                if title == "本地命令：synthetic-command" && output == "synthetic output")
        );
        let facts = bash
            .blocks
            .iter()
            .filter_map(|block| match block {
                MessageBlock::Notice { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            facts.contains("退出码：7")
                && facts.contains("/synthetic/full.txt")
                && facts.contains("未加入模型上下文")
        );
        let compact = snapshot
            .messages
            .iter()
            .find(|m| m.id == "compact")
            .unwrap();
        assert!(
            matches!(&compact.blocks[0], MessageBlock::ContextCompaction { text } if text == "上下文已压缩\n\nretained compact summary")
        );
        let t = transcript(snapshot);
        assert_eq!(t.turns().len(), 1);
        assert_eq!(t.turns()[0].last_message_id.as_deref(), Some("answer"));
        assert_eq!(ids(&t), ["user", "answer", "branch", "bash", "compact"]);
    }
}

#[test]
fn sdk_bash_cancellation_and_empty_success_remain_distinguishable() {
    for cancelled in [true, false] {
        let m = message_from_pi(&json!({"role":"bashExecution","command":"synthetic","output":"","cancelled":cancelled,"truncated":false,"exitCode":if cancelled { Value::Null } else { json!(0) }}), "bash".into()).unwrap();
        assert!(
            m.blocks.iter().any(
                |block| matches!(block, MessageBlock::Notice { text } if text == "命令已取消")
            ) == cancelled
        );
        assert!(
            m.blocks
                .iter()
                .any(|block| matches!(block, MessageBlock::Notice { text } if text == "退出码：0"))
                == !cancelled
        );
        assert!(m.blocks.iter().any(
                |block| matches!(block, MessageBlock::Tool { output: Some(output), .. } if output.is_empty())
            ));
        assert_eq!(
            m.blocks.len(),
            2,
            "empty output has no duplicate placeholder"
        );
    }
}

#[test]
fn legacy_hooks_and_hidden_custom_messages_follow_the_sdk_custom_contract() {
    for role in ["custom", "hookMessage"] {
        let mut p = projection();
        let messages = vec![
            user(),
            tool("before"),
            json!({"id":"hidden","role":role,"customType":"subagent-notify","display":false,"content":"hidden"}),
            json!({"id":"visible","role":role,"customType":"subagent-notify","display":true,"content":"**result**"}),
            json!({"id":"unknown","role":role,"customType":"unrelated","display":true,"content":"actionable"}),
            tool("after"),
        ];
        for m in &messages {
            message(&mut p, m.clone());
        }
        for _ in 0..2 {
            let snapshot = p.snapshot.as_ref().unwrap();
            assert_eq!(snapshot.work_notification_ids, ["visible"]);
            assert!(!snapshot.messages.iter().any(|m| m.id == "hidden"));
            let visible = snapshot
                .messages
                .iter()
                .find(|m| m.id == "visible")
                .unwrap();
            assert_eq!(visible.role, MessageRole::Client);
            assert_eq!(
                visible.blocks,
                [MessageBlock::Text {
                    text: "**result**".into()
                }]
            );
            let t = transcript(snapshot);
            assert_eq!(
                t.turns()[0].work_message_ids,
                ["before", "visible", "after"]
            );
            assert_eq!(ids(&t), ["user", "work", "unknown"]);
            p.replace_history(&json!({"messages":messages}));
        }
    }
}

#[test]
fn work_notifications_survive_live_confirmation_resync_and_parent_switch_without_stale_classification()
 {
    use crate::transcript::{TranscriptEvent, TranscriptProjection};
    let mut p = projection();
    message(&mut p, user());
    message(&mut p, tool("before"));
    let mut unconfirmed = notice("temporary");
    unconfirmed.as_object_mut().unwrap().remove("id");
    message(&mut p, unconfirmed);
    let snapshot = p.snapshot.as_ref().unwrap();
    assert_eq!(snapshot.work_notification_ids.len(), 1);
    let previous = snapshot.work_notification_ids[0].clone();
    let mut t = TranscriptProjection::default();
    t.apply(TranscriptEvent::ReplaceSnapshot(snapshot));
    assert_eq!(t.turns()[0].work_message_ids, ["before", &previous]);
    let identity = t
        .message_identities()
        .into_iter()
        .find(|m| m.message_id == previous)
        .unwrap()
        .id;
    p.replace_history(&json!({"messages":[user(),tool("before"),notice("native")],
    "identityConfirmations":[{"previousId":previous,"currentId":"native"}]}));
    let snapshot = p.snapshot.as_ref().unwrap();
    t.apply(TranscriptEvent::ConfirmIdentities(
        &snapshot.message_identity_confirmations,
    ));
    t.apply(TranscriptEvent::ReplaceSnapshot(snapshot));
    assert_eq!(t.turns()[0].work_message_ids, ["before", "native"]);
    assert_eq!(
        t.message_identities()
            .into_iter()
            .find(|m| m.message_id == "native")
            .unwrap()
            .id,
        identity
    );
    let mut parent = snapshot.conversation.clone();
    parent.id = "pi:another-synthetic-parent".into();
    p.set_conversation(parent);
    // The source changes to an unrelated custom message with the same id:
    // classification is rebuilt from native semantics, never cached by id.
    p.replace_history(&json!({"messages":[user(),tool("before"),
    {"id":"native","role":"custom","customType":"other","content":"arbitrary body"}]}));
    let snapshot = p.snapshot.as_ref().unwrap();
    assert!(snapshot.work_notification_ids.is_empty());
    t.apply(TranscriptEvent::ReplaceSnapshot(snapshot));
    assert_eq!(t.turns()[0].work_message_ids, ["before"]);
    assert_eq!(ids(&t), ["user", "work", "native"]);
}
