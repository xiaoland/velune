use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureKind {
    Invalid,
    Unsupported,
    Conflict,
    StaleInteraction,
    OutstandingWork,
    Unavailable,
    Internal,
    History,
    ProviderImport,
    RuntimeManagement,
    Io,
    Contract,
    Closed,
}
/// A user-visible business error; transport failures have their own contract.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct Failure {
    pub kind: FailureKind,
    pub message: String,
    pub code: String,
    pub phase: String,
    pub current_revision: Option<crate::Revision>,
}
impl Failure {
    pub fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            code: "application_failed".into(),
            phase: "application".into(),
            current_revision: None,
        }
    }
    pub fn invalid(message: &'static str) -> Self {
        Self::new(FailureKind::Invalid, message)
    }
}
impl From<serde_json::Error> for Failure {
    fn from(_: serde_json::Error) -> Self {
        Self::invalid("运行时设置格式无效")
    }
}
