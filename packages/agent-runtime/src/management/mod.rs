//! Manage a bounded set of native Harness resources on their execution machine.
//! Catalogs describe source files, not the Harness's merged effective configuration.
use crate::execution::ExecutionTarget;
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};

pub use canonical::{
    Canonical, McpTransport, RenderedFile, Rendering, SkillFile, fingerprint, parse, render,
    validate_name,
};

mod canonical;
mod fs;
mod layout;
mod mcp_definition;
mod validation;

#[derive(Clone, Debug)]
pub struct Config {
    pub variant_id: String,
    pub execution: ExecutionTarget,
    pub agent_dir: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResourceKind {
    UserInstructions,
    McpServers,
    Skills,
    NativeAgentDefinitions,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResourceScope {
    User,
    Project,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceKey {
    pub kind: ResourceKind,
    pub scope: ResourceScope,
    /// Relative source path, or an opaque server name for MCP entries.
    pub path: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Node {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EntryState {
    Present,
    Missing,
    Invalid { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Access {
    Writable,
    ReadOnly { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub key: ResourceKey,
    pub absolute_path: PathBuf,
    pub node: Node,
    pub revision: Option<String>,
    pub state: EntryState,
    pub access: Access,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Support {
    Supported,
    Unsupported { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub support: Support,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub entry: Entry,
    /// Only the requested file, or one MCP server's native JSON object.
    pub bytes: Vec<u8>,
    pub executable: bool,
}
impl fmt::Debug for Document {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Document")
            .field("entry", &self.entry)
            .field("bytes", &"[redacted]")
            .field("executable", &self.executable)
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "revision", rename_all = "camelCase")]
pub enum ExpectedRevision {
    Missing,
    Match(String),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    Conflict,
    Missing,
    Invalid,
    Unsupported,
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    code: ErrorCode,
    message: String,
}
impl Error {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn code(&self) -> ErrorCode {
        self.code
    }
}
pub type Result<T> = std::result::Result<T, Error>;

/// Enumerate only the declared native source scope. Missing roots are empty; no directory is created.
/// Skill trees include binary attachments and non-followed symlinks, rather than only SKILL.md.
pub fn catalog(config: &Config, kind: ResourceKind, scope: ResourceScope) -> Result<Catalog> {
    if let Some(reason) = layout::unsupported(config, kind) {
        return Ok(Catalog {
            support: Support::Unsupported {
                reason: reason.into(),
            },
            entries: vec![],
        });
    }
    let layout = layout::resolve(config, kind, scope)?;
    if layout.is_mcp() {
        return mcp_definition::catalog(config, &layout, scope);
    }
    let nodes = if matches!(layout.format, layout::Format::Instructions) {
        let mut nodes = Vec::new();
        for candidate in &layout.candidates {
            let node = fs::inspect(&config.execution, &layout.root, candidate)?;
            if !matches!(node.state, EntryState::Missing) || *candidate == layout.candidates[0] {
                nodes.push(node);
            }
        }
        nodes
    } else {
        fs::list(&config.execution, &layout.root, &layout.prefix, true)?
    };
    let mut entries = Vec::new();
    for node in nodes {
        let key = ResourceKey {
            kind,
            scope,
            path: layout.relative_path(&node.path).into(),
        };
        let mut entry = entry(&layout, key, node);
        if entry.node == Node::Directory
            && !matches!(layout.format, layout::Format::SkillTree { .. })
        {
            entry.access = Access::ReadOnly {
                reason: "此类别只管理声明文件，不管理目录".into(),
            };
        }
        if entry.node == Node::File {
            if let Err(error) = layout::validate_path(&layout, &entry.key) {
                entry.access = Access::ReadOnly {
                    reason: error.to_string(),
                };
            } else if matches!(entry.state, EntryState::Present)
                && validation::needs_text(&layout, &entry.key)
            {
                match fs::read(
                    &config.execution,
                    &layout.root,
                    &layout.source_path(&entry.key.path),
                ) {
                    Ok((_, bytes)) => {
                        if let Err(error) = validation::content(&layout, &entry.key, &bytes) {
                            entry.state = EntryState::Invalid {
                                reason: error.to_string(),
                            };
                        }
                    }
                    Err(error) => {
                        entry.state = EntryState::Invalid {
                            reason: error.to_string(),
                        };
                        entry.access = Access::ReadOnly {
                            reason: "无法读取此资源，不能安全修改".into(),
                        };
                    }
                }
            }
        }
        entries.push(entry);
    }
    Ok(Catalog {
        support: Support::Supported,
        entries,
    })
}

/// Read one native file or one MCP server declaration. Never returns the surrounding shared config.
pub fn read(config: &Config, key: &ResourceKey) -> Result<Document> {
    let layout = layout::resolve(config, key.kind, key.scope)?;
    if layout.is_mcp() {
        return mcp_definition::read(config, &layout, key);
    }
    layout::validate_path(&layout, key)?;
    let (node, bytes) = fs::read(
        &config.execution,
        &layout.root,
        &layout.source_path(&key.path),
    )?;
    let executable = node.executable;
    let mut entry = entry(&layout, key.clone(), node);
    if let Err(error) = validation::content(&layout, key, &bytes) {
        entry.state = EntryState::Invalid {
            reason: error.to_string(),
        };
    }
    Ok(Document {
        entry,
        bytes,
        executable,
    })
}

/// Atomically replace one file after a target-side revision check, or explicitly create a missing file.
/// Skill creation starts with SKILL.md and adds assets separately; this is not a multi-file transaction.
/// An omitted executable flag preserves existing permissions. Native processes must reload separately.
pub fn write(
    config: &Config,
    key: &ResourceKey,
    expected: &ExpectedRevision,
    bytes: &[u8],
    executable: Option<bool>,
) -> Result<Document> {
    let layout = layout::resolve(config, key.kind, key.scope)?;
    if layout.is_mcp() {
        if executable == Some(true) {
            return Err(Error::new(ErrorCode::Invalid, "MCP 声明不是可执行文件"));
        }
        return mcp_definition::write(config, &layout, key, expected, bytes);
    }
    layout::validate_path(&layout, key)?;
    validation::content(&layout, key, bytes)?;
    let node = fs::write(
        &config.execution,
        &layout.root,
        &layout.source_path(&key.path),
        expected,
        bytes,
        executable,
    )?;
    let executable = node.executable;
    Ok(Document {
        entry: entry(&layout, key.clone(), node),
        bytes: bytes.to_vec(),
        executable,
    })
}

/// Delete one file or an empty skill directory. Nonempty bundles require explicit asset deletions.
pub fn delete(config: &Config, key: &ResourceKey, expected: &ExpectedRevision) -> Result<()> {
    let layout = layout::resolve(config, key.kind, key.scope)?;
    if layout.is_mcp() {
        return mcp_definition::delete(config, &layout, key, expected);
    }
    layout::validate_path(&layout, key)?;
    fs::delete(
        &config.execution,
        &layout.root,
        &layout.source_path(&key.path),
        expected,
    )
}

fn entry(layout: &layout::Layout, key: ResourceKey, node: fs::NodeInfo) -> Entry {
    Entry {
        key,
        absolute_path: layout.root.join(node.path),
        node: node.node,
        revision: node.revision,
        state: node.state,
        access: node.access,
        size: node.size,
    }
}
