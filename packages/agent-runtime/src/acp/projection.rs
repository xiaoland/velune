use super::*;

pub(super) struct Stream {
    id: String,
    role: MessageRole,
    thought: bool,
    native_id: Option<String>,
}

impl Client {
    pub(super) fn update(&mut self, value: &Value) -> Result<()> {
        let update: wire::SessionUpdate = match serde_json::from_value(value.clone()) {
            Ok(update) => update,
            Err(error) if error.to_string().starts_with("unknown variant") => return Ok(()),
            Err(error) => return Err(Error::new(format!("ACP 会话更新无效：{error}"))),
        };
        match update {
            wire::SessionUpdate::UserMessageChunk(chunk) => {
                self.chunk(chunk, MessageRole::User, false)
            }
            wire::SessionUpdate::AgentMessageChunk(chunk) => {
                self.chunk(chunk, MessageRole::Assistant, false)
            }
            wire::SessionUpdate::AgentThoughtChunk(chunk) => {
                self.chunk(chunk, MessageRole::Assistant, true)
            }
            wire::SessionUpdate::ToolCall(tool) => self.tool(
                tool.tool_call_id.to_string(),
                Some(tool.title),
                Some(tool.status),
                Some(tool.content),
            ),
            wire::SessionUpdate::ToolCallUpdate(tool) => self.tool(
                tool.tool_call_id.to_string(),
                tool.fields.title,
                tool.fields.status,
                tool.fields.content,
            ),
            wire::SessionUpdate::ConfigOptionUpdate(update) => {
                self.options = config::project(update.config_options)?
            }
            wire::SessionUpdate::SessionInfoUpdate(update) => {
                let value =
                    serde_json::to_value(update).map_err(|error| Error::new(error.to_string()))?;
                if let Some(snapshot) = &mut self.snapshot {
                    if let Some(title) = value.get("title") {
                        snapshot.conversation.title =
                            conversation_title(title.as_str(), first_user_text(&snapshot.messages));
                    }
                    if let Some(at) = value.get("updatedAt") {
                        snapshot.conversation.updated_at_unix_ms = at.as_str().and_then(timestamp);
                    }
                    snapshot.revision += 1;
                }
            }
            wire::SessionUpdate::Plan(plan) => {
                let content = plan
                    .entries
                    .into_iter()
                    .map(|entry| {
                        let state = match entry.status {
                            wire::PlanEntryStatus::Pending => "待处理",
                            wire::PlanEntryStatus::InProgress => "进行中",
                            wire::PlanEntryStatus::Completed => "完成",
                            _ => "状态未知",
                        };
                        format!("{state}：{}", entry.content)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let parent = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| {
                        snapshot
                            .messages
                            .iter()
                            .rev()
                            .find(|message| message.role == MessageRole::User)
                    })
                    .map(|message| message.id.as_str())
                    .unwrap_or("initial");
                self.message(Message {
                    turn_settings: None,
                    turn_usage: None,
                    branch_point: None,
                    id: format!("acp:plan:{parent}"),
                    completed: true,
                    role: MessageRole::System,
                    timestamp_unix_ms: None,
                    blocks: vec![MessageBlock::Notice { text: content }],
                });
            }
            // No client filesystem, terminal, notices or compaction capabilities are advertised.
            // Optional commands, modes and usage do not become fabricated conversation messages.
            _ => {}
        }
        Ok(())
    }

    fn chunk(&mut self, chunk: wire::ContentChunk, role: MessageRole, thought: bool) {
        if role != MessageRole::User {
            self.pending_user_echo = None;
        }
        let native_id = chunk.message_id.map(|id| id.to_string());
        let reuse = self.stream.as_ref().is_some_and(|stream| {
            stream.role == role && stream.thought == thought && stream.native_id == native_id
        });
        if !reuse {
            self.complete_stream();
            // The first live user stream is the agent's authoritative echo of this prompt.
            // Replace the local placeholder by protocol position, never by matching message text.
            let echo = if role == MessageRole::User {
                self.pending_user_echo.take()
            } else {
                None
            };
            let fallback = echo.clone().unwrap_or_else(|| self.next_message_id());
            let id = native_id
                .as_ref()
                .map(|id| format!("acp:{id}:{}", if thought { "thought" } else { "message" }))
                .unwrap_or(fallback);
            if let Some(previous_id) = echo
                && let Some(snapshot) = &mut self.snapshot
                && let Some(previous) = snapshot
                    .messages
                    .iter_mut()
                    .find(|message| message.id == previous_id)
            {
                previous.blocks.clear();
                previous.id = id.clone();
                previous.completed = false;
                if previous_id != id {
                    snapshot
                        .message_identity_confirmations
                        .push(MessageIdentityConfirmation {
                            previous_id,
                            current_id: id.clone(),
                        });
                }
            }
            self.stream = Some(Stream {
                id,
                role: role.clone(),
                thought,
                native_id,
            });
        }
        let id = self.stream.as_ref().expect("active chunk").id.clone();
        let mut message = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.messages.iter().find(|message| message.id == id))
            .cloned()
            .unwrap_or(Message {
                turn_settings: None,
                turn_usage: None,
                branch_point: None,
                id,
                completed: false,
                role,
                timestamp_unix_ms: None,
                blocks: Vec::new(),
            });
        // A user's image comes back in the Agent's echo of the prompt.
        if let (MessageRole::User, wire::ContentBlock::Image(image)) =
            (&message.role, &chunk.content)
        {
            message.blocks.push(MessageBlock::Image {
                media_type: Some(image.mime_type.clone()),
                source: crate::conversation::ImageSource::Data {
                    data: image.data.clone(),
                },
            });
            self.message(message);
            return;
        }
        let text = content_text(chunk.content);
        match message.blocks.last_mut() {
            Some(MessageBlock::Text { text: previous }) if !thought => previous.push_str(&text),
            Some(MessageBlock::Reasoning { text: previous }) if thought => previous.push_str(&text),
            _ => message.blocks.push(if thought {
                MessageBlock::Reasoning { text }
            } else {
                MessageBlock::Text { text }
            }),
        }
        self.message(message);
    }

