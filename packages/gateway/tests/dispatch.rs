//! Native dispatch through a running gateway against a scripted local upstream:
//! what reaches the caller, and which failures move on to the next route member.
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Request, Response},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use velune_gateway::{
    AnalyticsOutcome, AnalyticsRecord, AnalyticsSink, CredentialMaterial, CredentialRecovery,
    CredentialResolutionError, CredentialResolver, CredentialTarget, ExecutionBackend,
    GatewayConfig, HttpAuthorization, ModelRoute, Protocol, ProviderDefinition, ProviderModel,
    QuotaLimit, QuotaSnapshot, QuotaSource, QuotaWindow, RequestCapabilities, RequestProfile,
    ResolvedCredential, RoutingStrategy, Runner, SchedulingState,
};

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}
impl Reply {
    fn json(status: u16, body: Value) -> Self {
        Self::json_bytes(status, &body.to_string())
    }
    fn json_bytes(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("content-type", "application/json".into())],
            body: body.into(),
        }
    }
    fn stream(body: &str) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type", "text/event-stream".into())],
            body: body.into(),
        }
    }
    fn header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

/// Replies by the first path segment, which names the provider.
#[derive(Clone, Default)]
struct Upstream {
    replies: Arc<Mutex<BTreeMap<String, Reply>>>,
    hits: Arc<Mutex<Vec<String>>>,
    requests: Arc<Mutex<Vec<Value>>>,
    request_headers: Arc<Mutex<Vec<HeaderMap>>>,
}
impl Upstream {
    async fn start() -> (Self, String) {
        let upstream = Self::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .fallback(Self::handle)
            .with_state(upstream.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (upstream, address)
    }
    fn reply(&self, provider: &str, reply: Reply) {
        self.replies.lock().unwrap().insert(provider.into(), reply);
    }
    fn hits(&self) -> Vec<String> {
        self.hits.lock().unwrap().clone()
    }
    async fn handle(State(upstream): State<Self>, request: Request<Body>) -> Response<Body> {
        let path = request.uri().path().to_owned();
        let provider = path.split('/').nth(1).unwrap_or_default().to_owned();
        upstream
            .hits
            .lock()
            .unwrap()
            .push(request.uri().to_string());
        upstream
            .request_headers
            .lock()
            .unwrap()
            .push(request.headers().clone());
        let bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
            .await
            .unwrap();
        upstream
            .requests
            .lock()
            .unwrap()
            .push(serde_json::from_slice(&bytes).unwrap());
        let reply = upstream.replies.lock().unwrap().get(&provider).cloned();
        let reply = reply.unwrap_or_else(|| Reply::json(404, json!({"error": "unscripted"})));
        let mut response = Response::builder().status(reply.status);
        for (name, value) in reply.headers {
            response = response.header(name, value);
        }
        response.body(Body::from(reply.body)).unwrap()
    }
}

struct Keys;
impl CredentialResolver for Keys {
    fn resolve(
        &self,
        _reference: String,
        target: CredentialTarget,
    ) -> velune_ai::OperationFuture<Result<ResolvedCredential, CredentialResolutionError>> {
        Box::pin(async move {
            Ok(ResolvedCredential {
                material: CredentialMaterial::Http {
                    authorization: if target.protocol == Protocol::Gemini {
                        HttpAuthorization::ApiKey("synthetic-gemini-key".into())
                    } else {
                        HttpAuthorization::Bearer("synthetic-key".into())
                    },
                    account_id: None,
                },
                recovery: CredentialRecovery::None,
                request_capabilities: RequestCapabilities::default(),
            })
        })
    }
}

#[derive(Default)]
struct Collect {
    records: Mutex<Vec<AnalyticsRecord>>,
}

impl AnalyticsSink for Collect {
    fn record(&self, record: AnalyticsRecord) {
        self.records.lock().unwrap().push(record);
    }
}

const GEMINI_TEXT: &str = include_str!("fixtures/gemini/google-text.json");
const GEMINI_TEXT_CHUNKS: &str = include_str!("fixtures/gemini/google-text.chunks.txt");
const GEMINI_TOOL: &str = include_str!("fixtures/gemini/google-tool-call-gemini3.json");
const GEMINI_TOOL_CHUNKS: &str =
    include_str!("fixtures/gemini/google-tool-call-gemini3.chunks.txt");
const GEMINI_QUOTA: &str = include_str!("fixtures/gemini/google-429-retry-info.json");

fn gemini_gateway(upstream: &str, analytics: Option<Arc<dyn AnalyticsSink>>) -> Runner {
    let mut config = gateway_config(Api::Chat, upstream);
    let primary = &mut config.providers[0];
    primary.protocol = Protocol::Gemini;
    primary.endpoint = format!("{upstream}/a/v1beta");
    primary.models[0].provider_model_id = "gemini-3-pro-preview".into();
    Runner::start_with_analytics(
        config,
        Arc::new(Keys),
        BTreeMap::new(),
        analytics,
        SchedulingState::default(),
    )
    .unwrap()
}

fn gemini_call(runner: &Runner, suffix: &str) -> reqwest::RequestBuilder {
    let gateway = runner.endpoint().strip_suffix("/v1").unwrap();
    reqwest::Client::new().post(format!("{gateway}/v1beta/models/{suffix}"))
}

fn replay_sse(chunks: &str) -> String {
    chunks
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| format!("data: {line}\n\n"))
        .collect()
}

