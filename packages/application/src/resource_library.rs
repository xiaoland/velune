//! Reusable resource templates and their managed bindings to runtime instances.
//! A template is Velune's canonical definition; applying it writes native entries that Velune
//! then manages. Everything else a runtime owns stays the runtime's.
use crate::{RuntimeResourceKey, RuntimeResourceKind};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ResourceTemplateContent {
    /// The whole user instruction file.
    Instructions {
        body: String,
    },
    /// `SKILL.md` and its assets, with paths relative to the bundle.
    Skill {
        files: Vec<TemplateSkillFile>,
    },
    Mcp {
        transport: TemplateMcpTransport,
    },
    Subagent {
        description: String,
        instructions: String,
        model: Option<String>,
        tools: Vec<String>,
    },
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateSkillFile {
    pub path: String,
    pub bytes: Vec<u8>,
    pub executable: bool,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "transport", rename_all = "camelCase", deny_unknown_fields)]
pub enum TemplateMcpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

impl ResourceTemplateContent {
    pub fn kind(&self) -> RuntimeResourceKind {
        match self {
            Self::Instructions { .. } => RuntimeResourceKind::UserInstructions,
            Self::Skill { .. } => RuntimeResourceKind::Skills,
            Self::Mcp { .. } => RuntimeResourceKind::McpServers,
            Self::Subagent { .. } => RuntimeResourceKind::NativeAgentDefinitions,
        }
    }
}

// Environment values and request headers can be secrets, like provider API keys; only an
// explicit read of a template returns them, and nothing prints them.
impl fmt::Debug for ResourceTemplateContent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "ResourceTemplateContent::{:?}([redacted])",
            self.kind()
        )
    }
}
impl fmt::Debug for TemplateMcpTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TemplateMcpTransport([redacted])")
    }
}
impl fmt::Debug for TemplateSkillFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TemplateSkillFile")
            .field("path", &self.path)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// A template with its content. An empty `id` on save asks for a new template.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceTemplate {
    pub id: String,
    /// The native name (skill directory, server name, agent file) or, for instructions, a label.
    pub name: String,
    pub content: ResourceTemplateContent,
}

/// What lists show: no content, so no secrets.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceTemplateSummary {
    pub id: String,
    pub name: String,
    pub kind: RuntimeResourceKind,
}

/// One native entry a template wrote, with the fingerprint of what Velune wrote there.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundEntry {
    pub key: RuntimeResourceKey,
    pub fingerprint: String,
}

/// A template applied to one runtime instance.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceBinding {
    pub runtime_id: String,
    pub template_id: String,
    pub entries: Vec<BoundEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TemplateSyncState {
    Synced,
    /// The template changed; the native entries are as Velune wrote them.
    TemplateUpdated,
    /// A native entry was changed outside Velune.
    Drifted {
        paths: Vec<String>,
    },
    /// A native entry was removed outside Velune.
    Missing {
        paths: Vec<String>,
    },
    Skipped {
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeTemplateStatus {
    pub template_id: String,
    pub template_name: String,
    pub kind: RuntimeResourceKind,
    pub state: TemplateSyncState,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TemplateApplyOutcome {
    Create,
    Update,
    Unchanged,
    /// Entries that block the application until the user chooses to overwrite them.
    Conflict {
        /// Native entries that exist but are not managed by this template.
        unmanaged: Vec<String>,
        /// Managed entries changed outside Velune.
        drifted: Vec<String>,
    },
    Skipped {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TemplateApplyReport {
    pub runtime_id: String,
    pub outcome: TemplateApplyOutcome,
    /// Template fields this runtime has no place for.
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TemplateApplyTarget {
    pub runtime_id: String,
    /// Replace conflicting entries instead of stopping.
    pub overwrite: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BindingResolution {
    /// Write the template over the native entries.
    Overwrite,
    /// Stop managing; the native entries stay as they are.
    Unmanage,
    /// Delete the native entries and stop managing.
    Remove,
}
