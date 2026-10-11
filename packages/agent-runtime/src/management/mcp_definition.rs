//! Edit a single native declaration without exposing the shared provider/auth configuration.
use super::{
    layout::{Format, Layout},
    *,
};
use serde_json::{Map, Value};

enum NativeDocument {
    Json(Value),
    Toml(toml_edit::DocumentMut),
}
struct Source {
    node: fs::NodeInfo,
    document: NativeDocument,
    servers: Map<String, Value>,
}

fn source(config: &Config, layout: &Layout) -> Result<Source> {
    let inspected = fs::inspect(&config.execution, &layout.root, &layout.prefix)?;
    let (node, bytes) = if matches!(inspected.state, EntryState::Missing) {
        (inspected, Vec::new())
    } else {
        fs::read(&config.execution, &layout.root, &layout.prefix)?
    };
    let (document, value) = match layout.format {
        Format::TomlMcp => {
            let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
            let document = text
                .parse::<toml_edit::DocumentMut>()
                .map_err(|_| invalid())?;
            let value: Value = toml_edit::de::from_str(text).map_err(|_| invalid())?;
            (NativeDocument::Toml(document), value)
        }
        _ => {
            let value: Value = if bytes.is_empty() && matches!(node.state, EntryState::Missing) {
                Value::Object(Map::new())
            } else {
                serde_json::from_slice(&bytes).map_err(|_| invalid())?
            };
            (NativeDocument::Json(value.clone()), value)
        }
    };
    if !value.is_object() {
        return Err(invalid());
    }
    let field = if matches!(layout.format, Format::TomlMcp) {
        "mcp_servers"
    } else {
        "mcpServers"
    };
    let servers = match value.get(field) {
        None => Map::new(),
        Some(Value::Object(servers)) => servers.clone(),
        Some(_) => return Err(invalid()),
    };
    Ok(Source {
        node,
        document,
        servers,
    })
}

fn invalid() -> Error {
    // Parser diagnostics can quote source lines containing env values or authorization headers.
    Error::new(
        ErrorCode::Invalid,
        "原生 MCP 配置语法或目标 section 无效；未修改文件",
    )
}

fn key_valid(key: &ResourceKey) -> Result<()> {
    if key.path.is_empty() || key.path.len() > 256 || key.path.chars().any(char::is_control) {
        Err(Error::new(ErrorCode::Invalid, "MCP 服务名称无效"))
    } else {
        Ok(())
    }
}

fn selected_entry(layout: &Layout, key: ResourceKey, source: &Source, value: &Value) -> Entry {
    Entry {
        key,
        absolute_path: layout.root.join(&layout.prefix),
        node: Node::File,
        revision: source.node.revision.clone(),
        state: if value.is_object() {
            EntryState::Present
        } else {
            EntryState::Invalid {
                reason: "MCP 服务声明必须是原生对象".into(),
            }
        },
        access: source.node.access.clone(),
        size: serde_json::to_vec(value).map_or(0, |bytes| bytes.len() as u64),
    }
}

pub(super) fn catalog(config: &Config, layout: &Layout, scope: ResourceScope) -> Result<Catalog> {
    let source = match source(config, layout) {
        Ok(source) => source,
        Err(error) if matches!(error.code(), ErrorCode::Invalid | ErrorCode::Unsupported) => {
            let node = fs::inspect(&config.execution, &layout.root, &layout.prefix)?;
            return Ok(Catalog {
                support: Support::Supported,
                entries: vec![Entry {
                    key: ResourceKey {
                        kind: ResourceKind::McpServers,
                        scope,
                        path: String::new(),
                    },
                    absolute_path: layout.root.join(&layout.prefix),
                    node: node.node,
                    revision: node.revision,
                    state: EntryState::Invalid {
                        reason: error.to_string(),
                    },
                    access: Access::ReadOnly {
                        reason: "无法安全定位原生 MCP section；请先修复源文件".into(),
                    },
                    size: node.size,
                }],
            });
        }
        Err(error) => return Err(error),
    };
    let entries = source
        .servers
        .iter()
        .map(|(name, value)| {
            selected_entry(
                layout,
                ResourceKey {
                    kind: ResourceKind::McpServers,
                    scope,
                    path: name.clone(),
                },
                &source,
                value,
            )
        })
        .collect();
    Ok(Catalog {
        support: Support::Supported,
        entries,
    })
}

