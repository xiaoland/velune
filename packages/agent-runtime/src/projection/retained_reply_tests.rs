use serde_json::json;

#[test]
fn sdk_reply_boundaries_survive_live_resync_and_remain_visible_once() {
    let mut projection = super::tests::projection();
    let messages = json!([
        {"id":"user1","role":"user","content":"first"},
        {"id":"reply1","role":"assistant","stopReason":"stop","content":[{"type":"text","text":"first reply"}]},
        {"id":"reply2","role":"assistant","stopReason":"stop","content":[{"type":"text","text":"second reply"}]}
    ]);
    projection
        .replace_history(&json!({"messages":messages,"retainedReplyIds":["reply1","reply2"]}));
    // RPC get_messages does not repeat the SDK inspect metadata.
    for _ in 0..2 {
        projection.replace_history(&json!({"data":{"messages":messages}}));
        let snapshot = projection.snapshot.as_ref().unwrap();
        assert_eq!(snapshot.retained_reply_ids, ["reply1", "reply2"]);
        let mut transcript = crate::transcript::TranscriptProjection::default();
        transcript.apply(crate::transcript::TranscriptEvent::Replace {
            messages: &snapshot.messages,
            executions: &snapshot.executions,
        });
        transcript.retain_reply_boundaries(&snapshot.retained_reply_ids);
        let visible: Vec<_> = transcript
            .items()
            .into_iter()
            .filter_map(|item| match item {
                crate::conversation::TranscriptItem::Message { message_id, .. } => Some(message_id),
                _ => None,
            })
            .collect();
        assert_eq!(visible, ["user1", "reply1", "reply2"]);
        assert_eq!(
            transcript.turns()[0].last_message_id.as_deref(),
            Some("reply1")
        );
    }
    projection.replace_history(&json!({"messages":[{"id":"user1","role":"user","content":"first"},{"id":"reply1","role":"assistant","stopReason":"stop","content":"first reply"}]}));
    assert_eq!(
        projection.snapshot.as_ref().unwrap().retained_reply_ids,
        ["reply1"]
    );
}
