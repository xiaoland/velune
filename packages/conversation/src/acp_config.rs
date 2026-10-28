//! Session configuration an ACP Agent offers, as it offers it.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum AcpConfigValue {
    Select(String),
    Boolean(bool),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AcpConfigChoice {
    pub value: String,
    pub name: String,
    pub description: Option<String>,
    pub group: Option<String>,
    pub group_name: Option<String>,
}
