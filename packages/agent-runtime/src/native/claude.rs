//! Claude Code 2.1.286 stream-json control; provider overrides exist only in the turn child.
use super::*;

/// Levels `--effort` accepts; the CLI applies one per launch and keeps none
/// in the session.
pub const CLAUDE_EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// `--agents` takes the definitions a file's frontmatter would hold, keyed
/// by name, with the instructions as `prompt`.
fn claude_agents(agents: &[SessionAgent]) -> Value {
    Value::Object(
        agents
            .iter()
            .map(|agent| {
                let mut definition = json!({
                    "description": agent.description,
                    "prompt": agent.instructions,
                });
                if let Some(model) = &agent.model {
                    definition["model"] = json!(model);
                }
                if let Some(effort) = agent
                    .effort
                    .as_deref()
                    .filter(|level| crate::management::CLAUDE_EFFORTS.contains(level))
                {
                    definition["effort"] = json!(effort);
                }
                if !agent.tools.is_empty() {
                    definition["tools"] = json!(agent.tools);
                }
                if !agent.skills.is_empty() {
                    definition["skills"] = json!(agent.skills);
                }
                (agent.name.clone(), definition)
            })
            .collect(),
    )
}

/// How the next Claude Code turn child takes up the conversation's session.
#[derive(Clone, Default)]
pub(super) enum ClaudeStart {
    /// Resume the session.
    #[default]
    Resume,
    /// Begin a new session under the conversation's identity.
    Fresh,
    /// Resume with the history cut after record `at`: the new prompt follows
    /// it, and what came after stays in the file on another branch.
    ResumeAt { at: String },
    /// Begin the conversation's session as a copy of session `from` up to
    /// record `at`; `from` is left as it is.
    Fork { from: String, at: String },
}

