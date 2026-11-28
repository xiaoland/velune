//! Exercise restart recovery through the native provider and its stream-json child.
#![cfg(unix)]

use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, sync::Arc};
use velune_ai::{NativeProvider, NativeRequest, Payload, ProviderModelId};
use velune_ai_provider::claude_code::{ClaudeCode, ClaudeCodeConfig, ClaudeCodePool};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("velune-claude-cache-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        let executable = path.join("claude");
        fs::write(
            &executable,
            concat!(
                "#!/usr/bin/env python3\n",
                include_str!("fixtures/claude_bridge.py")
            ),
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }

    fn pool(&self) -> Arc<ClaudeCodePool> {
        Arc::new(ClaudeCodePool::with_cache_directory(self.0.join("cache")))
    }

    async fn send(&self, pool: Arc<ClaudeCodePool>, account: &str, messages: &[Value]) -> bool {
        let provider = ClaudeCode::new(
            pool,
            ClaudeCodeConfig {
                executable: self.0.join("claude"),
                config_dir: Some(self.0.clone()),
                session: Some("gateway/session".into()),
                binding: account.into(),
                cache_scope: Some(account.into()),
            },
        )
        .unwrap();
        provider
            .send(
                NativeRequest {
                    model: ProviderModelId::new("claude-test").unwrap(),
                    body: Payload::new(json!({"model": "claude-test", "messages": messages})),
                    headers: vec![],
                    stream: false,
                },
                Box::new(|_| Box::pin(async { Ok(()) })),
            )
            .await
            .is_ok()
    }

    fn calls(&self) -> Vec<Value> {
        self.records("fixture.jsonl")
    }

    fn records(&self, file: &str) -> Vec<Value> {
        fs::read_to_string(self.0.join(file))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn user(text: &str) -> Value {
    json!({"role": "user", "content": text})
}

fn answer() -> Value {
    json!({"role": "assistant", "content": "fixture answer"})
}

#[tokio::test]
async fn resumes_only_the_exact_committed_prefix_for_the_same_account() {
    let fixture = Fixture::new();
    let history = user(&"long history ".repeat(2_000));

    assert!(
        fixture
            .send(fixture.pool(), "a", std::slice::from_ref(&history))
            .await
    );
    assert!(
        fixture
            .send(
                fixture.pool(),
                "a",
                &[history.clone(), answer(), user("new turn")]
            )
            .await
    );
    let calls = fixture.calls();
    assert_eq!(calls[0]["cwd"], calls[1]["cwd"]);
    assert_eq!(calls[1]["input"], "new turn");
    assert_eq!(
        calls[1]["history"],
        json!([history["content"], "fixture answer"])
    );

    let prefix = [history, answer(), user("new turn"), answer()];
    let mut pending = prefix.to_vec();
    pending.push(user("terminate before result"));
    assert!(!fixture.send(fixture.pool(), "a", &pending).await);
    let mut next = prefix.to_vec();
    next.push(user("after termination"));
    assert!(fixture.send(fixture.pool(), "a", &next).await);
    let calls = fixture.calls();
    assert_eq!(calls[2]["input"], "terminate before result");
    assert_eq!(calls[3]["history"], json!([]));
    assert!(calls[3]["input"].as_str().unwrap().contains("long history"));

    next.push(answer());
    next.push(user("after changing account"));
    assert!(fixture.send(fixture.pool(), "b", &next).await);
    next[0] = user("edited history");
    next.push(answer());
    next.push(user("after editing"));
    assert!(fixture.send(fixture.pool(), "b", &next).await);
    let calls = fixture.calls();
    assert_eq!(calls[4]["history"], json!([]));
    assert_eq!(calls[5]["history"], json!([]));
    assert!(
        calls[5]["input"]
            .as_str()
            .unwrap()
            .contains("edited history")
    );
}

#[tokio::test]
async fn rejected_private_resume_falls_back_once_with_complete_caller_history() {
    for mode in ["exit", "initialize"] {
        let fixture = Fixture::new();
        let first = user("previous turn");
        assert!(
            fixture
                .send(fixture.pool(), "a", std::slice::from_ref(&first))
                .await
        );
        fs::write(fixture.0.join("reject-resume"), mode).unwrap();

        assert!(
            fixture
                .send(fixture.pool(), "a", &[first, answer(), user("next turn")])
                .await,
            "{mode} rejection must fall back"
        );
        let events = fixture.records("attempts.jsonl");
        let launches: Vec<_> = events
            .iter()
            .filter(|event| event["event"] == "launch")
            .collect();
        assert_eq!(
            launches
                .iter()
                .map(|event| event["resume"].as_bool().unwrap())
                .collect::<Vec<_>>(),
            [true, true, false],
            "initial private session, rejected resume, stateless fallback: {mode}"
        );
        let rejected = &launches[1]["attempt"];
        let observed: Vec<_> = events
            .iter()
            .filter(|event| &event["attempt"] == rejected)
            .collect();
        let stages: Vec<_> = observed
            .iter()
            .map(|event| event["event"].as_str().unwrap())
            .collect();
        if mode == "exit" {
            // This receipt proves an attempted resume exited before reading;
            // it does not claim the parent had written no bytes into stdin.
            assert_eq!(stages, ["launch", "reject_exit"]);
        } else {
            assert_eq!(stages, ["launch", "first_frame", "reject_initialize"]);
            assert_eq!(observed[1]["frame"]["type"], "control_request");
            assert_eq!(observed[1]["frame"]["request"]["subtype"], "initialize");
        }
        let calls = fixture.calls();
        assert_eq!(
            calls.len(),
            2,
            "{mode}: one initial input and one fallback input"
        );
        assert_eq!(calls[1]["attempt"], launches[2]["attempt"]);
        assert_eq!(calls[1]["history"], json!([]));
        let replay = calls[1]["input"].as_str().unwrap();
        let (_, history) = replay
            .split_once('\n')
            .expect("fallback carries caller history");
        let history: Value = serde_json::from_str(history).unwrap();
        assert_eq!(
            history,
            json!([
                {"role": "user", "content": [{"type": "text", "text": "previous turn"}]},
                {"role": "assistant", "content": [{"type": "text", "text": "fixture answer"}]},
                {"role": "user", "content": [{"type": "text", "text": "next turn"}]},
            ])
        );
    }
}
