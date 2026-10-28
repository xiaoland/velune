//! Whether a failed attempt ends the request or leaves the next candidate open.
use serde_json::Value;
use std::time::Duration;
use velune_ai::{
    Payload,
    http::{Header, ResponseMeta},
};

/// How an attempt ended. Anything delivered, including a cancelled delivery,
/// settles the request; a failure before delivery is held back so the
/// request may continue with the next candidate.
pub(super) enum AttemptEnd {
    Completed,
    Settled,
    Interrupted { cooldown: Option<Duration> },
    Failed(PendingFailure),
}
pub(super) struct PendingFailure {
    pub(super) meta: Option<ResponseMeta>,
    pub(super) body: Option<Payload<Vec<u8>>>,
    pub(super) status: u16,
    pub(super) message: &'static str,
    pub(super) next: NextCandidate,
    pub(super) authentication: bool,
}
pub(super) enum NextCandidate {
    /// Replay is forbidden, independently of any provider health observation.
    Stop { cooldown: Option<Duration> },
    /// This target could not serve the request; another one may.
    Allowed { cooldown: Option<Duration> },
}
impl NextCandidate {
    pub(super) fn cooldown(&self) -> Option<Duration> {
        match self {
            Self::Stop { cooldown } | Self::Allowed { cooldown } => *cooldown,
        }
    }
}
impl PendingFailure {
    pub(super) fn new(status: u16, message: &'static str, next: NextCandidate) -> Self {
        Self {
            meta: None,
            body: None,
            status,
            message,
            next,
            authentication: false,
        }
    }
}

/// A 429 keeps the provider at the back of routes for its Retry-After, or a minute.
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_COOLDOWN: Duration = Duration::from_secs(24 * 60 * 60);

/// Whether a failure before delivery leaves the other candidates open.
/// Requests the target refused (4xx other than 429) and refused accounts
/// (401/403) stop; overload, rate limits and failures to reach it do not.
pub(super) fn next_candidate(
    rate_limited: bool,
    cancelled: bool,
    transport_failure: bool,
    status: Option<u16>,
    headers: &[Header],
) -> NextCandidate {
    if cancelled {
        return NextCandidate::Stop { cooldown: None };
    }
    match status {
        _ if rate_limited || status == Some(429) => NextCandidate::Allowed {
            cooldown: Some(
                headers
                    .iter()
                    .find(|header| header.name.eq_ignore_ascii_case("retry-after"))
                    .and_then(|header| header.value.trim().parse::<u64>().ok())
                    .map_or(DEFAULT_COOLDOWN, Duration::from_secs)
                    .min(MAX_COOLDOWN),
            ),
        },
        // No response: the connection failed or timed out before headers.
        // HTTP generation may be retried before downstream delivery. Execution
        // backends with side effects apply their stricter submitted gate later.
        None if transport_failure => NextCandidate::Allowed {
            cooldown: Some(Duration::from_secs(5)),
        },
        Some(status) if status >= 500 => NextCandidate::Allowed {
            cooldown: Some(Duration::from_secs(5)),
        },
        _ => NextCandidate::Stop { cooldown: None },
    }
}

pub(super) fn structured_quota_error(body: Option<&Vec<u8>>) -> bool {
    let Some(value) = body.and_then(|body| serde_json::from_slice::<Value>(body).ok()) else {
        return false;
    };
    [
        value["error"]["code"].as_str(),
        value["error"]["type"].as_str(),
    ]
    .into_iter()
    .flatten()
    .any(|code| {
        matches!(
            code,
            "insufficient_quota" | "usage_limit_reached" | "quota_exceeded"
        )
    })
}
