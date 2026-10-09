//! DeepSeek Harness 0.2.0-rc.2 ACP v1 control. huihua supplies historical projection.
use super::*;
use std::fs;

impl NativeSession {
    pub(super) fn dsh_command(config: &NativeConfig) -> Result<Command> {
        let api = match config.gateway.protocol.as_str() {
            "openai-completions" => "openai-completions",
            "openai-responses" => "openai-responses",
            "anthropic-messages" => "anthropic-messages",
            _ => return Err(Error::new("DeepSeek 运行时不支持此网关协议")),
        };
        fs::create_dir_all(&config.projection_directory)
            .map_err(|error| Error::new(format!("无法建立运行时网关注入目录：{error}")))?;
        let mut model = json!({"id":config.gateway.model_alias,"name":config.gateway.model_alias});
        if let Some(value) = config.gateway.context_window {
            model["contextWindow"] = json!(value);
        }
        // An explicit maxTokens is an upstream request default, not just catalog metadata.
        // Only pass the authoritative capability configured by the selected provider.
        if let Some(value) = config.gateway.max_output_tokens {
            model["maxTokens"] = json!(value);
        }
        let patch = json!([
            {"id":"llm-pi-ai","config":{"providers":{"velune-gateway":{
                "api":api,"baseURL":config.gateway.endpoint,"apiKeyEnv":"VELUNE_GATEWAY_TOKEN","models":[model]
            }}}},
            {"id":"acp","config":{"provider":"velune-gateway","model":config.gateway.model_alias}}
        ]);
        // JSON is valid YAML and avoids quoting upstream model IDs as YAML syntax.
        let path = config
            .projection_directory
            .join("deepseek-gateway.patch.yml");
        fs::write(
            &path,
            serde_json::to_vec(&patch)
                .map_err(|error| Error::new(format!("网关配置编码失败：{error}")))?,
        )
        .map_err(|error| Error::new(format!("网关配置写入失败：{error}")))?;
        let mut command = if let Some(node) = &config.node_binary {
            if !node.is_absolute() {
                return Err(Error::new("Node 可执行文件必须为绝对路径"));
            }
            let mut command = Command::new(node);
            command.arg(&config.binary);
            command
        } else {
            Command::new(&config.binary)
        };
        command.args(["--profile", "acp", "--patch"]).arg(path);
        Ok(command)
    }

    pub(super) fn dsh_initialize(&mut self) -> Result<()> {
        let result = self.rpc.request(
            "initialize",
            &json!({
                "protocolVersion":1,"clientInfo":{"name":"velune","version":"0.1.0-beta.1"},
                "clientCapabilities":{}
            }),
        )?;
        if result["protocolVersion"] != 1
            || result["agentCapabilities"]["sessionCapabilities"]
                .get("resume")
                .is_none()
        {
            return Err(Error::new("DeepSeek ACP 协议或恢复能力与适配器不匹配"));
        }
        // agentInfo.version is the ACP plugin version, not the CLI release gate.
        Ok(())
    }

    fn dsh_close_active(&mut self) -> Result<()> {
        if let Some(id) = &self.native_id {
            self.rpc
                .request("session/close", &json!({"sessionId":id}))?;
            self.native_id = None;
        }
        Ok(())
    }

