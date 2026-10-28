//! The adapter a runtime instance runs on.
use serde::{Deserialize, Serialize};

/// One adapter for one Harness version range; its identifier names the
/// range (`codex-0.159`). A Harness release that breaks an adapter gets a
/// new kind rather than a wider range.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, uniffi::Enum,
)]
#[uniffi::export(Display)]
pub enum RuntimeKind {
    #[serde(rename = "pi-1.0")]
    Pi,
    #[serde(rename = "codex-0.159")]
    Codex,
    #[serde(rename = "claude-code-2.1")]
    ClaudeCode,
    #[serde(rename = "dsh-acp-0.2")]
    DeepSeek,
    /// Any Agent speaking ACP protocol version 1.
    #[serde(rename = "acp-v1")]
    Acp,
}

impl RuntimeKind {
    pub const ALL: [Self; 5] = [
        Self::Acp,
        Self::ClaudeCode,
        Self::Pi,
        Self::Codex,
        Self::DeepSeek,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Pi => "pi-1.0",
            Self::Codex => "codex-0.159",
            Self::ClaudeCode => "claude-code-2.1",
            Self::DeepSeek => "dsh-acp-0.2",
            Self::Acp => "acp-v1",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }
}

impl std::fmt::Display for RuntimeKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.id())
    }
}
