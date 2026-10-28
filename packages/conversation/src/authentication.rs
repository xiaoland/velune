//! Progress of an interactive provider login, as its source adapter reports it.
use crate::SettingOption;
use serde::{Deserialize, Serialize};

/// A login's state and the events since the last poll.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoginProgress {
    pub running: bool,
    pub events: Vec<AuthenticationEvent>,
}

impl LoginProgress {
    pub fn idle() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationEvent {
    #[serde(rename = "type")]
    pub type_id: String,
    pub id: Option<String>,
    pub prompt: Option<AuthenticationPrompt>,
    pub notification: Option<AuthenticationNotification>,
    pub ok: Option<bool>,
    pub error: Option<String>,
    pub cancelled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationPrompt {
    pub kind: String,
    pub text: String,
    pub options: Option<Vec<SettingOption>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationNotification {
    pub kind: String,
    pub id: Option<String>,
    pub text: Option<String>,
    pub url: Option<String>,
    pub instructions: Option<String>,
    pub user_code: Option<String>,
    pub verification_uri: Option<String>,
}

impl AuthenticationEvent {
    pub fn result(ok: bool, error: Option<String>, cancelled: Option<bool>) -> Self {
        Self {
            type_id: "result".into(),
            id: None,
            prompt: None,
            notification: None,
            ok: Some(ok),
            error,
            cancelled,
        }
    }

    pub fn notify(notification: AuthenticationNotification) -> Self {
        Self {
            type_id: "notify".into(),
            id: None,
            prompt: None,
            notification: Some(notification),
            ok: None,
            error: None,
            cancelled: None,
        }
    }

    /// The login finished and its credential was written.
    pub fn succeeded(&self) -> bool {
        self.type_id == "result" && self.ok == Some(true)
    }
}
