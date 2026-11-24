//! Messages ingress through Runner, the ACP subprocess, and the local tool bridge.
#![cfg(unix)]

use reqwest::{Client, Response, StatusCode};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};
use velune_ai::{NativeErrorKind, OperationFuture};
use velune_ai_provider::antigravity::{Launch, Process};
use velune_gateway::{
    CredentialMaterial, CredentialRecovery, CredentialResolutionError, CredentialResolver,
    CredentialTarget, ExecutionBackend, ExecutionBackends, GatewayConfig, ModelRoute, Protocol,
    ProviderDefinition, ProviderModel, RequestCapabilities, RequestProfile, ResolvedCredential,
    RoutingStrategy, Runner, SchedulingState, SessionIdentity, UnknownQuotaSource,
};

const GATEWAY_ID: &str = "antigravity-fixture";
const PRIMARY: &str = "primary";
const BACKUP: &str = "backup";
const ROUTE: &str = "subscriptions";
const ALTERNATE_ROUTE: &str = "subscription-models";

struct FixtureLaunch {
    account: &'static str,
    scenario: &'static str,
    cwd: PathBuf,
}

impl Launch for FixtureLaunch {
    fn prepare(&self, loopback_mcp: String) -> OperationFuture<Result<Process, NativeErrorKind>> {
        let cwd = self.cwd.clone();
        let account = self.account;
        let scenario = self.scenario;
        Box::pin(async move {
            fs::File::create(cwd.join("launch")).unwrap();
            let script = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/antigravity/fake_agy.py");
            let mut command = Command::new("python3");
            command
                .env_clear()
                .current_dir(&cwd)
                .args(["-I", "-B"])
                .arg(script)
                .args([account, scenario, "trace.ndjson"]);
            Ok(Process {
                command,
                input_prefix: Vec::new(),
                cwd: cwd.to_str().unwrap().into(),
                mcp_endpoint: loopback_mcp,
                guard: None,
            })
        })
    }
}

struct Fixtures(BTreeMap<String, Arc<FixtureLaunch>>);

impl CredentialResolver for Fixtures {
    fn resolve(
        &self,
        reference: String,
        target: CredentialTarget,
    ) -> OperationFuture<Result<ResolvedCredential, CredentialResolutionError>> {
        assert_eq!(target.execution_backend, ExecutionBackend::AntigravityCli);
        assert_eq!(target.protocol, Protocol::Messages);
        assert!(target.endpoint.is_empty());
        let launch = self.0.get(&reference).cloned();
        Box::pin(async move {
            let launch = launch.ok_or(CredentialResolutionError::Unavailable)?;
            Ok(ResolvedCredential {
                material: CredentialMaterial::AntigravityCli { launch },
                recovery: CredentialRecovery::None,
                request_capabilities: RequestCapabilities::default(),
            })
        })
    }
}