impl NativeSession {
    pub(super) fn claude_command(
        config: &NativeConfig,
        selection: Option<&RuntimeModel>,
        session: Option<(&str, Option<&Path>, &ClaudeStart)>,
        effort: Option<&str>,
        permission_mode: Option<&str>,
    ) -> Result<Command> {
        let mut command = Command::new(&config.binary);
        command.env("CLAUDE_CODE_SDK_READS_SESSION_STATE", "1");
        command.args([
            "--print",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-prompt-tool",
            "stdio",
        ]);
        if let Some(effort) = effort {
            if !CLAUDE_EFFORT_LEVELS.contains(&effort) {
                return Err(Error::new("Claude Code 不接受此推理强度"));
            }
            command.arg("--effort").arg(effort);
        }
        if let Some(mode) = permission_mode {
            if !crate::version::CLAUDE_PERMISSION_MODES
                .iter()
                .any(|known| known.id == mode)
            {
                return Err(Error::new("Claude Code 不接受此权限模式"));
            }
            command.arg("--permission-mode").arg(mode);
        }
        if let Some((id, cwd, start)) = session {
            // `--resume-session-at` and `--fork-session` apply in print mode.
            match start {
                ClaudeStart::Resume => command.arg("--resume").arg(id),
                ClaudeStart::Fresh => command.arg("--session-id").arg(id),
                ClaudeStart::ResumeAt { at } => command
                    .arg("--resume")
                    .arg(id)
                    .arg("--resume-session-at")
                    .arg(at),
                ClaudeStart::Fork { from, at } => command
                    .arg("--resume")
                    .arg(from)
                    .arg("--resume-session-at")
                    .arg(at)
                    .arg("--fork-session")
                    .arg("--session-id")
                    .arg(id),
            };
            if let Some(cwd) = cwd {
                command.current_dir(cwd);
            }
        }
        if let Some(gateway) = &config.gateway {
            if gateway.protocol != "anthropic-messages" {
                return Err(Error::new("Claude Code 需要 Anthropic Messages 网关协议"));
            }
            // The host-managed flag prevents settings.env from replacing the turn's endpoint and auth.
            command
                .env("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST", "1")
                .env(
                    "ANTHROPIC_BASE_URL",
                    gateway
                        .endpoint
                        .strip_suffix("/v1")
                        .unwrap_or(&gateway.endpoint),
                )
                .env("ANTHROPIC_AUTH_TOKEN", &gateway.token)
                .env("ANTHROPIC_API_KEY", "")
                .env("CLAUDE_CODE_OAUTH_TOKEN", "")
                .env("CLAUDE_CODE_USE_BEDROCK", "0")
                .env("CLAUDE_CODE_USE_VERTEX", "0")
                .env("CLAUDE_CODE_USE_FOUNDRY", "0")
                .env("ANTHROPIC_CUSTOM_HEADERS", "")
                .env("ANTHROPIC_MODEL", &gateway.model_alias)
                .env("ANTHROPIC_DEFAULT_SONNET_MODEL", &gateway.model_alias)
                .env("ANTHROPIC_DEFAULT_OPUS_MODEL", &gateway.model_alias)
                .env("ANTHROPIC_DEFAULT_HAIKU_MODEL", &gateway.model_alias)
                .env("CLAUDE_CODE_SUBAGENT_MODEL", &gateway.model_alias)
                .arg("--model")
                .arg(&gateway.model_alias);
            if !config.session_agents.is_empty() {
                command
                    .arg("--agents")
                    .arg(claude_agents(&config.session_agents).to_string());
            }
        } else {
            command.env_remove("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST");
            if let Some(selection) = selection {
                command.arg("--model").arg(&selection.id);
            }
        }
        Ok(command)
    }
    pub(super) fn claude_initialize(&mut self) -> Result<()> {
        let response = self
            .rpc
            .claude_control(json!({"subtype":"initialize","hooks":{}}))?;
        self.available_models = response["models"].as_array().cloned().unwrap_or_default();
        // Velune's services are optional for the conversation.
        if !self.config.mcp_servers.is_empty()
            && let Err(error) = self.claude_add_mcp_servers()
        {
            tracing::warn!(%error, "Claude did not accept Velune's MCP services; running without them");
        }
        Ok(())
    }
    fn claude_add_mcp_servers(&mut self) -> Result<()> {
        let existing = self.rpc.claude_control(json!({"subtype":"mcp_status"}))?;
        let existing = existing["mcpServers"]
            .as_array()
            .ok_or_else(|| Error::new("Claude MCP 服务目录无效"))?;
        for server in &self.config.mcp_servers {
            if existing.iter().any(|value| value["name"] == server.name) {
                return Err(Error::new("临时 MCP 服务与 Claude 既有服务名称冲突"));
            }
        }
        // The fixed CLI's dynamic host control merges these servers without writing .mcp.json.
        // Sending headers on stdin also keeps the lease out of process arguments and files.
        let servers: serde_json::Map<String, Value> = self
            .config
            .mcp_servers
            .iter()
            .map(|server| (server.name.clone(), server.http()))
            .collect();
        let result = self
            .rpc
            .claude_control(json!({"subtype":"mcp_set_servers","servers":servers}))?;
        if result["errors"]
            .as_object()
            .is_none_or(|errors| !errors.is_empty())
        {
            return Err(Error::new("Claude 未能添加临时 MCP 服务"));
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            let status = self.rpc.claude_control(json!({"subtype":"mcp_status"}))?;
            let statuses = status["mcpServers"]
                .as_array()
                .ok_or_else(|| Error::new("Claude MCP 服务状态无效"))?;
            let mut ready = true;
            for server in &self.config.mcp_servers {
                match statuses
                    .iter()
                    .find(|value| value["name"] == server.name)
                    .and_then(|value| value["status"].as_str())
                {
                    Some("connected") => {}
                    Some("pending") => ready = false,
                    _ => return Err(Error::new("Claude 临时 MCP 服务未能连接")),
                }
            }
            if ready {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(Error::new("Claude 临时 MCP 服务连接超时"));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Ok(())
    }
    /// Claude Code 2.1.286 lists `supportedEffortLevels` for models that
    /// `supportsEffort`.
    pub(super) fn claude_models(&self) -> Result<Vec<ModelOption>> {
        Ok(self
            .available_models
            .iter()
            .filter_map(|model| {
                let id = model["value"].as_str()?;
                let efforts = if model["supportsEffort"] == true {
                    model["supportedEffortLevels"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|level| level.as_str().map(str::to_owned))
                        .collect()
                } else {
                    Vec::new()
                };
                Some(ModelOption {
                    model: RuntimeModel {
                        provider_id: None,
                        id: id.into(),
                        name: model["displayName"].as_str().unwrap_or(id).into(),
                    },
                    reasoning_efforts: efforts,
                })
            })
            .collect())
    }
    pub(super) fn claude_create(&mut self, cwd: Option<&Path>, runtime_id: &str) -> Result<()> {
        let id = claude_session_id()?;
        let cwd = cwd
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.default_cwd.clone());
        self.replace_snapshot(&id, Some(&cwd), runtime_id, Vec::new());
        self.claude_start = ClaudeStart::Fresh;
        Ok(())
    }
    /// The next turn child resumes at the record before the message; a fork
    /// copies the session up to there under a new identity, which the
    /// first turn creates.
    pub(super) fn claude_branch(&mut self, index: usize, placement: EditPlacement) -> Result<()> {
        let snapshot = self.snapshot.as_ref().expect("branched snapshot");
        let at = snapshot.messages[index]
            .branch_point
            .clone()
            .ok_or_else(|| Error::new("Claude Code 不能替换会话的第一条消息"))?;
        match placement {
            EditPlacement::InPlace => {
                self.claude_start = ClaudeStart::ResumeAt { at };
                self.truncate_snapshot(index);
            }
            EditPlacement::Fork => {
                let from = self
                    .native_id
                    .clone()
                    .ok_or_else(|| Error::new("没有原生会话身份"))?;
                let id = claude_session_id()?;
                let cwd = snapshot.conversation.cwd.as_deref().map(PathBuf::from);
                let runtime_id = snapshot.conversation.runtime_id.clone();
                let history = snapshot.messages[..index].to_vec();
                self.replace_snapshot(&id, cwd.as_deref(), &runtime_id, history);
                self.claude_start = ClaudeStart::Fork { from, at };
            }
        }
        Ok(())
    }
    pub(super) fn claude_open(
        &mut self,
        id: &str,
        cwd: Option<&Path>,
        runtime_id: &str,
        history: Vec<Message>,
    ) -> Result<()> {
        self.replace_snapshot(id, cwd, runtime_id, history);
        self.claude_start = ClaudeStart::Resume;
        Ok(())
    }
    pub(super) fn claude_send(&mut self, input: &crate::TurnInput) -> Result<()> {
        let text = input.text.as_str();
        self.claude_result = None;
        self.claude_tasks.clear();
        self.claude_requires_action = false;
        self.claude_stream_id = None;
        // Images go before the text, as Anthropic recommends.
        let content = input
            .images
            .iter()
            .map(|image| json!({"type":"image","source":{"type":"base64","media_type":image.kind.media_type(),"data":image.base64()}}))
            .chain((!text.is_empty()).then(|| json!({"type":"text","text":text})))
            .collect::<Vec<_>>();
        self.rpc.write(&json!({"type":"user","session_id":self.native_id,"parent_tool_use_id":null,"message":{"role":"user","content":content}}))?;
        self.turn_id = Some(format!(
            "{}:{}",
            self.epoch,
            self.snapshot.as_ref().map_or(0, |s| s.revision)
        ));
        self.message(Message {
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
            completed: true,
            id: format!("{}:user", self.turn_id.as_deref().unwrap_or_default()),
            role: MessageRole::User,
            timestamp_unix_ms: None,
            blocks: input
                .images
                .iter()
                .map(|image| MessageBlock::Image {
                    media_type: Some(image.kind.media_type().into()),
                    source: crate::conversation::ImageSource::Path {
                        path: image.path.to_string_lossy().into_owned(),
                    },
                })
                .chain((!text.is_empty()).then(|| MessageBlock::Text { text: text.into() }))
                .collect(),
        });
        self.claude_start = ClaudeStart::Resume;
        self.running();
        Ok(())
    }
    /// A user message written while a turn runs joins that turn: Claude Code
    /// takes it in at its next step instead of starting another.
    pub(super) fn claude_steer(&mut self, input: &crate::TurnInput) -> Result<()> {
        let text = input.text.as_str();
        let content = input
            .images
            .iter()
            .map(|image| json!({"type":"image","source":{"type":"base64","media_type":image.kind.media_type(),"data":image.base64()}}))
            .chain((!text.is_empty()).then(|| json!({"type":"text","text":text})))
            .collect::<Vec<_>>();
        self.rpc.write(&json!({"type":"user","session_id":self.native_id,"parent_tool_use_id":null,"message":{"role":"user","content":content}}))?;
        self.steer_sequence += 1;
        let id = format!(
            "{}:steer:{}",
            self.turn_id.as_deref().unwrap_or_default(),
            self.steer_sequence
        );
        self.message(Message {
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
            completed: true,
            id,
            role: MessageRole::User,
            timestamp_unix_ms: None,
            blocks: input
                .images
                .iter()
                .map(|image| MessageBlock::Image {
                    media_type: Some(image.kind.media_type().into()),
                    source: crate::conversation::ImageSource::Path {
                        path: image.path.to_string_lossy().into_owned(),
                    },
                })
                .chain((!text.is_empty()).then(|| MessageBlock::Text { text: text.into() }))
                .collect(),
        });
        Ok(())
    }
    pub(super) fn claude_cancel(&mut self) -> Result<()> {
        self.rpc.claude_control(json!({"subtype":"interrupt"}))?;
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.run_state = RunState::Stopping;
            snapshot.revision += 1;
        }
        Ok(())
    }
    pub(super) fn claude_handle_record(&mut self, record: Value) -> Result<()> {
        match record["type"].as_str() {
            Some("control_cancel_request") => {
                let expired = self
                    .interactions
                    .iter()
                    .find(|(_, pending)| pending.id == record["request_id"])
                    .map(|(id, _)| id.clone());
                if let Some(id) = expired {
                    self.interactions.remove(&id);
                    if let Some(snapshot) = &mut self.snapshot {
                        snapshot
                            .pending_interactions
                            .retain(|interaction| interaction.id != id);
                        snapshot.revision += 1;
                    }
                }
            }
            Some("control_request") => {
                let request = &record["request"];
                let id = record["request_id"].clone();
                if request["subtype"] == "can_use_tool"
                    && self.busy()
                    && request["tool_name"] == "AskUserQuestion"
                {
                    let questions = request["input"]["questions"]
                        .as_array()
                        .ok_or_else(|| Error::new("Claude 用户问题格式无效"))?
                        .iter()
                        .map(|question| {
                            let text = question["question"]
                                .as_str()
                                .filter(|text| !text.is_empty())
                                .ok_or_else(|| Error::new("Claude 用户问题为空"))?;
                            let options = question["options"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|option| {
                                    option["label"].as_str().map(|label| InteractionOption {
                                        id: label.into(),
                                        label: label.into(),
                                    })
                                })
                                .collect();
                            Ok(InteractionQuestion {
                                id: text.into(),
                                text: text.into(),
                                options,
                                secret: false,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    if questions.is_empty() {
                        return Err(Error::new("Claude 用户问题为空"));
                    }
                    self.add_interaction(
                        PendingRequest {
                            id,
                            method: "AskUserQuestion".into(),
                            options: Vec::new(),
                            question_ids: questions.iter().map(|q| q.id.clone()).collect(),
                            payload: request.clone(),
                        },
                        InteractionKind::UserInput { questions },
                    );
                } else if request["subtype"] == "can_use_tool" && self.busy() {
                    self.add_interaction(
                        PendingRequest {
                            id,
                            method: "can_use_tool".into(),
                            options: vec!["allow".into(), "deny".into()],
                            question_ids: Vec::new(),
                            payload: request.clone(),
                        },
                        InteractionKind::Approval {
                            title: format!(
                                "允许 {}？",
                                request["tool_name"].as_str().unwrap_or("工具操作")
                            ),
                            detail: serde_json::to_string(&request["input"]).unwrap_or_default(),
                            options: vec![
                                InteractionOption {
                                    id: "allow".into(),
                                    label: "允许这一次".into(),
                                },
                                InteractionOption {
                                    id: "deny".into(),
                                    label: "拒绝".into(),
                                },
                            ],
                        },
                    );
                } else {
                    self.rpc.write(&json!({"type":"control_response","response":{"subtype":"error","request_id":id,"error":"Unsupported control request"}}))?;
                }
            }
            Some("system") => match record["subtype"].as_str() {
                Some("session_state_changed") => {
                    self.claude_requires_action = record["state"] == "requires_action";
                    if record["state"] == "idle"
                        && self.busy()
                        && let Some((failed, _)) = self.claude_result.take()
                    {
                        self.finish_turn(if failed {
                            RunState::Failed
                        } else {
                            RunState::Idle
                        })?;
                        if failed {
                            self.notice("Claude Code 轮次未成功完成，请检查运行时诊断。");
                        }
                    }
                }
                Some("task_started")
                    if matches!(
                        record["task_type"].as_str(),
                        Some("local_agent" | "local_workflow")
                    ) =>
                {
                    if let Some(id) = record["task_id"].as_str() {
                        self.claude_tasks.insert(id.into());
                        if record["task_type"] == "local_agent"
                            && let Some(conversation) = self.claude_agent_conversation(id)
                        {
                            self.sub_agents.insert(conversation);
                        }
                    }
                }
                Some("task_notification") => {
                    if let Some(id) = record["task_id"].as_str() {
                        self.claude_tasks.remove(id);
                        let conversation = self.claude_agent_conversation(id);
                        if let Some(conversation) =
                            conversation.filter(|c| self.sub_agents.contains(c))
                        {
                            self.message(Message {
                                turn_settings: None,
                                turn_usage: None,
                                branch_point: None,
                                completed: true,
                                id: record["uuid"].as_str().unwrap_or(id).into(),
                                role: MessageRole::Tool,
                                timestamp_unix_ms: None,
                                blocks: vec![MessageBlock::Tool {
                                    tool_id: record["tool_use_id"].as_str().map(str::to_owned),
                                    title: "任务通知".into(),
                                    state: if record["status"] == "completed" {
                                        ToolState::Completed
                                    } else {
                                        ToolState::Failed
                                    },
                                    output: record["summary"].as_str().map(str::to_owned),
                                    conversation_id: Some(conversation),
                                }],
                            });
                        }
                    }
                }
                Some("task_updated")
                    if matches!(
                        record["patch"]["status"].as_str(),
                        Some("completed" | "failed" | "stopped" | "killed")
                    ) =>
                {
                    if let Some(id) = record["task_id"].as_str() {
                        self.claude_tasks.remove(id);
                    }
                }
                _ => {}
            },
            Some("assistant" | "user") => {
                if let Some((_, last_activity)) = &mut self.claude_result {
                    *last_activity = std::time::Instant::now();
                }
                // A sub-agent's own messages belong to its conversation, as
                // in the native history, not to this transcript.
                if record["parent_tool_use_id"].is_string() {
                    return Ok(());
                }
                let agent = record["tool_use_result"]["agentId"]
                    .as_str()
                    .and_then(|agent| self.claude_agent_conversation(agent));
                // What the latest reply's request held: where the context stands.
                let reply = &record["message"];
                if record["type"] == "assistant"
                    && let (Some(model), Some(tokens)) =
                        (reply["model"].as_str(), claude_context(&reply["usage"]))
                    && !model.starts_with('<')
                {
                    self.claude_context = Some((model.to_owned(), tokens));
                }
                let role = if record["type"] == "assistant" {
                    MessageRole::Assistant
                } else {
                    MessageRole::Tool
                };
                let id = record["message"]["id"]
                    .as_str()
                    .or_else(|| record["uuid"].as_str())
                    .unwrap_or("message");
                let mut blocks = claude_blocks(&record["message"]["content"], agent);
                if role == MessageRole::Tool {
                    blocks.retain(|result| !self.claude_complete_call(result));
                }
                if !blocks.is_empty() {
                    self.message(Message {
                        turn_settings: None,
                        turn_usage: None,
                        branch_point: None,
                        completed: true,
                        id: id.into(),
                        role,
                        timestamp_unix_ms: None,
                        blocks,
                    });
                }
            }
            Some("stream_event") => {
                if let Some((_, last_activity)) = &mut self.claude_result {
                    *last_activity = std::time::Instant::now();
                }
                let event = &record["event"];
                if event["type"] == "message_start" {
                    self.claude_stream_id = event["message"]["id"].as_str().map(str::to_owned);
                }
                if event["type"] == "content_block_delta" {
                    let thought = event["delta"]["type"] == "thinking_delta";
                    let delta = if thought {
                        event["delta"]["thinking"].as_str()
                    } else {
                        event["delta"]["text"].as_str()
                    };
                    if let Some(delta) = delta {
                        let id = self.claude_stream_id.clone().unwrap_or_else(|| {
                            format!(
                                "{}:stream:{}",
                                self.turn_id.as_deref().unwrap_or_default(),
                                event["index"]
                            )
                        });
                        let mut message = self
                            .snapshot
                            .as_ref()
                            .and_then(|s| s.messages.iter().find(|m| m.id == id))
                            .cloned()
                            .unwrap_or(Message {
                                turn_settings: None,
                                turn_usage: None,
                                branch_point: None,
                                completed: false,
                                id,
                                role: MessageRole::Assistant,
                                timestamp_unix_ms: None,
                                blocks: Vec::new(),
                            });
                        match message.blocks.last_mut() {
                            Some(MessageBlock::Text { text }) if !thought => text.push_str(delta),
                            Some(MessageBlock::Reasoning { text }) if thought => {
                                text.push_str(delta)
                            }
                            _ => message.blocks.push(if thought {
                                MessageBlock::Reasoning { text: delta.into() }
                            } else {
                                MessageBlock::Text { text: delta.into() }
                            }),
                        }
                        self.message(message);
                    }
                }
            }
            Some("result") => {
                // Partial records are replaced by the authoritative assistant message before terminal delivery.
                let prefix = format!("{}:stream:", self.turn_id.as_deref().unwrap_or_default());
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot
                        .messages
                        .retain(|message| !message.id.starts_with(&prefix));
                }
                let failed = record["is_error"] == true || record["subtype"] != "success";
                self.claude_result = Some((failed, std::time::Instant::now()));
                // The turn's usage over its requests, the context its last
                // reply left with that model's window, and the session's cost
                // while Claude Code knows every model's price.
                if record["usage"].is_object() {
                    let mut usage = claude_usage(&record["usage"]);
                    let models = record["modelUsage"].as_object();
                    if let Some((model, tokens)) = self.claude_context.take() {
                        usage.context_tokens = Some(tokens);
                        usage.context_window = models
                            .and_then(|models| models.get(&model))
                            .and_then(|entry| entry["contextWindow"].as_u64());
                    }
                    if models.is_some_and(|models| {
                        models.values().all(|entry| entry["costBasis"] != "unknown")
                    }) {
                        usage.session_cost_micro_usd =
                            record["total_cost_usd"].as_f64().and_then(micro_usd);
                    }
                    self.turn_usage(|total| *total = Some(usage));
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// Complete the call a tool result answers, as one row like the native
    /// transcript shows it; false when the call is not in this projection.
    fn claude_complete_call(&mut self, result: &MessageBlock) -> bool {
        let MessageBlock::Tool {
            tool_id: Some(id),
            state,
            output,
            conversation_id,
            ..
        } = result
        else {
            return false;
        };
        let Some(snapshot) = &mut self.snapshot else {
            return false;
        };
        let call = snapshot
            .messages
            .iter_mut()
            .rev()
            .flat_map(|message| message.blocks.iter_mut())
            .find_map(|block| match block {
                MessageBlock::Tool {
                    tool_id: Some(call),
                    state,
                    output,
                    conversation_id,
                    ..
                } if call == id => Some((state, output, conversation_id)),
                _ => None,
            });
        let Some((call_state, call_output, call_conversation)) = call else {
            return false;
        };
        *call_state = state.clone();
        *call_output = output.clone();
        *call_conversation = conversation_id.clone();
        snapshot.revision += 1;
        self.sub_agents.extend(conversation_id.clone());
        true
    }
    /// The sub-agents whose tasks have not ended.
    pub(super) fn claude_running_sub_agents(&self) -> Vec<String> {
        self.claude_tasks
            .iter()
            .filter_map(|task| self.claude_agent_conversation(task))
            .filter(|conversation| self.sub_agents.contains(conversation))
            .collect()
    }
    /// A sub-agent's session, as the history reader names it.
    fn claude_agent_conversation(&self, agent: &str) -> Option<String> {
        let runtime_id = &self.snapshot.as_ref()?.conversation.runtime_id;
        Some(format!(
            "{runtime_id}:{}/agent-{agent}",
            self.native_id.as_deref()?
        ))
    }
    pub(super) fn claude_check_terminal_timeout(&mut self) {
        // Claude SDK 2.1.286 can keep observers alive after result; its between-turn ceiling is ten minutes.
        if self
            .claude_result
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= std::time::Duration::from_secs(600))
            && self.claude_tasks.is_empty()
            && !self.claude_requires_action
            && self.interactions.is_empty()
        {
            self.claude_result = None;
            self.settle(RunState::Failed);
            self.notice("Claude Code 返回结果后未确认空闲；已结束本轮进程，请检查原生运行时状态。");
        }
    }
    pub(super) fn claude_reply(
        &mut self,
        pending: &PendingRequest,
        reply: RuntimeInteractionReply,
    ) -> Result<()> {
        let response = match reply {
            RuntimeInteractionReply::Answers { answers } if pending.method == "AskUserQuestion" => {
                let mut selected = serde_json::Map::new();
                for answer in answers {
                    if !pending.question_ids.contains(&answer.question_id)
                        || selected.contains_key(&answer.question_id)
                    {
                        return Err(Error::new("Claude 答案与当前问题不匹配"));
                    }
                    selected.insert(answer.question_id, json!(answer.values.join(", ")));
                }
                if selected.len() != pending.question_ids.len() {
                    return Err(Error::new("请回答所有问题"));
                }
                let mut input = pending.payload["input"].clone();
                input["answers"] = json!(selected);
                json!({"behavior":"allow","updatedInput":input})
            }
            RuntimeInteractionReply::Decision { option_id } if option_id == "allow" => {
                json!({"behavior":"allow","updatedInput":pending.payload["input"]})
            }
            RuntimeInteractionReply::Decision { option_id } if option_id == "deny" => {
                json!({"behavior":"deny","message":"User denied this operation"})
            }
            RuntimeInteractionReply::Cancel => {
                json!({"behavior":"deny","message":"User cancelled this operation","interrupt":true})
            }
            _ => return Err(Error::new("Claude 工具权限回复无效")),
        };
        self.rpc.write(&json!({"type":"control_response","response":{"subtype":"success","request_id":pending.id,"response":response}}))
    }
}
/// `agent` is the conversation of the sub-agent a tool result started.
fn claude_blocks(content: &Value, agent: Option<String>) -> Vec<MessageBlock> {
    if let Some(text) = content.as_str() {
        return vec![MessageBlock::Text { text: text.into() }];
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| match block["type"].as_str() {
            Some("text") => block["text"]
                .as_str()
                .map(|text| MessageBlock::Text { text: text.into() }),
            Some("thinking") => block["thinking"]
                .as_str()
                .map(|text| MessageBlock::Reasoning { text: text.into() }),
            Some("tool_use") => Some(MessageBlock::Tool {
                tool_id: block["id"].as_str().map(str::to_owned),
                title: block["name"].as_str().unwrap_or("工具操作").into(),
                state: ToolState::Running,
                output: None,
                conversation_id: None,
            }),
            Some("tool_result") => Some(MessageBlock::Tool {
                tool_id: block["tool_use_id"].as_str().map(str::to_owned),
                title: "工具结果".into(),
                state: if block["is_error"] == true {
                    ToolState::Failed
                } else {
                    ToolState::Completed
                },
                output: Some(if let Some(text) = block["content"].as_str() {
                    text.into()
                } else {
                    block["content"].to_string()
                }),
                conversation_id: agent.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// Anthropic counts input without the cache reads and writes; Velune's input
/// includes them. Claude Code copies thinking tokens from the response and
/// writes 0 when there were none to copy, so only more than 0 counts.
fn claude_usage(usage: &Value) -> TurnUsage {
    let read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
    let write = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
    TurnUsage {
        input_tokens: usage["input_tokens"]
            .as_u64()
            .map(|input| input + read + write),
        cache_read_tokens: Some(read),
        cache_write_tokens: Some(write),
        output_tokens: usage["output_tokens"].as_u64(),
        reasoning_tokens: usage["output_tokens_details"]["thinking_tokens"]
            .as_u64()
            .filter(|tokens| *tokens > 0),
        ..TurnUsage::default()
    }
}

/// What one reply's request held: its input, cache included, and its output.
fn claude_context(usage: &Value) -> Option<u64> {
    let read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
    let write = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
    Some(usage["input_tokens"].as_u64()? + read + write + usage["output_tokens"].as_u64()?)
}

/// A new random (version 4) UUID, the form Claude Code names sessions by.
fn claude_session_id() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| Error::new(format!("Claude 会话身份生成失败：{error}")))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}
