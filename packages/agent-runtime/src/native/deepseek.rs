//! DeepSeek Harness 0.2.0-rc.2 ACP v1 control. huihua supplies historical projection.
use super::*;

impl NativeSession {
    pub(super) fn dsh_command(config: &NativeConfig) -> Result<Command> {
        let path = if let Some(gateway) = &config.gateway {
            let api = match gateway.protocol.as_str() {
                "openai-completions" => "openai-completions",
                "openai-responses" => "openai-responses",
                "anthropic-messages" => "anthropic-messages",
                _ => return Err(Error::new("DeepSeek 运行时不支持此网关协议")),
            };
            config
                .execution
                .create_dir_all(&config.projection_directory)?;
            let mut model = json!({"id":gateway.model_alias,"name":gateway.model_alias});
            if let Some(value) = gateway.context_window {
                model["contextWindow"] = json!(value);
            }
            // An explicit maxTokens is an upstream request default, not just catalog metadata.
            // Only pass the authoritative capability configured by the selected provider.
            if let Some(value) = gateway.max_output_tokens {
                model["maxTokens"] = json!(value);
            }
            let path = config
                .projection_directory
                .join("deepseek-gateway.patch.yml");
            let mut models = if config.execution.path_exists(&path)? {
                let previous: Value = serde_json::from_slice(&config.execution.read_file(&path)?)
                    .map_err(|error| {
                    Error::new(format!("应用的 DeepSeek 模型投影无效：{error}"))
                })?;
                previous[0]["config"]["providers"]["velune-gateway"]["models"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            models.retain(|old| old["id"] != model["id"]);
            models.push(model);
            // Historical request headers need their model metadata to resume; no credential is stored here.
            let patch = json!([
                {"id":"llm-pi-ai","config":{"providers":{"velune-gateway":{
                    "api":api,"baseURL":gateway.endpoint,"apiKeyEnv":"VELUNE_GATEWAY_TOKEN","models":models
                }}}}
            ]);
            config.execution.write_file_atomic(
                &path,
                &serde_json::to_vec(&patch)
                    .map_err(|error| Error::new(format!("网关配置编码失败：{error}")))?,
            )?;
            Some(path)
        } else {
            let path = config
                .projection_directory
                .join("deepseek-gateway.patch.yml");
            if config.execution.path_exists(&path)? {
                let mut metadata: Value = serde_json::from_slice(
                    &config.execution.read_file(&path)?,
                )
                .map_err(|error| Error::new(format!("应用的 DeepSeek 模型投影无效：{error}")))?;
                let provider = metadata[0]["config"]["providers"]["velune-gateway"]
                    .as_object_mut()
                    .ok_or_else(|| Error::new("应用的 DeepSeek 模型投影缺少提供商"))?;
                // Port zero cannot address a listening service; old gateway ports may have been reused.
                provider.insert("baseURL".into(), json!("http://127.0.0.1:0"));
                let path = config
                    .projection_directory
                    .join("deepseek-native-history.patch.yml");
                config.execution.write_file_atomic(
                    &path,
                    &serde_json::to_vec(&metadata)
                        .map_err(|error| Error::new(error.to_string()))?,
                )?;
                Some(path)
            } else {
                None
            }
        };
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
        command.args(["--profile", "acp"]);
        if let Some(path) = path {
            command.arg("--patch").arg(path);
        }
        Ok(command)
    }

    pub(super) fn dsh_initialize(&mut self) -> Result<()> {
        initialize(&mut self.rpc, !self.config.mcp_servers.is_empty())
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
        let result = self.rpc.request(
            "session/new",
            &json!({"cwd":cwd,"mcpServers":self.dsh_mcp_servers()}),
        )?;
        let id = result["sessionId"]
            .as_str()
            .ok_or_else(|| Error::new("DeepSeek 未返回原生会话身份"))?;
        self.replace_snapshot(id, Some(&cwd), runtime_id, Vec::new());
        self.dsh_configure_model(&result)
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
            &json!({"sessionId":id,"cwd":cwd,"mcpServers":self.dsh_mcp_servers()}),
        )?;
        self.replace_snapshot(id, Some(&cwd), runtime_id, history);
        self.dsh_configure_model(&result)
    }

    fn dsh_mcp_servers(&self) -> Vec<Value> {
        self.config
            .mcp_servers
            .iter()
            .map(crate::mcp::Server::acp_http)
            .collect()
    }

    fn dsh_configure_model(&mut self, result: &Value) -> Result<()> {
        let outcome = self.dsh_apply_model(result);
        if outcome.is_err() {
            let _ = self.dsh_close_active();
            self.native_id = None;
            self.settle(RunState::Failed);
        }
        outcome
    }

    fn dsh_apply_model(&mut self, result: &Value) -> Result<()> {
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
        if self.config.gateway.is_none()
            && self.native_selection.is_none()
            && let Some(current) = option["currentValue"].as_str()
            && let Ok(pair) = serde_json::from_str::<Vec<String>>(current)
            && pair.len() == 2
            && pair[0] != "velune-gateway"
        {
            self.native_selection = Some(RuntimeModel {
                provider_id: Some(pair[0].clone()),
                id: pair[1].clone(),
                name: pair[1].clone(),
            });
        }
        let (provider, alias) = if let Some(gateway) = &self.config.gateway {
            ("velune-gateway", gateway.model_alias.as_str())
        } else if let Some(model) = &self.native_selection {
            (
                model
                    .provider_id
                    .as_deref()
                    .expect("DeepSeek native selections include their provider identity"),
                model.id.as_str(),
            )
        } else {
            return Ok(());
        };
        // Decode upstream's advertised opaque value solely to identify our route.
        // Send that exact advertised string back, never construct a replacement value.
        let advertised = values
            .iter()
            .flat_map(|group| group["options"].as_array().into_iter().flatten())
            .filter_map(|value| value["value"].as_str())
            .find(|value| {
                serde_json::from_str::<Vec<String>>(value)
                    .is_ok_and(|pair| pair.len() == 2 && pair[0] == provider && pair[1] == alias)
            })
            .ok_or_else(|| Error::new(format!("DeepSeek 未公布所选模型（{provider}/{alias}）")))?
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
        if self.config.gateway.is_none()
            && self
                .native_selection
                .as_ref()
                .is_none_or(|model| model.provider_id.as_deref() == Some("velune-gateway"))
        {
            return Err(Error::new("请先选择运行时既有提供商的模型"));
        }
        let id = self.rpc.begin(
            "session/prompt",
            &json!({"sessionId":self.native_id,"prompt":[{"type":"text","text":text}]}),
        )?;
        self.pending_rpc.insert(id, "prompt".into());
        self.turn_id = Some(id.to_string());
        self.message(Message {
            turn_settings: None,
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

    pub(super) fn dsh_models(&mut self) -> Result<Vec<ModelOption>> {
        if self.native_id.is_none() {
            self.available_models = self.dsh_discover_models()?;
        }
        let models = self
            .available_models
            .iter()
            .find(|option| option["id"] == "model")
            .and_then(|option| option["options"].as_array());
        let mut result = Vec::new();
        for value in models
            .into_iter()
            .flatten()
            .flat_map(|group| group["options"].as_array().into_iter().flatten())
        {
            let Some(value_id) = value["value"].as_str() else {
                continue;
            };
            let Ok(pair) = serde_json::from_str::<Vec<String>>(value_id) else {
                continue;
            };
            if pair.len() == 2 && pair[0] != "velune-gateway" {
                result.push(ModelOption {
                    model: RuntimeModel {
                        provider_id: Some(pair[0].clone()),
                        id: pair[1].clone(),
                        name: value["name"].as_str().unwrap_or(&pair[1]).into(),
                    },
                    reasoning_efforts: Vec::new(),
                });
            }
        }
        Ok(result)
    }

    fn dsh_discover_models(&self) -> Result<Vec<Value>> {
        // ACP exposes the catalog only on a session. DSH flushes even an empty
        // session, so discovery must use a private persistence root, then remove it.
        let directory = self
            .config
            .projection_directory
            .join(format!("dsh-catalog-{}", self.epoch));
        self.config.execution.create_dir_all(&directory)?;
        let outcome = (|| {
            let patch = directory.join("catalog.patch.json");
            self.config.execution.write_file_atomic(
                &patch,
                &serde_json::to_vec(&json!([
                    {"id":"session-persistence-jsonl","config":{"root":directory.join("sessions")}},
                    {"id":"storage-json","config":{"root":directory.join("storages")}},
                    {"id":"session-query-sqlite","config":{"path":":memory:","openAt":"never"}}
                ]))
                .map_err(|error| Error::new(error.to_string()))?,
            )?;
            let mut config = self.config.clone();
            config.gateway = None;
            let mut command = Self::dsh_command(&config)?;
            command
                .arg("--patch")
                .arg(&patch)
                .env("DSH_HOME", &config.agent_dir)
                .env_remove("VELUNE_GATEWAY_TOKEN");
            let mut rpc =
                rpc::RpcClient::spawn_at(command, &config.execution, config.os_home.as_deref())?;
            let result = (|| {
                initialize(&mut rpc, false)?;
                let response = rpc.request(
                    "session/new",
                    &json!({"cwd":self.default_cwd,"mcpServers":[]}),
                )?;
                let id = response["sessionId"]
                    .as_str()
                    .ok_or_else(|| Error::new("DeepSeek 模型目录未返回临时会话身份"))?;
                rpc.request("session/close", &json!({"sessionId":id}))?;
                response["configOptions"]
                    .as_array()
                    .cloned()
                    .ok_or_else(|| Error::new("DeepSeek 模型目录格式无效"))
            })();
            rpc.finish();
            result
        })();
        let cleanup = self.config.execution.remove_dir_all(&directory);
        match (outcome, cleanup) {
            (Ok(models), Ok(())) => Ok(models),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error.into()),
            (Err(error), Err(cleanup)) => Err(Error::new(format!(
                "{error}；DeepSeek 临时模型目录清理失败：{cleanup}"
            ))),
        }
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
                self.finish_turn(RunState::Failed)?;
            } else {
                self.finish_turn(RunState::Idle)?;
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
                        turn_settings: None,
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
                    turn_settings: None,
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
                        conversation_id: None,
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

fn initialize(rpc: &mut rpc::RpcClient, require_http_mcp: bool) -> Result<()> {
    let result = rpc.request(
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
    if require_http_mcp && result["agentCapabilities"]["mcpCapabilities"]["http"] != true {
        return Err(Error::new("DeepSeek ACP 未公布 HTTP MCP 能力"));
    }
    // agentInfo.version is the ACP plugin version, not the CLI release gate.
    Ok(())
}
