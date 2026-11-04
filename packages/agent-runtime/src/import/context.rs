//! Effective model context, after the source adapter has applied its native
//! compaction and branch semantics. This is distinct from display messages.
use super::*;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub messages: Vec<ContextMessage>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextMessage {
    pub role: Role,
    pub content: Vec<Block>,
}
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Block {
    Text {
        text: String,
    },
    Image {
        media_type: String,
        data: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        id: String,
        content: Vec<Block>,
        is_error: bool,
    },
}
impl Context {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        let mut context: Self = serde_json::from_slice(bytes)
            .map_err(|error| Error::new(format!("原生有效上下文格式无效：{error}")))?;
        let mut merged: Vec<ContextMessage> = Vec::new();
        for message in std::mem::take(&mut context.messages) {
            if let Some(last) = merged.last_mut()
                && last.role == message.role
            {
                last.content.extend(message.content);
            } else {
                merged.push(message);
            }
        }
        context.messages = merged;
        let mut seen = HashSet::new();
        let mut pending = HashSet::new();
        for message in &context.messages {
            if message.role == Role::Assistant && !pending.is_empty() {
                return Err(Error::new("原生上下文的工具结果不完整，未交接"));
            }
            for block in &message.content {
                match block {
                    Block::ToolCall {
                        id,
                        name,
                        arguments,
                    } => {
                        if message.role != Role::Assistant
                            || id.is_empty()
                            || name.is_empty()
                            || !arguments.is_object()
                            || !seen.insert(id)
                        {
                            return Err(Error::new("原生上下文的工具调用无效，未交接"));
                        }
                        pending.insert(id);
                    }
                    Block::ToolResult { id, content, .. } => {
                        if message.role != Role::User
                            || !pending.remove(id)
                            || content
                                .iter()
                                .any(|b| !matches!(b, Block::Text { .. } | Block::Image { .. }))
                        {
                            return Err(Error::new("原生上下文的工具结果无法配对，未交接"));
                        }
                    }
                    Block::Text { text } if text.starts_with("<velune-context:") => {
                        return Err(Error::new(
                            "来源仍包含旧式引用交接；请先显式迁移该会话，未重复导入巨型载荷",
                        ));
                    }
                    _ => {}
                }
            }
        }
        if !pending.is_empty() {
            return Err(Error::new("来源存在未完成工具调用，请完成或压缩后再交接"));
        }
        Ok(context)
    }
    pub(super) fn tool_ids(&self, prefix: &str) -> Result<HashMap<&str, String>> {
        self.messages
            .iter()
            .flat_map(|m| &m.content)
            .filter_map(|b| match b {
                Block::ToolCall { id, .. } => {
                    Some(prefixed_id(prefix).map(|new| (id.as_str(), new)))
                }
                _ => None,
            })
            .collect()
    }
}
