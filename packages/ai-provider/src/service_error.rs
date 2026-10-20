//! Service-specific facts; dispatch policy remains with the consumer.
use crate::config::RequestProfile;
use serde_json::Value;
#[derive(Clone, Copy)]
pub(crate) enum ServiceError {
    Authentication,
    Permission,
    QuotaExhausted,
    RateLimited,
    Overloaded,
}
/// Only documented message locations and prefixes are trusted. Unknown shapes
/// retain protocol status semantics; no substring search or speculative codes.
pub(crate) fn classify(
    profile: RequestProfile,
    status: u16,
    body: Option<&[u8]>,
) -> Option<ServiceError> {
    if profile != RequestProfile::KimiCoding {
        return None;
    }
    let bytes = body.filter(|bytes| bytes.len() <= 64 * 1024)?;
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let message = value
        .get("error")
        .and_then(|error| error.get("message"))
        .or_else(|| value.get("message"))?
        .as_str()?;
    let starts = |prefix: &str| message.starts_with(prefix);
    match status {
        401 if starts("The API Key appears to be invalid or may have expired")
            || message == "Invalid Authentication" =>
        {
            Some(ServiceError::Authentication)
        }
        401 if starts("Your current subscription does not have access to k3.")
            || starts(
                "Your current subscription does not have access to kimi-for-coding-highspeed.",
            )
            || starts("Your current plan supports only k3 up to 256K context.")
            || starts("Your current plan supports only kimi-k3 up to 256K context.")
            || starts("Your model id does not exist, recognized as other:") =>
        {
            Some(ServiceError::Permission)
        }
        403 if starts("You've reached your 5-hour usage limit.")
            || starts("You've reached your weekly (7-day) usage limit.")
            || starts("You've reached your monthly usage limit for this billing cycle.") =>
        {
            Some(ServiceError::QuotaExhausted)
        }
        429 if starts("The engine is currently overloaded") => Some(ServiceError::Overloaded),
        429 if starts("We're receiving too many requests") => Some(ServiceError::RateLimited),
        500 if starts("unauthenticated: not_found: 未找到该账号")
            || starts("unauthenticated: failed_precondition: 因违反用户协议，该账号已被禁用")
            || starts(
                "unauthenticated: failed_precondition: 因违反用户协议，该账号已被暂时禁用",
            )
            || starts("unauthenticated: failed_precondition: 因违反用户协议，该账号已被禁言") =>
        {
            Some(ServiceError::Permission)
        }
        _ => None,
    }
}
pub(crate) const USER_AGENT: &str = concat!("Velune/", env!("CARGO_PKG_VERSION"));
