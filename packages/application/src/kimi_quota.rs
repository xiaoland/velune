//! First-party subscription metadata has a separate, fixed credential purpose.
use serde_json::Value;
use std::time::Duration;
use velune_gateway::QuotaWindow;
const CN_BASE: &str = "https://api.kimi.com/coding/v1";
const GLOBAL_BASE: &str = "https://api.kimi.ai/coding/v1";
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Capture {
    pub region: &'static str,
    pub key: String,
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Owner {
    pub region: &'static str,
    pub user_id: String,
}
impl Owner {
    pub(crate) fn label(&self) -> String {
        use sha2::Digest;
        let digest = format!(
            "{:x}",
            sha2::Sha256::digest(format!("{}\0{}", self.region, self.user_id).as_bytes())
        );
        format!("账户 · {}", &digest[digest.len() - 8..])
    }
}
impl Capture {
    async fn get(&self, path: &str) -> Result<Value, ()> {
        if self.key.is_empty() {
            return Err(());
        }
        let base = match self.region {
            "mainland-cn" => CN_BASE,
            "global" => GLOBAL_BASE,
            _ => return Err(()),
        };
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
            .get(format!("{base}/{path}"))
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
        serde_json::from_slice(&body).map_err(|_| ())
    }
    pub(crate) async fn owner(&self) -> Result<Owner, ()> {
        let value = self.get("me").await?;
        let id = value["user_id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
            .ok_or(())?;
        Ok(Owner {
            region: self.region,
            user_id: id.to_owned(),
        })
    }
    pub(crate) async fn windows(&self, now: i64, ttl: i64) -> Result<Vec<QuotaWindow>, ()> {
        parse(&self.get("usages").await?, now, ttl)
    }
}
fn parse(value: &Value, now: i64, ttl: i64) -> Result<Vec<QuotaWindow>, ()> {
    let usage = value["usages"].as_object().ok_or(())?;
    let mut windows = Vec::new();
    for key in [
        "limit_5h",
        "limit_7d",
        "limit_month_total",
        "limit_month_code",
    ] {
        let Some(value) = usage.get(key).filter(|value| !value.is_null()) else {
            continue;
        };
        let ratio = value["used_ratio"]
            .as_f64()
            .or_else(|| {
                value["used_ratio"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .and_then(|s| s.trim().parse::<f64>().ok())
            })
            .filter(|r| r.is_finite() && (0.0..=1.0).contains(r))
            .ok_or(())?;
        let reset = chrono::DateTime::parse_from_rfc3339(value["reset_time"].as_str().ok_or(())?)
            .map_err(|_| ())?
            .timestamp_millis();
        if now <= 0 || reset <= now {
            return Err(());
        }
        windows.push(QuotaWindow {
            id: format!("kimi:{key}"),
            model_ids: None,
            used_percent: ratio * 100.0,
            reset_at_ms: Some(reset),
            observed_at_ms: now,
            expires_at_ms: reset.min(now + ttl),
        });
    }
    if windows.is_empty() {
        return Err(());
    }
    Ok(windows)
}
