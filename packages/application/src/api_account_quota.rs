//! An API service's own account metadata, read with the provider's key from
//! the official host its model requests already reach; never another host.
use crate::QuotaBalanceObservation;
use chrono::{Datelike, Duration as Days, TimeZone, Utc};
use serde_json::Value;
use std::time::Duration;
use velune_application_api::ApiAccountService as Service;
use velune_gateway::QuotaWindow;

fn url(service: Service) -> String {
    let host = service.host();
    match service {
        Service::DeepSeek => format!("https://{host}/user/balance"),
        Service::MoonshotCn | Service::MoonshotGlobal => {
            format!("https://{host}/v1/users/me/balance")
        }
        Service::OpenRouter => format!("https://{host}/api/v1/key"),
    }
}

pub(crate) struct Observation {
    pub windows: Vec<QuotaWindow>,
    pub balances: Vec<QuotaBalanceObservation>,
    pub available: Option<bool>,
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Capture {
    pub service: Service,
    pub key: String,
}
impl Capture {
    pub(crate) async fn observe(&self, now: i64, ttl: i64) -> Result<Observation, ()> {
        if self.key.is_empty() {
            return Err(());
        }
        let mut authorization =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", self.key))
                .map_err(|_| ())?;
        authorization.set_sensitive(true);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .user_agent("Velune/0.1")
            .build()
            .map_err(|_| ())?;
        let mut response = client
            .get(url(self.service))
            .header("authorization", authorization)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|_| ())?;
        if !response.status().is_success() {
            return Err(());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
            if body.len() + chunk.len() > 256 * 1024 {
                return Err(());
            }
            body.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&body).map_err(|_| ())?;
        parse(self.service, &value, now, ttl)
    }
}

/// A finite amount, given as a JSON number or a decimal string.
fn amount(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| {
            value
                .as_str()
                .filter(|s| s.len() <= 64)?
                .trim()
                .parse()
                .ok()
        })
        .filter(|n: &f64| n.is_finite())
}
pub(crate) fn parse(
    service: Service,
    value: &Value,
    now: i64,
    ttl: i64,
) -> Result<Observation, ()> {
    if now <= 0 {
        return Err(());
    }
    let balance = |id: &str, amount: f64, unit: &str| QuotaBalanceObservation {
        id: id.into(),
        amount,
        unit: unit.into(),
        observed_at_ms: now,
    };
    match service {
        Service::DeepSeek => {
            let available = value["is_available"].as_bool().ok_or(())?;
            let infos = value["balance_infos"].as_array().ok_or(())?;
            let mut balances = Vec::new();
            for info in infos {
                let currency = info["currency"]
                    .as_str()
                    .filter(|c| matches!(*c, "CNY" | "USD"))
                    .ok_or(())?;
                if balances
                    .iter()
                    .any(|b: &QuotaBalanceObservation| b.unit == currency)
                {
                    return Err(());
                }
                let total = amount(&info["total_balance"]).ok_or(())?;
                balances.push(balance(&format!("balance:{currency}"), total, currency));
            }
            Ok(Observation {
                windows: Vec::new(),
                balances,
                available: Some(available),
            })
        }
        Service::MoonshotCn | Service::MoonshotGlobal => {
            let currency = if service == Service::MoonshotCn {
                "CNY"
            } else {
                "USD"
            };
            if value["code"].as_i64() != Some(0) || value["status"].as_bool() != Some(true) {
                return Err(());
            }
            let available = amount(&value["data"]["available_balance"]).ok_or(())?;
            // Requests fail once the available balance is not positive.
            Ok(Observation {
                windows: Vec::new(),
                balances: vec![balance("balance", available, currency)],
                available: Some(available > 0.0),
            })
        }
        Service::OpenRouter => {
            let data = value["data"].as_object().ok_or(())?;
            let Some(limit) = data.get("limit").filter(|v| !v.is_null()) else {
                // A key without a spending limit reports no ceiling to observe.
                return Ok(Observation {
                    windows: Vec::new(),
                    balances: Vec::new(),
                    available: None,
                });
            };
            let limit = amount(limit).filter(|l| *l >= 0.0).ok_or(())?;
            let remaining = amount(data.get("limit_remaining").ok_or(())?).ok_or(())?;
            if remaining > limit + 1e-9 {
                return Err(());
            }
            let used = if limit > 0.0 {
                ((limit - remaining) / limit * 100.0).clamp(0.0, 100.0)
            } else {
                100.0
            };
            let period = match data.get("limit_reset") {
                None | Some(Value::Null) => None,
                Some(Value::String(reset)) => Some(period(reset, now).ok_or(())?),
                _ => return Err(()),
            };
            Ok(Observation {
                windows: vec![QuotaWindow {
                    id: "openrouter:keyLimit".into(),
                    model_ids: None,
                    used_percent: used,
                    reset_at_ms: period.map(|(reset, _)| reset),
                    window_ms: period.map(|(_, length)| length),
                    observed_at_ms: now,
                    expires_at_ms: period.map_or(now + ttl, |(reset, _)| reset.min(now + ttl)),
                }],
                balances: vec![balance(
                    "openrouter:keyLimitRemaining",
                    remaining.max(0.0),
                    "USD",
                )],
                available: None,
            })
        }
    }
}
/// The next reset and period length of a key limit. OpenRouter documents
/// these as UTC days, Monday-started weeks and calendar months.
fn period(reset: &str, now: i64) -> Option<(i64, i64)> {
    let now = Utc.timestamp_millis_opt(now).single()?;
    let today = now.date_naive();
    let (start, end) = match reset {
        "daily" => (today, today + Days::days(1)),
        "weekly" => {
            let monday = today - Days::days(i64::from(today.weekday().num_days_from_monday()));
            (monday, monday + Days::days(7))
        }
        "monthly" => {
            let first = today.with_day(1)?;
            let next = if first.month() == 12 {
                first.with_year(first.year() + 1)?.with_month(1)?
            } else {
                first.with_month(first.month() + 1)?
            };
            (first, next)
        }
        _ => return None,
    };
    let millis =
        |date: chrono::NaiveDate| Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis());
    let (start, end) = (millis(start)?, millis(end)?);
    Some((end, end - start))
}
