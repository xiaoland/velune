//! Velune's canonical definitions of reusable native resources, rendered per Harness.
//! A canonical definition is the template content; rendering produces exactly the source files
//! or declaration object the native Harness reads, or explains why this Harness cannot hold it.
use super::{layout, *};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt, path::Path};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Canonical {
    /// The whole user instruction file.
    Instructions {
        body: String,
    },
    /// A skill bundle: `SKILL.md` and its assets, with paths relative to the bundle.
    Skill {
        files: Vec<SkillFile>,
    },
    Mcp {
        transport: McpTransport,
    },
    Subagent {
        description: String,
        /// The agent's own system instructions.
        instructions: String,
        /// Hints that only Harnesses with a place for them keep.
        model: Option<String>,
        tools: Vec<String>,
    },
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillFile {
    pub path: String,
    pub bytes: Vec<u8>,
    pub executable: bool,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "transport", rename_all = "camelCase")]
pub enum McpTransport {
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

// Environment values and headers may be secrets; nothing of the content is printed.
impl fmt::Debug for Canonical {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Canonical::{:?}([redacted])", self.kind())
    }
}
impl fmt::Debug for McpTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("McpTransport([redacted])")
    }
}
impl fmt::Debug for SkillFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SkillFile")
            .field("path", &self.path)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

impl Canonical {
    pub fn kind(&self) -> ResourceKind {
        match self {
            Self::Instructions { .. } => ResourceKind::UserInstructions,
            Self::Skill { .. } => ResourceKind::Skills,
            Self::Mcp { .. } => ResourceKind::McpServers,
            Self::Subagent { .. } => ResourceKind::NativeAgentDefinitions,
        }
    }
}

/// A native file or declaration to write, always in the user scope.
#[derive(Clone, PartialEq, Eq)]
pub struct RenderedFile {
    pub key: ResourceKey,
    pub bytes: Vec<u8>,
    pub executable: Option<bool>,
}
impl fmt::Debug for RenderedFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenderedFile")
            .field("key", &self.key)
            .field("bytes", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rendering {
    Files {
        files: Vec<RenderedFile>,
        /// Fields the Harness has no place for, named without their values.
        notes: Vec<String>,
    },
    Skipped {
        reason: String,
    },
}

/// Names become file names or server names, so they stay conservative and portable.
pub fn validate_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::Invalid,
            "名称只能含字母、数字、-、_、.，不超过 64 个字符，且不以 . 开头",
        ))
    }
}

pub fn render(variant_id: &str, name: &str, canonical: &Canonical) -> Result<Rendering> {
    let kind = canonical.kind();
    if let Some(reason) = layout::unsupported_variant(variant_id, kind) {
        return Ok(Rendering::Skipped {
            reason: reason.into(),
        });
    }
    let claude = variant_id == "claude-code-2.1.286";
    let codex = variant_id == "codex-0.159.3";
    let key = |path: String| ResourceKey {
        kind,
        scope: ResourceScope::User,
        path,
    };
    let mut notes = Vec::new();
    let files = match canonical {
        Canonical::Instructions { body } => vec![RenderedFile {
            key: key(if claude { "CLAUDE.md" } else { "AGENTS.md" }.into()),
            bytes: body.clone().into_bytes(),
            executable: None,
        }],
        Canonical::Skill { files } => {
            validate_name(name)?;
            skill_files(name, files, &key)?
        }
        Canonical::Mcp { transport } => {
            validate_name(name)?;
            vec![RenderedFile {
                key: key(name.into()),
                bytes: serde_json::to_vec_pretty(&mcp_object(variant_id, transport))
                    .map_err(|_| Error::new(ErrorCode::Invalid, "MCP 声明无法序列化"))?,
                executable: None,
            }]
        }
        Canonical::Subagent {
            description,
            instructions,
            model,
            tools,
        } => {
            validate_name(name)?;
            if description.trim().is_empty() || instructions.trim().is_empty() {
                return Err(Error::new(ErrorCode::Invalid, "子 Agent 需要描述和指令"));
            }
            let (extension, bytes) = if codex {
                if model.is_some() {
                    notes.push("Codex 定义没有对应位置，模型提示未写入".into());
                }
                if !tools.is_empty() {
                    notes.push("Codex 定义没有对应位置，工具提示未写入".into());
                }
                ("toml", codex_agent(name, description, instructions))
            } else {
                (
                    "md",
                    claude_agent(name, description, instructions, model, tools)?,
                )
            };
            vec![RenderedFile {
                key: key(format!("{name}.{extension}")),
                bytes,
                executable: None,
            }]
        }
    };
    Ok(Rendering::Files { files, notes })
}

