use super::*;
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, StatusCode, Uri},
    response::Response,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

const DAY_MS: i64 = 86_400_000;

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
            let query: BTreeMap<_, _> = url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()).collect();
            let page_two = query.get("page").is_some_and(|page| page.as_ref() == "second");
            let cost = uri.path().ends_with("costs") || uri.path().ends_with("cost");
            let usage = uri.path().ends_with("completions") || uri.path().ends_with("usage");
            if page_two && ((cost && mode == "fail") || (usage && mode == "usage-fail")) {
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
                let results = if cost {
                    json!([{"amount":{"value":"0.06","currency":"usd"}}])
                } else if page_two {
                    json!([
                        {"model":"synthetic-model","input_tokens":70,"output_tokens":14,"num_model_requests":1,"input_cached_tokens":30},
                        {"model":null,"input_tokens":30,"output_tokens":6,"num_model_requests":1,"input_cached_tokens":10}
                    ])
                } else {
                    json!([{"model":"synthetic-model","input_tokens":100,"input_uncached_tokens":60,"output_tokens":20,"num_model_requests":2,"input_cached_tokens":40}])
                };
                let mut start = query["start_time"].parse::<i64>().unwrap();
                if page_two { start += DAY_MS / 1_000; }
                let more = !page_two || cost && mode == "repeat";
                json!({"data":[{"start_time":start,"end_time":start + DAY_MS / 1_000,"results":results}],"has_more":more,"next_page":if more { Some("second") } else { None }})
            } else {
                let row = if cost {
                    json!({"cost":{"amount":"41280.000000","currency":"USD"}})
                } else {
                    let mut start = DateTime::parse_from_rfc3339(&query["starting_at"]).unwrap();
                    if page_two { start += chrono::Duration::days(1); }
                    let end = start + chrono::Duration::days(1);
                    json!({
                        "starting_at":start.to_rfc3339(), "ending_at":end.to_rfc3339(),
                        "group":{"model":if page_two { None } else { Some("synthetic-model") }},
                        "usage":{"uncached_input_tokens":50,"cache_read_input_tokens":20,
                            "cache_creation":{"ephemeral_1h_input_tokens":15,"ephemeral_5m_input_tokens":10},
                            "output_tokens":25,"requests":3}
                    })
                };
                let more = usage && !page_two;
                json!({"data":[row],"has_more":more,"next_page":if more { Some("second") } else { None },"data_refreshed_at":"2026-11-01T12:00:00Z"})
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

fn starting() -> i64 {
    Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis()
        - 2 * DAY_MS
}

#[test]
fn official_units_scopes_pagination_and_unknown_fields_keep_their_meaning() {
    runtime().block_on(async {
        let starting = starting();
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
        let models = report.model_usage.unwrap();
        assert_eq!(models.len(), 2);
        let model = &models[0];
        assert_eq!(model.model, "synthetic-model");
        assert_eq!(model.from_ms, Some(starting));
        assert_eq!(model.to_ms, Some(starting + DAY_MS));
        let date = DateTime::from_timestamp_millis(starting)
            .unwrap()
            .date_naive()
            .to_string();
        assert_eq!(model.date.as_deref(), Some(date.as_str()));
        assert_eq!(model.input_tokens, Some(100));
        assert_eq!(model.uncached_input_tokens, Some(60));
        assert_eq!(model.cached_input_tokens, Some(40));
        assert_eq!(model.output_tokens, Some(20));
        assert_eq!(model.requests, Some(2));
        assert_eq!(model.credits, None);
        assert_eq!(models[1].model, model.model);
        assert_eq!(models[1].from_ms, model.to_ms);
        assert_eq!(models[1].input_tokens, Some(70));
        assert_eq!(models[1].uncached_input_tokens, None);
        assert_eq!(report.spend_limit.as_ref().unwrap().amount, 123.45);
        assert!(!report.spend_limit.unwrap().enforced);
        {
            let requests = requests.lock().unwrap();
            assert_usage_pages(&requests, "usage/completions");
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
        let models = report.model_usage.unwrap();
        assert_eq!(models.len(), 1);
        let model = &models[0];
        assert_eq!(model.model, "synthetic-model");
        assert_eq!(model.from_ms, Some(starting));
        assert_eq!(model.to_ms, Some(starting + DAY_MS));
        assert_eq!(model.date.as_deref(), Some(date.as_str()));
        assert_eq!(model.input_tokens, Some(95));
        assert_eq!(model.uncached_input_tokens, Some(50));
        assert_eq!(model.cached_input_tokens, Some(20));
        assert_eq!(model.output_tokens, Some(25));
        assert_eq!(model.requests, Some(3));
        assert_eq!(model.credits, None);
        assert_eq!(report.data_refreshed_at_ms, Some(1793534400000));
        assert_eq!(
            report.rate_limits.unwrap()[0].limit_type,
            "requests_per_minute"
        );
        let requests = requests.lock().unwrap();
        assert_usage_pages(&requests, "analytics/usage");
        assert!(
            requests
                .iter()
                .filter(|uri| uri.contains("analytics/usage"))
                .all(|uri| uri.contains("time_bucket_width=1d"))
        );
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

fn assert_usage_pages(requests: &[String], usage_path: &str) {
    let (usage, other): (Vec<_>, Vec<_>) =
        requests.iter().partition(|uri| uri.contains(usage_path));
    assert_eq!(usage.len(), 2);
    assert!(usage[1].contains("page=second"));
    assert!(usage.iter().all(|uri| uri.contains("group_by%5B%5D=model")));
    assert!(other.iter().all(|uri| !uri.contains("group_by")));
}

#[test]
fn incomplete_costs_and_redirects_never_publish_partial_totals_or_leak_secrets() {
    runtime().block_on(async {
        let starting = starting();
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
            assert_eq!(report.model_usage.unwrap().len(), 2);
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

#[test]
fn incomplete_usage_never_publishes_partial_model_rows_or_month_total() {
    runtime().block_on(async {
        for service in [BillingService::OpenAi, BillingService::Anthropic] {
            let (client, server, _) = fixture(service, "usage-fail").await;
            let report = collect_with_client(
                config(
                    service,
                    BillingScope::ApiKey {
                        id: "key_test".into(),
                    },
                ),
                &client,
                starting(),
            )
            .await
            .unwrap();
            assert!(report.usage.is_none());
            assert!(report.model_usage.is_none());
            assert!(report.costs.is_some());
            assert_eq!(report.problems.len(), 1);
            server.abort();
        }
    });
}
