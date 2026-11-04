//! Native dispatch through a running gateway against a scripted local upstream:
//! what reaches the caller, and which failures move on to the next route member.
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, Response},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use velune_gateway::{
    AnalyticsOutcome, AnalyticsRecord, AnalyticsSink, CredentialMaterial, CredentialRecovery,
    CredentialResolutionError, CredentialResolver, CredentialTarget, ExecutionBackend,
    GatewayConfig, HttpAuthorization, ModelRoute, ProviderDefinition, ProviderModel,
    RequestCapabilities, RequestProfile, ResolvedCredential, RoutingStrategy, Runner,
    SchedulingState,
};

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}
impl Reply {
    fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            headers: vec![("content-type", "application/json".into())],
            body: body.to_string(),
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
        upstream.hits.lock().unwrap().push(path);
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
        _target: CredentialTarget,
    ) -> velune_ai::OperationFuture<Result<ResolvedCredential, CredentialResolutionError>> {
        Box::pin(async {
            Ok(ResolvedCredential {
                material: CredentialMaterial::Http {
                    authorization: HttpAuthorization::Bearer("synthetic-key".into()),
                    account_id: None,
                },
                recovery: CredentialRecovery::None,
                request_capabilities: RequestCapabilities::default(),
            })
        })
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
        }],
        quota_limits: Vec::new(),
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

#[tokio::test]
async fn admission_rejections_are_recorded_in_analytics() {
    struct Collect {
        records: Mutex<Vec<AnalyticsRecord>>,
    }
    impl AnalyticsSink for Collect {
        fn record(&self, record: AnalyticsRecord) {
            self.records.lock().unwrap().push(record);
        }
    }
    let protocol = Api::Chat;
    let (_upstream, address) = Upstream::start().await;
    let analytics = Arc::new(Collect {
        records: Mutex::new(Vec::new()),
    });
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
    let response = post(
        json!({"model": "velune/model/route", "stream": "yes", "messages": []}).to_string(),
    )
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
async fn refused_requests_stop_with_the_upstream_answer() {
    for protocol in Api::ALL {
        for status in [400, 401, 403, 404] {
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
