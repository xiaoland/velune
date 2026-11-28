//! Exercise restart recovery through the native provider and its stream-json child.
#![cfg(unix)]

use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, sync::Arc};
use velune_ai::{
    NativeCompletion, NativeErrorKind, NativeProvider, NativeRequest, Payload, ProviderModelId,
};
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

    async fn send(
        &self,
        pool: Arc<ClaudeCodePool>,
        account: &str,
        messages: &[Value],
    ) -> NativeCompletion {
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

fn replayed_history(call: &Value) -> Value {
    let (_, history) = call["input"]
        .as_str()
        .unwrap()
        .split_once('\n')
        .expect("fallback carries complete caller history");
    serde_json::from_str(history).unwrap()
}

#[tokio::test]
async fn resumes_only_the_exact_committed_prefix_for_the_same_account() {
    let fixture = Fixture::new();
    let long_history = "long history ".repeat(2_000);
    let history = user(&long_history);
    fixture
        .send(fixture.pool(), "a", std::slice::from_ref(&history))
        .await
        .unwrap();
    fixture
        .send(
            fixture.pool(),
            "a",
            &[history.clone(), answer(), user("new turn")],
        )
        .await
        .unwrap();
    let calls = fixture.calls();
    assert_eq!(calls[0]["cwd"], calls[1]["cwd"]);
    assert_eq!(calls[1]["input"], "new turn");
    assert_eq!(calls[1]["history"], json!([long_history, "fixture answer"]));

    let prefix = [history, answer(), user("new turn"), answer()];
    let mut pending = prefix.to_vec();
    pending.push(user("terminate before result"));
    let failure = fixture
        .send(fixture.pool(), "a", &pending)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, NativeErrorKind::ProviderFailure);
    assert!(failure.submitted);
    let mut next = prefix.to_vec();
    next.push(user("after termination"));
    fixture.send(fixture.pool(), "a", &next).await.unwrap();
    let calls = fixture.calls();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[2]["input"], "terminate before result");
    assert_eq!(calls[3]["history"], json!([]));
    let mut expected = json!([
        {"role":"user","content":[{"type":"text","text":long_history}]},
        {"role":"assistant","content":[{"type":"text","text":"fixture answer"}]},
        {"role":"user","content":[{"type":"text","text":"new turn"}]},
        {"role":"assistant","content":[{"type":"text","text":"fixture answer"}]},
        {"role":"user","content":[{"type":"text","text":"after termination"}]},
    ]);
    assert_eq!(replayed_history(&calls[3]), expected);

    next.push(answer());
    next.push(user("after changing account"));
    fixture.send(fixture.pool(), "b", &next).await.unwrap();
    expected.as_array_mut().unwrap().extend([
        json!({"role":"assistant","content":[{"type":"text","text":"fixture answer"}]}),
        json!({"role":"user","content":[{"type":"text","text":"after changing account"}]}),
    ]);
    let calls = fixture.calls();
    assert_eq!(calls[4]["history"], json!([]));
    assert_eq!(replayed_history(&calls[4]), expected);

    next[0] = user("edited history");
    next.push(answer());
    next.push(user("after editing"));
    fixture.send(fixture.pool(), "b", &next).await.unwrap();
    expected[0]["content"][0]["text"] = json!("edited history");
    expected.as_array_mut().unwrap().extend([
        json!({"role":"assistant","content":[{"type":"text","text":"fixture answer"}]}),
        json!({"role":"user","content":[{"type":"text","text":"after editing"}]}),
    ]);
    let calls = fixture.calls();
    assert_eq!(calls.len(), 6);
    assert_eq!(calls[5]["history"], json!([]));
    assert_eq!(replayed_history(&calls[5]), expected);
}

#[tokio::test]
async fn rejected_private_resume_falls_back_once_with_complete_caller_history() {
    for mode in ["exit", "initialize"] {
        let fixture = Fixture::new();
        let first = user("previous turn");
        fixture
            .send(fixture.pool(), "a", std::slice::from_ref(&first))
            .await
            .unwrap();
        fs::write(fixture.0.join("reject-resume"), mode).unwrap();

        fixture
            .send(fixture.pool(), "a", &[first, answer(), user("next turn")])
            .await
            .unwrap_or_else(|error| panic!("{mode} rejection must fall back: {error:?}"));
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
