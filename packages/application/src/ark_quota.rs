//! Fixed Coding Plan control-plane query; these credentials never reach inference.
use crate::quota_authentication::AccessKey;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::{Digest, Sha256};
use velune_gateway::QuotaWindow;

const HOST: &str = "open.volcengineapi.com";
const QUERY: &str = "Action=GetCodingPlanUsage&Version=2024-01-01";
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Capture {
    pub key: AccessKey,
    pub generation: u64,
    pub confirmed: bool,
}
fn hex(data: impl AsRef<[u8]>) -> String {
    data.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}
fn mac(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}
impl Capture {
    pub(crate) async fn windows(&self, now: i64, ttl: i64) -> Result<Vec<QuotaWindow>, ()> {
        let time = chrono::DateTime::from_timestamp_millis(now).ok_or(())?;
        let date = time.format("%Y%m%d").to_string();
        let stamp = time.format("%Y%m%dT%H%M%SZ").to_string();
        let body = "{}";
        let digest = hex(Sha256::digest(body.as_bytes()));
        let signed = "content-type;host;x-content-sha256;x-date";
        let canonical = format!(
            "POST\n/\n{QUERY}\ncontent-type:application/json\nhost:{HOST}\nx-content-sha256:{digest}\nx-date:{stamp}\n\n{signed}\n{digest}"
        );
        let scope = format!("{date}/cn-beijing/ark/request");
        let to_sign = format!(
            "HMAC-SHA256\n{stamp}\n{scope}\n{}",
            hex(Sha256::digest(canonical.as_bytes()))
        );
        let mut key = mac(self.key.secret.as_bytes(), &date);
        for part in ["cn-beijing", "ark", "request"] {
            key = mac(&key, part);
        }
        let signature = hex(mac(&key, &to_sign));
        let mut authorization = reqwest::header::HeaderValue::from_str(&format!(
            "HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}",
            self.key.id
        ))
        .map_err(|_| ())?;
        authorization.set_sensitive(true);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .user_agent("Velune/0.1")
            .build()
            .map_err(|_| ())?;
        let mut response = client
            .post(format!("https://{HOST}/?{QUERY}"))
            .header("content-type", "application/json")
            .header("x-date", stamp)
            .header("x-content-sha256", digest)
            .header("authorization", authorization)
            .body(body)
            .send()
            .await
            .map_err(|_| ())?;
        if !response.status().is_success() {
            return Err(());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
            if bytes.len() + chunk.len() > 256 * 1024 {
                return Err(());
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| ())?;
        if value["ResponseMetadata"]
            .get("Error")
            .is_some_and(|v| !v.is_null())
        {
            return Err(());
        }
        parse(&value, now, ttl)
    }
}
fn number(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str()?.parse().ok())
        .filter(|v| v.is_finite())
}
fn parse(value: &Value, now: i64, ttl: i64) -> Result<Vec<QuotaWindow>, ()> {
    let usage = value["Result"]["QuotaUsage"].as_array().ok_or(())?;
    let mut windows = Vec::new();
    for item in usage {
        let level = item["Level"].as_str().ok_or(())?;
        if !matches!(level, "session" | "weekly" | "monthly") {
            continue;
        }
        let used = number(&item["Percent"])
            .filter(|v| (0.0..=100.0).contains(v))
            .ok_or(())?;
        let reset = if item["ResetTimestamp"].is_null() {
            None
        } else {
            let n = number(&item["ResetTimestamp"])
                .filter(|v| v.fract() == 0.0 && *v < i64::MAX as f64 / 1000.0)
                .ok_or(())?;
            if n <= 0.0 {
                None
            } else {
                let reset = (n as i64).checked_mul(1000).ok_or(())?;
                if reset <= now {
                    return Err(());
                }
                Some(reset)
            }
        };
        let id = format!("ark:{level}");
        if windows.iter().any(|w: &QuotaWindow| w.id == id) {
            return Err(());
        }
        windows.push(QuotaWindow {
            id,
            model_ids: None,
            used_percent: used,
            reset_at_ms: reset,
            observed_at_ms: now,
            expires_at_ms: reset.map_or(now + ttl, |r| r.min(now + ttl)),
        });
    }
    if windows.is_empty() {
        return Err(());
    }
    Ok(windows)
}
