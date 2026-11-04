use crate::conversation::{
    ConversationActions, ConversationSnapshot, ConversationSummary, ExecutionKind, Message,
    MessageBlock, MessageRole, RunState, ToolState, TurnOutcome, first_user_text,
};
use serde_json::Value;

#[derive(Debug, Default, PartialEq, Eq)]
enum Lifecycle {
    #[default]
    Idle,
    Running,
    Ended,
    Closed,
}

#[derive(Debug, Default)]
pub struct PiProjection {
    pub snapshot: Option<ConversationSnapshot>,
    next_message_id: u64,
    active_message_id: Option<String>,
    active_native_message: Option<Value>,
    next_native_identity: Option<String>,
    history_synchronized: bool,
    /// The current or latest turn ended a model response with an error.
    terminal: Option<TurnOutcome>,
    lifecycle: Lifecycle,
    failed_message_id: Option<String>,
    manual_compaction: Option<String>,
    compaction_notice: Option<Message>,
    compaction_aborted: bool,
}

impl PiProjection {
    pub fn new(conversation: ConversationSummary) -> Self {
        Self {
            snapshot: Some(ConversationSnapshot {
                revision: 0,
                context_runtime_id: conversation.runtime_id.clone(),
                conversation,
                resource_id: None,
                model_record_key: None,
                run_state: RunState::Idle,
                messages: Vec::new(),
                executions: Default::default(),
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
                    input_access: Default::default(),
                    can_send: true,
                    can_cancel: false,
                    can_switch: true,
                },
            }),
            next_message_id: 0,
            active_message_id: None,
            active_native_message: None,
            next_native_identity: None,
            history_synchronized: false,
            terminal: None,
            lifecycle: Lifecycle::Idle,
            failed_message_id: None,
            manual_compaction: None,
            compaction_notice: None,
            compaction_aborted: false,
        }
    }

    /// Pi records a failed model response as an assistant message and keeps
    /// the conversation usable; this tells such a turn from a completed one.
    pub fn turn_failed(&self) -> bool {
        self.terminal == Some(TurnOutcome::Failed)
    }

    /// An accepted prompt may still be waiting for the runtime to start.
    pub fn begin_turn(&mut self) {
        self.compaction_notice = None;
        self.terminal = None;
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.executions.begin(ExecutionKind::UserTurn, false);
        }
        if !matches!(self.lifecycle, Lifecycle::Running | Lifecycle::Closed) {
            self.lifecycle = Lifecycle::Idle;
        }
    }

    /// Completion stays observable until the next accepted prompt.
    pub fn turn_ended(&self) -> bool {
        matches!(self.lifecycle, Lifecycle::Ended | Lifecycle::Closed)
    }

    /// A terminal transport failure leaves the current projection readable.
    pub fn can_sync_history(&self) -> bool {
        self.lifecycle != Lifecycle::Closed
    }

    pub fn invalidate_history(&mut self) {
        self.history_synchronized = false;
        self.compaction_notice = None;
    }

    pub fn begin_compaction(&mut self, command_id: String) {
        self.manual_compaction = Some(command_id);
        self.compaction_notice = None;
        self.compaction_aborted = false;
        self.terminal = None;
        self.lifecycle = Lifecycle::Running;
        if let Some(snapshot) = &mut self.snapshot {
            snapshot
                .executions
                .begin(ExecutionKind::ContextMaintenance, false);
        }
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.run_state = RunState::Running;
            snapshot.actions.can_send = false;
            snapshot.actions.can_cancel = true;
            snapshot.revision += 1;
        }
    }

    pub fn history_synchronized(&self) -> bool {
        self.history_synchronized
    }

    pub fn replace_history(&mut self, response: &Value) {
        if self.lifecycle == Lifecycle::Closed {
            return;
        }
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
        let mut projected: Vec<Message> = Vec::new();
        // The user message that starts the turn the following replies belong to.
        let mut turn = None;
        // Pi's estimate of every reply so far, by its price list.
        let mut cost = 0.0;
        for (index, message) in messages.iter().enumerate() {
            if message["role"] == "toolResult" {
                apply_tool_result(&mut projected, message, message["id"].as_str());
            } else if let Some(mut message) = message_from_pi(message, format!("message-{index}")) {
                if message.role == MessageRole::User {
                    message.turn_settings = Some(crate::conversation::TurnSettings::default());
                    turn = Some(projected.len());
                }
                projected.push(message);
            }
            // Pi records the model on each reply; the turn's is its first reply's.
            if message["role"] == "assistant"
                && let Some(model) = message["model"].as_str().filter(|model| !model.is_empty())
                && let Some(settings) =
                    turn.and_then(|index| projected[index].turn_settings.as_mut())
            {
                settings.model.get_or_insert_with(|| model.to_owned());
            }
            if message["role"] == "assistant" {
                cost += message["usage"]["cost"]["total"].as_f64().unwrap_or(0.0);
            }
            if message["role"] == "assistant"
                && let Some(index) = turn
                && message["usage"].is_object()
            {
                let mut usage = pi_usage(&message["usage"]);
                // A model without a price costs 0 in Pi's estimate: unknown, not free.
                usage.session_cost_micro_usd = crate::conversation::micro_usd(cost);
                match &mut projected[index].turn_usage {
                    Some(total) => total.add(&usage),
                    total => *total = Some(usage),
                }
            }
        }
        self.history_synchronized = true;
        let mut confirmed: std::collections::BTreeSet<_> = snapshot
            .message_identity_confirmations
            .iter()
            .map(|confirmation| confirmation.previous_id.clone())
            .collect();
        if let Some(confirmations) = response["identityConfirmations"].as_array() {
            for confirmation in confirmations {
                let confirmation = serde_json::from_value::<
                    crate::conversation::MessageIdentityConfirmation,
                >(confirmation.clone())
                .expect("Pi client validates identity confirmations at the RPC boundary");
                if confirmed.insert(confirmation.previous_id.clone()) {
                    snapshot.message_identity_confirmations.push(confirmation);
                }
            }
        }
        let native_ids: std::collections::BTreeSet<_> = projected
            .iter()
            .map(|message| message.id.as_str())
            .collect();
        snapshot
            .message_identity_confirmations
            .retain(|confirmation| native_ids.contains(confirmation.current_id.as_str()));
        retain_diagnostics(
            &snapshot.messages,
            &mut projected,
            &snapshot.message_identity_confirmations,
        );
        if let Some(notice) = &self.compaction_notice {
            projected.push(notice.clone());
        }
        snapshot
            .executions
            .confirm(&snapshot.message_identity_confirmations);
        snapshot.messages = projected;
        snapshot.executions.retain(&snapshot.messages);
        let visible: std::collections::BTreeSet<_> = snapshot
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect();
        let claimed: std::collections::BTreeSet<_> = snapshot
            .executions
            .records
            .iter()
            .flat_map(|record| record.message_ids.iter().cloned())
            .collect();
        let mut start = 0;
        for (index, native) in messages.iter().enumerate() {
            if let Some(outcome) = pi_outcome(native) {
                let ids: Vec<_> = messages[start..=index]
                    .iter()
                    .filter_map(|m| m["id"].as_str())
                    .filter(|id| visible.contains(id) && !claimed.contains(*id))
                    .map(str::to_owned)
                    .collect();
                if !ids.is_empty() {
                    snapshot.executions.history(
                        format!("pi-history-{index}"),
                        ExecutionKind::UserTurn,
                        ids,
                        Some(outcome),
                    );
                }
                start = index + 1;
            }
        }
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
        if self.lifecycle == Lifecycle::Closed {
            return;
        }
        let Some(mut snapshot) = self.snapshot.take() else {
            return;
        };
        let before = snapshot.messages.len();
        match event["type"].as_str().unwrap_or_default() {
            "agent_start" | "turn_start" => {
                if event["type"] == "agent_start" {
                    self.terminal = None;
                    self.lifecycle = Lifecycle::Running;
                }
                snapshot.run_state = RunState::Running;
                snapshot.actions.can_send = false;
                snapshot.actions.can_cancel = true;
            }
            "message_update" => {
                // Pi 1.0.2 JSON/RPC deliberately omits cumulative `message` and
                // `partial`. Reconstruct its native content from ordered deltas.
                if let (Some(id), Some(native)) =
                    (&self.active_message_id, &mut self.active_native_message)
                {
                    apply_assistant_delta(native, &event["assistantMessageEvent"]);
                    if let Some(mut message) = message_from_pi(native, id.clone()) {
                        message.completed = event["type"] == "message_end"
                            || message.role != MessageRole::Assistant;
                        if let Some(existing) = snapshot.messages.iter_mut().find(|m| m.id == *id) {
                            *existing = message;
                        } else {
                            snapshot.messages.push(message);
                        }
                    }
                }
            }
            "message_start" | "message_end" => {
                let native = &event["message"];
                if event["type"] == "message_end" && native["role"] == "assistant" {
                    self.terminal = pi_outcome(native);
                }
                if native["role"] == "toolResult" {
                    if event["type"] == "message_start" {
                        self.active_message_id = self.next_native_identity.take();
                    }
                    apply_tool_result(
                        &mut snapshot.messages,
                        native,
                        self.active_message_id.as_deref(),
                    );
                    if event["type"] == "message_end" {
                        self.active_message_id = None;
                    }
                } else {
                    let id = if event["type"] == "message_start" {
                        let id = self
                            .next_native_identity
                            .take()
                            .or_else(|| native["id"].as_str().map(str::to_owned))
                            .unwrap_or_else(|| {
                                let id = format!("pi-unconfirmed-{}", self.next_message_id);
                                self.next_message_id += 1;
                                id
                            });
                        self.active_message_id = Some(id.clone());
                        self.active_native_message =
                            (native["role"] == "assistant").then(|| native.clone());
                        id
                    } else {
                        self.active_message_id
                            .clone()
                            .unwrap_or_else(|| format!("message-{}", self.next_message_id))
                    };
                    if let Some(mut message) = message_from_pi(native, id.clone()) {
                        if event["type"] == "message_end" && native["role"] == "assistant" {
                            self.failed_message_id =
                                (self.terminal == Some(TurnOutcome::Failed)).then(|| id.clone());
                        }
                        message.completed = event["type"] == "message_end"
                            || message.role != MessageRole::Assistant;
                        if let Some(existing) = snapshot.messages.iter_mut().find(|m| m.id == id) {
                            *existing = message;
                        } else {
                            snapshot.messages.push(message);
                        }
                    }
                    if event["type"] == "message_end" {
                        self.active_message_id = None;
                        self.active_native_message = None;
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
                        turn_settings: None,
                        turn_usage: None,
                        branch_point: None,
                        completed: true,
                        id: format!("tool-{}", id.unwrap_or_default()),
                        role: MessageRole::Tool,
                        timestamp_unix_ms: None,
                        blocks: vec![MessageBlock::Tool {
                            tool_id: id.map(str::to_owned),
                            title: event["toolName"].as_str().unwrap_or("工具").into(),
                            state,
                            output,
                            conversation_id: None,
                        }],
                    });
                }
            }
            "extension_ui_request"
                if event["method"] == "setStatus"
                    && event["statusKey"] == "velune.message-identity" =>
            {
                self.next_native_identity = event["statusText"].as_str().map(str::to_owned);
                self.snapshot = Some(snapshot);
                return;
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
                        turn_settings: None,
                        turn_usage: None,
                        branch_point: None,
                        completed: true,
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
            "compaction_start" => {
                self.compaction_aborted = false;
                self.compaction_status(&mut snapshot, "正在压缩上下文".into(), false);
            }
            "compaction_end" => {
                self.compaction_aborted = event["aborted"] == true;
                let succeeded = event["result"].is_object();
                let text = if self.compaction_aborted {
                    "上下文压缩已取消".into()
                } else if let Some(error) = event["errorMessage"].as_str() {
                    format!("上下文压缩失败：{error}")
                } else if succeeded {
                    "上下文已压缩".into()
                } else {
                    "上下文未压缩".into()
                };
                self.compaction_status(&mut snapshot, text, true);

                self.history_synchronized = false;
            }
            "response"
                if event["command"] == "compact"
                    && event["id"].as_str() == self.manual_compaction.as_deref() =>
            {
                let failed = event["success"] != true && !self.compaction_aborted;
                if failed
                    && self
                        .compaction_notice
                        .as_ref()
                        .is_none_or(|message| !message.completed)
                {
                    self.compaction_status(
                        &mut snapshot,
                        format!(
                            "上下文压缩失败：{}",
                            event["error"].as_str().unwrap_or("运行时未提供原因")
                        ),
                        true,
                    );
                }
                self.manual_compaction = None;
                self.terminal = Some(if self.compaction_aborted {
                    TurnOutcome::Cancelled
                } else if failed {
                    TurnOutcome::Failed
                } else {
                    TurnOutcome::Completed
                });
                self.lifecycle = Lifecycle::Ended;
                snapshot.executions.finish(self.terminal);
                snapshot.run_state = RunState::Idle;
                snapshot.actions.can_send = true;
                snapshot.actions.can_cancel = false;
                self.history_synchronized = false;
            }
            "agent_settled" => {
                if self.manual_compaction.is_some() {
                    self.snapshot = Some(snapshot);
                    return;
                }
                if self.lifecycle != Lifecycle::Running {
                    self.snapshot = Some(snapshot);
                    return;
                }
                // Pi 1.1+ also reports cancellation before an assistant message
                // exists, for example while a before_agent_start hook runs.
                if event["aborted"] == true {
                    self.terminal = Some(TurnOutcome::Cancelled);
                }
                self.lifecycle = Lifecycle::Ended;
                snapshot.executions.finish(self.terminal);
                for message in &mut snapshot.messages {
                    message.completed = true;
                }
                self.history_synchronized = false;
                snapshot.run_state = if self.terminal == Some(TurnOutcome::Failed) {
                    RunState::Failed
                } else {
                    RunState::Idle
                };
                snapshot.actions.can_send = true;
                snapshot.actions.can_cancel = false;
                self.active_message_id = None;
                self.active_native_message = None;
                self.next_native_identity = None;
            }
            "auto_retry_start" => {
                if let Some(id) = self.failed_message_id.take() {
                    snapshot.messages.retain(|message| message.id != id);
                }
                self.terminal = None;
                let text = event["errorMessage"]
                    .as_str()
                    .unwrap_or("Pi 请求失败")
                    .to_owned();
                let message = Message {
                    id: "pi-auto-retry".into(),
                    role: MessageRole::System,
                    timestamp_unix_ms: None,
                    blocks: vec![MessageBlock::RuntimeFailure {
                        failure: crate::conversation::RuntimeFailureKind::Other,
                        detail: text,
                        retry: Some(crate::conversation::RuntimeRetry::Native),
                    }],
                    completed: false,
                    turn_settings: None,
                    turn_usage: None,
                    branch_point: None,
                };
                snapshot.executions.attach(&message.id);
                if let Some(old) = snapshot.messages.iter_mut().find(|m| m.id == message.id) {
                    *old = message;
                } else {
                    snapshot.messages.push(message);
                }
            }
            "auto_retry_end" => {
                snapshot
                    .messages
                    .retain(|message| message.id != "pi-auto-retry");
            }
            "extension_error" => {
                let text = format!(
                    "Pi 扩展错误：{}\n{} · {}",
                    event["error"].as_str().unwrap_or("上游未提供错误详情"),
                    event["extensionPath"].as_str().unwrap_or_default(),
                    event["event"].as_str().unwrap_or_default()
                );
                self.diagnostic(&mut snapshot, text);
            }
            "velune_error" | "velune_closed" => {
                let compacting = self.manual_compaction.is_some();
                let error = event["error"]
                    .as_str()
                    .unwrap_or("Pi 控制连接已关闭，请重新连接。");
                if compacting
                    || self
                        .compaction_notice
                        .as_ref()
                        .is_some_and(|message| !message.completed)
                {
                    self.compaction_status(
                        &mut snapshot,
                        format!("上下文压缩已中断：{error}"),
                        true,
                    );
                }
                self.manual_compaction = None;
                self.lifecycle = Lifecycle::Closed;
                self.terminal = Some(TurnOutcome::Failed);
                snapshot.run_state = RunState::Failed;
                snapshot.actions.can_send = false;
                snapshot.actions.can_cancel = false;
                snapshot
                    .messages
                    .retain(|message| message.id != "pi-auto-retry");
                for message in &mut snapshot.messages {
                    message.completed = true;
                    for block in &mut message.blocks {
                        if let MessageBlock::Tool { state, .. } = block
                            && matches!(state, ToolState::Pending | ToolState::Running)
                        {
                            *state = ToolState::Failed;
                        }
                    }
                }
                if !compacting {
                    self.diagnostic(&mut snapshot, error.into());
                }
                for message in &snapshot.messages[before.min(snapshot.messages.len())..] {
                    snapshot.executions.attach(&message.id);
                }
                snapshot.executions.finish(Some(TurnOutcome::Failed));
            }
            _ => {
                self.snapshot = Some(snapshot);
                return;
            }
        }
        for message in &snapshot.messages[before.min(snapshot.messages.len())..] {
            snapshot.executions.attach(&message.id);
        }
        refresh_title(&mut snapshot);
        snapshot.revision = snapshot.revision.saturating_add(1);
        self.snapshot = Some(snapshot);
    }

    fn diagnostic(&mut self, snapshot: &mut ConversationSnapshot, text: String) {
        snapshot.messages.push(Message {
            id: format!("pi-diagnostic-{}", self.next_message_id),
            role: MessageRole::System,
            timestamp_unix_ms: None,
            blocks: vec![MessageBlock::Notice { text }],
            completed: true,
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
        });
        self.next_message_id += 1;
    }

    fn compaction_status(
        &mut self,
        snapshot: &mut ConversationSnapshot,
        text: String,
        completed: bool,
    ) {
        let id = self
            .compaction_notice
            .as_ref()
            .filter(|message| !message.completed)
            .map(|message| message.id.clone())
            .unwrap_or_else(|| {
                self.next_message_id += 1;
                format!("pi-compaction-{}", self.next_message_id)
            });
        let message = Message {
            id,
            role: MessageRole::System,
            timestamp_unix_ms: None,
            blocks: vec![MessageBlock::ContextCompaction { text }],
            completed,
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
        };
        if let Some(old) = snapshot
            .messages
            .iter_mut()
            .find(|old| old.id == message.id)
        {
            *old = message.clone();
        } else {
            snapshot.messages.push(message.clone());
        }
        snapshot.executions.attach(&message.id);
        self.compaction_notice = Some(message);
    }
}