fn provider(account: &str, model: &str) -> ProviderDefinition {
    ProviderDefinition {
        id: account.into(),
        name: account.into(),
        protocol: Protocol::Messages,
        execution_backend: ExecutionBackend::AntigravityCli,
        request_profile: RequestProfile::Native,
        endpoint: String::new(),
        credential_ref: Some(format!("fixture-{account}")),
        models: vec![ProviderModel {
            record_key: account.into(),
            provider_model_id: model.into(),
            nickname: String::new(),
            icon: None,
            context_window: None,
            max_output_tokens: None,
            reasoning_levels: None,
            request_parameters: BTreeMap::new(),
        }],
        quota_limits: Vec::new(),
        codex_client_version: None,
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).unwrap();
        let path = std::env::temp_dir().join(format!(
            "velune-ACP-fixture-{:032x}",
            u128::from_ne_bytes(random)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Gateway {
    runner: Runner,
    backends: ExecutionBackends,
    client: Client,
    session: Option<SessionIdentity>,
    scratch: Scratch,
}

impl Gateway {
    fn start(scenario: &'static str) -> Self {
        Self::start_with_session(scenario, Some("antigravity"), ExecutionBackends::default())
    }

    fn start_with_session(
        scenario: &'static str,
        session_id: Option<&str>,
        backends: ExecutionBackends,
    ) -> Self {
        let scratch = Scratch::new();
        let mut providers = vec![
            provider(PRIMARY, "claude-test-medium"),
            provider(BACKUP, "gemini-4-argon-medium"),
        ];
        let mut alternate_model = providers[0].models[0].clone();
        alternate_model.record_key = "alternate".into();
        alternate_model.provider_model_id = "gemini-test".into();
        providers[0].models.insert(0, alternate_model);
        let fixtures = [PRIMARY, BACKUP]
            .into_iter()
            .map(|account| {
                let cwd = scratch.0.join(account);
                fs::create_dir(&cwd).unwrap();
                (
                    format!("fixture-{account}"),
                    Arc::new(FixtureLaunch {
                        account,
                        scenario: if account == PRIMARY { scenario } else { "text" },
                        cwd,
                    }),
                )
            })
            .collect();
        let config = GatewayConfig {
            id: GATEWAY_ID.into(),
            name: "Antigravity fixture".into(),
            providers,
            routes: [
                (ROUTE, vec![PRIMARY.into(), BACKUP.into()]),
                (ALTERNATE_ROUTE, vec!["alternate".into(), PRIMARY.into(), BACKUP.into()]),
            ]
            .into_iter()
            .map(|(key, members)| ModelRoute {
                record_key: key.into(),
                name: key.into(),
                members,
                strategy: RoutingStrategy::Order,
                sticky: false,
                weights: BTreeMap::new(),
            })
            .collect(),
        };
        let session = session_id.map(|id| SessionIdentity {
            namespace: "tests".into(),
            id: id.into(),
        });
        let runner = Runner::start_with_scheduling(
            config,
            Arc::new(Fixtures(fixtures)),
            BTreeMap::new(),
            None,
            SchedulingState::default(),
            Arc::new(UnknownQuotaSource),
            session.clone(),
            backends.clone(),
        )
        .unwrap();
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();
        Self {
            runner,
            backends,
            client,
            session,
            scratch,
        }
    }

    async fn messages(&self, record_key: &str, messages: Value, tools: &[Value]) -> Response {
        self.client
            .post(format!("{}/messages", self.runner.endpoint()))
            .bearer_auth(self.runner.token())
            .header("anthropic-version", "2023-06-01")
            .json(&json!({
                "model": format!("velune/model/{record_key}"),
                "max_tokens": 64,
                "stream": true,
                "messages": messages,
                "tools": tools,
            }))
            .send()
            .await
            .unwrap()
    }

    fn launched(&self, account: &str) -> bool {
        self.scratch.0.join(account).join("launch").exists()
    }

    fn trace(&self, account: &str) -> Vec<Value> {
        let path = self.scratch.0.join(account).join("trace.ndjson");
        match fs::read_to_string(path) {
            Ok(data) => data
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => panic!("cannot read fake agy trace: {error}"),
        }
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        if let Some(session) = &self.session {
            self.backends.cancel_session(GATEWAY_ID, session);
        }
    }
}

fn user_turn() -> Value {
    json!([{"role": "user", "content": "fixture prompt"}])
}

async fn events(response: Response) -> Vec<Value> {
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    body.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn assert_delta(events: &[Value], text: &str) {
    assert!(
        events
            .iter()
            .any(|event| event["type"] == "content_block_delta" && event["delta"]["text"] == text),
        "{events:?}"
    );
}

#[tokio::test]
async fn two_subscriptions_negotiate_their_own_advertised_models_and_stream() {
    let gateway = Gateway::start("text");
    let primary = events(gateway.messages(ROUTE, user_turn(), &[]).await).await;
    assert_delta(&primary, "primary 中文😀");
    assert!(primary.iter().any(|event| event["type"] == "message_stop"));
    assert!(gateway.trace(BACKUP).is_empty());
    assert!(gateway.trace(PRIMARY).contains(&json!({
        "method": "session/set_config_option", "configId": "model", "value": "claude-test-medium"
    })));

    let backup = events(gateway.messages(BACKUP, user_turn(), &[]).await).await;
    assert_delta(&backup, "backup 中文😀");
    assert!(backup.iter().any(|event| event["type"] == "message_stop"));
    assert!(gateway.trace(BACKUP).contains(&json!({
        "method": "session/set_model", "modelId": "gemini-4-argon-medium"
    })));
}

#[tokio::test]
async fn authentication_and_disconnect_after_prompt_stop_subscription_failover() {
    for (scenario, expected, submitted) in [
        ("auth", StatusCode::UNAUTHORIZED, false),
        ("prompt_exit", StatusCode::BAD_GATEWAY, true),
    ] {
        let gateway = Gateway::start(scenario);
        let response = gateway.messages(ROUTE, user_turn(), &[]).await;
        assert_eq!(
            response.status(),
            expected,
            "{}",
            response.text().await.unwrap()
        );
        let primary = gateway.trace(PRIMARY);
        assert!(primary.iter().any(|event| event["method"] == "session/new"));
        assert_eq!(
            primary
                .iter()
                .any(|event| event["method"] == "session/prompt"),
            submitted
        );
        assert!(gateway.trace(BACKUP).is_empty());
    }
}

#[tokio::test]
async fn aggregate_route_uses_another_subscription_after_a_startup_disconnect() {
    let gateway = Gateway::start("startup_exit");
    let reply = events(gateway.messages(ROUTE, user_turn(), &[]).await).await;
    assert_delta(&reply, "backup 中文😀");
    assert_eq!(gateway.trace(PRIMARY), [json!({"method": "initialize"})]);
    assert!(
        gateway
            .trace(BACKUP)
            .iter()
            .any(|event| event["method"] == "session/prompt")
    );
}

fn echo_tool() -> [Value; 1] {
    [json!({
        "name": "echo",
        "description": "Echo the test input",
        "input_schema": {
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"]
        }
    })]
}

async fn request_tool_result(gateway: &Gateway, tools: &[Value]) -> Value {
    let first = events(gateway.messages(ROUTE, user_turn(), tools).await).await;
    let block = first
        .iter()
        .find_map(|event| {
            let block = &event["content_block"];
            (event["type"] == "content_block_start" && block["type"] == "tool_use").then_some(block)
        })
        .unwrap_or_else(|| panic!("missing tool use: {first:?}"));
    assert_eq!(block["name"], "echo");
    let input_json: String = first
        .iter()
        .filter_map(|event| event["delta"]["partial_json"].as_str())
        .collect();
    let tool_input: Value = serde_json::from_str(&input_json).unwrap();
    assert_eq!(tool_input, json!({"text": "probe"}));
    assert!(
        first
            .iter()
            .any(|event| event["delta"]["stop_reason"] == "tool_use")
    );
    let tool_id = block["id"].as_str().unwrap();
    json!([
        {"role": "user", "content": "fixture prompt"},
        {"role": "assistant", "content": [
            {"type": "tool_use", "id": tool_id, "name": "echo", "input": tool_input}
        ]},
        {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": tool_id, "content": "fixture-result"}
        ]}
    ])
}

#[tokio::test]
async fn tool_result_resumes_the_same_acp_prompt_through_the_aggregate_route() {
    let gateway = Gateway::start("tool");
    let tools = echo_tool();
    let resumed = request_tool_result(&gateway, &tools).await;
    let final_turn = events(gateway.messages(ALTERNATE_ROUTE, resumed, &tools).await).await;
    assert_delta(&final_turn, "After tools. fixture-result");
    assert!(
        final_turn
            .iter()
            .any(|event| event["delta"]["stop_reason"] == "end_turn")
    );
    assert_eq!(
        gateway
            .trace(PRIMARY)
            .iter()
            .filter(|event| event["method"] == "session/prompt")
            .count(),
        1
    );
    assert!(gateway.trace(BACKUP).is_empty());
}

#[tokio::test]
async fn cancelled_or_foreign_tool_results_never_restart_a_subscription() {
    let gateway = Gateway::start("tool");
    let tools = echo_tool();
    let result = request_tool_result(&gateway, &tools).await;

    let foreign_session =
        Gateway::start_with_session("text", Some("different-session"), gateway.backends.clone());
    let no_session = Gateway::start_with_session("text", None, gateway.backends.clone());
    for foreign in [&foreign_session, &no_session] {
        let response = foreign.messages(ROUTE, result.clone(), &tools).await;
        assert_eq!(
            response.status(),
            StatusCode::CONFLICT,
            "session {:?}: {}",
            foreign.session.as_ref().map(|session| &session.id),
            response.text().await.unwrap()
        );
        assert!(!foreign.launched(PRIMARY));
        assert!(!foreign.launched(BACKUP));
    }

    gateway
        .backends
        .cancel_session(GATEWAY_ID, gateway.session.as_ref().unwrap());
    let response = gateway.messages(ROUTE, result, &tools).await;
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "{}",
        response.text().await.unwrap()
    );
    assert!(!gateway.launched(BACKUP));
    let methods = gateway.trace(PRIMARY);
    assert_eq!(
        methods
            .iter()
            .filter(|event| event["method"] == "initialize")
            .count(),
        1
    );
    assert_eq!(
        methods
            .iter()
            .filter(|event| event["method"] == "session/prompt")
            .count(),
        1
    );
}

#[tokio::test]
async fn chat_completions_streams_through_the_messages_backend() {
    let gateway = Gateway::start("text");
    let response = gateway
        .client
        .post(format!("{}/chat/completions", gateway.runner.endpoint()))
        .bearer_auth(gateway.runner.token())
        .json(&json!({
            "model": format!("velune/model/{PRIMARY}"),
            "max_tokens": 64,
            "stream": true,
            "messages": user_turn(),
        }))
        .send()
        .await
        .unwrap();

    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    let chunks: Vec<Value> = body
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str(data).unwrap())
        .collect();
    let content: String = chunks
        .iter()
        .filter_map(|chunk| chunk["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert_eq!(content, "primary 中文😀");
    assert!(
        chunks
            .iter()
            .any(|chunk| chunk["choices"][0]["finish_reason"] == "stop")
    );
    assert!(body.lines().any(|line| line == "data: [DONE]"), "{body}");
}
