//! Observe the provider's physical HTTP boundary, independently of gateway failover.
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, Response},
};
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use velune_ai::{
    NativeErrorKind, NativeEvent, NativeProvider, NativeRequest, Payload, Protocol, ProviderModelId,
};
use velune_ai_provider::{
    HttpClient, NativeHttp,
    config::{
        CredentialMaterial, CredentialRecovery, HttpAuthorization, HttpEndpoint, HttpTarget,
        RequestCapabilities, RequestProfile, ResolvedCredential,
    },
};

#[derive(Clone)]
struct Upstream {
    status: u16,
    content_type: Option<&'static str>,
    body: &'static str,
    calls: Arc<Mutex<Vec<(String, String)>>>,
}
impl Upstream {
    async fn start(
        status: u16,
        content_type: Option<&'static str>,
        body: &'static str,
    ) -> (Self, String) {
        let state = Self {
            status,
            content_type,
            body,
            calls: Arc::default(),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .fallback(Self::respond)
            .with_state(state.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (state, address)
    }
    async fn respond(State(state): State<Self>, request: Request<Body>) -> Response<Body> {
        state.calls.lock().unwrap().push((
            request.uri().path().into(),
            request.headers()["authorization"].to_str().unwrap().into(),
        ));
        // Consume the request normally; an unread body must not create the failure.
        axum::body::to_bytes(request.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let mut response = Response::builder().status(state.status);
        if let Some(content_type) = state.content_type {
            response = response.header("content-type", content_type);
        }
        if state.status == 307 {
            response = response.header("location", "/redirect-destination");
        }
        response.body(Body::from(state.body)).unwrap()
    }
    fn assert_one_call(&self, path: &str) {
        assert_eq!(
            *self.calls.lock().unwrap(),
            [(path.into(), "Bearer synthetic-provider-secret".into())]
        );
    }
}

fn provider(address: &str, protocol: Protocol) -> NativeHttp {
    // Deliberately permissive caller settings: the owned boundary must override
    // both policies, rather than pass merely because reqwest's defaults did.
    let retry = reqwest::retry::for_host("127.0.0.1")
        .no_budget()
        .max_retries_per_request(2)
        .classify_fn(|response| {
            if response.status() == Some(reqwest::StatusCode::SERVICE_UNAVAILABLE) {
                response.retryable()
            } else {
                response.success()
            }
        });
    NativeHttp::new(
        HttpTarget::new(
            protocol,
            HttpEndpoint::parse(address).unwrap(),
            RequestProfile::Native,
            None,
        )
        .unwrap(),
        HttpClient::new(
            reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::limited(2))
                .retry(retry),
        )
        .unwrap(),
        ResolvedCredential {
            material: CredentialMaterial::Http {
                authorization: HttpAuthorization::Bearer("synthetic-provider-secret".into()),
                account_id: None,
            },
            recovery: CredentialRecovery::None,
            request_capabilities: RequestCapabilities::default(),
        },
    )
    .unwrap()
}

fn request(stream: bool) -> NativeRequest {
    NativeRequest {
        model: ProviderModelId::new("synthetic-model").unwrap(),
        body: Payload::new(
            json!({"model":"synthetic-model","messages":[{"role":"user","content":"synthetic"}],"stream":stream}),
        ),
        headers: Vec::new(),
        stream,
    }
}

#[tokio::test]
async fn model_requests_do_not_follow_redirects_or_retry_even_with_a_permissive_builder() {
    const SUCCESS: &str =
        r#"{"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#;
    for (status, body) in [
        (200, SUCCESS),
        (307, "redirect refused"),
        (503, "retry refused"),
    ] {
        let (upstream, address) = Upstream::start(status, Some("application/json"), body).await;
        let result = provider(&address, Protocol::ChatCompletions)
            .send(request(false), Box::new(|_| Box::pin(async { Ok(()) })))
            .await;
        upstream.assert_one_call("/chat/completions");
        if status == 200 {
            let success = result.unwrap();
            let velune_ai::NativeOutput::Json(response) = success.output else {
                panic!("expected JSON response")
            };
            assert_eq!(response.meta.status, 200);
            assert_eq!(response.body.get(), body.as_bytes());
        } else {
            let failure = result.unwrap_err();
            assert!(failure.submitted);
            assert_eq!(failure.response.unwrap().status, status);
            assert_eq!(failure.body.unwrap().get(), body.as_bytes());
        }
    }
}

#[tokio::test]
async fn responses_stream_requires_sse_headers_and_a_real_model_terminal() {
    const TERMINAL: &str = "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n";
    const UNFINISHED: &str = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n";
    for (mime, body, failure, forwarded_headers) in [
        (
            Some("Text/Event-Stream; charset=utf-8"),
            TERMINAL,
            None,
            true,
        ),
        (None, TERMINAL, Some(NativeErrorKind::Unsupported), false),
        (
            Some("text/html"),
            TERMINAL,
            Some(NativeErrorKind::Unsupported),
            false,
        ),
        (
            Some("text/event-stream"),
            UNFINISHED,
            Some(NativeErrorKind::ProviderFailure),
            true,
        ),
    ] {
        let (upstream, address) = Upstream::start(200, mime, body).await;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observed = seen.clone();
        let mut request = request(true);
        request.body =
            Payload::new(json!({"model":"synthetic-model","input":"synthetic","stream":true}));
        let result = provider(&address, Protocol::Responses)
            .send(
                request,
                Box::new(move |event| {
                    observed.lock().unwrap().push(event);
                    Box::pin(async { Ok(()) })
                }),
            )
            .await;
        if let Some(kind) = failure {
            let failure = result.unwrap_err();
            assert_eq!(failure.kind, kind);
            assert!(failure.submitted);
            assert_eq!(failure.response.unwrap().status, 200);
        } else {
            assert!(result.unwrap().completed);
        }
        assert_eq!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, NativeEvent::Headers(_))),
            forwarded_headers
        );
        upstream.assert_one_call("/responses");
    }
}