    fn tool(
        &mut self,
        native_id: String,
        title: Option<String>,
        status: Option<wire::ToolCallStatus>,
        content: Option<Vec<wire::ToolCallContent>>,
    ) {
        self.complete_stream();
        self.stream = None;
        self.pending_user_echo = None;
        let id = format!("acp:tool:{native_id}");
        let previous = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.messages.iter().find(|message| message.id == id))
            .and_then(|message| message.blocks.first());
        let (old_title, old_state, old_output) = match previous {
            Some(MessageBlock::Tool {
                title,
                state,
                output,
                ..
            }) => (title.clone(), state.clone(), output.clone()),
            _ => ("工具操作".into(), ToolState::Pending, None),
        };
        let state = status
            .map(|status| match status {
                wire::ToolCallStatus::Pending => ToolState::Pending,
                wire::ToolCallStatus::InProgress => ToolState::Running,
                wire::ToolCallStatus::Completed => ToolState::Completed,
                wire::ToolCallStatus::Failed => ToolState::Failed,
                _ => ToolState::Pending,
            })
            .unwrap_or(old_state);
        let output = content
            .map(|parts| {
                parts
                    .into_iter()
                    .map(|part| match part {
                        wire::ToolCallContent::Content(content) => content_text(content.content),
                        wire::ToolCallContent::Diff(diff) => {
                            format!("{}\n{}", diff.path.display(), diff.new_text)
                        }
                        wire::ToolCallContent::Terminal(_) => {
                            "Agent 引用了未由此客户端创建的终端。".into()
                        }
                        _ => "Agent 返回了尚未支持的工具内容。".into(),
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .or(old_output);
        self.message(Message {
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
            id,
            completed: matches!(state, ToolState::Completed | ToolState::Failed),
            role: MessageRole::Tool,
            timestamp_unix_ms: None,
            blocks: vec![MessageBlock::Tool {
                tool_id: Some(native_id),
                title: title.unwrap_or(old_title),
                state,
                output,
                conversation_id: None,
            }],
        });
    }

    pub(super) fn next_id(&mut self, kind: &str) -> String {
        self.sequence += 1;
        format!("{}:{kind}:{}", self.epoch, self.sequence)
    }
    pub(super) fn next_message_id(&mut self) -> String {
        self.message_sequence += 1;
        format!(
            "acp:{}:position:{}",
            self.native_id
                .as_deref()
                .expect("message belongs to session"),
            self.message_sequence
        )
    }
    pub(super) fn complete_messages(&mut self) {
        if let Some(snapshot) = &mut self.snapshot {
            for message in &mut snapshot.messages {
                message.completed = true;
            }
        }
    }
    fn complete_stream(&mut self) {
        if let Some(stream) = &self.stream
            && let Some(snapshot) = &mut self.snapshot
            && let Some(message) = snapshot
                .messages
                .iter_mut()
                .find(|message| message.id == stream.id)
        {
            message.completed = true;
        }
    }
    pub(super) fn message(&mut self, message: Message) {
        if let Some(snapshot) = &mut self.snapshot {
            if let Some(previous) = snapshot
                .messages
                .iter_mut()
                .find(|previous| previous.id == message.id)
            {
                *previous = message;
            } else {
                snapshot.messages.push(message);
            }
            snapshot
                .conversation
                .title
                .refresh(first_user_text(&snapshot.messages));
            snapshot.revision += 1;
        }
    }
    pub fn notice(&mut self, text: &str) {
        let id = self.next_id("notice");
        self.message(Message {
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
            id,
            completed: true,
            role: MessageRole::System,
            timestamp_unix_ms: None,
            blocks: vec![MessageBlock::Notice { text: text.into() }],
        });
    }
    pub(super) fn state(&mut self, run_state: RunState, can_send: bool, can_cancel: bool) {
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.run_state = run_state;
            snapshot.actions = ConversationActions {
                can_send,
                can_cancel,
                can_switch: !can_cancel,
            };
            snapshot.revision += 1;
        }
    }
    pub(super) fn replace_snapshot(&mut self, native_id: &str, cwd: &Path, runtime_id: &str) {
        self.native_id = Some(native_id.into());
        self.message_sequence = 0;
        self.snapshot = Some(ConversationSnapshot {
            revision: 1,
            conversation: ConversationSummary {
                id: format!("{runtime_id}:{native_id}"),
                runtime_id: runtime_id.into(),
                title: ConversationTitle::Untitled,
                cwd: Some(cwd.to_string_lossy().into_owned()),
                created_at_unix_ms: None,
                updated_at_unix_ms: None,
                can_rename: false,
                can_delete: self.capabilities.delete_session,
                parent_conversation_id: None,
            },
            context_runtime_id: runtime_id.into(),
            resource_id: None,
            model_record_key: None,
            run_state: RunState::Idle,
            messages: Vec::new(),
            transcript_turns: Vec::new(),
            transcript_items: Vec::new(),
            transcript_outline: Vec::new(),
            transcript_message_identities: Vec::new(),
            message_identity_confirmations: Vec::new(),
            pending_interactions: Vec::new(),
            follow_up: Default::default(),
            sub_agents: Vec::new(),
            collaborators: Vec::new(),
            actions: ConversationActions {
                can_send: true,
                can_cancel: false,
                can_switch: true,
            },
        });
    }
}

fn content_text(content: wire::ContentBlock) -> String {
    match content {
        wire::ContentBlock::Text(text) => text.text,
        wire::ContentBlock::ResourceLink(link) => format!("{} ({})", link.name, link.uri),
        wire::ContentBlock::Resource(resource) => match resource.resource {
            wire::EmbeddedResourceResource::TextResourceContents(text) => text.text,
            _ => "Agent 返回了二进制资源。".into(),
        },
        wire::ContentBlock::Image(_) => "Agent 返回了图像内容；当前会话投影仅呈现文本。".into(),
        wire::ContentBlock::Audio(_) => "Agent 返回了音频内容；当前会话投影仅呈现文本。".into(),
        _ => "Agent 返回了尚未支持的内容。".into(),
    }
}