    pub(super) fn dsh_create(&mut self, cwd: Option<&Path>, runtime_id: &str) -> Result<()> {
        let cwd = cwd
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.default_cwd.clone());
        self.validate_cwd(Some(&cwd))?;
        self.dsh_close_active()?;
        let result = self
            .rpc
            .request("session/new", &json!({"cwd":cwd,"mcpServers":[]}))?;
        let id = result["sessionId"]
            .as_str()
            .ok_or_else(|| Error::new("DeepSeek 未返回原生会话身份"))?;
        self.replace_snapshot(id, Some(&cwd), runtime_id, Vec::new());
        self.dsh_gateway_model(&result)
    }

    pub(super) fn dsh_open(
        &mut self,
        id: &str,
        cwd: Option<&Path>,
        runtime_id: &str,
        history: Vec<Message>,
    ) -> Result<()> {
        let cwd = cwd
            .map(Path::to_path_buf)
            .ok_or_else(|| Error::new("DeepSeek 原生会话缺少已保存的工作目录，无法恢复"))?;
        self.dsh_close_active()?;
        let result = self.rpc.request(
            "session/resume",
            &json!({"sessionId":id,"cwd":cwd,"mcpServers":[]}),
        )?;
        self.replace_snapshot(id, Some(&cwd), runtime_id, history);
        self.dsh_gateway_model(&result)
    }

    fn dsh_gateway_model(&mut self, result: &Value) -> Result<()> {
        let outcome = self.dsh_configure_gateway_model(result);
        if outcome.is_err() {
            let _ = self.dsh_close_active();
            self.native_id = None;
            self.settle(RunState::Failed);
        }
        outcome
    }

    fn dsh_configure_gateway_model(&mut self, result: &Value) -> Result<()> {
        self.available_models = result["configOptions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let option = self
            .available_models
            .iter()
            .find(|option| option["id"] == "model")
            .ok_or_else(|| Error::new("DeepSeek 未提供模型选择能力"))?;
        let values = option["options"]
            .as_array()
            .ok_or_else(|| Error::new("DeepSeek 模型目录无效"))?;
        let alias = &self.config.gateway.model_alias;
        // Decode upstream's advertised opaque value solely to identify our route.
        // Send that exact advertised string back, never construct a replacement value.
        let advertised = values
            .iter()
            .flat_map(|group| group["options"].as_array().into_iter().flatten())
            .filter_map(|value| value["value"].as_str())
            .find(|value| {
                serde_json::from_str::<Vec<String>>(value)
                    .is_ok_and(|pair| pair == ["velune-gateway", alias.as_str()])
            })
            .ok_or_else(|| Error::new("DeepSeek 未公布已注入的网关模型"))?
            .to_owned();
        let response = self.rpc.request(
            "session/set_config_option",
            &json!({
                "sessionId":self.native_id,"configId":"model","value":advertised
            }),
        )?;
        self.available_models = response["configOptions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(())
    }

    pub(super) fn dsh_send(&mut self, text: &str) -> Result<()> {
        let id = self.rpc.begin(
            "session/prompt",
            &json!({"sessionId":self.native_id,"prompt":[{"type":"text","text":text}]}),
        )?;
        self.pending_rpc.insert(id, "prompt".into());
        self.turn_id = Some(id.to_string());
        self.message(Message {
            completed: true,
            id: format!("{}:user:{id}", self.epoch),
            role: crate::conversation::MessageRole::User,
            timestamp_unix_ms: None,
            blocks: vec![MessageBlock::Text { text: text.into() }],
        });
        self.running();
        Ok(())
    }

    pub(super) fn dsh_cancel(&mut self) -> Result<()> {
        self.rpc
            .notify("session/cancel", &json!({"sessionId":self.native_id}))?;
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.run_state = RunState::Stopping;
            snapshot.revision += 1;
        }
        Ok(())
    }

    pub(super) fn dsh_select_model(&mut self, injection: GatewayInjection) -> Result<()> {
        if self.native_id.is_none() {
            self.config.gateway = injection;
            return Ok(());
        }
        let snapshot = self
            .snapshot
            .clone()
            .ok_or_else(|| Error::new("当前会话不存在"))?;
        let id = self
            .native_id
            .clone()
            .ok_or_else(|| Error::new("当前会话身份不存在"))?;
        let cwd = PathBuf::from(
            snapshot
                .conversation
                .cwd
                .as_ref()
                .ok_or_else(|| Error::new("原生会话缺少工作目录"))?,
        );
        self.dsh_close_active()?;
        self.rpc.shutdown();
        self.config.gateway = injection;
        self.rpc = Self::spawn_rpc(&self.config)?;
        self.dsh_initialize()?;
        // Resume restores native execution state; it does not replay the old transcript.
        let result = self.rpc.request(
            "session/resume",
            &json!({"sessionId":id,"cwd":cwd,"mcpServers":[]}),
        )?;
        self.native_id = Some(id);
        self.dsh_gateway_model(&result)?;
        Ok(())
    }

    pub(super) fn dsh_handle_record(&mut self, record: Value) -> Result<()> {
        if let Some(method) = record["method"].as_str() {
            let params = &record["params"];
            if method == "session/request_permission" {
                let request_id = record
                    .get("id")
                    .cloned()
                    .ok_or_else(|| Error::new("ACP 权限请求缺少身份"))?;
                if params["sessionId"].as_str() != self.native_id.as_deref() {
                    return self
                        .rpc
                        .respond(&request_id, &json!({"outcome":{"outcome":"cancelled"}}));
                }
                let options = params["options"]
                    .as_array()
                    .ok_or_else(|| Error::new("ACP 权限选项无效"))?;
                let options = options
                    .iter()
                    .map(|option| {
                        Ok(InteractionOption {
                            id: option["optionId"]
                                .as_str()
                                .ok_or_else(|| Error::new("ACP 权限选项缺少身份"))?
                                .into(),
                            label: option["name"].as_str().unwrap_or("权限选项").into(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                if options.is_empty() {
                    return Err(Error::new("ACP 权限请求没有可用选项"));
                }
                self.add_interaction(
                    PendingRequest {
                        id: request_id,
                        method: method.into(),
                        options: options.iter().map(|o| o.id.clone()).collect(),
                        question_ids: Vec::new(),
                        payload: params.clone(),
                    },
                    InteractionKind::Approval {
                        title: "运行时请求权限".into(),
                        detail: params["toolCall"]["title"]
                            .as_str()
                            .unwrap_or("请决定是否允许此工具操作")
                            .into(),
                        options,
                    },
                );
            } else if method == "session/update" {
                if params["sessionId"].as_str() == self.native_id.as_deref() {
                    self.dsh_update(&params["update"])?;
                }
            } else if let Some(id) = record.get("id") {
                self.rpc.reject(id, "Unsupported client capability")?;
            }
            return Ok(());
        }
        if let Some(id) = record["id"].as_u64()
            && self.pending_rpc.remove(&id).as_deref() == Some("prompt")
        {
            if record.get("error").is_some() {
                self.notice("DeepSeek 执行请求失败");
                self.settle(RunState::Failed);
            } else {
                self.settle(RunState::Idle);
            }
        }
        Ok(())
    }

    fn dsh_update(&mut self, update: &Value) -> Result<()> {
        match update["sessionUpdate"].as_str() {
            Some("agent_message_chunk" | "agent_thought_chunk") => {
                let Some(text) = update["content"]["text"].as_str() else {
                    self.notice("运行时返回了非文本内容");
                    return Ok(());
                };
                let native = update["messageId"].as_str().unwrap_or("assistant");
                let thought = update["sessionUpdate"] == "agent_thought_chunk";
                let id = format!(
                    "{}:{}:{native}:{}",
                    self.epoch,
                    self.turn_id.as_deref().unwrap_or("idle"),
                    if thought { "thought" } else { "message" }
                );
                let mut message = self
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.messages.iter().find(|m| m.id == id))
                    .cloned()
                    .unwrap_or(Message {
                        completed: false,
                        id,
                        role: crate::conversation::MessageRole::Assistant,
                        timestamp_unix_ms: None,
                        blocks: Vec::new(),
                    });
                if thought {
                    message
                        .blocks
                        .push(MessageBlock::Reasoning { text: text.into() });
                } else {
                    message
                        .blocks
                        .push(MessageBlock::Text { text: text.into() });
                }
                self.message(message);
            }
            Some("tool_call" | "tool_call_update") => {
                let native = update["toolCallId"]
                    .as_str()
                    .ok_or_else(|| Error::new("ACP 工具更新缺少身份"))?;
                let id = format!("{}:tool:{native}", self.epoch);
                let old = self
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.messages.iter().find(|m| m.id == id));
                let title = update["title"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| {
                        old.and_then(|m| {
                            m.blocks.iter().find_map(|b| {
                                if let MessageBlock::Tool { title, .. } = b {
                                    Some(title.clone())
                                } else {
                                    None
                                }
                            })
                        })
                    })
                    .unwrap_or_else(|| "工具操作".into());
                self.message(Message {
                    completed: true,
                    id,
                    role: crate::conversation::MessageRole::Tool,
                    timestamp_unix_ms: None,
                    blocks: vec![MessageBlock::Tool {
                        tool_id: Some(native.into()),
                        title,
                        state: match update["status"].as_str() {
                            Some("completed") => ToolState::Completed,
                            Some("failed") => ToolState::Failed,
                            Some("in_progress") => ToolState::Running,
                            Some("pending") => ToolState::Pending,
                            _ => old
                                .and_then(|m| m.blocks.first())
                                .and_then(|b| {
                                    if let MessageBlock::Tool { state, .. } = b {
                                        Some(state.clone())
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or(ToolState::Pending),
                        },
                        output: update["content"]
                            .as_array()
                            .map(|parts| {
                                parts
                                    .iter()
                                    .filter_map(|part| part["content"]["text"].as_str())
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .or_else(|| {
                                old.and_then(|m| m.blocks.first()).and_then(|b| {
                                    if let MessageBlock::Tool { output, .. } = b {
                                        output.clone()
                                    } else {
                                        None
                                    }
                                })
                            }),
                    }],
                });
            }
            Some("config_option_update") => {
                self.available_models = update["configOptions"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
            }
            Some("usage_update") => {} // Context occupancy isn't a transcript message.
            _ => {}                    // No guessing of unsupported ACP presentation payloads.
        }
        Ok(())
    }

    pub(super) fn dsh_reply(
        &mut self,
        pending: &PendingRequest,
        reply: RuntimeInteractionReply,
    ) -> Result<()> {
        if pending.method != "session/request_permission"
            || pending.payload["sessionId"].as_str() != self.native_id.as_deref()
        {
            return Err(Error::new("此权限请求不属于当前原生会话"));
        }
        let outcome = match reply {
            RuntimeInteractionReply::Cancel => json!({"outcome":"cancelled"}),
            RuntimeInteractionReply::Decision { option_id }
                if pending.options.contains(&option_id) =>
            {
                json!({"outcome":"selected","optionId":option_id})
            }
            _ => return Err(Error::new("此权限请求不接受该回复")),
        };
        self.rpc.respond(&pending.id, &json!({"outcome":outcome}))
    }
}
