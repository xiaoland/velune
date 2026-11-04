//! Parse untrusted account responses once, then aggregate typed rows.
use crate::{
    BilledCost, BillingConfiguration, BillingRateLimit, BillingReport, BillingScope,
    BillingService, BillingSpendLimit, BillingUsage,
};
use chrono::{TimeZone, Utc};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, de::DeserializeOwned};
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, String>;
struct Client {
    http: reqwest::Client,
    origin: String,
}
impl Client {
    fn new(service: BillingService, key: &str, organization: &str) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let (name, value) = match service {
            BillingService::OpenAi => ("authorization", format!("Bearer {key}")),
            BillingService::Anthropic => ("x-api-key", key.into()),
        };
        let mut value = HeaderValue::from_str(&value).map_err(|_| "查询认证格式无效")?;
        value.set_sensitive(true);
        headers.insert(name, value);
        if service == BillingService::OpenAi {
            headers.insert(
                "openai-organization",
                HeaderValue::from_str(organization).map_err(|_| "组织标识无效")?,
            );
        } else {
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            headers.insert(
                "anthropic-beta",
                HeaderValue::from_static("analytics-usage-cost-2026-08-24"),
            );
        }
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(std::time::Duration::from_secs(12))
            .user_agent("Velune/0.1")
            .build()
            .map_err(|_| "无法创建账单查询连接")?;
        Ok(Self {
            http,
            origin: match service {
                BillingService::OpenAi => "https://api.openai.com",
                BillingService::Anthropic => "https://api.anthropic.com",
            }
            .into(),
        })
    }
    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(String, String)]) -> Result<T> {
        let mut target =
            url::Url::parse(&format!("{}{path}", self.origin)).map_err(|_| "账单查询地址无效")?;
        if !query.is_empty() {
            target
                .query_pairs_mut()
                .extend_pairs(query.iter().map(|(key, value)| (key, value)));
        }
        let mut response = self
            .http
            .get(target)
            .send()
            .await
            .map_err(|_| "无法连接官方账单接口")?;
        if !response.status().is_success() {
            return Err(match response.status().as_u16() {
                401 | 403 => "查询认证无效或缺少组织查询权限".into(),
                429 => "官方账单接口暂时限流，请稍后刷新".into(),
                code => format!("官方账单接口返回 HTTP {code}"),
            });
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "账单响应读取中断")? {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err("账单单页响应过大".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| "官方账单响应格式不支持".into())
    }
    async fn pages<T: DeserializeOwned>(
        &self,
        path: &str,
        mut query: Vec<(String, String)>,
    ) -> Result<(Vec<T>, Option<i64>)> {
        let mut rows = Vec::new();
        let mut seen = BTreeSet::new();
        let mut refreshed = None;
        for _ in 0..32 {
            let page: Page<T> = self.get(path, &query).await?;
            if let Some(value) = page.data_refreshed_at {
                let value = chrono::DateTime::parse_from_rfc3339(&value)
                    .map_err(|_| "官方数据更新时间无效")?
                    .timestamp_millis();
                refreshed = Some(refreshed.map_or(value, |previous: i64| previous.min(value)));
            }
            rows.extend(page.data);
            if rows.len() > 32000 {
                return Err("账单记录过多，请缩小统计范围".into());
            }
            if page.has_more == Some(false) || (page.has_more.is_none() && page.next_page.is_none())
            {
                return Ok((rows, refreshed));
            }
            let cursor = page
                .next_page
                .filter(|s| !s.is_empty() && s.len() <= 4096)
                .ok_or("账单分页游标缺失")?;
            if !seen.insert(cursor.clone()) {
                return Err("账单分页游标重复".into());
            }
            query.retain(|(key, _)| key != "page");
            query.push(("page".into(), cursor));
        }
        Err("账单页数超过上限，请缩小统计范围".into())
    }
}
#[derive(Deserialize)]
struct Page<T> {
    data: Vec<T>,
    has_more: Option<bool>,
    next_page: Option<String>,
    data_refreshed_at: Option<String>,
}
#[derive(Deserialize)]
struct Bucket<T> {
    results: Vec<T>,
}
#[derive(Deserialize)]
struct OpenAiUsage {
    input_tokens: u64,
    output_tokens: u64,
    num_model_requests: u64,
    input_cached_tokens: Option<u64>,
    input_cache_write_tokens: Option<u64>,
}
#[derive(Deserialize)]
struct OpenAiCost {
    amount: Money,
}
#[derive(Deserialize)]
struct Money {
    #[serde(deserialize_with = "decimal")]
    value: f64,
    currency: String,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Decimal {
    Number(f64),
    Text(String),
}
fn decimal<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<f64, D::Error> {
    use serde::de::Error;
    let value = match Decimal::deserialize(deserializer)? {
        Decimal::Number(value) => value,
        Decimal::Text(value) => value.parse().map_err(D::Error::custom)?,
    };
    if value.is_finite() {
        Ok(value)
    } else {
        Err(D::Error::custom("nonfinite amount"))
    }
}
#[derive(Deserialize)]
struct AnthropicUsageRow {
    usage: AnthropicUsage,
}
#[derive(Deserialize)]
struct AnthropicUsage {
    uncached_input_tokens: u64,
    cache_read_input_tokens: u64,
    cache_creation: BTreeMap<String, u64>,
    output_tokens: u64,
    requests: u64,
}
#[derive(Deserialize)]
struct AnthropicCostRow {
    cost: AnthropicCost,
}
#[derive(Deserialize)]
struct AnthropicCost {
    #[serde(deserialize_with = "decimal")]
    amount: f64,
    currency: String,
}
#[derive(Deserialize)]
struct SpendLimit {
    threshold_amount: u64,
    currency: String,
    interval: Month,
    enforcement: Enforcement,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Month {
    Month,
}
#[derive(Deserialize)]
struct Enforcement {
    status: EnforcementStatus,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum EnforcementStatus {
    Inactive,
    Enforcing,
}
#[derive(Deserialize)]
struct RateGroup {
    group: Group,
    models: Option<Vec<String>>,
    limits: Vec<Rate>,
}
#[derive(Deserialize)]
struct Group {
    id: String,
    display_name: Option<String>,
    r#type: String,
}
#[derive(Deserialize)]
struct Rate {
    r#type: String,
    value: u64,
}
fn add(target: &mut u64, value: u64) -> Result<()> {
    *target = target.checked_add(value).ok_or("官方用量总数溢出")?;
    Ok(())
}
fn money_totals(rows: impl IntoIterator<Item = (String, f64)>) -> Result<Vec<BilledCost>> {
    let mut totals = BTreeMap::<String, f64>::new();
    for (currency, amount) in rows {
        if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_alphabetic()) {
            return Err("官方费用币种无效".into());
        }
        let total = totals.entry(currency.to_ascii_uppercase()).or_default();
        *total += amount;
        if !total.is_finite() {
            return Err("官方费用总额无效".into());
        }
    }
    Ok(totals
        .into_iter()
        .map(|(currency, amount)| BilledCost { currency, amount })
        .collect())
}
pub(super) async fn collect(
    configuration: BillingConfiguration,
    key: &str,
    starting: i64,
) -> Result<BillingReport> {
    let client = Client::new(configuration.service, key, &configuration.organization_id)?;
    collect_with_client(configuration, &client, starting).await
}
async fn collect_with_client(
    configuration: BillingConfiguration,
    client: &Client,
    starting: i64,
) -> Result<BillingReport> {
    let observed = Utc::now().timestamp_millis();
    // Analytics rounds whole-range queries down to UTC hours; preserve its real interval.
    let precision = match configuration.service {
        BillingService::OpenAi => 1000,
        BillingService::Anthropic => 3_600_000,
    };
    let ending = observed - observed.rem_euclid(precision);
    if ending <= starting {
        return Err("本月统计时段尚未产生，请稍后刷新".into());
    }
    let mut report = BillingReport {
        configuration: configuration.clone(),
        starting_at_ms: starting,
        ending_at_ms: ending,
        observed_at_ms: observed,
        data_refreshed_at_ms: None,
        usage: None,
        costs: None,
        spend_limit: None,
        rate_limits: None,
        problems: Vec::new(),
    };
    let mut params = match configuration.service {
        BillingService::OpenAi => vec![
            ("start_time".into(), (starting / 1000).to_string()),
            ("end_time".into(), (ending / 1000).to_string()),
            ("bucket_width".into(), "1d".into()),
            ("limit".into(), "31".into()),
        ],
        BillingService::Anthropic => vec![
            (
                "starting_at".into(),
                Utc.timestamp_millis_opt(starting)
                    .single()
                    .expect("month")
                    .to_rfc3339(),
            ),
            (
                "ending_at".into(),
                Utc.timestamp_millis_opt(ending)
                    .single()
                    .expect("now")
                    .to_rfc3339(),
            ),
            ("limit".into(), "1000".into()),
        ],
    };
    match &configuration.scope {
        BillingScope::Organization => {}
        BillingScope::Project { id } => params.push(("project_ids[]".into(), id.clone())),
        BillingScope::Workspace { id } => params.push(("workspace_ids[]".into(), id.clone())),
        BillingScope::ApiKey { id } => params.push(("api_key_ids[]".into(), id.clone())),
    }
    match configuration.service {
        BillingService::OpenAi => {
            let (usage, costs) = tokio::join!(
                client.pages::<Bucket<OpenAiUsage>>(
                    "/v1/organization/usage/completions",
                    params.clone()
                ),
                client.pages::<Bucket<OpenAiCost>>("/v1/organization/costs", params)
            );
            match usage.and_then(|(rows, _)| {
                let mut total = BillingUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_input_tokens: Some(0),
                    cache_creation_input_tokens: Some(0),
                    requests: 0,
                };
                for row in rows.into_iter().flat_map(|r| r.results) {
                    add(&mut total.input_tokens, row.input_tokens)?;
                    add(&mut total.output_tokens, row.output_tokens)?;
                    add(&mut total.requests, row.num_model_requests)?;
                    for (target, value) in [
                        (&mut total.cached_input_tokens, row.input_cached_tokens),
                        (
                            &mut total.cache_creation_input_tokens,
                            row.input_cache_write_tokens,
                        ),
                    ] {
                        match (target.as_mut(), value) {
                            (Some(target), Some(value)) => add(target, value)?,
                            _ => *target = None,
                        }
                    }
                }
                Ok(total)
            }) {
                Ok(value) => report.usage = Some(value),
                Err(error) => report.problems.push(format!("用量：{error}")),
            }
            match costs.and_then(|(rows, _)| {
                money_totals(
                    rows.into_iter()
                        .flat_map(|r| r.results)
                        .map(|r| (r.amount.currency, r.amount.value)),
                )
            }) {
                Ok(value) => report.costs = Some(value),
                Err(error) => report.problems.push(format!("费用：{error}")),
            }
            let path = match &configuration.scope {
                BillingScope::Organization => Some("/v1/organization/spend_limit".to_owned()),
                BillingScope::Project { id } => {
                    Some(format!("/v1/organization/projects/{id}/spend_limit"))
                }
                _ => None,
            };
            if let Some(path) = path {
                match client.get::<SpendLimit>(&path, &[]).await {
                    Ok(limit) => {
                        let Month::Month = limit.interval;
                        report.spend_limit = Some(BillingSpendLimit {
                            amount: limit.threshold_amount as f64 / 100.,
                            currency: limit.currency,
                            enforced: matches!(
                                limit.enforcement.status,
                                EnforcementStatus::Enforcing
                            ),
                            interval: "month".into(),
                        });
                    }
                    Err(error) => report.problems.push(format!("支出限额：{error}")),
                }
            }
        }
        BillingService::Anthropic => {
            let (usage, costs) = tokio::join!(
                client.pages::<AnthropicUsageRow>(
                    "/v1/organizations/analytics/usage",
                    params.clone()
                ),
                client.pages::<AnthropicCostRow>("/v1/organizations/analytics/cost", params)
            );
            match usage.and_then(|(rows, refreshed)| {
                let mut total = BillingUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    cached_input_tokens: Some(0),
                    cache_creation_input_tokens: Some(0),
                    requests: 0,
                };
                for row in rows {
                    let row = row.usage;
                    add(&mut total.input_tokens, row.uncached_input_tokens)?;
                    add(&mut total.input_tokens, row.cache_read_input_tokens)?;
                    add(
                        total.cached_input_tokens.as_mut().expect("initialized"),
                        row.cache_read_input_tokens,
                    )?;
                    for value in row.cache_creation.values() {
                        add(&mut total.input_tokens, *value)?;
                        add(
                            total
                                .cache_creation_input_tokens
                                .as_mut()
                                .expect("initialized"),
                            *value,
                        )?;
                    }
                    add(&mut total.output_tokens, row.output_tokens)?;
                    add(&mut total.requests, row.requests)?;
                }
                Ok((total, refreshed))
            }) {
                Ok((value, refreshed)) => {
                    report.usage = Some(value);
                    report.data_refreshed_at_ms = refreshed;
                }
                Err(error) => report.problems.push(format!("用量：{error}")),
            }
            match costs.and_then(|(rows, refreshed)| {
                Ok((
                    money_totals(
                        rows.into_iter()
                            .map(|r| (r.cost.currency, r.cost.amount / 100.)),
                    )?,
                    refreshed,
                ))
            }) {
                Ok((value, refreshed)) => {
                    report.costs = Some(value);
                    if let Some(time) = refreshed {
                        report.data_refreshed_at_ms = Some(
                            report
                                .data_refreshed_at_ms
                                .map_or(time, |previous| previous.min(time)),
                        );
                    }
                }
                Err(error) => report.problems.push(format!("费用：{error}")),
            }
            let path = match &configuration.scope {
                BillingScope::Organization => Some("/v1/organizations/rate_limits".to_owned()),
                BillingScope::Workspace { id } => {
                    Some(format!("/v1/organizations/workspaces/{id}/rate_limits"))
                }
                _ => None,
            };
            if let Some(path) = path {
                let query = if matches!(configuration.scope, BillingScope::Workspace { .. }) {
                    vec![("include_inherited".into(), "true".into())]
                } else {
                    vec![]
                };
                match client.pages::<RateGroup>(&path, query).await {
                    Ok((rows, _)) => {
                        report.rate_limits = Some(
                            rows.into_iter()
                                .flat_map(|row| {
                                    row.limits.into_iter().map(move |limit| BillingRateLimit {
                                        group_id: row.group.id.clone(),
                                        name: row
                                            .group
                                            .display_name
                                            .clone()
                                            .unwrap_or_else(|| row.group.r#type.clone()),
                                        model_ids: row.models.clone().unwrap_or_default(),
                                        limit_type: limit.r#type,
                                        value: limit.value,
                                    })
                                })
                                .collect(),
                        )
                    }
                    Err(error) => report.problems.push(format!("速率限制：{error}")),
                }
            }
        }
    }
    if report.usage.is_none()
        && report.costs.is_none()
        && report.spend_limit.is_none()
        && report.rate_limits.is_none()
    {
        return Err(report.problems.join("；"));
    }
    report.observed_at_ms = Utc::now().timestamp_millis();
    Ok(report)
}

#[cfg(test)]
#[path = "billing_http_tests.rs"]
mod tests;
