//! The production account-data HTTP boundary, with only the upstream replaced.
use super::*;
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::Response,
};
use std::sync::Mutex;

#[tokio::test]
async fn account_http_preserves_headers_and_rejects_redirects_errors_and_invalid_bodies() {
    let payload = r#"{"available_count":3,"credits":[]}"#;
    // Fixed sizes are independent of the production limit. The exact boundary
    // must succeed; valid JSON beyond it must fail before it can be published.
    let padded = |size: usize| format!("{payload}{}", " ".repeat(size - payload.len()));
    for (case, status, body) in [
        ("ok", StatusCode::OK, payload.to_owned()),
        ("at-limit", StatusCode::OK, padded(65_536)),
        ("over-limit", StatusCode::OK, padded(65_537)),
        ("invalid-json", StatusCode::OK, "{broken".into()),
        ("redirect", StatusCode::TEMPORARY_REDIRECT, String::new()),
        (
            "unavailable",
            StatusCode::SERVICE_UNAVAILABLE,
            "synthetic-secret".into(),
        ),
    ] {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let captured = observed.clone();
        let server_body = body.clone();
        let app = Router::new().fallback(move |method: Method, uri: Uri, headers: HeaderMap| {
            let captured = captured.clone();
            let body = server_body.clone();
            async move {
                captured.lock().unwrap().push(uri.path().to_owned());
                assert_eq!(method, Method::GET);
                assert_eq!(headers["authorization"], "Bearer synthetic-token");
                assert_eq!(headers["chatgpt-account-id"], "synthetic-account");
                assert_eq!(headers["accept"], "application/json");
                assert_eq!(headers["openai-beta"], "codex-1");
                assert_eq!(headers["originator"], "Codex Desktop");
                assert_eq!(headers["user-agent"], "codex-cli/1.0.0");
                // A following client would receive a valid response, making
                // the wrong policy observable instead of failing elsewhere.
                if uri.path() == "/credential-leak" {
                    return Response::new(Body::from(r#"{"leaked":true}"#));
                }
                Response::builder()
                    .status(status)
                    .header("location", "/credential-leak")
                    .body(Body::from(body))
                    .unwrap()
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = account_data_client(
            reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2)),
        )
        .unwrap();
        let mut bearer = reqwest::header::HeaderValue::from_static("Bearer synthetic-token");
        bearer.set_sensitive(true);
        let response = account_data_request(
            &client,
            &format!("http://{address}/account"),
            bearer,
            "synthetic-account",
            AccountDataEndpoint::Resets,
            "0.166.7",
        )
        .send()
        .await
        .unwrap();
        let result = account_data_body(response, AccountDataEndpoint::Resets).await;
        server.abort();
        let _ = server.await;
        assert_eq!(
            *observed.lock().unwrap(),
            ["/account"],
            "{case}: the upstream must receive exactly one request"
        );
        match case {
            "ok" | "at-limit" => assert_eq!(
                result.unwrap(),
                serde_json::json!({"available_count":3,"credits":[]}),
                "{case}"
            ),
            "over-limit" | "invalid-json" => assert!(
                matches!(result, Err(CredentialResolutionError::InvalidContract)),
                "{case}: {result:?}"
            ),
            _ => assert!(
                matches!(result, Err(CredentialResolutionError::Unavailable)),
                "{case}: {result:?}"
            ),
        }
    }
}