#[tokio::test]
async fn gemini_native_routes_url_alias_and_preserves_signed_history_and_stream() {
    let (upstream, address) = Upstream::start().await;
    upstream.reply("a", Reply::json_bytes(200, GEMINI_TOOL));
    let analytics = Arc::new(Collect::default());
    let runner = gemini_gateway(&address, Some(analytics.clone()));
    let tool_turn = json!({
        "contents": [{"role": "user", "parts": [{"text": "What's the weather in San Francisco?"}]}],
        "tools": [{"functionDeclarations": [{"name":"weather","parameters":{"type":"OBJECT","properties":{"location":{"type":"STRING"}}}}]}],
    });
    let response = gemini_call(&runner, "velune%2Fmodel%2Fa-model:generateContent")
        .header("x-goog-api-key", runner.token())
        .json(&tool_turn)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), GEMINI_TOOL);

    let recorded: Value = serde_json::from_str(GEMINI_TOOL).unwrap();
    let resumed = json!({
        "contents": [
            tool_turn["contents"][0],
            recorded["candidates"][0]["content"],
            {"role":"user","parts":[{"functionResponse":{"name":"weather","response":{"temperature":22}}}]},
        ],
        "tools": tool_turn["tools"],
        "cachedContent": "cachedContents/synthetic",
    });
    let stream = replay_sse(GEMINI_TEXT_CHUNKS);
    upstream.reply("a", Reply::stream(&stream));
    let response = gemini_call(
        &runner,
        "velune/model/a-model:streamGenerateContent?alt=sse",
    )
    .bearer_auth(runner.token())
    .header("x-goog-api-key", "must-not-be-forwarded")
    .json(&resumed)
    .send()
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(response.text().await.unwrap(), stream);

    for field in [
        "cachedContent",
        "cached_content",
        "continuationToken",
        "continuation_token",
    ] {
        let mut stateful = tool_turn.clone();
        stateful[field] = json!("synthetic-upstream-state");
        for model in ["route", "b-model"] {
            let denied = gemini_call(&runner, &format!("velune/model/{model}:generateContent"))
                .header("x-goog-api-key", runner.token())
                .json(&stateful)
                .send()
                .await
                .unwrap();
            assert_eq!(
                denied.status(),
                400,
                "{field} must stay with one Gemini target"
            );
        }
    }
    let unauthorized = gemini_call(&runner, "velune/model/a-model:generateContent")
        .header("x-api-key", runner.token())
        .json(&tool_turn)
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), 401);

    assert_eq!(
        upstream.hits(),
        [
            "/a/v1beta/models/gemini-3-pro-preview:generateContent",
            "/a/v1beta/models/gemini-3-pro-preview:streamGenerateContent?alt=sse",
        ],
    );
    assert_eq!(*upstream.requests.lock().unwrap(), [tool_turn, resumed]);
    for headers in upstream.request_headers.lock().unwrap().iter() {
        assert_eq!(headers["x-goog-api-key"], "synthetic-gemini-key");
        assert!(!headers.contains_key("authorization"));
    }
    let records = analytics.records.lock().unwrap();
    assert_eq!(records[0].protocol, Protocol::Gemini);
    assert_eq!(records[0].outcome, AnalyticsOutcome::Completed);
    assert_eq!(records[0].usage.input, Some(29));
    assert_eq!(records[0].usage.output, Some(1816));
    assert_eq!(records[0].usage.reasoning_output, Some(1801));
}

#[tokio::test]
async fn gemini_upstream_converts_responses_requests_and_replays_recorded_outputs() {
    let (upstream, address) = Upstream::start().await;
    upstream.reply("a", Reply::json_bytes(200, GEMINI_TEXT));
    let runner = gemini_gateway(&address, None);
    let responses_call = |stream| {
        reqwest::Client::new()
            .post(format!("{}/responses", runner.endpoint()))
            .bearer_auth(runner.token())
            .json(&json!({
                "model": "velune/model/a-model",
                "input": [{"role":"user","content":"What's the weather in San Francisco?"}],
                "tools": [{"type":"function","name":"weather","parameters":{"type":"object","properties":{"location":{"type":"string"}}}}],
                "max_output_tokens": 4096,
                "stream": stream,
            }))
    };
    let response = responses_call(false).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let answer = response.json::<Value>().await.unwrap();
    assert_eq!(answer["status"], "completed");
    assert!(answer.to_string().contains("strawberry"));

    upstream.reply("a", Reply::stream(&replay_sse(GEMINI_TOOL_CHUNKS)));
    let response = responses_call(true).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let answer = response.text().await.unwrap();
    assert!(answer.contains("response.completed"), "{answer}");
    assert!(answer.contains("function_call"), "{answer}");
    assert!(answer.contains("San Francisco"), "{answer}");

    upstream.reply("a", Reply::json_bytes(429, GEMINI_QUOTA));
    let response = responses_call(false).send().await.unwrap();
    assert_eq!(response.status(), 429);
    assert_eq!(response.text().await.unwrap(), GEMINI_QUOTA);

    let requests = upstream.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    for body in requests.iter() {
        assert!(body.get("model").is_none());
        assert!(body.get("stream").is_none());
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 4096);
        assert_eq!(
            body["tools"][0]["functionDeclarations"][0]["name"],
            "weather"
        );
    }
}

