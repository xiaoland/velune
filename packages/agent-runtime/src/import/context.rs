//! Effective model context, after the source adapter has applied its native
//! compaction and branch semantics. This is distinct from display messages.
use super::*;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub messages: Vec<ContextMessage>,
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ContextMessage {
    /// Native display identity of this prompt; never inferred from its text.
    #[serde(default)]
    pub source_message_id: Option<String>,
    pub role: Role,
    pub content: Vec<Block>,
}
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}
#[derive(Debug, Deserialize, PartialEq)]
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
        Self::parse_boundary(bytes, false)
    }

    pub(crate) fn parse_before(bytes: &[u8]) -> Result<Self> {
        Self::parse_boundary(bytes, true)
    }

    fn parse_boundary(bytes: &[u8], cancel_unfinished: bool) -> Result<Self> {
        let mut context: Self = serde_json::from_slice(bytes)
            .map_err(|error| Error::new(format!("原生有效上下文格式无效：{error}")))?;
        if cancel_unfinished {
            let mut pending = std::collections::BTreeSet::new();
            for block in context.messages.iter().flat_map(|message| &message.content) {
                match block {
                    Block::ToolCall { id, .. } => {
                        pending.insert(id.clone());
                    }
                    Block::ToolResult { id, .. } => {
                        pending.remove(id);
                    }
                    _ => {}
                }
            }
            if !pending.is_empty() {
                context.messages.push(ContextMessage {
                    source_message_id: None,
                    role: Role::User,
                    content: pending.into_iter().map(|id| Block::ToolResult {
                        id,
                        content: vec![Block::Text {
                            text: "Cancelled in the reconstructed conversation at the edit boundary. No tool result was retained. Do not replay this historical call.".into(),
                        }],
                        is_error: true,
                    }).collect(),
                });
            }
        }
        let mut merged: Vec<ContextMessage> = Vec::new();
        for message in std::mem::take(&mut context.messages) {
            if let Some(last) = merged.last_mut()
                && last.role == message.role
                && last.source_message_id == message.source_message_id
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_export_cancels_unfinished_calls_without_changing_completed_results() {
        let input = serde_json::to_vec(&json!({"messages":[
            {"role":"assistant","content":[
                {"type":"tool_call","id":"completed","name":"lookup","arguments":{}},
                {"type":"tool_call","id":"unfinished","name":"lookup","arguments":{}}
            ]},
            {"role":"user","content":[{"type":"tool_result","id":"completed","content":[{"type":"text","text":"original output"}],"is_error":false}]}
        ]})).unwrap();
        assert!(Context::parse(&input).is_err());
        let context = Context::parse_before(&input).unwrap();
        let results: Vec<_> = context
            .messages
            .iter()
            .flat_map(|m| &m.content)
            .filter_map(|block| {
                if let Block::ToolResult { id, is_error, .. } = block {
                    Some((id.as_str(), *is_error))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(results, [("completed", false), ("unfinished", true)]);
    }
}
