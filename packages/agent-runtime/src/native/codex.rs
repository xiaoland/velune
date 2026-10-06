//! Codex app-server 0.159.3 control and projection, independent of Pi.
use super::*;
impl NativeSession {
    pub(super) fn codex_initialize(&mut self) -> Result<()> {
        if self.config.gateway.protocol != "openai-responses" {
            return Err(Error::new("此 Codex 适配器仅支持原生 Responses 模型"));
        }
        self.rpc.request(
            "initialize",
            &json!({
                "clientInfo": {"name":"velune", "title":"Velune", "version":"0.1.0-beta.1"},
                "capabilities": {"experimentalApi":true}
            }),
        )?;
        self.rpc.notify("initialized", &json!({}))
    }
    fn codex_options(&self, cwd: &Path) -> Value {
        json!({
            "model": self.config.gateway.model_alias,
            "modelProvider": "velune",
            "cwd": cwd,
            "approvalPolicy": "on-request",
            "sandbox": "workspace-write",
            "config": {
                "model_provider": "velune",
                "model_providers": {"velune": {
                    "name": "Velune LLM Gateway",
                    "base_url": self.config.gateway.endpoint,
                    "env_key": "VELUNE_GATEWAY_TOKEN",
                    "wire_api": "responses",
                    "requires_openai_auth": false
                }},
                "features": {"responses_websockets":false, "responses_websockets_v2":false}
            }
        })
    }
    pub(super) fn codex_create(&mut self, cwd: &Path, runtime_id: &str) -> Result<()> {
        let result = self.rpc.request("thread/start", &self.codex_options(cwd))?;
        let id = result["thread"]["id"]
            .as_str()
            .ok_or_else(|| Error::new("Codex 未返回会话身份"))?;
        self.replace_snapshot(id, cwd, runtime_id, Vec::new());
        if let Some(snapshot) = self.snapshot.as_mut() {
            // app-server Thread dates are Unix seconds, including before the
            // first rollout is persisted. Preserve that native creation date.
            snapshot.conversation.created_at_unix_ms = result["thread"]["createdAt"]
                .as_i64()
                .and_then(|seconds| seconds.checked_mul(1000));
            snapshot.conversation.updated_at_unix_ms = result["thread"]["updatedAt"]
                .as_i64()
                .and_then(|seconds| seconds.checked_mul(1000));
        }
        Ok(())
    }
    pub(super) fn codex_open(
        &mut self,
        id: &str,
        cwd: &Path,
        runtime_id: &str,
        history: Vec<Message>,
    ) -> Result<()> {
        let mut options = self.codex_options(cwd);
        options["threadId"] = json!(id);
        options["excludeTurns"] = json!(false);
        let result = self.rpc.request("thread/resume", &options)?;
        if result["thread"]["id"].as_str() != Some(id) {
            return Err(Error::new("Codex 恢复了不同的会话"));
        }
        self.replace_snapshot(id, cwd, runtime_id, history);
        if let Some(turns) = result["thread"]["turns"].as_array() {
            // A resumed native snapshot owns current history, including compaction and rollback.
            if let Some(snapshot) = &mut self.snapshot {
                snapshot.messages.clear();
            }
            for turn in turns {
                if let Some(items) = turn["items"].as_array() {
                    for item in items {
                        self.codex_item(item);
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn codex_send(&mut self, text: &str) -> Result<()> {
        let id = self
            .native_id
            .as_deref()
            .ok_or_else(|| Error::new("Codex 没有活动会话"))?;
        let result = self.rpc.request(
            "turn/start",
            &json!({
                "threadId": id,
                "model": self.config.gateway.model_alias,
                "input": [{"type":"text", "text":text}]
            }),
        )?;
        let turn_id = result["turn"]["id"]
            .as_str()
            .ok_or_else(|| Error::new("Codex 未返回轮次身份"))?;
        self.turn_id = Some(turn_id.into());
        self.running();
        Ok(())
    }
    pub(super) fn codex_cancel(&mut self) -> Result<()> {
        let thread = self
            .native_id
            .as_deref()
            .ok_or_else(|| Error::new("Codex 没有活动会话"))?;
        let turn = self
            .turn_id
            .as_deref()
            .ok_or_else(|| Error::new("Codex 没有可取消轮次"))?;
        self.rpc
            .request("turn/interrupt", &json!({"threadId":thread,"turnId":turn}))?;
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.run_state = RunState::Stopping;
            snapshot.revision += 1;
        }
        Ok(())
    }
    fn codex_item(&mut self, item: &Value) {
        let Some(id) = item["id"].as_str() else {
            return;
        };
        let (kind, blocks) = match item["type"].as_str().unwrap_or_default() {
            "userMessage" => {
                let blocks = item["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|part| match part["type"].as_str() {
                        Some("text") => part["text"]
                            .as_str()
                            .map(|t| MessageBlock::Text { text: t.into() }),
                        Some("image" | "localImage") => Some(MessageBlock::Notice {
                            text: "图片输入".into(),
                        }),
                        _ => None,
                    })
                    .collect();
                (MessageRole::User, blocks)
            }
            "agentMessage" => (
                MessageRole::Assistant,
                vec![MessageBlock::Text {
                    text: item["text"].as_str().unwrap_or_default().into(),
                }],
            ),
            "reasoning" => {
                let text = item["summary"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.is_empty() {
                    return;
                }
                (
                    MessageRole::Assistant,
                    vec![MessageBlock::Reasoning { text }],
                )
            }
            "commandExecution" => (
                MessageRole::Tool,
                vec![MessageBlock::Tool {
                    tool_id: Some(id.into()),
                    title: item["command"].as_str().unwrap_or("命令执行").into(),
                    state: match item["status"].as_str() {
                        Some("completed") => ToolState::Completed,
                        Some("failed" | "declined") => ToolState::Failed,
                        Some("inProgress") => ToolState::Running,
                        _ => ToolState::Pending,
                    },
                    output: item["aggregatedOutput"].as_str().map(str::to_owned),
                }],
            ),
            "fileChange" => (
                MessageRole::Tool,
                vec![MessageBlock::Tool {
                    tool_id: Some(id.into()),
                    title: "文件修改".into(),
                    state: match item["status"].as_str() {
                        Some("completed") => ToolState::Completed,
                        Some("failed" | "declined") => ToolState::Failed,
                        Some("inProgress") => ToolState::Running,
                        _ => ToolState::Pending,
                    },
                    output: item["aggregatedOutput"].as_str().map(str::to_owned),
                }],
            ),
            "mcpToolCall" | "dynamicToolCall" | "collabAgentToolCall" => (
                MessageRole::Tool,
                vec![MessageBlock::Tool {
                    tool_id: Some(id.into()),
                    title: item["tool"].as_str().unwrap_or("工具调用").into(),
                    state: match item["status"].as_str() {
                        Some("completed") => ToolState::Completed,
                        Some("failed" | "declined") => ToolState::Failed,
                        Some("inProgress") => ToolState::Running,
                        _ => ToolState::Pending,
                    },
                    output: item["aggregatedOutput"].as_str().map(str::to_owned),
                }],
            ),
            "plan" => (
                MessageRole::System,
                vec![MessageBlock::Notice {
                    text: item["text"].as_str().unwrap_or_default().into(),
                }],
            ),
            _ => return,
        };
        self.message(Message {
            id: id.into(),
            role: kind,
            timestamp_unix_ms: None,
            blocks,
        });
    }
    pub(super) fn codex_handle_record(&mut self, record: Value) -> Result<()> {
        let method = record["method"].as_str().unwrap_or_default();
        let params = &record["params"];
        if let Some(thread) = params["threadId"].as_str()
            && Some(thread) != self.native_id.as_deref()
        {
            if let Some(id) = record.get("id") {
                self.rpc.reject(id, "Inactive conversation")?;
            }
            return Ok(());
        }
        if record.get("id").is_some() && record.get("method").is_some() {
            return self.codex_request(&record);
        }
        match method {
            "item/started" | "item/completed" => self.codex_item(&params["item"]),
            "item/agentMessage/delta" => {
                if let (Some(id), Some(delta)) =
                    (params["itemId"].as_str(), params["delta"].as_str())
                {
                    let old = self
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.messages.iter().find(|m| m.id == id))
                        .and_then(|m| m.blocks.first())
                        .and_then(|b| {
                            if let MessageBlock::Text { text } = b {
                                Some(text.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();
                    self.message(Message {
                        id: id.into(),
                        role: crate::conversation::MessageRole::Assistant,
                        timestamp_unix_ms: None,
                        blocks: vec![MessageBlock::Text { text: old + delta }],
                    });
                }
            }
            "turn/completed" => {
                if params["turn"]["id"].as_str() == self.turn_id.as_deref() {
                    let status = params["turn"]["status"].as_str();
                    self.settle(if status == Some("failed") {
                        RunState::Failed
                    } else {
                        RunState::Idle
                    });
                    if status == Some("failed") {
                        self.notice("Codex 执行失败；请通过诊断日志关联操作，或检查上游配置。");
                    }
                }
            }
            "serverRequest/resolved" => {
                let id = &params["requestId"];
                let expired = self
                    .interactions
                    .iter()
                    .filter(|(_, p)| &p.id == id)
                    .map(|(k, _)| k.clone())
                    .collect::<Vec<_>>();
                for key in expired {
                    self.interactions.remove(&key);
                    if let Some(s) = &mut self.snapshot {
                        s.pending_interactions.retain(|i| i.id != key);
                        s.revision += 1;
                    }
                }
            }
            "error" => self.notice("Codex 返回执行诊断；原始错误内容不写入日志。"),
            _ => {}
        }
        Ok(())
    }
    fn codex_request(&mut self, record: &Value) -> Result<()> {
        let method = record["method"].as_str().unwrap_or_default();
        let params = &record["params"];
        let id = record["id"].clone();
        if !self.busy() {
            self.rpc.reject(&id, "No active turn")?;
            return Ok(());
        }
        match method {
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                let allowed = [
                    ("accept", "允许这一次"),
                    ("decline", "拒绝"),
                    ("cancel", "取消"),
                ];
                let options = allowed
                    .iter()
                    .filter(|(choice, _)| {
                        params["availableDecisions"]
                            .as_array()
                            .is_none_or(|a| a.iter().any(|v| v.as_str() == Some(choice)))
                    })
                    .map(|(choice, label)| InteractionOption {
                        id: (*choice).into(),
                        label: (*label).into(),
                    })
                    .collect::<Vec<_>>();
                if options.is_empty() {
                    self.rpc.reject(&id, "Unsupported approval decisions")?;
                    self.notice("此 Codex 审批方式尚不受支持。");
                    return Ok(());
                }
                let title = if method.contains("commandExecution") {
                    "允许执行命令？"
                } else {
                    "允许修改文件？"
                };
                let detail = [
                    params["command"].as_str(),
                    params["reason"].as_str(),
                    params["cwd"].as_str(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n");
                self.add_interaction(
                    PendingRequest {
                        id,
                        method: method.into(),
                        options: options.iter().map(|o| o.id.clone()).collect(),
                        question_ids: Vec::new(),
                        payload: Value::Null,
                    },
                    InteractionKind::Approval {
                        title: title.into(),
                        detail,
                        options,
                    },
                );
            }
            "item/tool/requestUserInput" => {
                let questions = params["questions"]
                    .as_array()
                    .ok_or_else(|| Error::new("Codex 用户输入请求缺少问题"))?
                    .iter()
                    .map(|q| {
                        let options = q["options"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|o| {
                                o["label"].as_str().map(|label| InteractionOption {
                                    id: label.into(),
                                    label: label.into(),
                                })
                            })
                            .collect();
                        InteractionQuestion {
                            id: q["id"].as_str().unwrap_or_default().into(),
                            text: q["question"].as_str().unwrap_or_default().into(),
                            options,
                            secret: q["isSecret"].as_bool().unwrap_or(false),
                        }
                    })
                    .collect::<Vec<_>>();
                if questions.is_empty() || questions.iter().any(|q| q.id.is_empty()) {
                    self.rpc.reject(&id, "Invalid questions")?;
                    return Ok(());
                }
                self.add_interaction(
                    PendingRequest {
                        id,
                        method: method.into(),
                        options: Vec::new(),
                        question_ids: questions.iter().map(|q| q.id.clone()).collect(),
                        payload: Value::Null,
                    },
                    InteractionKind::UserInput { questions },
                );
            }
            _ => {
                self.rpc
                    .reject(&id, "This interaction is not supported by Velune")?;
                self.notice("运行时请求了一种尚未支持的交互，已明确拒绝。");
            }
        }
        Ok(())
    }
    pub(super) fn codex_reply(
        &mut self,
        pending: &PendingRequest,
        reply: RuntimeInteractionReply,
    ) -> Result<()> {
        let result = if pending.method == "item/tool/requestUserInput" {
            match reply {
                RuntimeInteractionReply::Answers { answers } => {
                    let mut map = serde_json::Map::new();
                    for answer in answers {
                        if !pending.question_ids.contains(&answer.question_id)
                            || map.contains_key(&answer.question_id)
                        {
                            return Err(Error::new("用户输入答案与当前问题不匹配"));
                        }
                        map.insert(answer.question_id, json!({"answers":answer.values}));
                    }
                    if map.len() != pending.question_ids.len() {
                        return Err(Error::new("请回答所有问题"));
                    }
                    json!({"answers":map})
                }
                RuntimeInteractionReply::Cancel => json!({"answers":{}}),
                _ => return Err(Error::new("此请求需要用户输入答案")),
            }
        } else {
            let decision = match reply {
                RuntimeInteractionReply::Decision { option_id } => {
                    if !pending.options.contains(&option_id) {
                        return Err(Error::new("审批选项已经失效"));
                    }
                    option_id
                }
                RuntimeInteractionReply::Cancel => pending
                    .options
                    .iter()
                    .find(|choice| choice.as_str() == "cancel")
                    .or_else(|| {
                        pending
                            .options
                            .iter()
                            .find(|choice| choice.as_str() == "decline")
                    })
                    .cloned()
                    .ok_or_else(|| Error::new("此审批请求没有可拒绝的选项"))?,
                _ => return Err(Error::new("此请求需要审批决定")),
            };
            json!({"decision":decision})
        };
        self.rpc.respond(&pending.id, &result)
    }
}

/// Native thread metadata management without model selection or gateway startup.
pub fn manage_thread(binary: &Path, home: &Path, id: &str, title: Option<&str>) -> Result<()> {
    crate::version::check_version("codex-0.159.3", binary, None)?;
    let mut command = Command::new(binary);
    command.arg("app-server");
    let mut rpc = rpc::RpcClient::spawn(
        command,
        &[("CODEX_HOME".into(), home.to_string_lossy().into_owned())],
    )?;
    rpc.request("initialize", &json!({"clientInfo":{"name":"velune","version":"0.1.0-beta.1"},"capabilities":{"experimentalApi":true}}))?;
    rpc.notify("initialized", &json!({}))?;
    let result = match title {
        Some(name) => rpc.request("thread/name/set", &json!({"threadId":id,"name":name})),
        None => rpc.request("thread/delete", &json!({"threadId":id})),
    };
    rpc.shutdown();
    result.map(|_| ())
}