#[derive(Clone, Copy)]
enum Api {
    Chat,
    Responses,
    Messages,
}
impl Api {
    const ALL: [Self; 3] = [Self::Chat, Self::Responses, Self::Messages];
    fn gateway(self) -> velune_gateway::Protocol {
        match self {
            Self::Chat => velune_gateway::Protocol::ChatCompletions,
            Self::Responses => velune_gateway::Protocol::Responses,
            Self::Messages => velune_gateway::Protocol::Messages,
        }
    }
    /// The provider endpoint below the upstream's provider segment.
    fn base(self) -> &'static str {
        match self {
            Self::Chat | Self::Responses => "/v1",
            Self::Messages => "",
        }
    }
    fn path(self) -> &'static str {
        match self {
            Self::Chat => "/chat/completions",
            Self::Responses => "/responses",
            Self::Messages => "/messages",
        }
    }
    fn success(self) -> Value {
        match self {
            Self::Chat => json!({"id": "chat", "choices": []}),
            Self::Responses => json!({"id": "resp", "status": "completed", "output": []}),
            Self::Messages => json!({"id": "msg", "type": "message", "content": []}),
        }
    }
    fn events(self) -> &'static str {
        match self {
            Self::Chat => "data: {\"choices\":[]}\n\ndata: [DONE]\n\n",
            Self::Responses => {
                "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n"
            }
            Self::Messages => "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        }
    }
}

