//! Presentation facts for portable-summary preparation. The source native
//! transcript and its token/accounting facts remain unchanged.
use super::*;
use crate::conversation::{ConversationSnapshot, Message, MessageBlock, MessageRole};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HandoffCompactionEvent {
    pub work_id: String,
    pub conversation_id: String,
    pub physical_runtime_id: String,
    pub physical_native_id: String,
    pub source_anchor: Option<Option<String>>,
    pub state: HandoffCompactionState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HandoffCompactionState {
    Running,
    Cancelling,
    Ready,
    Failed { code: String, detail: String },
    Cancelled,
}

pub(super) fn compaction_message(work_id: &str, state: &HandoffCompactionState) -> Message {
    let text = match state {
        HandoffCompactionState::Running => "正在为切换运行时压缩上下文",
        HandoffCompactionState::Cancelling => "正在取消接续压缩",
        HandoffCompactionState::Ready => "已为接续生成摘要",
        HandoffCompactionState::Failed { .. } => "接续压缩失败，尚未发送消息",
        HandoffCompactionState::Cancelled => "已取消接续压缩",
    };
    let text = match state {
        HandoffCompactionState::Failed { code, detail } => {
            format!("{text}\n\n{detail}\n\n错误代码 {code}\n诊断编号 {work_id}")
        }
        _ => text.to_owned(),
    };
    Message {
        id: format!("velune-handoff-compaction:{work_id}"),
        role: MessageRole::System,
        completed: !matches!(
            state,
            HandoffCompactionState::Running | HandoffCompactionState::Cancelling
        ),
        timestamp_unix_ms: None,
        turn_settings: None,
        turn_usage: None,
        branch_point: None,
        blocks: vec![MessageBlock::ContextCompaction { text }],
    }
}

impl CoreRuntime {
    pub(crate) fn handoff_compaction_anchor(
        &self,
        conversation_id: &str,
        runtime_id: &str,
        native_id: &str,
    ) -> Option<Option<String>> {
        std::iter::once(&self.session)
            .chain(&self.parked)
            .filter(|session| session.conversation_id() == Some(conversation_id))
            .filter_map(|session| session.projected())
            .find(|snapshot| {
                snapshot.conversation.runtime_id == runtime_id
                    && snapshot
                        .conversation
                        .id
                        .strip_prefix(&format!("{runtime_id}:"))
                        .unwrap_or(&snapshot.conversation.id)
                        == native_id
            })
            .map(|snapshot| snapshot.messages.last().map(|message| message.id.clone()))
    }
    /// Replaced from the service's already bounded work registry; no history,
    /// filesystem or runtime preparation is involved in snapshot decoration.
    pub(crate) fn replace_handoff_compaction_events(
        &mut self,
        events: Vec<HandoffCompactionEvent>,
    ) {
        self.handoff_compaction_events = events;
    }

    pub(super) fn apply_handoff_compaction_events(
        &self,
        snapshot: &mut ConversationSnapshot,
        physical_id: &str,
        physical_anchor: Option<&str>,
    ) {
        append_events(
            snapshot,
            physical_id,
            physical_anchor,
            &self.handoff_compaction_events,
        );
    }
}

fn append_events(
    snapshot: &mut ConversationSnapshot,
    physical_id: &str,
    physical_anchor: Option<&str>,
    events: &[HandoffCompactionEvent],
) {
    for event in events {
        // Source history changed after this work's boundary. Its terminal
        // receipt must not migrate to the bottom of a newer conversation.
        if event
            .source_anchor
            .as_ref()
            .is_none_or(|anchor| anchor.as_deref() != physical_anchor)
        {
            continue;
        }
        if event.conversation_id != snapshot.conversation.id
            || event.physical_runtime_id != snapshot.context_runtime_id
            || physical_id
                .strip_prefix(&format!("{}:", event.physical_runtime_id))
                .unwrap_or(physical_id)
                != event.physical_native_id
        {
            continue;
        }
        let message = compaction_message(&event.work_id, &event.state);
        if !snapshot
            .messages
            .iter()
            .any(|existing| existing.id == message.id)
        {
            snapshot.messages.push(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velune_agent_runtime::transcript::{TranscriptEvent, TranscriptProjection};

    fn snapshot() -> ConversationSnapshot {
        serde_json::from_value(json!({
            "revision": 4,
            "conversation": {"id":"origin:logical", "title":{"source":"untitled"},"runtimeId":"origin"},
            "contextRuntimeId":"codex-tail", "resourceId":null,"modelRecordKey":null,
            "runState":"idle","messages":[],"transcriptTurns":[],"transcriptItems":[],
            "transcriptOutline":[],"transcriptMessageIdentities":[],
            "actions":{"canSend":true,"canCancel":false,"canSwitch":true}
        })).unwrap()
    }
    fn event() -> HandoffCompactionEvent {
        HandoffCompactionEvent {
            work_id: "work-1".into(),
            conversation_id: "origin:logical".into(),
            physical_runtime_id: "codex-tail".into(),
            physical_native_id: "thread-1".into(),
            source_anchor: Some(None),
            state: HandoffCompactionState::Running,
        }
    }

    #[test]
    fn maintenance_event_is_scoped_to_logical_and_exact_physical_source_without_changing_native_facts()
     {
        let mut value = snapshot();
        let native = value.clone();
        let record = event();
        let mut later = native.clone();
        append_events(
            &mut later,
            "codex-tail:thread-1",
            Some("newer-native-message"),
            std::slice::from_ref(&record),
        );
        assert_eq!(
            later, native,
            "an old receipt must not slide after newly added source messages"
        );
        append_events(
            &mut value,
            "codex-tail:thread-other",
            None,
            std::slice::from_ref(&record),
        );
        assert_eq!(value, native);
        let mut wrong = record.clone();
        wrong.conversation_id = "origin:other".into();
        append_events(&mut value, "codex-tail:thread-1", None, &[wrong]);
        assert_eq!(value, native);
        let mut wrong = record.clone();
        wrong.physical_runtime_id = "another-account".into();
        append_events(&mut value, "codex-tail:thread-1", None, &[wrong]);
        assert_eq!(value, native);
        append_events(
            &mut value,
            "codex-tail:thread-1",
            None,
            std::slice::from_ref(&record),
        );
        append_events(&mut value, "codex-tail:thread-1", None, &[record]);
        assert_eq!(value.messages.len(), 1);
        assert_eq!(value.messages[0].role, MessageRole::System);
        assert!(value.messages[0].turn_usage.is_none());
        assert!(value.messages[0].turn_settings.is_none());
        assert_eq!(value.actions, native.actions);
        assert_eq!(value.run_state, native.run_state);
        assert_eq!(value.revision, native.revision);
        let mut transcript = TranscriptProjection::default();
        transcript.apply(TranscriptEvent::Replace {
            messages: &value.messages,
            executions: &value.executions,
        });
        assert!(
            transcript.turns().is_empty(),
            "portable compaction is not a user/model turn"
        );
        assert_eq!(
            transcript.items().len(),
            1,
            "ordinary standalone maintenance row"
        );
    }

    #[test]
    fn work_lifecycle_keeps_one_identity_and_terminal_states_do_not_claim_a_native_compaction() {
        let mut identities = BTreeSet::new();
        for state in [
            HandoffCompactionState::Running,
            HandoffCompactionState::Cancelling,
            HandoffCompactionState::Ready,
            HandoffCompactionState::Failed {
                code: "summary_sample_timeout".into(),
                detail: "Codex 在 30 秒内未生成摘要。".into(),
            },
            HandoffCompactionState::Cancelled,
        ] {
            let message = compaction_message("work-1", &state);
            identities.insert(message.id);
            assert_eq!(
                message.completed,
                !matches!(
                    state,
                    HandoffCompactionState::Running | HandoffCompactionState::Cancelling
                )
            );
            assert!(
                matches!(&message.blocks[0], MessageBlock::ContextCompaction {text} if !text.contains("上下文已压缩"))
            );
            if matches!(state, HandoffCompactionState::Failed { .. }) {
                assert!(matches!(&message.blocks[0], MessageBlock::ContextCompaction { text }
                    if text == "接续压缩失败，尚未发送消息\n\nCodex 在 30 秒内未生成摘要。\n\n错误代码 summary_sample_timeout\n诊断编号 work-1"));
            }
        }
        assert_eq!(identities.len(), 1);
    }
}
