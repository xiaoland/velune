use super::*;
use crate::conversation::{
    InteractionAnswer, InteractionKind, InteractionQuestion, RuntimeInteraction,
    RuntimeInteractionReply,
};
use std::os::unix::fs::PermissionsExt;
use velune_test_support::TempRoot;

struct Fixture {
    root: TempRoot,
    application: crate::Application,
}

impl Fixture {
    fn new(label: &str, model: Option<&str>) -> Self {
        let root = TempRoot::new(&format!("question-{label}"));
        let binary = root.join("codex");
        fs::write(
            &binary,
            r##"#!/usr/bin/env python3
import json, pathlib, sys
root = pathlib.Path(__file__).parent
if '--version' in sys.argv:
    print('codex-cli 0.166.0')
    sys.exit(0)
for line in sys.stdin:
    message = json.loads(line)
    if 'id' not in message:
        continue
    with (root / 'rpc.jsonl').open('a') as log:
        log.write(json.dumps(message) + '\n')
    method, params = message['method'], message.get('params', {})
    result = {}
    if method == 'config/read':
        result = {'config': {'model_provider': 'openai', 'model': 'synthetic-default'}}
    elif method == 'model/list':
        result = {'data': [{'model': model, 'displayName': model,
                  'supportedReasoningEfforts': [{'reasoningEffort': 'high'}]}
                  for model in ['synthetic-default', 'synthetic-current']], 'nextCursor': None}
    elif method in ['thread/read', 'thread/resume']:
        result = {'thread': json.loads((root / 'thread.json').read_text())}
    elif method == 'turn/start':
        if (root / 'reject').exists():
            print(json.dumps({'id': message['id'], 'error': {'code': -32000,
                             'message': 'synthetic rejection'}}), flush=True)
            continue
        result = {'turn': {'id': 'answer-turn'}}
    elif method != 'initialize':
        print(json.dumps({'id': message['id'], 'error': {'code': -32601,
                         'message': 'unexpected synthetic method: ' + method}}), flush=True)
        continue
    print(json.dumps({'id': message['id'], 'result': result}), flush=True)
"##,
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let mut application = crate::Application::open(crate::Options {
            home_directory: root.join("velune"),
            resources_directory: root.to_path_buf(),
        })
        .unwrap();
        application.runtime.gateways.push(GatewayConfig {
            id: "gateway".into(),
            name: "Synthetic".into(),
            providers: vec![],
            routes: vec![],
        });
        let mut runtime = RuntimeInstance {
            enabled: true,
            id: "codex".into(),
            name: "Synthetic".into(),
            type_id: RuntimeKind::Codex166,
            gateway_id: "gateway".into(),
            settings: BTreeMap::from([
                ("binary".into(), binary.to_string_lossy().into_owned()),
                (
                    "agentDir".into(),
                    root.join("native").to_string_lossy().into_owned(),
                ),
            ]),
        };
        runtime
            .set_new_conversation_choice(crate::config::NewConversationChoice::Chosen {
                choice: crate::config::TurnChoice {
                    model: Some(crate::ModelSelection::Native {
                        provider_id: None,
                        model_id: "synthetic-default".into(),
                    }),
                    ..Default::default()
                },
            })
            .unwrap();
        application.runtime.runtime_instances = vec![runtime];
        let mut snapshot = PiProjection::new(ConversationSummary {
            id: "codex:abc-def".into(),
            runtime_id: "codex".into(),
            title: velune_conversation::ConversationTitle::Untitled,
            cwd: Some(root.to_string_lossy().into_owned()),
            created_at_unix_ms: None,
            updated_at_unix_ms: None,
            can_rename: false,
            can_delete: false,
            parent_conversation_id: None,
            native_read_state: None,
        })
        .snapshot
        .unwrap();
        snapshot.messages = serde_json::from_value(json!([
            {"id":"original-user","role":"user","completed":true,"timestamp_unix_ms":null,
             "blocks":[{"kind":"text","text":"Synthetic input"}],
             "turn_settings":{"runtimeId":"codex","model":model,"effort":"high","permissionMode":"full-access"}},
            {"id":"original-answer","role":"assistant","completed":true,"timestamp_unix_ms":null,
             "blocks":[{"kind":"text","text":"Synthetic question"}]}
        ]))
        .unwrap();
        snapshot.pending_interactions.push(RuntimeInteraction {
            id: "codex-async:question".into(),
            kind: InteractionKind::AsyncUserInput {
                delivery: crate::conversation::AsyncQuestionDelivery::ConversationInput,
                questions: vec![InteractionQuestion {
                    id: "0".into(),
                    text: "Synthetic question".into(),
                    options: vec![],
                    secret: false,
                    default_text: None,
                    multiline: false,
                    allow_other: true,
                    multiple: false,
                }],
            },
        });
        fs::write(
            root.join("thread.json"),
            serde_json::to_vec(&json!({
                "id":"abc-def","modelProvider":"openai","historyMode":"legacy",
                "cwd":root,"path":root.join("rollout.jsonl"),
                "turns":[{"id":"original-turn","status":"completed","items":[
                    {"id":"original-user","type":"userMessage","content":[{"type":"text","text":"Synthetic input"}]},
                    {"id":"original-answer","type":"agentMessage","text":"Synthetic question"},
                    {"id":"question","type":"agentMessage","text":"Synthetic question","questions":[{"title":"Synthetic question","options":null}]}
                ]}]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("rollout.jsonl"),
            format!(
                "{}\n",
                json!({"type":"turn_context","payload":{
                    "turn_id":"original-turn","model":model,"effort":"high",
                    "approval_policy":"never","sandbox_policy":{"type":"danger-full-access"}
                }})
            ),
        )
        .unwrap();
        application.runtime.present_history(snapshot, None);
        // Observe the actual native lock admission, without replacing it in
        // the test. This synthetic context has no writer lock.
        for _ in 0..100 {
            if application
                .runtime
                .current_snapshot()
                .unwrap()
                .actions
                .can_send
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            application
                .runtime
                .current_snapshot()
                .unwrap()
                .actions
                .can_send
        );
        assert!(application.runtime.session.model_selection.is_none());
        assert!(
            application
                .runtime
                .history_send_selection("codex")
                .unwrap()
                .is_none()
        );
        Self { root, application }
    }

    fn answer(&mut self) -> Result<crate::SnapshotResult, RuntimeError> {
        self.application.runtime.reply_runtime_interaction(
            "codex",
            "codex-async:question",
            RuntimeInteractionReply::Answers {
                answers: vec![InteractionAnswer {
                    question_id: "0".into(),
                    values: vec!["Synthetic answer".into()],
                }],
            },
        )
    }

    fn requests(&self) -> Vec<Value> {
        fs::read_to_string(self.root.join("rpc.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn add_api_models(&mut self, model_id: &str) {
        for id in ["first", "second"] {
            self.application.runtime.gateways[0].providers.push(
                crate::config::ProviderDefinition {
                    id: id.into(),
                    name: id.into(),
                    protocol: Protocol::Responses,
                    endpoint: "http://127.0.0.1:1/v1".into(),
                    execution_backend: crate::config::ExecutionBackend::Http,
                    request_profile: crate::config::RequestProfile::Native,
                    codex_client_version: None,
                    quota_source: crate::config::ProviderQuotaSource::None,
                    authentication: Default::default(),
                    quota_authentication: Default::default(),
                    billing: Default::default(),
                    models: vec![ProviderModel {
                        record_key: format!("{id}-model"),
                        provider_model_id: model_id.into(),
                        nickname: String::new(),
                        icon: None,
                        context_window: None,
                        max_output_tokens: None,
                        reasoning_levels: None,
                        request_parameters: None,
                        pi_projection: None,
                    }],
                    quota_limits: vec![],
                },
            );
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.application.runtime.shutdown_active().unwrap();
    }
}

#[test]
fn restored_question_answers_with_the_recorded_native_model_without_a_velune_send_record() {
    let mut fixture = Fixture::new("restored", Some("synthetic-current"));
    let answered = fixture.answer().unwrap().snapshot.unwrap();
    assert!(answered.pending_interactions.is_empty());
    let requests = fixture.requests();
    let turns: Vec<_> = requests
        .iter()
        .filter(|record| record["method"] == "turn/start")
        .collect();
    assert_eq!(turns.len(), 1);
    let params = &turns[0]["params"];
    assert_eq!(params["threadId"], "abc-def");
    assert_eq!(params["model"], "synthetic-current");
    assert_eq!(params["effort"], "high");
    assert_eq!(params["approvalPolicy"], "never");
    assert_eq!(
        params["input"][0]["text"],
        "Synthetic question\n回答：Synthetic answer"
    );
    assert!(fixture.answer().is_err());
    assert_eq!(
        fixture
            .requests()
            .iter()
            .filter(|record| record["method"] == "turn/start")
            .count(),
        1
    );
    // The hidden state is durable even when the same native question is read
    // again after reopening the application.
    let runtimes = fixture.application.runtime.runtime_instances.clone();
    let gateways = fixture.application.runtime.gateways.clone();
    let mut restored = fixture.application.runtime.native_snapshot().unwrap();
    restored.pending_interactions.push(RuntimeInteraction {
        id: "codex-async:question".into(),
        kind: InteractionKind::AsyncUserInput {
            delivery: crate::conversation::AsyncQuestionDelivery::ConversationInput,
            questions: vec![],
        },
    });
    fixture.application.runtime.shutdown_active().unwrap();
    let placeholder = crate::Application::open(crate::Options {
        home_directory: fixture.root.join("reopen-placeholder"),
        resources_directory: fixture.root.to_path_buf(),
    })
    .unwrap();
    drop(std::mem::replace(&mut fixture.application, placeholder));
    fixture.application = crate::Application::open(crate::Options {
        home_directory: fixture.root.join("velune"),
        resources_directory: fixture.root.to_path_buf(),
    })
    .unwrap();
    fixture.application.runtime.runtime_instances = runtimes;
    fixture.application.runtime.gateways = gateways;
    fixture.application.runtime.present_history(restored, None);
    assert!(
        fixture
            .application
            .runtime
            .current_snapshot()
            .unwrap()
            .pending_interactions
            .is_empty()
    );
}

#[test]
fn unknown_question_model_does_not_borrow_the_new_conversation_default() {
    let mut fixture = Fixture::new("unknown", None);
    assert!(fixture.answer().is_err());
    assert!(
        fixture
            .requests()
            .iter()
            .all(|record| record["method"] != "turn/start")
    );
    assert_eq!(
        fixture
            .application
            .runtime
            .current_snapshot()
            .unwrap()
            .pending_interactions
            .len(),
        1
    );
}

#[test]
fn rejected_question_answer_keeps_the_question_visible_and_unresolved() {
    let mut fixture = Fixture::new("rejected", Some("synthetic-current"));
    fs::write(fixture.root.join("reject"), "synthetic").unwrap();
    assert!(fixture.answer().is_err());
    assert_eq!(
        fixture
            .application
            .runtime
            .current_snapshot()
            .unwrap()
            .pending_interactions
            .len(),
        1
    );
    assert!(
        fixture
            .application
            .runtime
            .organization
            .conversations
            .iter()
            .all(|marks| marks.resolved_async_questions.is_empty())
    );
    assert_eq!(
        fixture
            .requests()
            .iter()
            .filter(|record| record["method"] == "turn/start")
            .count(),
        1
    );
}

#[test]
fn confirmed_runtime_default_remains_a_valid_question_execution_selection() {
    let mut fixture = Fixture::new("default", None);
    fixture.application.runtime.session.model_selection = Some(None);
    assert!(
        fixture
            .answer()
            .unwrap()
            .snapshot
            .unwrap()
            .pending_interactions
            .is_empty()
    );
    let requests = fixture.requests();
    let turns: Vec<_> = requests
        .iter()
        .filter(|record| record["method"] == "turn/start")
        .collect();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0]["params"]["model"], "synthetic-default");
    assert!(
        requests
            .iter()
            .all(|record| record["method"] != "model/list")
    );
}

#[test]
fn ambiguous_api_model_does_not_choose_a_provider_for_the_question() {
    let mut fixture = Fixture::new("ambiguous", Some("shared-api-model"));
    fixture.add_api_models("shared-api-model");
    assert!(fixture.answer().is_err());
    assert!(
        fixture
            .requests()
            .iter()
            .all(|record| record["method"] != "turn/start")
    );
    assert_eq!(
        fixture
            .application
            .runtime
            .current_snapshot()
            .unwrap()
            .pending_interactions
            .len(),
        1
    );
}

#[test]
fn a_native_catalog_id_cannot_change_a_restored_questions_managed_provider() {
    let mut fixture = Fixture::new("provider-collision", Some("synthetic-current"));
    fixture.add_api_models("synthetic-current");
    let path = fixture.root.join("thread.json");
    let mut thread: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    thread["modelProvider"] = json!("velune");
    fs::write(path, serde_json::to_vec(&thread).unwrap()).unwrap();
    let error = fixture
        .answer()
        .expect_err("provider must remain unresolved");
    assert!(error.to_string().contains("提供商与模型"));
    assert!(fixture.requests().iter().all(|record| {
        !matches!(
            record["method"].as_str(),
            Some("turn/start" | "thread/start" | "thread/fork" | "thread/resume")
        )
    }));
    assert_eq!(
        fixture
            .application
            .runtime
            .current_snapshot()
            .unwrap()
            .pending_interactions
            .len(),
        1
    );
    assert!(
        fixture
            .application
            .runtime
            .organization
            .conversations
            .iter()
            .all(|marks| marks.resolved_async_questions.is_empty())
    );
}
