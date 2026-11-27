use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn compaction_connection_failures_preserve_reply_and_confirmed_success() {
    for phase in ["prepare", "pending", "completed"] {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root =
            std::env::temp_dir().join(format!("velune-compact-{:x}", u64::from_le_bytes(nonce)));
        std::fs::create_dir(&root).unwrap();
        let binary = root.join("codex");
        std::fs::write(
            &binary,
            r#"#!/bin/sh
if [ "$1" = --version ]; then echo 'codex-cli 0.166.0'; exit 0; fi
if [ -f "$CODEX_HOME/fail-initialize" ]; then exit 1; fi
while IFS= read -r line; do
  case "$line" in
    *'"id":'*) printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9]*\).*/{"id":\1,"result":{}}/p' ;;
  esac
done
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
        let messages = serde_json::from_value(json!([
                {"id":"user","role":"user","completed":true,"blocks":[{"kind":"text","text":"hello"}]},
                {"id":"answer","role":"assistant","completed":true,"blocks":[{"kind":"text","text":"done"}]}
            ])).unwrap();
        session.replace_snapshot("synthetic", Some(&root), "codex", messages);
        if phase == "prepare" {
            session.restart_required = true;
            std::fs::write(root.join("fail-initialize"), "synthetic").unwrap();
            assert!(session.compact().is_err());
        } else {
            session.compact().unwrap();
            session.snapshot.as_mut().unwrap().messages[0].turn_usage = Some(TurnUsage {
                input_tokens: Some(90),
                context_tokens: Some(100),
                ..Default::default()
            });
            session.turn_id = Some("maintenance".into());
            session.codex_handle_record(json!({"method":"thread/tokenUsage/updated","params":{
                    "turnId":"maintenance","tokenUsage":{"last":{"inputTokens":10,"outputTokens":5},"modelContextWindow":200}
                }})).unwrap();
            let usage = session.snapshot.as_ref().unwrap().messages[0]
                .turn_usage
                .as_ref()
                .unwrap();
            assert_eq!(usage.context_tokens, Some(15));
            assert_eq!(usage.input_tokens, Some(90));
            session.codex_handle_record(json!({"method":if phase == "completed" {"item/completed"} else {"item/started"},
                    "params":{"item":{"id":"compact","type":"contextCompaction"}}})).unwrap();
            let question = super::codex_questions::project(&json!({"type":"agentMessage","id":"ask-output","questions":[{"title":"Question","options":["A","B"]}]})).unwrap();
            session
                .snapshot
                .as_mut()
                .unwrap()
                .pending_interactions
                .push(question);
            session.sub_agents.insert("codex:child".into());
            session.codex_handle_record(json!({"id":78,"method":"item/tool/requestUserInput","params":{"threadId":"child","turnId":"child-turn","questions":[{"id":"q","question":"Child RPC","options":null,"isOther":true}]}})).unwrap();
            let child_request = session
                .snapshot()
                .unwrap()
                .pending_interactions
                .last()
                .unwrap()
                .id
                .clone();
            assert!(session.interactions[&child_request].background);
            session
                .reply(
                    &child_request,
                    RuntimeInteractionReply::Answers {
                        answers: vec![InteractionAnswer {
                            question_id: "q".into(),
                            values: vec!["child answer".into()],
                        }],
                    },
                )
                .unwrap();
            assert!(!session.interactions.contains_key(&child_request));
            assert_eq!(session.snapshot().unwrap().pending_interactions.len(), 1);
            session.codex_handle_record(json!({"id":77,"method":"item/tool/requestUserInput","params":{"threadId":"synthetic","turnId":"maintenance","isBlocking":false,"questions":[{"id":"q","question":"Native RPC","options":null}]}})).unwrap();
            assert_eq!(session.snapshot().unwrap().pending_interactions.len(), 2);
            session.fail_connection();
            let pending = &session.snapshot().unwrap().pending_interactions;
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].id, "codex-async:ask-output");
        }
        assert!(!session.compacting);
        let snapshot = session.snapshot().unwrap();
        assert_eq!(snapshot.run_state, RunState::Failed);
        assert!(snapshot.messages.iter().all(|message| message.completed));
        assert!(
            matches!(&snapshot.messages.last().unwrap().blocks[0], MessageBlock::ContextCompaction { text } if text.contains("连接"))
        );
        if phase == "completed" {
            let confirmed = snapshot
                .messages
                .iter()
                .find(|message| message.id == "compact")
                .unwrap();
            assert!(
                matches!(&confirmed.blocks[0], MessageBlock::ContextCompaction { text } if text == "上下文已压缩")
            );
        }
        let mut transcript = crate::transcript::TranscriptProjection::default();
        transcript.apply(crate::transcript::TranscriptEvent::Replace {
            messages: &snapshot.messages,
            executions: &snapshot.executions,
        });
        assert_eq!(
            transcript.turns()[0].last_message_id.as_deref(),
            Some("answer")
        );
        drop(session);
        std::fs::remove_dir_all(root).unwrap();
    }
}
