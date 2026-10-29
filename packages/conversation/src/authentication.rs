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

/// One step of a login, as the source adapter reports it; the wire form is
/// `{"type": "prompt" | "notify" | "result", ...}`.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "type")]
pub enum AuthenticationEvent {
    /// The login waits for an answer to `prompt`, sent back under `id`.
    #[serde(rename = "prompt")]
    Asked {
        id: String,
        prompt: AuthenticationPrompt,
    },
    #[serde(rename = "notify")]
    Notified {
        notification: AuthenticationNotification,
    },
    /// The login ended; `ok` means its credential was written.
    #[serde(rename = "result")]
    Finished {
        ok: bool,
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        cancelled: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationPrompt {
    pub kind: String,
    pub text: String,
    pub options: Option<Vec<SettingOption>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
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
    pub fn result(ok: bool, error: Option<String>, cancelled: bool) -> Self {
        Self::Finished {
            ok,
            error,
            cancelled,
        }
    }

    pub fn notify(notification: AuthenticationNotification) -> Self {
        Self::Notified { notification }
    }

    /// The login finished and its credential was written.
    pub fn succeeded(&self) -> bool {
        matches!(self, Self::Finished { ok: true, .. })
    }
}
