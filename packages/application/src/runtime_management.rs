//! Native source resources, independent of active conversations and collaboration grants.
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeResourceKind {
    UserInstructions,
    McpServers,
    Skills,
    NativeAgentDefinitions,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeResourceScope {
    User,
    Project,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeResourceKey {
    pub kind: RuntimeResourceKind,
    pub scope: RuntimeResourceScope,
    /// Relative source path, or an opaque server name for MCP entries.
    pub path: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeResourceNode {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RuntimeResourceState {
    Present,
    Missing,
    Invalid { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RuntimeResourceAccess {
    Writable,
    ReadOnly { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeResourceEntry {
    pub key: RuntimeResourceKey,
    pub absolute_path: String,
    pub node: RuntimeResourceNode,
    pub revision: Option<String>,
    pub state: RuntimeResourceState,
    pub access: RuntimeResourceAccess,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RuntimeResourceSupport {
    Supported,
    Unsupported { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeResourceCatalog {
    pub support: RuntimeResourceSupport,
    pub entries: Vec<RuntimeResourceEntry>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeResourceDocument {
    pub entry: RuntimeResourceEntry,
    /// Only the requested file, or one MCP server's native JSON object.
    pub bytes: Vec<u8>,
    pub executable: bool,
}
impl fmt::Debug for RuntimeResourceDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeResourceDocument")
            .field("entry", &self.entry)
            .field("bytes", &"[redacted]")
            .field("executable", &self.executable)
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "revision", rename_all = "camelCase")]
pub enum RuntimeResourceRevision {
    Missing,
    Match(String),
}