fn provider(id: &str, protocol: Api, upstream: &str) -> ProviderDefinition {
    ProviderDefinition {
        id: id.into(),
        name: id.into(),
        protocol: protocol.gateway(),
        execution_backend: ExecutionBackend::Http,
        request_profile: RequestProfile::Native,
        endpoint: format!("{upstream}/{id}{}", protocol.base()),
        credential_ref: None,
        models: vec![ProviderModel {
            record_key: format!("{id}-model"),
            provider_model_id: format!("{id}-native"),
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

/// A gateway with providers `a` and `b` and a route trying `a` first.
fn gateway(protocol: Api, upstream: &str) -> Runner {
    Runner::start(
        gateway_config(protocol, upstream),
        Arc::new(Keys),
        BTreeMap::new(),
    )
    .unwrap()
}

struct Answer {
    status: u16,
    headers: reqwest::header::HeaderMap,
    body: String,
}

async fn send(runner: &Runner, protocol: Api, model: &str, stream: bool) -> Answer {
    let response = reqwest::Client::new()
        .post(format!("{}{}", runner.endpoint(), protocol.path()))
        .bearer_auth(runner.token())
        .header("anthropic-version", "2023-06-01")
        .json(&json!({"model": model, "stream": stream, "max_tokens": 16, "messages": []}))
        .send()
        .await
        .unwrap();
    Answer {
        status: response.status().as_u16(),
        headers: response.headers().clone(),
        body: response.text().await.unwrap(),
    }
}

#[derive(Clone)]
struct ObservedQuota(QuotaSnapshot);
impl QuotaSource for ObservedQuota {
    fn snapshot(&self, _: &str) -> QuotaSnapshot {
        self.0.clone()
    }
}

fn quota_window(reset_at_ms: Option<i64>) -> QuotaWindow {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    QuotaWindow {
        id: "daily".into(),
        model_ids: None,
        used_percent: 100.0,
        reset_at_ms,
        window_ms: None,
        observed_at_ms: now,
        expires_at_ms: now + 60_000,
    }
}

#[tokio::test]
async fn pending_hard_cap_waits_before_messages_to_responses_dispatch_and_never_replays() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;
    struct PendingQuota {
        ready: AtomicBool,
        reads: AtomicUsize,
    }
    impl QuotaSource for PendingQuota {
        fn snapshot(&self, _: &str) -> QuotaSnapshot {
            self.reads.fetch_add(1, Ordering::Relaxed);
            if !self.ready.load(Ordering::Acquire) {
                return QuotaSnapshot::Unknown;
            }
            let mut window = quota_window(None);
            window.used_percent = 0.0;
            QuotaSnapshot::Known {
                available: Some(true),
                windows: vec![window],
            }
        }
    }
    let (upstream, address) = Upstream::start().await;
    upstream.reply("a", Reply::json(200, Api::Responses.success()));
    let mut config = gateway_config(Api::Responses, &address);
    config.providers[0].quota_limits = vec![QuotaLimit {
        window_id: "daily".into(),
        max_used_percent: 80,
    }];
    let quota = Arc::new(PendingQuota {
        ready: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
    });
    let runner = Arc::new(
        Runner::start_with_scheduling(
            config,
            Arc::new(Keys),
            BTreeMap::new(),
            None,
            SchedulingState::default(),
            quota.clone(),
            None,
            Default::default(),
        )
        .unwrap(),
    );
    let request_runner = runner.clone();
    let request = tokio::spawn(async move {
        reqwest::Client::new()
            .post(format!("{}/messages", request_runner.endpoint()))
            .bearer_auth(request_runner.token())
            .header("anthropic-version", "2023-06-01")
            .json(&json!({"model":"velune/model/a-model","max_tokens":16,
                "messages":[{"role":"user","content":"synthetic input"}]}))
            .send()
            .await
            .unwrap()
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while quota.reads.load(Ordering::Acquire) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        upstream.hits().is_empty(),
        "pending quota must not dispatch upstream"
    );
    assert!(
        !request.is_finished(),
        "cold quota must wait before rejecting"
    );
    quota.ready.store(true, Ordering::Release);
    let response = tokio::time::timeout(Duration::from_secs(2), request)
        .await
        .unwrap()
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status.as_u16(), 200, "{body}");
    assert_eq!(upstream.hits(), vec!["/a/v1/responses"]);
    let requests = upstream.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "only predispatch admission may be retried"
    );
    assert_eq!(requests[0]["model"], "a-native");
    assert!(requests[0]["input"].is_array());
}

#[tokio::test]
async fn quota_admission_reports_the_reason_across_all_protocols() {
    let mut checks = Vec::new();
    for protocol in Api::ALL {
        let (upstream, address) = Upstream::start().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let cases = [
            (
                QuotaSnapshot::Known {
                    available: Some(false),
                    windows: vec![quota_window(Some(now + 60_000))],
                },
                true,
                429,
                "quota_exhausted",
                Some("60"),
            ),
            (
                QuotaSnapshot::Known {
                    available: Some(false),
                    windows: vec![quota_window(None)],
                },
                true,
                429,
                "quota_exhausted",
                None,
            ),
            (
                QuotaSnapshot::Unknown,
                true,
                503,
                "quota_pending",
                Some("1"),
            ),
            (
                QuotaSnapshot::Stale {
                    available: Some(false),
                    windows: vec![quota_window(Some(now + 60_000))],
                },
                true,
                503,
                "quota_pending",
                Some("1"),
            ),
            (
                QuotaSnapshot::Unsupported,
                true,
                503,
                "quota_window_missing",
                None,
            ),
        ];
        for (snapshot, limited, status, code, retry) in cases {
            let upstream = upstream.clone();
            let address = address.clone();
            checks.push(async move {
                let mut config = gateway_config(protocol, &address);
                if limited {
                    for provider in &mut config.providers {
                        provider.quota_limits = vec![QuotaLimit {
                            window_id: "daily".into(),
                            max_used_percent: 80,
                        }];
                    }
                }
                let runner = Runner::start_with_scheduling(
                    config,
                    Arc::new(Keys),
                    BTreeMap::new(),
                    None,
                    SchedulingState::default(),
                    Arc::new(ObservedQuota(snapshot)),
                    None,
                    Default::default(),
                )
                .unwrap();
                // Each row owns its runner and scheduling state. The two request
                // shapes share only that immutable quota observation, so their
                // real five-second cold-admission intervals can overlap.
                let answers = tokio::time::timeout(std::time::Duration::from_secs(15), async {
                    tokio::join!(
                        send(&runner, protocol, "velune/model/route", false),
                        send(&runner, protocol, "velune/model/route", true),
                    )
                })
                .await
                .unwrap_or_else(|_| panic!("{} {code}: admission did not finish", protocol.path()));
                for (stream, answer) in [(false, answers.0), (true, answers.1)] {
                    assert_eq!(
                        answer.status,
                        status,
                        "{} {code}, stream={stream}",
                        protocol.path()
                    );
                    let actual_retry = answer
                        .headers
                        .get("retry-after")
                        .map(|h| h.to_str().unwrap());
                    if retry == Some("60") {
                        // Wall time elapses while the runners start; the delay
                        // must still point at the same reset.
                        assert!((1..=60).contains(&actual_retry.unwrap().parse::<u64>().unwrap()));
                    } else {
                        assert_eq!(actual_retry, retry);
                    }
                    let body: Value = serde_json::from_str(&answer.body).unwrap();
                    assert_eq!(body["error"]["code"], code);
                    assert!(!body["error"]["message"].as_str().unwrap().is_empty());
                }
                assert!(upstream.hits().is_empty());
            });
        }
    }
    // Unknown and stale quota each exhaust the production admission interval;
    // preserve every protocol/stream row without paying those waits serially.
    futures_util::future::join_all(checks).await;
}

#[tokio::test]
async fn health_and_invalidated_bindings_remain_service_unavailable() {
    for protocol in Api::ALL {
        let (upstream, address) = Upstream::start().await;
        upstream.reply("a", Reply::json(500, json!({"error":"temporary"})));
        let health = SchedulingState::default();
        let runner = Runner::start_with_analytics(
            gateway_config(protocol, &address),
            Arc::new(Keys),
            BTreeMap::new(),
            None,
            health.clone(),
        )
        .unwrap();
        assert_eq!(
            send(&runner, protocol, "velune/model/a-model", false)
                .await
                .status,
            500
        );
        let answer = send(&runner, protocol, "velune/model/a-model", false).await;
        assert_eq!(answer.status, 503);
        assert_eq!(
            serde_json::from_str::<Value>(&answer.body).unwrap()["error"]["code"],
            "provider_unavailable"
        );
        assert!(answer.headers.contains_key("retry-after"));
        health.invalidate_provider("gateway", "a");
        let answer = send(&runner, protocol, "velune/model/a-model", false).await;
        assert_eq!(answer.status, 503);
        assert_eq!(
            serde_json::from_str::<Value>(&answer.body).unwrap()["error"]["code"],
            "binding_invalidated"
        );
        assert!(!answer.headers.contains_key("retry-after"));
        assert_eq!(upstream.hits().len(), 1);
    }
}

#[tokio::test]
async fn quota_change_between_ranking_and_acquiring_keeps_the_same_error_contract() {
    struct ChangedQuota(std::sync::atomic::AtomicUsize);
    impl QuotaSource for ChangedQuota {
        fn snapshot(&self, _: &str) -> QuotaSnapshot {
            let mut window = quota_window(None);
            window.used_percent = if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                10.0
            } else {
                80.0
            };
            QuotaSnapshot::Known {
                available: Some(false),
                windows: vec![window],
            }
        }
    }
    for protocol in Api::ALL {
        let (upstream, address) = Upstream::start().await;
        let mut config = gateway_config(protocol, &address);
        config.providers[0].quota_limits = vec![QuotaLimit {
            window_id: "daily".into(),
            max_used_percent: 80,
        }];
        let runner = Runner::start_with_scheduling(
            config,
            Arc::new(Keys),
            BTreeMap::new(),
            None,
            SchedulingState::default(),
            Arc::new(ChangedQuota(std::sync::atomic::AtomicUsize::new(0))),
            None,
            Default::default(),
        )
        .unwrap();
        let answer = send(&runner, protocol, "velune/model/a-model", true).await;
        assert_eq!(answer.status, 429);
        let body: Value = serde_json::from_str(&answer.body).unwrap();
        assert_eq!(body["error"]["code"], "quota_exhausted");
        assert!(!answer.headers.contains_key("retry-after"));
        assert!(upstream.hits().is_empty());
    }
}

#[tokio::test]
async fn automatic_quota_observations_do_not_block_upstream_requests() {
    for protocol in Api::ALL {
        let (upstream, address) = Upstream::start().await;
        let mut weekly = quota_window(None);
        weekly.id = "window:604800".into();
        weekly.window_ms = Some(604_800_000);
        for snapshot in [
            QuotaSnapshot::Known {
                available: Some(false),
                windows: vec![weekly.clone()],
            },
            QuotaSnapshot::Known {
                available: Some(true),
                windows: vec![weekly.clone()],
            },
            QuotaSnapshot::Known {
                available: Some(false),
                windows: vec![],
            },
            QuotaSnapshot::Stale {
                available: Some(false),
                windows: vec![weekly.clone()],
            },
        ] {
            upstream.reply("a", Reply::json(200, protocol.success()));
            let runner = Runner::start_with_scheduling(
                gateway_config(protocol, &address),
                Arc::new(Keys),
                BTreeMap::new(),
                None,
                SchedulingState::default(),
                Arc::new(ObservedQuota(snapshot)),
                None,
                Default::default(),
            )
            .unwrap();
            let before = upstream.hits().len();
            for model in ["velune/model/a-model", "velune/model/route"] {
                assert_eq!(send(&runner, protocol, model, false).await.status, 200);
            }
            assert_eq!(upstream.hits().len(), before + 2);
            // Actual upstream rejection remains authoritative, with its body
            // and retry hint preserved; advisory observations do not replace it.
            upstream.reply(
                "a",
                Reply::json(429, json!({"error":{"message":"upstream usage limit"}}))
                    .header("retry-after", "17"),
            );
            let answer = send(&runner, protocol, "velune/model/a-model", false).await;
            assert_eq!(answer.status, 429);
            assert_eq!(answer.headers["retry-after"], "17");
            assert_eq!(
                serde_json::from_str::<Value>(&answer.body).unwrap()["error"]["message"],
                "upstream usage limit"
            );
            assert_eq!(upstream.hits().len(), before + 3);
        }
    }
}

#[tokio::test]
async fn admission_rejections_are_recorded_in_analytics() {
    let protocol = Api::Chat;
    let (_upstream, address) = Upstream::start().await;
    let analytics = Arc::new(Collect::default());
    let runner = Runner::start_with_analytics(
        gateway_config(protocol, &address),
        Arc::new(Keys),
        BTreeMap::new(),
        Some(analytics.clone()),
        SchedulingState::default(),
    )
    .unwrap();
    let client = reqwest::Client::new();
    let url = format!("{}{}", runner.endpoint(), protocol.path());
    let post = |body: String| {
        client
            .post(&url)
            .bearer_auth(runner.token())
            .header("content-type", "application/json")
            .body(body)
    };
    // Bodies rejected before route resolution carry no provider identity.
    for body in [
        "{ not json".to_owned(),
        json!({}).to_string(),
        json!({"model": "velune/model/absent"}).to_string(),
    ] {
        let response = post(body).send().await.unwrap();
        assert_eq!(response.status(), 400);
    }
    // Rejections after route resolution record the route's first member.
    let response =
        post(json!({"model": "velune/model/route", "stream": "yes", "messages": []}).to_string())
            .send()
            .await
            .unwrap();
    assert_eq!(response.status(), 400);
    // Unauthenticated requests are not usage and stay out of analytics.
    let response = client
        .post(&url)
        .header("content-type", "application/json")
        .body("{ not json")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);

    let records = analytics.records.lock().unwrap();
    assert_eq!(records.len(), 4);
    assert!(records.iter().all(|record| {
        record.outcome == AnalyticsOutcome::Rejected
            && record.status == Some(400)
            && record.protocol == velune_gateway::Protocol::ChatCompletions
    }));
    assert!(
        records[..3]
            .iter()
            .all(|record| record.provider_id.is_empty() && record.model_record_key.is_empty())
    );
    assert_eq!(records[3].provider_id, "a");
    assert_eq!(records[3].model_record_key, "a-model");
    assert_eq!(records[3].provider_model_id, "a-native");
    let ids: std::collections::BTreeSet<_> =
        records.iter().map(|record| &record.request_id).collect();
    assert_eq!(ids.len(), 4);
}

