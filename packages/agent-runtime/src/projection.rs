use crate::conversation::{
    ConversationActions, ConversationSnapshot, ConversationSummary, Message, MessageBlock, RunState,
};
use serde_json::Value;

#[derive(Debug, Default)]
pub struct PiProjection {
    pub snapshot: Option<ConversationSnapshot>,
    next_message_id: u64,
    active_message_id: Option<String>,
}

impl PiProjection {
    pub fn new(conversation: ConversationSummary) -> Self {
        Self {
            snapshot: Some(ConversationSnapshot {
                revision: 0,
                conversation,
                resource_id: None,
                model_id: None,
                run_state: RunState::Idle,
                messages: Vec::new(),
                actions: ConversationActions {
                    can_send: true,
                    can_cancel: false,
                    can_switch: true,
                },
            }),
            next_message_id: 1,
            active_message_id: None,
        }
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
        snapshot.messages = messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| self.message_from_pi(message, index as u64))
            .collect();
        snapshot.revision = snapshot.revision.saturating_add(1);
        self.snapshot = Some(snapshot);
    }

    pub fn append_user(&mut self, text: &str) {
        let Some(mut snapshot) = self.snapshot.take() else {
            return;
        };
        snapshot.messages.push(Message {
            id: format!("user-{}", self.next_message_id),
            role: "user".into(),
            blocks: vec![MessageBlock::Text { text: text.into() }],
        });
        self.next_message_id += 1;
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
        let event_type = event["type"].as_str().unwrap_or("");
        match event_type {
            "agent_start" | "turn_start" | "message_start" => {
                snapshot.run_state = RunState::Running;
                snapshot.actions.can_send = false;
                snapshot.actions.can_cancel = true;
                if event_type == "message_start" {
                    let message = event.get("message").unwrap_or(event);
                    let id = self.message_id(message);
                    self.active_message_id = Some(id.clone());
                    if !snapshot.messages.iter().any(|item| item.id == id) {
                        snapshot.messages.push(Message {
                            id,
                            role: "assistant".into(),
                            blocks: vec![MessageBlock::Text {
                                text: String::new(),
                            }],
                        });
                    }
                }
            }
            "message_update" => {
                snapshot.run_state = RunState::Running;
                snapshot.actions.can_send = false;
                snapshot.actions.can_cancel = true;
                let update = event.get("assistantMessageEvent").unwrap_or(event);
                if update["type"] == "text_delta"
                    && let Some(delta) = update["delta"].as_str()
                {
                    let id = self.active_message_id.clone().unwrap_or_else(|| {
                        let value = format!("assistant-{}", self.next_message_id);
                        self.next_message_id += 1;
                        self.active_message_id = Some(value.clone());
                        value
                    });
                    let message_index = snapshot.messages.iter().position(|item| item.id == id);
                    let message = if let Some(index) = message_index {
                        &mut snapshot.messages[index]
                    } else {
                        snapshot.messages.push(Message {
                            id: id.clone(),
                            role: "assistant".into(),
                            blocks: vec![MessageBlock::Text {
                                text: String::new(),
                            }],
                        });
                        snapshot.messages.last_mut().expect("message just inserted")
                    };
                    if let Some(MessageBlock::Text { text }) = message.blocks.first_mut() {
                        text.push_str(delta);
                    }
                }
            }
            "message_end" => {
                if let Some(message) = event.get("message") {
                    let id = self.message_id(message);
                    if let Some(projected) = self.message_from_pi(message, 0) {
                        if let Some(existing) =
                            snapshot.messages.iter_mut().find(|item| item.id == id)
                        {
                            *existing = projected;
                        } else {
                            snapshot.messages.push(projected);
                        }
                    }
                }
                self.active_message_id = None;
            }
            "tool_execution_start" => {
                let tool_id = event["toolCallId"].as_str().map(str::to_owned);
                let title = event["toolName"].as_str().unwrap_or("工具").to_owned();
                let id = self.active_message_id.clone().unwrap_or_else(|| {
                    let value = format!("tool-{}", self.next_message_id);
                    self.next_message_id += 1;
                    value
                });
                let message_index = snapshot.messages.iter().position(|item| item.id == id);
                let message = if let Some(index) = message_index {
                    &mut snapshot.messages[index]
                } else {
                    snapshot.messages.push(Message {
                        id: id.clone(),
                        role: "tool".into(),
                        blocks: Vec::new(),
                    });
                    snapshot.messages.last_mut().expect("message just inserted")
                };
                message.blocks.push(MessageBlock::Tool {
                    tool_id,
                    title,
                    state: Some("running".into()),
                });
            }
            "agent_settled" => {
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
                    role: "notice".into(),
                    blocks: vec![MessageBlock::Notice {
                        text: "运行时发生错误".into(),
                    }],
                });
                self.next_message_id += 1;
            }
            _ => {}
        }
        snapshot.revision = snapshot.revision.saturating_add(1);
        self.snapshot = Some(snapshot);
    }

    fn message_id(&mut self, message: &Value) -> String {
        message["id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                let id = format!("message-{}", self.next_message_id);
                self.next_message_id += 1;
                id
            })
    }

    fn message_from_pi(&mut self, message: &Value, index: u64) -> Option<Message> {
        let role = message["role"].as_str()?.to_owned();
        let id = message["id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("message-{index}"));
        let content = message["content"].as_array();
        let mut blocks = Vec::new();
        if let Some(content) = content {
            for block in content {
                match block["type"].as_str().unwrap_or("") {
                    "text" => blocks.push(MessageBlock::Text {
                        text: block["text"].as_str().unwrap_or("").into(),
                    }),
                    "toolCall" | "tool_call" => blocks.push(MessageBlock::Tool {
                        tool_id: block["id"].as_str().map(str::to_owned),
                        title: block["name"].as_str().unwrap_or("工具").into(),
                        state: Some("done".into()),
                    }),
                    _ => {}
                }
            }
        } else if let Some(text) = message["content"].as_str() {
            blocks.push(MessageBlock::Text { text: text.into() });
        }
        if blocks.is_empty() {
            return None;
        }
        Some(Message { id, role, blocks })
    }
}
