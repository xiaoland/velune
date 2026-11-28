//! The adapter a runtime instance runs on.
use serde::{Deserialize, Serialize};

/// One adapter for one Harness version range; its identifier names the
/// range (`codex-0.166`). A Harness release that breaks an adapter gets a
/// new kind rather than a wider range.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, uniffi::Enum,
)]
#[uniffi::export(Display)]
pub enum RuntimeKind {
    #[serde(rename = "pi-1.0")]
    Pi,
    #[serde(rename = "codex-0.166")]
    Codex166,
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
        Self::Codex166,
        Self::DeepSeek,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Pi => "pi-1.0",
            Self::Codex166 => "codex-0.166",
            Self::ClaudeCode => "claude-code-2.1",
            Self::DeepSeek => "dsh-acp-0.2",
            Self::Acp => "acp-v1",
        }
    }
}

impl std::fmt::Display for RuntimeKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.id())
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeKind;
    use serde::Deserialize;

    #[test]
    fn unsupported_adapter_identifier_is_rejected_by_wire_decode() {
        let retired =
            serde::de::value::StrDeserializer::<serde::de::value::Error>::new("codex-0.159");
        assert!(RuntimeKind::deserialize(retired).is_err());
    }
}
