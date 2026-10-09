#!/usr/bin/env python3
"""Explicit isolated projection acceptance; no runtime data, provider, or test runner."""
from pathlib import Path
import os
import subprocess
import tempfile

repository = Path(__file__).resolve().parents[1]
source = r'''
use velune_agent_runtime::transcript::{TranscriptEvent, TranscriptProjection};
use velune_conversation::{Message, MessageBlock, MessageIdentityConfirmation, MessageRole, RunState, TranscriptItem};
fn message(id: &str, role: MessageRole, completed: bool, blocks: Vec<MessageBlock>) -> Message {
    Message { id: id.into(), role, completed, timestamp_unix_ms: None, blocks }
}
fn text(id: &str, role: MessageRole, completed: bool) -> Message {
    message(id, role, completed, vec![MessageBlock::Text { text: id.into() }])
}
fn replace(p: &mut TranscriptProjection, messages: &[Message]) {
    p.apply(TranscriptEvent::ReplaceMessages(messages));
    let turns=p.turns();
    let displayed:Vec<String>=p.items().into_iter().flat_map(|item| match item {
        TranscriptItem::Message{message_id,..}=>vec![message_id],
        TranscriptItem::Work{turn_id}=>turns.iter().find(|turn|turn.id==turn_id).unwrap().work_message_ids.clone(),
    }).collect();
    assert_eq!(displayed, messages.iter().map(|message|message.id.clone()).collect::<Vec<_>>(), "every snapshot message belongs to exactly one ordered item");
}
fn main() {
    let mut projection = TranscriptProjection::default();
    let user = text("live-user", MessageRole::User, true);
    let thought = message("thought", MessageRole::Assistant, true, vec![MessageBlock::Reasoning { text:"reasoning".into() }]);
    let streaming = text("answer", MessageRole::Assistant, false);
    replace(&mut projection, &[user.clone(), thought.clone(), streaming.clone()]);
    projection.apply(TranscriptEvent::ExecutionState(RunState::Running));
    assert!(projection.turns()[0].is_running);
    assert_eq!(projection.turns()[0].last_message_id.as_deref(), Some("answer"));
    let progress=text("progress", MessageRole::Assistant, true);
    let call=message("call", MessageRole::Tool, true, vec![MessageBlock::Notice { text:"tool".into() }]);
    let final_text=text("final", MessageRole::Assistant, true);
    let mut regression=TranscriptProjection::default();
    replace(&mut regression, &[user.clone(),progress.clone()]);
    regression.apply(TranscriptEvent::ExecutionState(RunState::Running));
    assert!(regression.turns()[0].is_running);
    replace(&mut regression, &[user.clone(),progress.clone(),call.clone()]);
    assert_eq!(regression.turns()[0].last_message_id,None);
    let mut final_stream=final_text.clone(); final_stream.completed=false;
    replace(&mut regression, &[user.clone(),progress.clone(),call.clone(),final_stream.clone()]);
    assert_eq!(regression.turns()[0].last_message_id.as_deref(),Some("final"));
    final_stream.blocks.push(MessageBlock::Reasoning{text:"thinking".into()});
    replace(&mut regression, &[user.clone(),progress.clone(),call.clone(),final_stream]);
    assert_eq!(regression.turns()[0].last_message_id,None);
    replace(&mut regression, &[user.clone(),progress.clone(),call.clone(),final_text]);
    assert!(regression.turns()[0].is_running);
    regression.apply(TranscriptEvent::ExecutionState(RunState::Idle));
    assert!(!regression.turns()[0].is_running);
    assert_eq!(regression.turns()[0].work_message_ids,["progress","call"], "completed progress must stay inside work");
    let original_turn = projection.turns()[0].id.clone();
    let original_outline = projection.outline()[0].id.clone();
    let original_answer = projection.message_identities()[2].id.clone();
    let mixed = message("answer", MessageRole::Assistant, false, vec![MessageBlock::Text {text:"partial".into()}, MessageBlock::Notice {text:"tool".into()}]);
    replace(&mut projection, &[user.clone(), thought.clone(), mixed.clone()]);
    assert_eq!(projection.turns()[0].last_message_id, None);
    assert_eq!(projection.turns()[0].work_message_ids, ["thought", "answer"]);
    assert_eq!(projection.items().len(), 2);
    assert!(matches!(projection.items()[1], TranscriptItem::Work { .. }));
    let mut final_answer = streaming.clone(); final_answer.completed = true;
    let tail = message("tail", MessageRole::System, true, vec![MessageBlock::Notice {text:"settled".into()}]);
    replace(&mut projection, &[user.clone(), thought.clone(), final_answer.clone(), tail.clone()]);
    assert!(projection.turns()[0].is_running); // Only the harness lifecycle ends execution.
    assert_eq!(projection.turns()[0].last_message_id, None);
    assert_eq!(projection.turns()[0].work_message_ids, ["thought", "answer", "tail"]);
    assert_eq!(projection.items().len(), 2); // A system tail is folded, without searching backward.
    projection.apply(TranscriptEvent::ConfirmIdentities(&[MessageIdentityConfirmation {previous_id:"live-user".into(), current_id:"native-user".into()}, MessageIdentityConfirmation {previous_id:"answer".into(), current_id:"native-answer".into()}]));
    let mut native_user=user.clone(); native_user.id="native-user".into();
    final_answer.id="native-answer".into();
    replace(&mut projection, &[native_user.clone(), thought.clone(), final_answer.clone(), tail]);
    assert_eq!(projection.turns()[0].id, original_turn);
    assert_eq!(projection.outline()[0].id, original_outline);
    assert_eq!(projection.message_identities()[2].id, original_answer);
    replace(&mut projection, &[native_user.clone(), mixed]); // Authoritative branch replaces prior answer.
    assert_eq!(projection.turns()[0].last_message_id, None);
    assert_eq!(projection.turns()[0].work_message_ids, ["answer"]);
    assert_eq!(projection.message_identities().len(), 2);
    let empty = message("empty", MessageRole::Assistant, true, vec![MessageBlock::Text {text:"  ".into()}]);
    replace(&mut projection, &[native_user.clone(), empty]);
    assert_eq!(projection.turns()[0].last_message_id, None);
    assert!(projection.turns()[0].is_running);
    let next_user=text("next-user", MessageRole::User, true);
    let tool=message("tool", MessageRole::Tool, true, vec![MessageBlock::Notice {text:"tool result".into()}]);
    replace(&mut projection, &[native_user, tool.clone(), next_user.clone()]);
    assert_eq!(projection.turns().len(), 2);
    assert_eq!(projection.turns()[0].work_message_ids, ["tool"]);
    assert_eq!(projection.turns()[0].last_message_id, None);
    assert_eq!(projection.turns()[1].last_message_id, None);
    assert_eq!(projection.outline().len(), 2);
    projection.apply(TranscriptEvent::ExecutionState(RunState::Running));
    assert!(!projection.turns()[0].is_running);
    assert!(projection.turns()[1].is_running);
    assert!(projection.turns()[0].duration_ms.is_some());
    let mut pi = velune_agent_runtime::PiProjection::new(velune_conversation::ConversationSummary {
        id:"synthetic".into(), title:velune_conversation::ConversationTitle::Untitled,
        updated_at_unix_ms:None, created_at_unix_ms:None, runtime_id:"pi".into(), cwd:None, can_rename:true, can_delete:true,
    });
    pi.apply_event(&serde_json::json!({"type":"message_start","message":{"id":"pi-user","role":"user","content":"hello"}}));
    pi.apply_event(&serde_json::json!({"type":"message_end","message":{"id":"pi-user","role":"user","content":"hello"}}));
    pi.apply_event(&serde_json::json!({"type":"message_start","message":{"id":"pi-assistant","role":"assistant","content":[{"type":"text","text":"partial"}]}}));
    assert!(!pi.snapshot.as_ref().unwrap().messages[1].completed);
    let mut pi_projection=TranscriptProjection::default();
    replace(&mut pi_projection, &pi.snapshot.as_ref().unwrap().messages);
    pi_projection.apply(TranscriptEvent::ExecutionState(RunState::Running));
    assert!(pi_projection.turns()[0].is_running);
    pi.apply_event(&serde_json::json!({"type":"message_end","message":{"id":"pi-assistant","role":"assistant","content":[{"type":"text","text":"partial"},{"type":"toolCall","id":"tool-call","name":"shell"}]}}));
    assert!(pi.snapshot.as_ref().unwrap().messages[1].completed);
    replace(&mut pi_projection, &pi.snapshot.as_ref().unwrap().messages);
    assert_eq!(pi_projection.turns()[0].last_message_id, None);
    assert!(pi_projection.turns()[0].is_running);
    let mut repeated = TranscriptProjection::default();
    replace(&mut repeated, &[next_user.clone(), tool]);
    let first=repeated.items();
    let mut history = TranscriptProjection::default();
    replace(&mut history, &[next_user, message("tool",MessageRole::Tool,true,vec![MessageBlock::Notice{text:"tool result".into()}])]);
    assert_eq!(history.items(), first);
    println!("projection acceptance passed: streaming, mixed content, full user range, strict tail, stable confirmation, branch replacement, outline, deterministic history");
}
'''
with tempfile.TemporaryDirectory(prefix="velune-projection-acceptance-") as directory:
    temporary = Path(directory)
    (temporary / "src").mkdir()
    (temporary / "Cargo.toml").write_text(f'''[package]\nname="velune-projection-acceptance"\nversion="0.0.0"\nedition="2024"\n[dependencies]\nvelune-agent-runtime={{path="{repository / 'packages/agent-runtime'}"}}\nvelune-conversation={{path="{repository / 'packages/conversation'}"}}\nserde_json="1"\n''')
    (temporary / "src/main.rs").write_text(source)
    environment = dict(os.environ, CARGO_TARGET_DIR=str(repository / "target"))
    subprocess.run(["cargo", "run", "--offline", "--manifest-path", str(temporary / "Cargo.toml")], env=environment, check=True)