fn retain_diagnostics(
    old: &[Message],
    messages: &mut Vec<Message>,
    confirmations: &[crate::conversation::MessageIdentityConfirmation],
) {
    use std::collections::HashMap;
    let mut positions: HashMap<_, _> = messages
        .iter()
        .enumerate()
        .map(|(index, message)| (message.id.as_str(), index + 1))
        .collect();
    for confirmation in confirmations {
        if let Some(position) = positions.get(confirmation.current_id.as_str()).copied() {
            positions.insert(confirmation.previous_id.as_str(), position);
        }
    }
    let mut notices: HashMap<usize, Vec<Message>> = HashMap::new();
    let mut anchor = Some(0);
    for message in old {
        if message.id.starts_with("pi-diagnostic-") {
            if let Some(position) = anchor {
                notices.entry(position).or_default().push(message.clone());
            }
        } else if message.id != "pi-auto-retry" {
            anchor = positions.get(message.id.as_str()).copied();
        }
    }
    let native = std::mem::take(messages);
    let native_len = native.len();
    for (index, message) in native.into_iter().enumerate() {
        messages.extend(notices.remove(&index).unwrap_or_default());
        messages.push(message);
    }
    messages.extend(notices.remove(&native_len).unwrap_or_default());
}

fn apply_assistant_delta(message: &mut Value, event: &Value) {
    if event["type"] == "done" {
        *message = event["message"].clone();
        return;
    }
    if event["type"] == "error" {
        *message = event["error"].clone();
        return;
    }
    let Some(index) = event["contentIndex"]
        .as_u64()
        .and_then(|index| usize::try_from(index).ok())
    else {
        return;
    };
    let Some(content) = message["content"].as_array_mut() else {
        return;
    };
    // The fixed protocol indexes blocks consecutively; an invalid index cannot
    // allocate an unbounded sparse message or silently attach to another block.
    if index > content.len() {
        tracing::error!(target: "velune_agent_runtime", event="pi_stream_block_index_invalid", index, expected_max=content.len(), detail="Pi 流式块索引跳过已有内容范围");
        return;
    }
    if index == content.len() {
        content.push(Value::Null);
    }
    let block = &mut content[index];
    match event["type"].as_str() {
        Some("text_start") => *block = serde_json::json!({"type":"text","text":""}),
        Some("thinking_start") => *block = serde_json::json!({"type":"thinking","thinking":""}),
        Some("text_delta" | "thinking_delta") => {
            let key = if event["type"] == "text_delta" {
                "text"
            } else {
                "thinking"
            };
            if let Some(delta) = event["delta"].as_str()
                && let Some(text) = block[key].as_str()
            {
                block[key] = Value::String(format!("{text}{delta}"));
            }
        }
        Some("text_end" | "thinking_end") => {
            let key = if event["type"] == "text_end" {
                "text"
            } else {
                "thinking"
            };
            if let Some(text) = event["content"].as_str() {
                block[key] = Value::String(text.into());
            }
        }
        Some("toolcall_start") => {
            *block =
                serde_json::json!({"type":"toolCall","id":event["id"],"name":event["toolName"]})
        }
        Some("toolcall_end") => *block = event["toolCall"].clone(),
        _ => {} // Tool argument fragments are not a separate presentation block.
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
    if let Some(text) = content.as_str() {
        return Some(text.into());
    }
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
fn apply_tool_result(messages: &mut Vec<Message>, native: &Value, identity: Option<&str>) {
    let id = native["toolCallId"].as_str();
    let state = if native["isError"] == true {
        ToolState::Failed
    } else {
        ToolState::Completed
    };
    let output = text_content(&native["content"]);
    if !update_tool(messages, id, state.clone(), output.clone()) {
        messages.push(Message {
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
            completed: true,
            id: identity
                .map(str::to_owned)
                .unwrap_or_else(|| format!("tool-result-{}", messages.len())),
            role: MessageRole::Tool,
            timestamp_unix_ms: native["timestamp"].as_i64(),
            blocks: vec![MessageBlock::Tool {
                tool_id: id.map(str::to_owned),
                title: native["toolName"].as_str().unwrap_or("工具").into(),
                state,
                output,
                conversation_id: None,
            }],
        });
    }
}
fn message_from_pi(message: &Value, fallback_id: String) -> Option<Message> {
    if message["role"] == "custom" && message["display"] == false {
        return None;
    }
    let role = match message["role"].as_str()? {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        "toolResult" => MessageRole::Tool,
        "custom" => MessageRole::Client,
        _ => MessageRole::System,
    };
    let id = message["id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or(fallback_id);
    let mut blocks = Vec::new();
    if message["role"] == "compactionSummary" {
        blocks.push(MessageBlock::ContextCompaction {
            text: "上下文已压缩".into(),
        });
    }
    if let Some(content) = message["content"].as_array() {
        for block in content {
            match block["type"].as_str().unwrap_or_default() {
                "text"
                    if block["text"]
                        .as_str()
                        .is_some_and(|text| !text.trim().is_empty()) =>
                {
                    blocks.push(MessageBlock::Text {
                        text: block["text"].as_str().unwrap_or_default().into(),
                    })
                }
                "thinking"
                    if block["thinking"]
                        .as_str()
                        .is_some_and(|text| !text.trim().is_empty()) =>
                {
                    blocks.push(MessageBlock::Reasoning {
                        text: block["thinking"].as_str().unwrap_or_default().into(),
                    })
                }
                "toolCall" => blocks.push(MessageBlock::Tool {
                    tool_id: block["id"].as_str().map(str::to_owned),
                    title: block["name"].as_str().unwrap_or("工具").into(),
                    state: ToolState::Pending,
                    output: None,
                    conversation_id: None,
                }),
                "image" => blocks.push(match block["data"].as_str() {
                    Some(data) => MessageBlock::Image {
                        media_type: block["mimeType"].as_str().map(str::to_owned),
                        source: crate::conversation::ImageSource::Data { data: data.into() },
                    },
                    None => MessageBlock::Notice {
                        text: "图片".into(),
                    },
                }),
                _ => {}
            }
        }
    } else if let Some(text) = message["content"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
    {
        blocks.push(MessageBlock::Text { text: text.into() });
    }
    if let Some(error) = message["errorMessage"]
        .as_str()
        .filter(|error| !error.trim().is_empty())
    {
        blocks.push(MessageBlock::Notice { text: error.into() });
    } else if message["role"] == "assistant" && message["stopReason"] == "error" {
        blocks.push(MessageBlock::Notice {
            text: "Pi 请求失败（上游未提供错误详情）。".into(),
        });
    }
    (!blocks.is_empty()).then(|| Message {
        turn_settings: None,
        turn_usage: None,
        branch_point: None,
        completed: true,
        id,
        role,
        timestamp_unix_ms: message["timestamp"].as_i64(),
        blocks,
    })
}

/// Pi counts input without cache reads and writes, and reasoning, when the
/// provider reports it, within output. A reply's request holds its input and
/// output, which is where the context stands after it.
fn pi_usage(usage: &Value) -> crate::conversation::TurnUsage {
    let read = usage["cacheRead"].as_u64();
    let write = usage["cacheWrite"].as_u64();
    let input = usage["input"]
        .as_u64()
        .map(|input| input + read.unwrap_or(0) + write.unwrap_or(0));
    let output = usage["output"].as_u64();
    crate::conversation::TurnUsage {
        input_tokens: input,
        cache_read_tokens: read,
        cache_write_tokens: write,
        output_tokens: output,
        reasoning_tokens: usage["reasoning"].as_u64(),
        context_tokens: input.zip(output).map(|(input, output)| input + output),
        ..Default::default()
    }
}

fn pi_outcome(message: &Value) -> Option<TurnOutcome> {
    if message["role"] != "assistant" {
        return None;
    }
    match message["stopReason"].as_str() {
        Some("stop") => Some(TurnOutcome::Completed),
        Some("error") => Some(TurnOutcome::Failed),
        Some("aborted") => Some(TurnOutcome::Cancelled),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(super) fn projection() -> PiProjection {
        PiProjection::new(ConversationSummary {
            native_read_state: None,
            id: "pi:synthetic".into(),
            runtime_id: "pi".into(),
            title: crate::conversation::conversation_title(None, None),
            cwd: None,
            created_at_unix_ms: None,
            updated_at_unix_ms: None,
            can_rename: false,
            can_delete: false,
            parent_conversation_id: None,
        })
    }

    fn message(projection: &mut PiProjection, value: Value) {
        projection.apply_event(&json!({"type":"message_start","message":value}));
        projection.apply_event(&json!({"type":"message_end","message":value}));
    }

    #[test]
    fn extension_notifications_are_visible_client_events_without_a_user_turn() {
        let mut p = projection();
        message(
            &mut p,
            json!({"role":"custom","customType":"background-task","content":"Task completed","display":true,"timestamp":1234}),
        );
        let snapshot = p.snapshot.as_ref().unwrap();
        assert_eq!(snapshot.messages.last().unwrap().role, MessageRole::Client);
        assert_eq!(
            snapshot.messages.last().unwrap().timestamp_unix_ms,
            Some(1234)
        );
        assert!(
            !snapshot
                .messages
                .iter()
                .any(|message| message.role == MessageRole::User)
        );
        assert!(
            message_from_pi(
                &json!({"role":"custom","content":"hidden context","display":false}),
                "hidden".into()
            )
            .is_none()
        );
    }

    #[test]
    fn compaction_disconnect_keeps_the_previous_reply_visible() {
        for started in [false, true] {
            let mut p = projection();
            p.replace_history(&json!({"messages":[
                {"id":"user","role":"user","content":[{"type":"text","text":"hello"}]},
                {"id":"answer","role":"assistant","content":[{"type":"text","text":"done"}]}
            ]}));
            p.begin_compaction("compact-1".into());
            if started {
                p.apply_event(&json!({"type":"compaction_start","reason":"manual"}));
            }
            p.apply_event(&json!({"type":"velune_closed","error":"synthetic EOF"}));
            let snapshot = p.snapshot.as_ref().unwrap();
            assert_eq!(snapshot.run_state, RunState::Failed);
            assert!(snapshot.messages.iter().all(|message| message.completed));
            assert!(
                matches!(&snapshot.messages.last().unwrap().blocks[0], MessageBlock::ContextCompaction { text } if text.contains("synthetic EOF"))
            );
            let mut transcript = crate::transcript::TranscriptProjection::default();
            transcript.apply(crate::transcript::TranscriptEvent::Replace {
                messages: &snapshot.messages,
                executions: &snapshot.executions,
            });
            assert_eq!(
                transcript.turns()[0].last_message_id.as_deref(),
                Some("answer")
            );
        }
    }

    #[test]
    fn manual_compaction_is_pollable_survives_refresh_and_cancels() {
        let mut p = projection();
        p.begin_compaction("compact-1".into());
        p.apply_event(&json!({"type":"compaction_start","reason":"manual"}));
        let id = p
            .snapshot
            .as_ref()
            .unwrap()
            .messages
            .last()
            .unwrap()
            .id
            .clone();
        p.replace_history(&json!({"messages":[]}));
        assert_eq!(p.snapshot.as_ref().unwrap().messages[0].id, id);
        assert_eq!(p.snapshot.as_ref().unwrap().run_state, RunState::Running);
        p.apply_event(&json!({"type":"compaction_end","reason":"manual","aborted":true}));
        p.apply_event(&json!({"type":"response","command":"compact","id":"compact-1","success":false,"error":"cancelled"}));
        let snapshot = p.snapshot.as_ref().unwrap();
        assert_eq!(snapshot.run_state, RunState::Idle);
        assert!(p.turn_ended() && !p.turn_failed());
        assert_eq!(snapshot.messages[0].id, id);
        assert_eq!(
            snapshot.messages[0].blocks,
            vec![MessageBlock::ContextCompaction {
                text: "上下文压缩已取消".into()
            }]
        );
        p.apply_event(&json!({"type":"velune_closed"}));
        assert!(
            !p.snapshot
                .as_ref()
                .unwrap()
                .messages
                .iter()
                .any(|message| !message.completed)
        );
    }

    #[test]
    fn extension_diagnostic_keeps_its_position_when_native_history_is_replaced() {
        let mut p = projection();
        p.begin_turn();
        p.apply_event(&json!({"type":"agent_start"}));
        let user = json!({"id":"prompt","role":"user","content":"question"});
        let answer =
            json!({"id":"answer","role":"assistant","stopReason":"stop","content":"answer"});
        message(&mut p, user.clone());
        p.apply_event(
            &json!({"type":"extension_error","extensionPath":"synthetic.ts",
            "event":"before_agent_start","error":"synthetic extension failure"}),
        );
        assert_eq!(p.snapshot.as_ref().unwrap().run_state, RunState::Running);
        message(&mut p, answer.clone());
        p.apply_event(&json!({"type":"agent_settled"}));
        assert!(p.turn_ended());
        assert!(!p.turn_failed());
        let newer = json!({"id":"newer","role":"user","content":"next question"});
        for _ in 0..2 {
            p.replace_history(&json!({"messages":[user,answer,newer]}));
            let snapshot = p.snapshot.as_ref().unwrap();
            assert_eq!(snapshot.run_state, RunState::Idle);
            assert!(snapshot.actions.can_send);
            assert_eq!(snapshot.messages.len(), 4);
            assert!(snapshot.messages[1].id.starts_with("pi-diagnostic-"));
            let MessageBlock::Notice { text } = &snapshot.messages[1].blocks[0] else {
                panic!()
            };
            assert!(text.contains("synthetic extension failure") && text.contains("synthetic.ts"));
            assert_eq!(snapshot.messages[2].id, "answer");
        }
    }

    #[test]
    fn retry_outcome_comes_from_the_last_native_message_not_retry_end() {
        for stop in ["stop", "error", "aborted"] {
            let mut p = projection();
            p.begin_turn();
            p.apply_event(&json!({"type":"agent_start"}));
            message(
                &mut p,
                json!({"id":"omitted","role":"assistant","stopReason":"error",
                "errorMessage":"first failed attempt","content":[]}),
            );
            p.apply_event(
                &json!({"type":"auto_retry_start","attempt":1,"maxAttempts":3,
                "delayMs":10,"errorMessage":"first failed attempt"}),
            );
            assert!(!p.turn_ended());
            assert!(
                !p.snapshot
                    .as_ref()
                    .unwrap()
                    .messages
                    .iter()
                    .any(|m| m.id == "omitted")
            );
            p.apply_event(&json!({"type":"agent_start"}));
            let last = json!({"id":"last","role":"assistant","stopReason":stop,
                "errorMessage":if stop=="error" {"final native failure"} else {""},
                "content":[{"type":"text","text":"last response"}]});
            message(&mut p, last.clone());
            p.apply_event(&json!({"type":"auto_retry_end","success":stop=="stop","finalError":"Retry cancelled"}));
            p.apply_event(&json!({"type":"agent_settled"}));
            assert!(p.turn_ended());
            assert_eq!(p.turn_failed(), stop == "error");
            p.replace_history(&json!({"messages":[last]}));
            let snapshot = p.snapshot.as_ref().unwrap();
            assert_eq!(snapshot.messages.len(), 1);
            assert_eq!(snapshot.messages[0].id, "last");
            assert_eq!(
                snapshot.run_state,
                if stop == "error" {
                    RunState::Failed
                } else {
                    RunState::Idle
                }
            );
            assert!(snapshot.actions.can_send);
        }
    }

    #[test]
    fn settled_abort_without_assistant_is_cancelled_but_idle_does_not_imply_success() {
        for aborted in [false, true] {
            let mut p = projection();
            p.begin_turn();
            p.apply_event(&json!({"type":"agent_start"}));
            message(
                &mut p,
                json!({"id":"user","role":"user","content":"question"}),
            );
            p.apply_event(&json!({"type":"agent_settled","aborted":aborted}));
            let snapshot = p.snapshot.as_ref().unwrap();
            assert_eq!(
                snapshot.executions.record("user").unwrap().outcome,
                aborted.then_some(TurnOutcome::Cancelled)
            );
            assert!(p.turn_ended());
            assert!(!p.turn_failed());
        }
    }

    #[test]
    fn terminal_connection_failure_survives_late_lifecycle_and_history_records() {
        let mut p = projection();
        p.apply_event(&json!({"type":"agent_settled"}));
        assert!(!p.turn_ended());
        p.begin_turn();
        p.apply_event(&json!({"type":"velune_error","error":"invalid JSON; synthetic stderr"}));
        for value in [
            json!({"type":"agent_start"}),
            json!({"type":"agent_settled"}),
        ] {
            p.apply_event(&value);
        }
        p.replace_history(&json!({"messages":[]}));
        p.begin_turn();
        let snapshot = p.snapshot.as_ref().unwrap();
        assert!(p.turn_ended());
        assert!(!p.can_sync_history());
        assert_eq!(snapshot.run_state, RunState::Failed);
        assert!(!snapshot.actions.can_send && !snapshot.actions.can_cancel);
        assert_eq!(snapshot.messages.len(), 1);
        let MessageBlock::Notice { text } = &snapshot.messages[0].blocks[0] else {
            panic!()
        };
        assert_eq!(text, "invalid JSON; synthetic stderr");
    }

    #[test]
    fn connection_failure_ends_a_visible_retry() {
        let mut p = projection();
        p.begin_turn();
        p.apply_event(&json!({"type":"agent_start"}));
        p.apply_event(
            &json!({"type":"auto_retry_start","attempt":1,"maxAttempts":3,
            "errorMessage":"temporary failure"}),
        );
        p.apply_event(&json!({"type":"velune_error","error":"synthetic connection lost"}));
        assert!(
            !p.snapshot
                .as_ref()
                .unwrap()
                .messages
                .iter()
                .any(|m| m.id == "pi-auto-retry")
        );
        assert!(p.turn_ended() && !p.can_sync_history());
    }
}

#[cfg(test)]
mod execution_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn settled_without_terminal_message_is_unknown() {
        let mut p = super::tests::projection();
        p.begin_turn();
        p.apply_event(&json!({"type":"agent_start"}));
        p.apply_event(&json!({"type":"agent_settled"}));
        let snapshot = p.snapshot.unwrap();
        assert_eq!(snapshot.run_state, RunState::Idle);
        assert!(snapshot.executions.current.unwrap().ended);
        assert_eq!(snapshot.executions.records[0].outcome, None);
    }
    #[test]
    fn pi_terminal_evidence_survives_native_confirmation_and_namespace() {
        for (stop, expected) in [
            ("stop", Some(TurnOutcome::Completed)),
            ("error", Some(TurnOutcome::Failed)),
            ("aborted", Some(TurnOutcome::Cancelled)),
            ("toolUse", None),
        ] {
            let mut p = super::tests::projection();
            p.begin_turn();
            p.apply_event(&json!({"type":"agent_start"}));
            for message in [
                json!({"id":"local-user","role":"user","content":"question"}),
                json!({"id":"reply","role":"assistant","stopReason":stop,"content":"answer"}),
            ] {
                p.apply_event(&json!({"type":"message_start","message":message}));
                p.apply_event(&json!({"type":"message_end","message":message}));
            }
            p.apply_event(&json!({"type":"agent_settled"}));
            p.replace_history(&json!({"messages":[{"id":"native-user","role":"user","content":"question"},{"id":"reply","role":"assistant","stopReason":stop,"content":"answer"}], "identityConfirmations":[{"previousId":"local-user","currentId":"native-user"}]}));
            let snapshot = p.snapshot.as_mut().unwrap();
            snapshot.executions.map_ids(&|id| format!("segment-2:{id}"));
            let record = snapshot.executions.record("segment-2:native-user").unwrap();
            assert_eq!(record.outcome, expected, "{stop}");
        }
    }
}