fn skill_files(
    name: &str,
    files: &[SkillFile],
    key: &dyn Fn(String) -> ResourceKey,
) -> Result<Vec<RenderedFile>> {
    if !files.iter().any(|file| file.path == "SKILL.md") {
        return Err(Error::new(ErrorCode::Invalid, "技能需要根目录的 SKILL.md"));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut rendered = Vec::new();
    for file in files {
        let path = Path::new(&file.path);
        if file.path.contains(['\0', '\\'])
            || path
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            || !seen.insert(file.path.as_str())
        {
            return Err(Error::new(ErrorCode::Invalid, "技能文件路径无效或重复"));
        }
        rendered.push(RenderedFile {
            key: key(format!("{name}/{}", file.path)),
            bytes: file.bytes.clone(),
            executable: Some(file.executable),
        });
    }
    // The entry point first: a bundle is created entry-first and filled in afterwards.
    rendered.sort_by_key(|file| file.key.path != format!("{name}/SKILL.md"));
    Ok(rendered)
}

fn mcp_object(variant_id: &str, transport: &McpTransport) -> Value {
    let strings = |map: &BTreeMap<String, String>| {
        Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                .collect(),
        )
    };
    let mut object = Map::new();
    let claude = variant_id == "claude-code-2.1.286";
    let codex = variant_id == "codex-0.159.3";
    match transport {
        McpTransport::Stdio { command, args, env } => {
            if claude {
                object.insert("type".into(), "stdio".into());
            }
            object.insert("command".into(), command.clone().into());
            if !args.is_empty() {
                object.insert("args".into(), args.clone().into());
            }
            if !env.is_empty() {
                object.insert("env".into(), strings(env));
            }
        }
        McpTransport::Http { url, headers } => {
            if claude {
                object.insert("type".into(), "http".into());
            }
            object.insert("url".into(), url.clone().into());
            if !headers.is_empty() {
                let field = if codex { "http_headers" } else { "headers" };
                object.insert(field.into(), strings(headers));
            }
        }
    }
    Value::Object(object)
}

fn claude_agent(
    name: &str,
    description: &str,
    instructions: &str,
    model: &Option<String>,
    tools: &[String],
) -> Result<Vec<u8>> {
    let mut header = serde_yaml_ng::Mapping::new();
    header.insert("name".into(), name.into());
    header.insert("description".into(), description.into());
    if !tools.is_empty() {
        header.insert("tools".into(), tools.join(", ").into());
    }
    if let Some(model) = model {
        header.insert("model".into(), model.as_str().into());
    }
    let header = serde_yaml_ng::to_string(&header)
        .map_err(|_| Error::new(ErrorCode::Invalid, "子 Agent 头部无法序列化"))?;
    Ok(format!("---\n{header}---\n\n{}\n", instructions.trim_end()).into_bytes())
}

fn codex_agent(name: &str, description: &str, instructions: &str) -> Vec<u8> {
    let mut document = toml_edit::DocumentMut::new();
    document["name"] = toml_edit::value(name);
    document["description"] = toml_edit::value(description);
    document["developer_instructions"] = toml_edit::value(instructions.trim_end());
    document.to_string().into_bytes()
}

/// A hash of what a native entry means, so that reformatting a declaration by hand or by the
/// Harness does not look like a content change. MCP objects compare by sorted JSON value.
pub fn fingerprint(kind: ResourceKind, bytes: &[u8]) -> String {
    let normalized = if kind == ResourceKind::McpServers {
        serde_json::from_slice::<Value>(bytes)
            .ok()
            .map(|value| sorted(&value))
    } else {
        None
    };
    let mut digest = Sha256::new();
    match normalized {
        Some(text) => digest.update(text.as_bytes()),
        None => digest.update(bytes),
    }
    format!("{:x}", digest.finalize())
}

fn sorted(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            let items: Vec<_> = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_default(),
                        sorted(&map[key])
                    )
                })
                .collect();
            format!("{{{}}}", items.join(","))
        }
        Value::Array(items) => {
            let items: Vec<_> = items.iter().map(sorted).collect();
            format!("[{}]", items.join(","))
        }
        other => other.to_string(),
    }
}
