use super::*;
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, StatusCode, Uri},
    response::Response,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

// Loopback is available only to this private test client, never to saved configuration.
async fn fixture(
    service: BillingService,
    mode: &'static str,
) -> (Client, tokio::task::JoinHandle<()>, Arc<Mutex<Vec<String>>>) {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let captured = observed.clone();
    let app = Router::new().fallback(move |uri: Uri, headers: HeaderMap| {
        let captured = captured.clone();
        async move {
            captured.lock().unwrap().push(uri.to_string());
            assert_eq!(headers.get(if service == BillingService::OpenAi { "authorization" } else { "x-api-key" }).unwrap(), if service == BillingService::OpenAi { "Bearer synthetic-admin" } else { "synthetic-admin" });
            if service == BillingService::OpenAi { assert_eq!(headers["openai-organization"], "org_test"); }
            else { assert_eq!(headers["anthropic-beta"], "analytics-usage-cost-2026-08-24"); }
            let page_two = uri.query().is_some_and(|q| q.contains("page=second"));
            let cost = uri.path().ends_with("costs") || uri.path().ends_with("cost");
            if cost && page_two && mode == "fail" {
                return Response::builder().status(StatusCode::FORBIDDEN).body(Body::from("synthetic-admin must never appear in errors")).unwrap();
            }
            if cost && mode == "redirect" {
                return Response::builder().status(StatusCode::TEMPORARY_REDIRECT).header("location", "/credential-leak").body(Body::empty()).unwrap();
            }
            let payload = if uri.path().ends_with("spend_limit") {
                json!({"threshold_amount":12345,"currency":"USD","interval":"month","enforcement":{"status":"inactive"}})
            } else if uri.path().ends_with("rate_limits") {
                json!({"data":[{"group":{"id":"rlg_test","type":"model_group","display_name":"Test model"},"models":["synthetic-model"],"limits":[{"type":"requests_per_minute","value":1000}]}],"next_page":null})
            } else if service == BillingService::OpenAi {
                let row = if cost { json!({"amount":{"value":"0.06","currency":"usd"}}) }
                else { json!({"input_tokens":100,"output_tokens":20,"num_model_requests":2,"input_cached_tokens":40}) };
                let more = !page_two || cost && mode == "repeat";
                json!({"data":[{"results":[row]}],"has_more":more,"next_page":if more { Some("second") } else { None }})
            } else {
                let row = if cost { json!({"cost":{"amount":"41280.000000","currency":"USD"}}) }
                else { json!({"usage":{"uncached_input_tokens":100,"cache_read_input_tokens":40,"cache_creation":{"ephemeral_1h_input_tokens":30,"ephemeral_5m_input_tokens":20},"output_tokens":25,"requests":3}}) };
                json!({"data":[row],"has_more":false,"next_page":null,"data_refreshed_at":"2026-11-01T12:00:00Z"})
            };
            Response::builder().header("content-type", "application/json").body(Body::from(payload.to_string())).unwrap()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut client = Client::new(service, "synthetic-admin", "org_test").unwrap();
    client.origin = format!("http://{address}");
    (client, server, observed)
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn config(service: BillingService, scope: BillingScope) -> BillingConfiguration {
    BillingConfiguration {
        service,
        organization_id: "org_test".into(),
        scope,
    }
}

#[test]
fn official_units_scopes_pagination_and_unknown_fields_keep_their_meaning() {
    runtime().block_on(async {
        let starting = Utc::now().timestamp_millis() - 2 * 24 * 60 * 60 * 1000;
        let (client, server, requests) = fixture(BillingService::OpenAi, "ok").await;
        let report = collect_with_client(
            config(
                BillingService::OpenAi,
                BillingScope::Project {
                    id: "proj_test".into(),
                },
            ),
            &client,
            starting,
        )
        .await
        .unwrap();
        assert!((report.costs.unwrap()[0].amount - 0.12).abs() < 1e-10);
        assert_eq!(report.usage.as_ref().unwrap().input_tokens, 200);
        assert_eq!(report.usage.unwrap().cache_creation_input_tokens, None);
        assert_eq!(report.spend_limit.as_ref().unwrap().amount, 123.45);
        assert!(!report.spend_limit.unwrap().enforced);
        {
            let requests = requests.lock().unwrap();
            assert!(requests.iter().any(|uri| uri.contains("page=second")));
            assert!(
                requests
                    .iter()
                    .filter(|uri| !uri.contains("spend_limit"))
                    .all(|uri| uri.contains("project_ids%5B%5D=proj_test"))
            );
            assert!(
                requests
                    .iter()
                    .any(|uri| uri == "/v1/organization/projects/proj_test/spend_limit")
            );
        }
        server.abort();

        let (client, server, requests) = fixture(BillingService::Anthropic, "ok").await;
        let report = collect_with_client(
            config(
                BillingService::Anthropic,
                BillingScope::Workspace {
                    id: "wrkspc_test".into(),
                },
            ),
            &client,
            starting,
        )
        .await
        .unwrap();
        assert_eq!(report.costs.unwrap()[0].amount, 412.8);
        assert_eq!(report.usage.as_ref().unwrap().input_tokens, 190);
        assert_eq!(report.usage.unwrap().cache_creation_input_tokens, Some(50));
        assert_eq!(report.data_refreshed_at_ms, Some(1793534400000));
        assert_eq!(
            report.rate_limits.unwrap()[0].limit_type,
            "requests_per_minute"
        );
        let requests = requests.lock().unwrap();
        assert!(
            requests
                .iter()
                .filter(|uri| uri.contains("analytics"))
                .all(|uri| uri.contains("workspace_ids%5B%5D=wrkspc_test"))
        );
        assert!(requests.iter().any(|uri| uri
            == "/v1/organizations/workspaces/wrkspc_test/rate_limits?include_inherited=true"));
        server.abort();
    });
}
#[test]
fn incomplete_costs_and_redirects_never_publish_partial_totals_or_leak_secrets() {
    runtime().block_on(async {
        let starting = Utc::now().timestamp_millis() - 2 * 24 * 60 * 60 * 1000;
        for mode in ["fail", "repeat", "redirect"] {
            let (client, server, requests) = fixture(BillingService::OpenAi, mode).await;
            let report = collect_with_client(
                config(
                    BillingService::OpenAi,
                    BillingScope::ApiKey {
                        id: "key_test".into(),
                    },
                ),
                &client,
                starting,
            )
            .await
            .unwrap();
            assert!(report.costs.is_none());
            assert_eq!(report.usage.unwrap().requests, 4);
            assert!(report.spend_limit.is_none());
            assert_eq!(report.problems.len(), 1);
            assert!(!report.problems[0].contains("synthetic-admin"));
            assert!(
                !requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|uri| uri.contains("credential-leak") || uri.contains("spend_limit"))
            );
            assert!(
                requests
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|uri| uri.contains("api_key_ids%5B%5D=key_test"))
            );
            server.abort();
        }
    });
}
