//! Session configuration an ACP Agent offers, as it offers it.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AcpConfigOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Optional agent-owned UX category; config identity never depends on it.
    pub category: Option<String>,
    pub value: AcpConfigValue,
    pub choices: Vec<AcpConfigChoice>,
}

impl AcpConfigOption {
    /// These session controls change the target or its permissions, so they
    /// cannot be used as a remembered automatic compaction control.
    pub fn can_bind_auto_compaction(&self) -> bool {
        fn is_reserved(value: &str) -> bool {
            matches!(
                value.replace('-', "_").trim_start_matches('_'),
                "model"
                    | "mode"
                    | "permission"
                    | "permissions"
                    | "permission_mode"
                    | "approval"
                    | "approval_mode"
                    | "sandbox"
                    | "sandbox_mode"
                    | "auth"
                    | "thought_level"
                    | "reasoning"
                    | "reasoning_effort"
            )
        }
        !is_reserved(&self.id) && !self.category.as_deref().is_some_and(is_reserved)
    }

    /// ACP leaves compaction settings to Agents. A generic model setting or
    /// localized name alone cannot identify an automatic compaction control.
    pub fn is_auto_compaction(&self) -> bool {
        fn has_compaction_id(id: &str) -> bool {
            matches!(
                id.replace('-', "_").trim_start_matches('_'),
                "auto_compaction"
                    | "auto_compact"
                    | "auto_compaction_enabled"
                    | "auto_compact_enabled"
                    | "auto_compaction_window"
                    | "auto_compact_window"
                    | "auto_compaction_threshold"
                    | "auto_compact_threshold"
            )
        }
        self.can_bind_auto_compaction()
            && (has_compaction_id(&self.id)
                || self.category.as_deref().is_some_and(has_compaction_id))
    }

    pub fn accepts(&self, value: &AcpConfigValue) -> bool {
        match (&self.value, value) {
            (AcpConfigValue::Boolean(_), AcpConfigValue::Boolean(_)) => true,
            (AcpConfigValue::Select(_), AcpConfigValue::Select(value)) => {
                self.choices.iter().any(|choice| choice.value == *value)
            }
            _ => false,
        }
    }
}

pub fn current_model(options: &[AcpConfigOption]) -> Option<&str> {
    match &options
        .iter()
        .find(|option| option.category.as_deref() == Some("model"))?
        .value
    {
        AcpConfigValue::Select(id) => Some(id),
        AcpConfigValue::Boolean(_) => None,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum AcpConfigValue {
    Select(String),
    Boolean(bool),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AcpConfigChoice {
    pub value: String,
    pub name: String,
    pub description: Option<String>,
    pub group: Option<String>,
    pub group_name: Option<String>,
}
