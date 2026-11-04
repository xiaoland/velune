//! Private query credentials. Neither Debug nor public configuration exposes secrets.
use serde::{Deserialize, Serialize};
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AccessKey {
    pub id: String,
    pub secret: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum QueryCredential {
    ApiKey { value: String },
    AccessKeyPair { key: AccessKey },
}
impl std::fmt::Debug for QueryCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("QueryCredential([redacted])")
    }
}
impl QueryCredential {
    pub(crate) fn api_key(&self) -> Option<&str> {
        match self {
            Self::ApiKey { value } => Some(value),
            Self::AccessKeyPair { .. } => None,
        }
    }
    pub(crate) fn access_key(&self) -> Option<&AccessKey> {
        match self {
            Self::AccessKeyPair { key } => Some(key),
            Self::ApiKey { .. } => None,
        }
    }
}
pub(crate) fn valid_secret(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= 4096
        && !value.chars().any(char::is_control)
}