#[tokio::test]
async fn request_parameters_merge_into_the_upstream_body() {
    let protocol = Api::Chat;
    let (upstream, address) = Upstream::start().await;
    let mut config = gateway_config(protocol, &address);
    config.providers[0]
        .models
        .iter_mut()
        .find(|model| model.record_key == "a-model")
        .unwrap()
        .request_parameters
        .insert("service_tier".into(), "priority".into());
    upstream.reply("a", Reply::json(200, protocol.success()));
    let runner = Runner::start(config, Arc::new(Keys), BTreeMap::new()).unwrap();
    let answer = send(&runner, protocol, "velune/model/a-model", false).await;
    assert_eq!(answer.status, 200);
    let requests = upstream.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["service_tier"], "priority");
}

#[tokio::test]
async fn delivers_upstream_json_and_streams_unchanged() {
    for protocol in Api::ALL {
        let (upstream, address) = Upstream::start().await;
        let runner = gateway(protocol, &address);
        upstream.reply("a", Reply::json(200, protocol.success()));
        let answer = send(&runner, protocol, "velune/model/a-model", false).await;
        assert_eq!(answer.status, 200);
        assert_eq!(
            serde_json::from_str::<Value>(&answer.body).unwrap(),
            protocol.success()
        );
        upstream.reply("a", Reply::stream(protocol.events()));
        let answer = send(&runner, protocol, "velune/model/a-model", true).await;
        assert_eq!(answer.status, 200);
        assert_eq!(answer.headers["content-type"], "text/event-stream");
        assert_eq!(answer.body, protocol.events());
        assert_eq!(
            upstream.hits(),
            vec![format!("/a/v1{}", protocol.path()); 2]
        );
    }
}

