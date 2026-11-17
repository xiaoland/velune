//! A long successful half-open stream must not pause unrelated callers.
use axum::{Router, body::Body, extract::State, http::Response};
use futures_util::stream;
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use velune_gateway::{
    CredentialMaterial, CredentialRecovery, CredentialResolutionError, CredentialResolver,
    CredentialTarget, GatewayConfig, HttpAuthorization, RequestCapabilities, ResolvedCredential,
    Runner, SchedulingState,
};

struct Keys;
impl CredentialResolver for Keys {
    fn resolve(
        &self,
        _: String,
        _: CredentialTarget,
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

#[tokio::test]
async fn half_open_response_recovers_admission_and_late_transport_failure_cools_again() {
    async fn upstream(
        State((hits, disconnect)): State<(Arc<AtomicUsize>, Arc<tokio::sync::Notify>)>,
    ) -> Response<Body> {
        match hits.fetch_add(1, Ordering::SeqCst) {
            0 => Response::builder()
                .status(429)
                .header("retry-after", "0")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"error":{"message":"synthetic rate limit"}}"#,
                ))
                .unwrap(),
            1 => {
                // The accepted response has no model output yet. Keep the
                // probe alive so neither progress nor completion can heal it.
                let body = stream::once(async move {
                    disconnect.notified().await;
                    Err::<&'static str, _>(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "synthetic upstream disconnect after response headers",
                    ))
                });
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from_stream(body))
                    .unwrap()
            }
            _ => Response::builder()
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"id":"synthetic","status":"completed","output":[]}"#,
                ))
                .unwrap(),
        }
    }
    let hits = Arc::new(AtomicUsize::new(0));
    let disconnect = Arc::new(tokio::sync::Notify::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .fallback(upstream)
        .with_state((hits.clone(), disconnect.clone()));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut config: GatewayConfig = serde_json::from_value(json!({
        "id":"gateway", "name":"synthetic gateway", "routes":[],
        "providers":[{"id":"provider", "name":"synthetic provider",
            "protocol":"responsesV1", "executionBackend":"http", "requestProfile":"native",
            "endpoint":format!("http://{address}/v1"), "credentialRef":null, "quotaLimits":[],
            "models":[{"recordKey":"model", "providerModelId":"synthetic-model",
                "nickname":"", "icon":null, "contextWindow":null,
                "maxOutputTokens":null, "reasoningLevels":null}]}]
    }))
    .unwrap();
    // The probe uses an aggregate route with another available provider, so
    // hit counts also detect a forbidden replay after its response is delivered.
    let mut fallback = config.providers[0].clone();
    fallback.id = "fallback".into();
    fallback.models[0].record_key = "fallback-model".into();
    config.providers.push(fallback);
    config.routes.push(velune_gateway::ModelRoute {
        record_key: "route".into(),
        name: "synthetic route".into(),
        members: vec!["model".into(), "fallback-model".into()],
        strategy: velune_gateway::RoutingStrategy::Order,
        sticky: false,
        weights: BTreeMap::new(),
    });
    let health = SchedulingState::default();
    let first = Runner::start_with_analytics(
        config.clone(),
        Arc::new(Keys),
        BTreeMap::new(),
        None,
        health.clone(),
    )
    .unwrap();
    let second =
        Runner::start_with_analytics(config, Arc::new(Keys), BTreeMap::new(), None, health)
            .unwrap();
    let client = reqwest::Client::new();
    let request = |runner: &Runner, streaming, model: &str| {
        client
            .post(format!("{}/responses", runner.endpoint()))
            .bearer_auth(runner.token())
            .json(&json!({"model":model,"stream":streaming,"input":[]}))
    };
    let initial = request(&first, false, "velune/model/model")
        .send()
        .await
        .unwrap();
    assert_eq!(initial.status(), 429);
    initial.bytes().await.unwrap();
    let probe = request(&first, true, "velune/model/route")
        .send()
        .await
        .unwrap();
    assert_eq!(probe.status(), 200);
    // Keep its body alive: neither model completion nor cancellation has happened.
    let other = tokio::time::timeout(
        Duration::from_secs(2),
        request(&second, false, "velune/model/model").send(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(other.status(), 200, "{}", other.text().await.unwrap());
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    // The accepted probe now loses its upstream transport. Consuming the
    // interrupted downstream body waits until relay/dispatch has classified it.
    disconnect.notify_one();
    let interrupted = tokio::time::timeout(Duration::from_secs(2), probe.bytes())
        .await
        .unwrap();
    assert!(interrupted.is_err());
    assert_eq!(
        hits.load(Ordering::SeqCst),
        3,
        "delivered stream must not fail over"
    );
    let blocked = request(&second, false, "velune/model/model")
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), 503);
    assert!(blocked.headers().contains_key("retry-after"));
    assert_eq!(
        blocked.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "provider_unavailable"
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        3,
        "cooldown rejection must remain local"
    );
    server.abort();
}