pub(super) fn read(config: &Config, layout: &Layout, key: &ResourceKey) -> Result<Document> {
    key_valid(key)?;
    let source = source(config, layout)?;
    let value = source
        .servers
        .get(&key.path)
        .ok_or_else(|| Error::new(ErrorCode::Missing, "原生 MCP 服务声明不存在"))?;
    Ok(Document {
        entry: selected_entry(layout, key.clone(), &source, value),
        bytes: serde_json::to_vec_pretty(value).map_err(|_| invalid())?,
        executable: false,
    })
}

pub(super) fn write(
    config: &Config,
    layout: &Layout,
    key: &ResourceKey,
    expected: &ExpectedRevision,
    bytes: &[u8],
) -> Result<Document> {
    key_valid(key)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(Error::new(ErrorCode::Invalid, "MCP 声明超过 16 MiB 上限"));
    }
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| Error::new(ErrorCode::Invalid, "MCP 声明不是有效 JSON"))?;
    if !value.is_object() {
        return Err(Error::new(
            ErrorCode::Invalid,
            "MCP 声明必须是原生 JSON 对象",
        ));
    }
    let mut source = source(config, layout)?;
    check_expected(&source, key, expected)?;
    mutate(&mut source.document, &key.path, Some(&value))?;
    source.node = save(config, layout, &source)?;
    Ok(Document {
        entry: selected_entry(layout, key.clone(), &source, &value),
        bytes: serde_json::to_vec_pretty(&value).map_err(|_| invalid())?,
        executable: false,
    })
}

pub(super) fn delete(
    config: &Config,
    layout: &Layout,
    key: &ResourceKey,
    expected: &ExpectedRevision,
) -> Result<()> {
    key_valid(key)?;
    let mut source = source(config, layout)?;
    if !source.servers.contains_key(&key.path) {
        return Err(Error::new(ErrorCode::Missing, "原生 MCP 服务声明不存在"));
    }
    check_expected(&source, key, expected)?;
    mutate(&mut source.document, &key.path, None)?;
    save(config, layout, &source).map(|_| ())
}

fn check_expected(source: &Source, key: &ResourceKey, expected: &ExpectedRevision) -> Result<()> {
    let exists = source.servers.contains_key(&key.path);
    let matches = match expected {
        ExpectedRevision::Missing => !exists,
        ExpectedRevision::Match(revision) => {
            exists && source.node.revision.as_ref() == Some(revision)
        }
    };
    if matches {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::Conflict,
            "原生 MCP 配置已变化，请重新读取",
        ))
    }
}

fn mutate(document: &mut NativeDocument, name: &str, value: Option<&Value>) -> Result<()> {
    match document {
        NativeDocument::Json(document) => {
            if document.get("mcpServers").is_none() {
                document["mcpServers"] = Value::Object(Map::new());
            }
            let servers = document["mcpServers"].as_object_mut().ok_or_else(invalid)?;
            if let Some(value) = value {
                servers.insert(name.into(), value.clone());
            } else {
                servers.remove(name);
            }
        }
        NativeDocument::Toml(document) => {
            if document.get("mcp_servers").is_none() {
                document["mcp_servers"] = toml_edit::Item::Table(toml_edit::Table::new());
            }
            let servers = document["mcp_servers"]
                .as_table_like_mut()
                .ok_or_else(invalid)?;
            if let Some(value) = value {
                let selected = toml_edit::ser::to_document(value).map_err(|_| {
                    Error::new(ErrorCode::Invalid, "MCP 声明含无法表示为原生 TOML 的值")
                })?;
                servers.insert(name, toml_edit::Item::Table(selected.as_table().clone()));
            } else {
                servers.remove(name);
            }
        }
    }
    Ok(())
}

fn save(config: &Config, layout: &Layout, source: &Source) -> Result<fs::NodeInfo> {
    let bytes = match &source.document {
        NativeDocument::Json(value) => serde_json::to_vec_pretty(value).map_err(|_| invalid())?,
        NativeDocument::Toml(value) => value.to_string().into_bytes(),
    };
    let expected = source
        .node
        .revision
        .clone()
        .map_or(ExpectedRevision::Missing, ExpectedRevision::Match);
    fs::write(
        &config.execution,
        &layout.root,
        &layout.prefix,
        &expected,
        &bytes,
        None,
    )
}