#[tokio::test]
async fn retryable_failures_move_to_the_next_member() {
    for protocol in Api::ALL {
        for failure in [
            Reply::json(429, json!({"error": {"message": "slow down"}})).header("retry-after", "7"),
            Reply::json(500, json!({"error": {"message": "broken"}})),
            Reply::json(529, json!({"error": {"message": "overloaded"}})),
        ] {
            let (upstream, address) = Upstream::start().await;
            let runner = gateway(protocol, &address);
            upstream.reply("a", failure);
            upstream.reply("b", Reply::json(200, protocol.success()));
            let answer = send(&runner, protocol, "velune/model/route", false).await;
            assert_eq!(answer.status, 200);
            assert_eq!(
                serde_json::from_str::<Value>(&answer.body).unwrap(),
                protocol.success()
            );
            assert_eq!(upstream.hits().len(), 2);
        }
    }
}

#[tokio::test]
async fn forbidden_member_fails_over_and_preserves_the_final_refusal() {
    for protocol in Api::ALL {
        let (upstream, address) = Upstream::start().await;
        let runner = gateway(protocol, &address);
        let first_error = json!({"error": {"message": "first member refused"}});
        let final_error = json!({"error": {"message": "last member refused"}});
        upstream.reply("a", Reply::json(403, first_error.clone()));
        upstream.reply("b", Reply::stream(protocol.events()));

        let answer = send(&runner, protocol, "velune/model/route", true).await;
        assert_eq!(answer.status, 200);
        assert_eq!(answer.body, protocol.events());

        let direct = send(&runner, protocol, "velune/model/a-model", false).await;
        assert_eq!(direct.status, 403);
        assert_eq!(
            serde_json::from_str::<Value>(&direct.body).unwrap(),
            first_error
        );
        assert!(!direct.headers.contains_key("retry-after"));

        upstream.reply("b", Reply::json(403, final_error.clone()));
        let exhausted = send(&runner, protocol, "velune/model/route", false).await;
        assert_eq!(exhausted.status, 403);
        assert_eq!(
            serde_json::from_str::<Value>(&exhausted.body).unwrap(),
            final_error
        );
        assert!(!exhausted.headers.contains_key("retry-after"));
        assert_eq!(
            upstream.hits(),
            ["a", "b", "a", "a", "b"].map(|id| format!("/{id}/v1{}", protocol.path()))
        );
    }
}

