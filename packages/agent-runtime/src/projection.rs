use crate::conversation::{
    ConversationActions, ConversationSnapshot, ConversationSummary, Message, MessageBlock,
    MessageRole, RunState, ToolState, first_user_text,
};
use serde_json::Value;

#[derive(Debug, Default)]
pub struct PiProjection {
    pub snapshot: Option<ConversationSnapshot>,
    next_message_id: u64,
    active_message_id: Option<String>,
    history_synchronized: bool,
}

impl PiProjection {
    pub fn new(conversation: ConversationSummary) -> Self {
        Self {
            snapshot: Some(ConversationSnapshot {
                revision: 0,
                conversation,
                resource_id: None,
                model_record_key: None,
                run_state: RunState::Idle,
                messages: Vec::new(),
                pending_interactions: Vec::new(),
                actions: ConversationActions {
                    can_send: true,
                    can_cancel: false,
                    can_switch: true,
                },
            }),
            next_message_id: 0,
            active_message_id: None,
            history_synchronized: false,
        }
    }

    pub fn invalidate_history(&mut self) {
        self.history_synchronized = false;
    }

    pub fn history_synchronized(&self) -> bool {
        self.history_synchronized
    }

    pub fn replace_history(&mut self, response: &Value) {
        let Some(mut snapshot) = self.snapshot.take() else {
            return;
        };
        let messages = response["data"]["messages"]
            .as_array()
            .or_else(|| response["messages"].as_array());
        let Some(messages) = messages else {
            self.snapshot = Some(snapshot);
            return;
        };
        let mut projected = Vec::new();
        for (index, message) in messages.iter().enumerate() {
            if message["role"] == "toolResult" {
                apply_tool_result(&mut projected, message);
            } else if let Some(message) = message_from_pi(message, format!("message-{index}")) {
                projected.push(message);
            }
        }
        self.history_synchronized = true;
        self.next_message_id = messages.len() as u64;
        snapshot.messages = projected;
        refresh_title(&mut snapshot);
        snapshot.revision = snapshot.revision.saturating_add(1);
        self.snapshot = Some(snapshot);
    }

    pub fn append_user(&mut self, text: &str) {
        let Some(mut snapshot) = self.snapshot.take() else {
            return;
        };
        snapshot.messages.push(Message {
            id: format!("message-{}", self.next_message_id),
            role: crate::conversation::MessageRole::User,
            timestamp_unix_ms: None,
            blocks: vec![MessageBlock::Text { text: text.into() }],
        });
        self.next_message_id += 1;
        refresh_title(&mut snapshot);
        snapshot.revision = snapshot.revision.saturating_add(1);
        self.snapshot = Some(snapshot);
    }

    pub fn set_conversation(&mut self, conversation: ConversationSummary) {
        if let Some(snapshot) = self.snapshot.as_mut() {
            snapshot.conversation = conversation;
            snapshot.revision = snapshot.revision.saturating_add(1);
        }
    }

    pub fn apply_event(&mut self, event: &Value) {
        let Some(mut snapshot) = self.snapshot.take() else {
            return;
        };
        match event["type"].as_str().unwrap_or_default() {
            "agent_start" | "turn_start" => {
                snapshot.run_state = RunState::Running;
                snapshot.actions.can_send = false;
                snapshot.actions.can_cancel = true;
            }
            "message_start" | "message_update" | "message_end" => {
                let native = &event["message"];
                if native["role"] == "toolResult" {
                    if event["type"] == "message_start" {
                        self.next_message_id += 1;
                    }
                    apply_tool_result(&mut snapshot.messages, native);
                } else {
                    let id = if event["type"] == "message_start" {
                        let id = native["id"].as_str().map(str::to_owned).unwrap_or_else(|| {
                            // User submission is already projected after RPC acceptance.
                            if native["role"] == "user"
                                && snapshot
                                    .messages
                                    .last()
                                    .is_some_and(|m| m.role == MessageRole::User)
                            {
                                return snapshot.messages.last().expect("checked").id.clone();
                            }
                            let id = format!("message-{}", self.next_message_id);
                            self.next_message_id += 1;
                            id
                        });
                        self.active_message_id = Some(id.clone());
                        id
                    } else {
                        self.active_message_id
                            .clone()
                            .unwrap_or_else(|| format!("message-{}", self.next_message_id))
                    };
                    if let Some(message) = message_from_pi(native, id.clone()) {
                        if let Some(existing) = snapshot.messages.iter_mut().find(|m| m.id == id) {
                            *existing = message;
                        } else {
                            snapshot.messages.push(message);
                        }
                    }
                    if event["type"] == "message_end" {
                        self.active_message_id = None;
                    }
                }
            }
            "tool_execution_start" | "tool_execution_update" | "tool_execution_end" => {
                let id = event["toolCallId"].as_str();
                let state = match event["type"].as_str() {
                    Some("tool_execution_end") if event["isError"] == true => ToolState::Failed,
                    Some("tool_execution_end") => ToolState::Completed,
                    _ => ToolState::Running,
                };
                let output = text_content(
                    event
                        .get("result")
                        .or_else(|| event.get("partialResult"))
                        .unwrap_or(&Value::Null),
                );
                if !update_tool(&mut snapshot.messages, id, state.clone(), output.clone()) {
                    snapshot.messages.push(Message {
                        id: format!("tool-{}", id.unwrap_or_default()),
                        role: MessageRole::Tool,
                        timestamp_unix_ms: None,
                        blocks: vec![MessageBlock::Tool {
                            tool_id: id.map(str::to_owned),
                            title: event["toolName"].as_str().unwrap_or("工具").into(),
                            state,
                            output,
                        }],
                    });
                }
            }
            "extension_ui_request"
                if event["method"] == "setStatus"
                    && event["statusKey"] == "velune.session-metadata" =>
            {
                let Some(metadata) = event["statusText"]
                    .as_str()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
                else {
                    tracing::warn!(target: "velune_agent_runtime", event="pi_projection_metadata_invalid");
                    snapshot.run_state = RunState::Failed;
                    snapshot.actions.can_send = false;
                    snapshot.messages.push(Message {
                        id: format!("notice-{}", self.next_message_id),
                        role: MessageRole::System,
                        timestamp_unix_ms: None,
                        blocks: vec![MessageBlock::Notice {
                            text: "运行时会话元数据格式无效".into(),
                        }],
                    });
                    self.next_message_id += 1;
                    snapshot.revision += 1;
                    self.snapshot = Some(snapshot);
                    return;
                };
                if let Some(path) = metadata["sessionFile"].as_str() {
                    snapshot.conversation.id =
                        format!("{}:{path}", snapshot.conversation.runtime_id);
                }
                snapshot.conversation.cwd = metadata["cwd"].as_str().map(str::to_owned);
                snapshot.conversation.created_at_unix_ms = metadata["createdUnixMs"].as_i64();
                snapshot.conversation.title =
                    crate::conversation::conversation_title(metadata["name"].as_str(), None);
                self.history_synchronized = false;
            }
            "session_info_changed" => {
                snapshot.conversation.title = crate::conversation::conversation_title(
                    event["name"].as_str(),
                    first_user_text(&snapshot.messages),
                );
            }
            "compaction_end" => {
                self.history_synchronized = false;
            }
            "agent_settled" => {
                self.history_synchronized = false;
                snapshot.run_state = RunState::Idle;
                snapshot.actions.can_send = true;
                snapshot.actions.can_cancel = false;
                self.active_message_id = None;
            }
            "error" | "extension_error" | "velune_error" | "velune_closed" => {
                snapshot.run_state = RunState::Failed;
                snapshot.actions.can_send = false;
                snapshot.actions.can_cancel = false;
                snapshot.messages.push(Message {
                    id: format!("notice-{}", self.next_message_id),
                    role: MessageRole::System,
                    timestamp_unix_ms: None,
                    blocks: vec![MessageBlock::Notice {
                        text: "运行时发生错误".into(),
                    }],
                });
                self.next_message_id += 1;
            }
            _ => {
                self.snapshot = Some(snapshot);
                return;
            }
        }
        refresh_title(&mut snapshot);
        snapshot.revision = snapshot.revision.saturating_add(1);
        self.snapshot = Some(snapshot);
    }
}

