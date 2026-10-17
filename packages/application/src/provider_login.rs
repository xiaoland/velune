//! Provider login adapters share progress and staging semantics without sharing secrets.
use serde_json::Value;
pub(crate) enum Login {
    Codex(crate::subscription_auth::Login),
    Claude(crate::claude_cli::Login),
}
impl Login {
    pub(crate) fn poll(&mut self) -> Value {
        match self {
            Self::Codex(v) => v.poll(),
            Self::Claude(v) => v.poll(),
        }
    }
    pub(crate) fn credential_id(&self) -> &str {
        match self {
            Self::Codex(v) => v.credential_id(),
            Self::Claude(v) => v.credential_id(),
        }
    }
    pub(crate) fn commit(&mut self) {
        match self {
            Self::Codex(v) => v.commit(),
            Self::Claude(v) => v.commit(),
        }
    }
    pub(crate) fn cancel(&mut self) {
        match self {
            Self::Codex(v) => v.cancel(),
            Self::Claude(v) => v.cancel(),
        }
    }
    pub(crate) fn is_running(&self) -> bool {
        match self {
            Self::Codex(v) => v.is_running(),
            Self::Claude(v) => v.is_running(),
        }
    }
    pub(crate) fn claude_status(&self) -> Option<&crate::claude_cli::Status> {
        match self {
            Self::Claude(v) => v.status(),
            _ => None,
        }
    }
}