#[tokio::test]
async fn refused_requests_stop_with_the_upstream_answer() {
    for protocol in Api::ALL {
        for status in [400, 401, 404] {
            let (upstream, address) = Upstream::start().await;
            let runner = gateway(protocol, &address);
            let body = json!({"error": {"message": "refused"}});
            upstream.reply("a", Reply::json(status, body.clone()));
            upstream.reply("b", Reply::json(200, protocol.success()));
            let answer = send(&runner, protocol, "velune/model/route", false).await;
            assert_eq!(answer.status, status);
            assert_eq!(serde_json::from_str::<Value>(&answer.body).unwrap(), body);
            assert_eq!(upstream.hits().len(), 1);
        }
    }
}

#[tokio::test]
async fn unreachable_member_moves_to_the_next() {
    for protocol in Api::ALL {
        let (upstream, address) = Upstream::start().await;
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let unreachable = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        let mut config = gateway_config(protocol, &address);
        config.providers[0] = provider("a", protocol, &unreachable);
        let runner = Runner::start(config, Arc::new(Keys), BTreeMap::new()).unwrap();
        upstream.reply("b", Reply::json(200, protocol.success()));
        let answer = send(&runner, protocol, "velune/model/route", false).await;
        assert_eq!(answer.status, 200);
        assert_eq!(upstream.hits().len(), 1);
    }
}

#[tokio::test]
async fn exhausted_route_reports_the_last_failure() {
    for protocol in Api::ALL {
        let (upstream, address) = Upstream::start().await;
        let runner = gateway(protocol, &address);
        upstream.reply("a", Reply::json(503, json!({"error": {"message": "a"}})));
        upstream.reply("b", Reply::json(502, json!({"error": {"message": "b"}})));
        let answer = send(&runner, protocol, "velune/model/route", false).await;
        assert_eq!(answer.status, 502);
        assert_eq!(
            serde_json::from_str::<Value>(&answer.body).unwrap(),
            json!({"error": {"message": "b"}})
        );
        assert!(answer.headers.contains_key("retry-after"));
        assert_eq!(upstream.hits().len(), 2);
    }
}

#[tokio::test]
async fn quota_exhaustion_moves_to_the_next_member() {
    let protocol = Api::Responses;
    let (upstream, address) = Upstream::start().await;
    let runner = gateway(protocol, &address);
    upstream.reply(
        "a",
        Reply::json(402, json!({"error": {"code": "insufficient_quota"}})),
    );
    upstream.reply("b", Reply::json(200, protocol.success()));
    let answer = send(&runner, protocol, "velune/model/route", false).await;
    assert_eq!(answer.status, 200);
    assert_eq!(upstream.hits().len(), 2);
}

#[tokio::test]
async fn translated_attempts_use_the_selected_models_rules_and_limit() {
    for failover in [false, true] {
        let (upstream, address) = Upstream::start().await;
        let mut config = gateway_config(Api::Messages, &address);
        config.providers[0].models[0].provider_model_id = "claude-sonnet-4-5".into();
        config.providers[0].models[0].max_output_tokens = Some(2048);
        config.providers[1].models[0].provider_model_id = "claude-opus-4-7".into();
        config.providers[1].models[0].max_output_tokens = Some(4096);
        if !failover {
            config.routes[0].strategy = RoutingStrategy::Rotate;
        }
        let runner = Runner::start(config, Arc::new(Keys), BTreeMap::new()).unwrap();
        let success = json!({"id":"msg", "type":"message", "role":"assistant",
            "model":"fixture", "content":[{"type":"text", "text":"answer"}],
            "stop_reason":"end_turn", "usage":{"input_tokens":10,"output_tokens":2}});
        upstream.reply(
            "a",
            if failover {
                Reply::json(500, json!({"error":{"message":"retry"}}))
            } else {
                Reply::json(200, success.clone())
            },
        );
        upstream.reply("b", Reply::json(200, success));
        for _ in 0..if failover { 1 } else { 2 } {
            let response = reqwest::Client::new()
                .post(format!("{}/chat/completions", runner.endpoint()))
                .timeout(std::time::Duration::from_secs(5))
                .bearer_auth(runner.token())
                .json(
                    &json!({"model":"velune/model/route", "reasoning_effort":"high",
                    "messages":[{"role":"user","content":"fixture"}]}),
                )
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let body: Value = response.json().await.unwrap();
            assert_eq!(body["choices"][0]["message"]["content"], "answer");
        }
        let requests = upstream.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["model"], "claude-sonnet-4-5");
        assert_eq!(requests[0]["max_tokens"], 2048);
        assert_eq!(requests[0]["thinking"]["type"], "enabled");
        assert_eq!(requests[1]["model"], "claude-opus-4-7");
        assert_eq!(requests[1]["max_tokens"], 4096);
        assert_eq!(requests[1]["thinking"]["type"], "adaptive");
        assert_eq!(requests[1]["output_config"]["effort"], "high");
    }
}