fn refresh_title(snapshot: &mut ConversationSnapshot) {
    snapshot
        .conversation
        .title
        .refresh(first_user_text(&snapshot.messages));
    snapshot.conversation.updated_at_unix_ms = snapshot
        .messages
        .iter()
        .filter_map(|m| m.timestamp_unix_ms)
        .max()
        .or(snapshot.conversation.updated_at_unix_ms);
}
fn text_content(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.into());
    }
    let content = value.get("content").unwrap_or(value);
    content.as_array().map(|blocks| {
        blocks
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    })
}
fn update_tool(
    messages: &mut [Message],
    id: Option<&str>,
    state: ToolState,
    output: Option<String>,
) -> bool {
    let Some(id) = id else {
        return false;
    };
    for message in messages.iter_mut().rev() {
        for block in &mut message.blocks {
            if let MessageBlock::Tool {
                tool_id,
                state: current,
                output: current_output,
                ..
            } = block
                && tool_id.as_deref() == Some(id)
            {
                *current = state;
                if output.is_some() {
                    *current_output = output;
                }
                return true;
            }
        }
    }
    false
}
fn apply_tool_result(messages: &mut Vec<Message>, native: &Value) {
    let id = native["toolCallId"].as_str();
    let state = if native["isError"] == true {
        ToolState::Failed
    } else {
        ToolState::Completed
    };
    let output = text_content(&native["content"]);
    if !update_tool(messages, id, state.clone(), output.clone()) {
        messages.push(Message {
            id: format!("tool-result-{}", messages.len()),
            role: MessageRole::Tool,
            timestamp_unix_ms: native["timestamp"].as_i64(),
            blocks: vec![MessageBlock::Tool {
                tool_id: id.map(str::to_owned),
                title: native["toolName"].as_str().unwrap_or("工具").into(),
                state,
                output,
            }],
        });
    }
}
fn message_from_pi(message: &Value, fallback_id: String) -> Option<Message> {
    let role = match message["role"].as_str()? {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        "toolResult" => MessageRole::Tool,
        _ => MessageRole::System,
    };
    let id = message["id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or(fallback_id);
    let mut blocks = Vec::new();
    if let Some(content) = message["content"].as_array() {
        for block in content {
            match block["type"].as_str().unwrap_or_default() {
                "text" => blocks.push(MessageBlock::Text {
                    text: block["text"].as_str().unwrap_or_default().into(),
                }),
                "thinking" => blocks.push(MessageBlock::Reasoning {
                    text: block["thinking"].as_str().unwrap_or_default().into(),
                }),
                "toolCall" => blocks.push(MessageBlock::Tool {
                    tool_id: block["id"].as_str().map(str::to_owned),
                    title: block["name"].as_str().unwrap_or("工具").into(),
                    state: ToolState::Pending,
                    output: None,
                }),
                "image" => blocks.push(MessageBlock::Notice {
                    text: "图片".into(),
                }),
                _ => {}
            }
        }
    } else if let Some(text) = message["content"].as_str() {
        blocks.push(MessageBlock::Text { text: text.into() });
    }
    (!blocks.is_empty()).then(|| Message {
        id,
        role,
        timestamp_unix_ms: message["timestamp"].as_i64(),
        blocks,
    })
}
