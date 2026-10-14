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
    Instructions { body: String },
    /// `SKILL.md` and its assets, with paths relative to the bundle.
    Skill { files: Vec<TemplateSkillFile> },
    Mcp {
        transport: TemplateMcpTransport,
        #[serde(default)]
        timeout_seconds: Option<u32>,
    },
    Subagent {
        description: String,
        instructions: String,
        /// A model ID the runtime's own provider serves, written into the
        /// native definition. Turns through Velune's gateway route it to the
        /// one Velune model with the same provider model ID, if any.
        model: Option<String>,
        /// A Velune model (record key) instead: the agent is then never
        /// written and is supplied only to turns through Velune's gateway.
        #[serde(default)]
        velune_model: Option<String>,
        tools: Vec<String>,
        /// A reasoning level; each runtime writes it where its definition has a place for it.
        #[serde(default)]
        effort: Option<String>,
        /// Names of skills to load into the agent, where the runtime can.
        #[serde(default)]
        skills: Vec<String>,
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
        #[serde(default)]
        cwd: Option<String>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

/// Which model a sub-agent runs, as one of the two places it can come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubagentModel<'a> {
    Inherit,
    Native(&'a str),
    Velune(&'a str),
}

impl ResourceTemplateContent {
    /// The sub-agent's model; a template naming both is refused when stored.
    pub fn subagent_model(&self) -> Option<SubagentModel<'_>> {
        match self {
            Self::Subagent {
                velune_model: Some(key),
                ..
            } => Some(SubagentModel::Velune(key)),
            Self::Subagent {
                model: Some(id), ..
            } => Some(SubagentModel::Native(id)),
            Self::Subagent { .. } => Some(SubagentModel::Inherit),
            _ => None,
        }
    }

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

/// How one resource stands on one runtime instance.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResourceSyncState {
    /// Not enabled; enabling writes it.
    NotEnabled,
    /// This runtime has no place for the resource, or cannot be reached.
    Unsupported {
        reason: String,
    },
    Synced,
    /// Enabled, and the native entries are as Velune wrote them, but the resource changed since:
    /// the write is still due, usually because a turn was running when it was saved.
    Pending,
    /// A native entry was changed outside Velune; the user decides.
    Drifted {
        paths: Vec<String>,
    },
    /// A native entry was removed outside Velune; the user decides.
    Missing {
        paths: Vec<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSyncStatus {
    pub template_id: String,
    pub runtime_id: String,
    pub state: ResourceSyncState,
}

/// A native entry of a runtime that no resource manages.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnmanagedRuntimeResource {
    pub runtime_id: String,
    pub kind: RuntimeResourceKind,
    /// The native key path: a skill directory, MCP server name, or agent/instruction file.
    pub path: String,
}

/// A saved resource and how writing it to its enabled runtimes went.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSaveResult {
    pub summary: ResourceTemplateSummary,
    /// Only runtimes that were written to, or failed to be.
    pub reports: Vec<TemplateApplyReport>,
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
    /// Update the resource from the native entries, which become its new
    /// baseline here, and write it to its other runtimes as saving does.
    TakeNative,
}

/// A template read back from a runtime's native entry; not saved until the user saves it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceTemplateImport {
    pub template: ResourceTemplate,
    /// Native fields the template has no place for.
    pub notes: Vec<String>,
}

/// One native entry that differs from what the library would write there. Texts are `None`
/// when that side has no such entry; non-text content is not shown, only reported.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDriftFile {
    pub path: String,
    pub native: Option<String>,
    pub library: Option<String>,
    pub binary: bool,
}
// MCP environment values and headers are part of the texts.
impl fmt::Debug for ResourceDriftFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceDriftFile")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}