/// Routing must convert each attempt independently, including a native first
/// member followed by a different protocol and two differently translated ones.
#[tokio::test]
async fn mixed_protocol_routes_translate_each_selected_candidate() {
    for ingress in Api::ALL {
        for (index, first) in Api::ALL.into_iter().enumerate() {
            let second = Api::ALL[(index + 1) % Api::ALL.len()];
            for status in [200, 403, 429] {
                let failover = status != 200;
                let (upstream, address) = Upstream::start().await;
                let mut config = gateway_config(first, &address);
                config.providers[1] = provider("b", second, &address);
                for provider in &mut config.providers {
                    provider.models[0].max_output_tokens = Some(128);
                }
                if !failover {
                    config.routes[0].strategy = RoutingStrategy::Rotate;
                }
                let success = |protocol| match protocol {
                    Api::Chat => json!({"id":"chat", "object":"chat.completion", "model":"fixture",
                        "choices":[{"index":0,"message":{"role":"assistant","content":"answer"},"finish_reason":"stop"}]}),
                    Api::Responses => {
                        json!({"id":"resp", "object":"response", "model":"fixture", "status":"completed",
                        "output":[{"id":"msg","type":"message","role":"assistant","status":"completed",
                            "content":[{"type":"output_text","text":"answer","annotations":[]}]}]})
                    }
                    Api::Messages => {
                        json!({"id":"msg", "type":"message", "role":"assistant", "model":"fixture",
                        "content":[{"type":"text","text":"answer"}],"stop_reason":"end_turn"})
                    }
                };
                upstream.reply(
                    "a",
                    if failover {
                        Reply::json(status, json!({"error":{"message":"retry"}}))
                    } else {
                        Reply::json(200, success(first))
                    },
                );
                upstream.reply("b", Reply::json(200, success(second)));
                let runner = Runner::start(config, Arc::new(Keys), BTreeMap::new()).unwrap();
                let mut body = match ingress {
                    Api::Responses => {
                        json!({"input":[{"role":"user","content":"question"}], "max_output_tokens":64})
                    }
                    _ => {
                        json!({"messages":[{"role":"user","content":"question"}], "max_tokens":64})
                    }
                };
                body["model"] = json!("velune/model/route");
                for _ in 0..if failover { 1 } else { 2 } {
                    let response = reqwest::Client::new()
                        .post(format!("{}{}", runner.endpoint(), ingress.path()))
                        .timeout(std::time::Duration::from_secs(5))
                        .bearer_auth(runner.token())
                        .header("anthropic-version", "2023-06-01")
                        .json(&body)
                        .send()
                        .await
                        .unwrap();
                    assert_eq!(response.status(), 200);
                    let response: Value = response.json().await.unwrap();
                    let text = match ingress {
                        Api::Chat => &response["choices"][0]["message"]["content"],
                        Api::Responses => &response["output"][0]["content"][0]["text"],
                        Api::Messages => &response["content"][0]["text"],
                    };
                    assert_eq!(text, "answer");
                }
                assert_eq!(
                    upstream.hits(),
                    vec![
                        format!("/a/v1{}", first.path()),
                        format!("/b/v1{}", second.path()),
                    ]
                );
                let requests = upstream.requests.lock().unwrap();
                for (i, protocol) in [first, second].into_iter().enumerate() {
                    assert_eq!(
                        requests[i]["model"],
                        if i == 0 { "a-native" } else { "b-native" }
                    );
                    let field = match protocol {
                        Api::Responses => "input",
                        _ => "messages",
                    };
                    assert!(
                        requests[i][field]
                            .as_array()
                            .is_some_and(|items| !items.is_empty())
                    );
                    assert!(requests[i].to_string().contains("question"));
                }
            }
        }
    }
}

fn gateway_config(protocol: Api, upstream: &str) -> GatewayConfig {
    GatewayConfig {
        id: "gateway".into(),
        name: "Gateway".into(),
        providers: vec![
            provider("a", protocol, upstream),
            provider("b", protocol, upstream),
        ],
        routes: vec![ModelRoute {
            record_key: "route".into(),
            name: "Route".into(),
            members: vec!["a-model".into(), "b-model".into()],
            strategy: RoutingStrategy::Order,
            sticky: false,
            weights: BTreeMap::new(),
        }],
    }
}

#[tokio::test]
async fn messages_metadata_is_preserved_for_standard_responses_api() {
    let (upstream, address) = Upstream::start().await;
    let config = gateway_config(Api::Responses, &address);
    let runner = Runner::start(config, Arc::new(Keys), BTreeMap::new()).unwrap();
    upstream.reply("a", Reply::stream(Api::Responses.events()));
    let response = reqwest::Client::new()
        .post(format!("{}/messages", runner.endpoint()))
        .bearer_auth(runner.token())
        .json(
            &json!({"model":"velune/model/a-model", "stream":true, "max_tokens":32,
            "metadata":{"user_id":"synthetic-user"},
            "messages":[{"role":"user","content":"synthetic"}]}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let _ = response.bytes().await.unwrap();
    let requests = upstream.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["metadata"]["user_id"], "synthetic-user");
}
