//! Codex app-server 0.159.3 control and projection, independent of Pi.
use super::*;
impl NativeSession {
    pub(super) fn codex_initialize(&mut self) -> Result<()> {
        if self
            .config
            .gateway
            .as_ref()
            .is_some_and(|g| g.protocol != "openai-responses")
        {
            return Err(Error::new("此 Codex 适配器仅支持原生 Responses 模型"));
        }
        self.rpc.request(
            "initialize",
            &json!({
                "clientInfo": {"name":"velune", "title":"Velune", "version":"0.1.0-beta.1"},
                "capabilities": {"experimentalApi":true}
            }),
        )?;
        self.rpc.notify("initialized", &json!({}))?;
        Ok(())
    }
    fn codex_capture_provider(&mut self, cwd: Option<&Path>) -> Result<()> {
        let source = self
            .rpc
            .request("config/read", &json!({"includeLayers":false,"cwd":cwd}))?;
        // Velune's services are optional; one named like the user's own is left out.
        for server in &self.config.mcp_servers {
            if source["config"]["mcp_servers"].get(&server.name).is_some() {
                tracing::warn!(server = %server.name, "Codex already has an MCP service of this name; running without Velune's");
                self.shadowed_mcp_servers.insert(server.name.clone());
            }
        }
        self.native_provider = Some(
            source["config"]["model_provider"]
                .as_str()
                .unwrap_or("openai")
                .into(),
        );
        if self.native_selection.is_none()
            && let Some(model) = source["config"]["model"].as_str()
        {
            self.native_selection = Some(RuntimeModel {
                provider_id: self.native_provider.clone(),
                id: model.into(),
                name: model.into(),
            });
        }
        Ok(())
    }
    /// Where a gateway turn's agent role lives: the execution lane's own
    /// projection, never the Harness's agents directory.
    fn codex_role_path(config: &NativeConfig, name: &str) -> PathBuf {
        config
            .projection_directory
            .join("agents")
            .join(format!("{name}.toml"))
    }

    /// Writes the role file of each per-turn sub-agent on the executing
    /// machine; `codex_options` names them in the thread's config.
    pub(super) fn codex_write_roles(config: &NativeConfig) -> Result<()> {
        for agent in &config.session_agents {
            crate::management::validate_name(&agent.name)
                .map_err(|error| Error::new(error.to_string()))?;
            let path = Self::codex_role_path(config, &agent.name);
            if let Some(parent) = path.parent() {
                config.execution.create_dir_all(parent)?;
            }
            let effort = agent
                .effort
                .as_deref()
                .filter(|level| crate::management::CODEX_EFFORTS.contains(level));
            config.execution.write_file_atomic(
                &path,
                &crate::management::codex_agent(
                    &agent.name,
                    &agent.description,
                    &agent.instructions,
                    &agent.model,
                    effort,
                ),
            )?;
        }
        Ok(())
    }

    fn codex_options(&self, cwd: Option<&Path>) -> Value {
        let mut options = json!({"approvalPolicy":"on-request","sandbox":"workspace-write"});
        if let Some(gateway) = &self.config.gateway {
            options["model"] = json!(gateway.model_alias);
            options["modelProvider"] = json!("velune");
            options["config"] = json!({
                "model_provider":"velune",
                "model_providers":{"velune":{"name":"Velune LLM Gateway","base_url":self.tunnel.as_ref().map(|t| t.endpoint()).unwrap_or(&gateway.endpoint),"env_key":"VELUNE_GATEWAY_TOKEN","wire_api":"responses","requires_openai_auth":false}},
                "features":{"responses_websockets":false,"responses_websockets_v2":false}
            });
            if !self.config.session_agents.is_empty() {
                options["config"]["agents"] = Value::Object(
                    self.config
                        .session_agents
                        .iter()
                        .map(|agent| {
                            (
                                agent.name.clone(),
                                json!({
                                    "description": agent.description,
                                    "config_file": Self::codex_role_path(&self.config, &agent.name),
                                }),
                            )
                        })
                        .collect(),
                );
            }
        } else {
            let provider = self
                .native_selection
                .as_ref()
                .and_then(|m| m.provider_id.as_ref())
                .or(self.native_provider.as_ref());
            options["modelProvider"] = json!(provider);
            options["config"] = json!({"model_provider":provider});
            if let Some(model) = &self.native_selection {
                options["model"] = json!(model.id);
            }
        }
        if let Some(cwd) = cwd {
            options["cwd"] = json!(cwd);
        }
        if !self.config.mcp_servers.is_empty() {
            options["config"]["mcp_servers"] = Value::Object(
                self.config
                    .mcp_servers
                    .iter()
                    .enumerate()
                    .filter(|(_, server)| !self.shadowed_mcp_servers.contains(&server.name))
                    .map(|(index, server)| {
                        (
                            server.name.clone(),
                            json!({"url":server.endpoint,"bearer_token_env_var":crate::mcp::token_environment(index)}),
                        )
                    })
                    .collect(),
            );
        }
        options
    }
    pub(super) fn codex_create(&mut self, cwd: Option<&Path>, runtime_id: &str) -> Result<()> {
        self.codex_capture_provider(cwd)?;
        let result = self.rpc.request("thread/start", &self.codex_options(cwd))?;
        if self.native_selection.is_none()
            && self.config.gateway.is_none()
            && let Some(model) = result["model"].as_str()
        {
            self.native_selection = Some(RuntimeModel {
                provider_id: self.native_provider.clone(),
                id: model.into(),
                name: model.into(),
            });
        }
        let id = result["thread"]["id"]
            .as_str()
            .ok_or_else(|| Error::new("Codex 未返回会话身份"))?;
        let resolved_cwd = result["thread"]["cwd"]
            .as_str()
            .map(PathBuf::from)
            .or_else(|| cwd.map(Path::to_path_buf));
        self.replace_snapshot(id, resolved_cwd.as_deref(), runtime_id, Vec::new());
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
        cwd: Option<&Path>,
        runtime_id: &str,
        history: Vec<Message>,
    ) -> Result<()> {
        self.codex_capture_provider(cwd)?;
        let mut options = self.codex_options(cwd);
        options["threadId"] = json!(id);
        options["excludeTurns"] = json!(false);
        let result = self.rpc.request("thread/resume", &options)?;
        if result["thread"]["id"].as_str() != Some(id) {
            return Err(Error::new("Codex 恢复了不同的会话"));
        }
        if self.native_selection.is_none()
            && self.config.gateway.is_none()
            && let Some(model) = result["model"].as_str()
        {
            self.native_selection = Some(RuntimeModel {
                provider_id: self.native_provider.clone(),
                id: model.into(),
                name: model.into(),
            });
        }
        let resolved_cwd = result["thread"]["cwd"]
            .as_str()
            .map(PathBuf::from)
            .or_else(|| cwd.map(Path::to_path_buf));
        self.replace_snapshot(id, resolved_cwd.as_deref(), runtime_id, history);
        if let Some(turns) = result["thread"]["turns"].as_array() {
            // A resumed native snapshot owns current history, including compaction and rollback.
            if let Some(snapshot) = &mut self.snapshot {
                snapshot.messages.clear();
            }
            // What each turn ran with and used, as history reads it.
            let records = match (&self.config.execution, result["thread"]["path"].as_str()) {
                (crate::execution::ExecutionTarget::Local, Some(path)) => {
                    super::codex_history::rollout_turns(Path::new(path))
                }
                _ => Default::default(),
            };
            for turn in turns {
                let start = self.snapshot.as_ref().map_or(0, |s| s.messages.len());
                if let Some(items) = turn["items"].as_array() {
                    for item in items {
                        self.codex_item(item, turn["id"].as_str());
                    }
                }
                if let Some(record) = turn["id"].as_str().and_then(|id| records.get(id))
                    && let Some(snapshot) = &mut self.snapshot
                {
                    let start = start.min(snapshot.messages.len());
                    for message in snapshot.messages[start..]
                        .iter_mut()
                        .filter(|message| message.role == MessageRole::User)
                    {
                        message.turn_settings = Some(record.settings.clone());
                        message.turn_usage = record.usage.clone();
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn codex_send(&mut self, input: &crate::TurnInput) -> Result<()> {
        let id = self
            .native_id
            .as_deref()
            .ok_or_else(|| Error::new("Codex 没有活动会话"))?;
        let items = codex_input(input);
        // A permission mode is thread state: Codex keeps a turn's override for
        // later turns, so a turn without a mode sends none and keeps the
        // thread's current mode, as Claude Code does.
        let policy = match input.permission_mode.as_deref() {
            None => None,
            Some("read-only") => Some(("on-request", json!({"type":"readOnly"}), "user")),
            Some("workspace") => Some(("on-request", json!({"type":"workspaceWrite"}), "user")),
            Some("auto-review") => Some((
                "on-request",
                json!({"type":"workspaceWrite"}),
                "auto_review",
            )),
            Some("full-access") => Some(("never", json!({"type":"dangerFullAccess"}), "user")),
            Some(_) => return Err(Error::new("Codex 不接受此权限模式")),
        };
        let mut params = json!({
            "threadId": id,
            "model": self.config.gateway.as_ref().map(|g| g.model_alias.as_str()).or_else(|| self.native_selection.as_ref().map(|m| m.id.as_str())),
            "effort": input.effort,
            "input": items
        });
        if let Some((approval, sandbox, reviewer)) = policy {
            params["approvalPolicy"] = json!(approval);
            params["sandboxPolicy"] = sandbox;
            params["approvalsReviewer"] = json!(reviewer);
        }
        let result = self.rpc.request("turn/start", &params)?;
        let turn_id = result["turn"]["id"]
            .as_str()
            .ok_or_else(|| Error::new("Codex 未返回轮次身份"))?;
        self.turn_id = Some(turn_id.into());
        self.running();
        Ok(())
    }
    /// `turn/steer` names the turn it expects to be running; Codex refuses
    /// the message when that turn has already ended.
    pub(super) fn codex_steer(&mut self, input: &crate::TurnInput) -> Result<()> {
        let (Some(thread), Some(turn)) = (self.native_id.as_deref(), self.turn_id.as_deref())
        else {
            return Err(Error::new("Codex 没有正在运行的轮次"));
        };
        let params =
            json!({"threadId": thread, "expectedTurnId": turn, "input": codex_input(input)});
        self.rpc.request("turn/steer", &params)?;
        Ok(())
    }
    /// In place, `thread/revert` drops the message's turn and all later ones
    /// from the loaded thread. A fork copies the thread through the turn
    /// before it with `thread/fork` and resumes the copy here.
    pub(super) fn codex_branch(&mut self, index: usize, placement: EditPlacement) -> Result<()> {
        let snapshot = self.snapshot.as_ref().expect("branched snapshot");
        let thread = self
            .native_id
            .clone()
            .ok_or_else(|| Error::new("Codex 没有活动会话"))?;
        let turn = snapshot.messages[index]
            .branch_point
            .clone()
            .ok_or_else(|| Error::new("这条消息没有对应的 Codex 轮次"))?;
        match placement {
            EditPlacement::InPlace => {
                self.rpc.request(
                    "thread/revert",
                    &json!({"threadId": thread, "beforeTurnId": turn}),
                )?;
                self.truncate_snapshot(index);
            }
            EditPlacement::Fork => {
                let previous = snapshot.messages[..index]
                    .iter()
                    .rev()
                    .filter(|message| message.role == MessageRole::User)
                    .find_map(|message| message.branch_point.clone().filter(|point| *point != turn))
                    .ok_or_else(|| Error::new("第一条消息没有可复制的上文"))?;
                let cwd = snapshot.conversation.cwd.as_deref().map(PathBuf::from);
                let runtime_id = snapshot.conversation.runtime_id.clone();
                let mut options = self.codex_options(cwd.as_deref());
                options["threadId"] = json!(thread);
                options["lastTurnId"] = json!(previous);
                options["excludeTurns"] = json!(true);
                let result = self.rpc.request("thread/fork", &options)?;
                let fork = result["thread"]["id"]
                    .as_str()
                    .ok_or_else(|| Error::new("Codex 未返回 fork 会话身份"))?
                    .to_owned();
                self.native_id = None;
                self.codex_open(&fork, cwd.as_deref(), &runtime_id, Vec::new())?;
            }
        }
        Ok(())
    }
    /// The configured model first, then the catalog. Codex 0.159.3 lists each
    /// model's `supportedReasoningEfforts`.
    pub(super) fn codex_models(&mut self) -> Result<Vec<ModelOption>> {
        let cwd = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.conversation.cwd.as_deref());
        let source = self
            .rpc
            .request("config/read", &json!({"includeLayers":false,"cwd":cwd}))?;
        let configured = source["config"]["model"].as_str().map(str::to_owned);
        let provider = source["config"]["model_provider"]
            .as_str()
            .unwrap_or("openai")
            .to_owned();
        let mut catalog = Vec::new();
        let mut cursor = Value::Null;
        let mut seen = std::collections::BTreeSet::new();
        loop {
            let page = self.rpc.request(
                "model/list",
                &json!({"cursor":cursor,"limit":100,"includeHidden":false}),
            )?;
            for model in page["data"]
                .as_array()
                .ok_or_else(|| Error::new("Codex 模型目录格式无效"))?
            {
                let id = model["model"]
                    .as_str()
                    .ok_or_else(|| Error::new("Codex 模型缺少身份"))?;
                let efforts = model["supportedReasoningEfforts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|effort| effort["reasoningEffort"].as_str().map(str::to_owned))
                    .collect();
                catalog.push(ModelOption {
                    model: RuntimeModel {
                        provider_id: None,
                        id: id.into(),
                        name: model["displayName"].as_str().unwrap_or(id).into(),
                    },
                    reasoning_efforts: efforts,
                });
            }
            let Some(next) = page["nextCursor"].as_str() else {
                break;
            };
            if !seen.insert(next.to_owned()) {
                return Err(Error::new("Codex 模型分页游标重复"));
            }
            cursor = json!(next);
        }
        let Some(configured) = configured else {
            return Ok(catalog);
        };
        let efforts = catalog
            .iter()
            .find(|option| option.model.id == configured)
            .map(|option| option.reasoning_efforts.clone())
            .unwrap_or_default();
        catalog.retain(|option| option.model.id != configured);
        catalog.insert(
            0,
            ModelOption {
                model: RuntimeModel {
                    provider_id: Some(provider),
                    id: configured.clone(),
                    name: configured,
                },
                reasoning_efforts: efforts,
            },
        );
        Ok(catalog)
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
    fn codex_item(&mut self, item: &Value, turn: Option<&str>) {
        let Some(runtime_id) = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.conversation.runtime_id.clone())
        else {
            return;
        };
        if let Some(mut message) = project_item(item, &runtime_id) {
            if message.role == MessageRole::User {
                message.branch_point = turn.map(str::to_owned);
            }
            self.message(message);
        }
    }
    pub(super) fn codex_handle_record(&mut self, record: Value) -> Result<()> {
        let method = record["method"].as_str().unwrap_or_default();
        let params = &record["params"];
        if let Some(thread) = params["threadId"].as_str()
            && Some(thread) != self.native_id.as_deref()
        {
            // A native sub-agent's thread runs in this app-server; its requests
            // are answered in this conversation, its own items stay in its own.
            let runtime_id = self
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.conversation.runtime_id.clone())
                .unwrap_or_default();
            self.codex_track_thread(thread, method, params);
            let sub_agent = self.codex_threads.contains_key(thread)
                || self.sub_agents.contains(&format!("{runtime_id}:{thread}"));
            match record.get("id") {
                Some(_) if sub_agent && record.get("method").is_some() => {
                    return self.codex_request(&record, true);
                }
                Some(id) => self.rpc.reject(id, "Inactive conversation")?,
                None if sub_agent && params["item"]["type"] == "collabAgentToolCall" => {
                    let nested = params["item"]["receiverThreadIds"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(|nested| format!("{runtime_id}:{nested}"));
                    self.sub_agents.extend(nested);
                }
                None => {}
            }
            return Ok(());
        }
        if record.get("id").is_some() && record.get("method").is_some() {
            return self.codex_request(&record, false);
        }
        match method {
            "item/started" | "item/completed" => {
                self.codex_item(&params["item"], params["turnId"].as_str());
                if let Some(message) = self.snapshot.as_mut().and_then(|snapshot| {
                    snapshot
                        .messages
                        .iter_mut()
                        .find(|message| Some(message.id.as_str()) == params["item"]["id"].as_str())
                }) {
                    message.completed = method == "item/completed";
                }
            }
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
                        turn_settings: None,
                        turn_usage: None,
                        branch_point: None,
                        completed: false,
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
                    self.finish_turn(if status == Some("failed") {
                        RunState::Failed
                    } else {
                        RunState::Idle
                    })?;
                    if status == Some("failed") {
                        self.notice("Codex 执行失败；请通过诊断日志关联操作，或检查上游配置。");
                    }
                }
            }
            // Usage of each model response in the turn; the reread rollout has the turn's total.
            "thread/tokenUsage/updated" => {
                if params["turnId"].as_str() == self.turn_id.as_deref() {
                    use super::codex_history::{APP_SERVER_KEYS, codex_usage, context_tokens};
                    let last = &params["tokenUsage"]["last"];
                    let mut usage = codex_usage(last, APP_SERVER_KEYS);
                    usage.context_tokens = context_tokens(last, APP_SERVER_KEYS);
                    usage.context_window = params["tokenUsage"]["modelContextWindow"].as_u64();
                    self.turn_usage(|total| match total {
                        Some(total) => total.add(&usage),
                        None => *total = Some(usage),
                    });
                }
            }
            // The account's limits follow each response's usage; Velune's gateway has none.
            "account/rateLimits/updated" if self.turn_id.is_some() => {
                let limits = super::codex_history::rate_limits(
                    &params["rateLimits"],
                    "usedPercent",
                    "windowDurationMins",
                    "resetsAt",
                );
                if !limits.is_empty() {
                    self.turn_usage(|total| {
                        if let Some(total) = total {
                            total.rate_limits = limits;
                        }
                    });
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
    /// Follows another thread of this app-server: a native sub-agent the
    /// conversation started, or one of theirs, works while its status is
    /// active. Tracking every thread, not only those the spawning item
    /// names, does not depend on which of their events arrives first.
    fn codex_track_thread(&mut self, thread: &str, method: &str, params: &Value) {
        if !self.codex_ran_threads.contains(thread) {
            self.codex_ran_threads.insert(thread.into());
        }
        match method {
            "thread/status/changed" => {
                let status = &params["status"];
                if status["type"] != "active" {
                    self.codex_threads.remove(thread);
                    return;
                }
                let flags = status["activeFlags"].as_array();
                let flagged = |flag: &str| flags.is_some_and(|f| f.iter().any(|v| v == flag));
                let entry = self.codex_threads.entry(thread.into()).or_default();
                if entry.state != BackgroundState::Stopping {
                    entry.state = if flagged("waitingOnApproval") {
                        BackgroundState::AwaitingApproval
                    } else if flagged("waitingOnUserInput") {
                        BackgroundState::AwaitingInput
                    } else {
                        BackgroundState::Running
                    };
                }
            }
            "turn/started" => {
                self.codex_threads.entry(thread.into()).or_default().turn =
                    params["turn"]["id"].as_str().map(str::to_owned);
            }
            "turn/completed" => {
                if let Some(entry) = self.codex_threads.get_mut(thread) {
                    entry.turn = None;
                }
            }
            _ => {}
        }
    }
    /// Codex keeps the provider a thread last ran with, so the threads a
    /// gateway turn's app-server ran, the conversation's and its sub-agents',
    /// would look for the gateway when resumed later. Once that process has
    /// ended, a short-lived app-server resumes each on the runtime's own
    /// provider: one that already has a thread loaded keeps its provider.
    pub(super) fn codex_restore_provider(&mut self) -> Result<()> {
        let threads = self
            .native_id
            .clone()
            .into_iter()
            .chain(std::mem::take(&mut self.codex_ran_threads))
            .collect::<Vec<_>>();
        let provider = self
            .native_selection
            .as_ref()
            .and_then(|model| model.provider_id.clone())
            .or_else(|| self.native_provider.clone())
            .ok_or_else(|| Error::new("Codex 原生提供商未知，未能恢复会话的提供商"))?;
        let mut rpc = history_client_at(
            &self.config.binary,
            &self.config.agent_dir,
            &self.config.execution,
            self.config.os_home.as_deref(),
        )?;
        let result = threads.iter().try_for_each(|thread| {
            let mut options = json!({"threadId":thread,"excludeTurns":true,"modelProvider":provider,
                "config":{"model_provider":provider},"approvalPolicy":"on-request","sandbox":"workspace-write"});
            if let Some(model) = &self.native_selection {
                options["model"] = json!(model.id);
            }
            rpc.request("thread/resume", &options).map(drop)
        });
        rpc.shutdown();
        result
    }
    /// Interrupts each sub-agent thread's turn; interrupting the
    /// conversation's own turn does not stop them. They end when their
    /// status turns idle.
    pub(super) fn codex_stop_background(&mut self) -> Result<()> {
        let turns = self
            .codex_threads
            .iter()
            .filter(|(_, entry)| entry.state != BackgroundState::Stopping)
            .filter_map(|(thread, entry)| Some((thread.clone(), entry.turn.clone()?)))
            .collect::<Vec<_>>();
        for (thread, turn) in turns {
            self.rpc
                .request("turn/interrupt", &json!({"threadId":thread,"turnId":turn}))?;
            if let Some(entry) = self.codex_threads.get_mut(&thread) {
                entry.state = BackgroundState::Stopping;
            }
        }
        Ok(())
    }
    /// `sub_agent` marks a request from a native sub-agent's thread.
    fn codex_request(&mut self, record: &Value, sub_agent: bool) -> Result<()> {
        let method = record["method"].as_str().unwrap_or_default();
        let params = &record["params"];
        let id = record["id"].clone();
        // A sub-agent may outlive the turn that started it; its requests
        // still come to this conversation.
        if !self.turn_running() && !sub_agent {
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
                let action = if method.contains("commandExecution") {
                    "允许执行命令？"
                } else {
                    "允许修改文件？"
                };
                let title = if sub_agent {
                    format!("子 Agent：{action}")
                } else {
                    action.to_owned()
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
                        background: sub_agent,
                        id,
                        method: method.into(),
                        options: options.iter().map(|o| o.id.clone()).collect(),
                        question_ids: Vec::new(),
                        payload: Value::Null,
                    },
                    InteractionKind::Approval {
                        title,
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
                        background: sub_agent,
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

/// A change to a native thread's metadata.
#[derive(Debug, Clone, Copy)]
pub enum ThreadChange<'a> {
    Rename(&'a str),
    Delete,
    Archive,
    Unarchive,
}

/// Native thread metadata management without model selection or gateway startup.
pub fn manage_thread(binary: &Path, home: &Path, id: &str, change: ThreadChange) -> Result<()> {
    manage_thread_at(
        binary,
        home,
        id,
        change,
        &crate::execution::ExecutionTarget::Local,
        None,
    )
}
pub fn manage_thread_at(
    binary: &Path,
    home: &Path,
    id: &str,
    change: ThreadChange,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> Result<()> {
    let mut rpc = history_client_at(binary, home, execution, os_home)?;
    let result = match change {
        ThreadChange::Rename(name) => {
            rpc.request("thread/name/set", &json!({"threadId":id,"name":name}))
        }
        ThreadChange::Delete => rpc.request("thread/delete", &json!({"threadId":id})),
        ThreadChange::Archive => rpc.request("thread/archive", &json!({"threadId":id})),
        ThreadChange::Unarchive => rpc.request("thread/unarchive", &json!({"threadId":id})),
    };
    rpc.shutdown();
    result.map(|_| ())
}
pub(super) fn history_client_at(
    binary: &Path,
    home: &Path,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> Result<rpc::RpcClient> {
    if !binary.is_absolute() || !home.is_absolute() {
        return Err(Error::new("Codex 历史配置必须使用绝对路径"));
    }
    crate::version::check_version_at("codex-0.159", binary, None, execution, os_home)?;
    let mut command = Command::new(binary);
    command.arg("app-server");
    command.env("CODEX_HOME", home);
    let mut rpc = rpc::RpcClient::spawn_at(command, execution, os_home)?;
    rpc.request("initialize", &json!({"clientInfo":{"name":"velune","version":"0.1.0-beta.1"},"capabilities":{"experimentalApi":true}}))?;
    rpc.notify("initialized", &json!({}))?;
    Ok(rpc)
}

pub(super) fn project_item(item: &Value, runtime_id: &str) -> Option<Message> {
    let id = item["id"].as_str()?;
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
                    Some("localImage") => part["path"].as_str().map(|path| MessageBlock::Image {
                        media_type: None,
                        source: crate::conversation::ImageSource::Path { path: path.into() },
                    }),
                    Some("image") => Some(
                        part["url"]
                            .as_str()
                            .and_then(crate::conversation::ImageSource::from_data_url)
                            .map_or_else(
                                || MessageBlock::Notice {
                                    text: "图片输入".into(),
                                },
                                |(media_type, source)| MessageBlock::Image { media_type, source },
                            ),
                    ),
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
                return None;
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
                conversation_id: None,
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
                conversation_id: None,
            }],
        ),
        "mcpToolCall" | "dynamicToolCall" => (
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
                output: item["aggregatedOutput"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| mcp_output(item)),
                conversation_id: None,
            }],
        ),
        // A native sub-agent call names the sub-agent threads it started or
        // addressed; one thread is the call's conversation.
        "collabAgentToolCall" => {
            let receivers = item["receiverThreadIds"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default();
            let states = receivers.iter().filter_map(|thread| {
                let state = &item["agentsStates"][thread.as_str()?];
                let status = state["status"].as_str()?;
                Some(match state["message"].as_str() {
                    Some(message) => format!("{status}: {message}"),
                    None => status.to_owned(),
                })
            });
            let output = item["prompt"]
                .as_str()
                .map(str::to_owned)
                .into_iter()
                .chain(states)
                .collect::<Vec<_>>()
                .join("\n");
            (
                MessageRole::Tool,
                vec![MessageBlock::Tool {
                    tool_id: Some(id.into()),
                    title: item["tool"].as_str().unwrap_or("sub-agent").into(),
                    state: match item["status"].as_str() {
                        Some("completed") => ToolState::Completed,
                        Some("failed") => ToolState::Failed,
                        Some("inProgress") => ToolState::Running,
                        _ => ToolState::Pending,
                    },
                    output: (!output.is_empty()).then_some(output),
                    conversation_id: match receivers {
                        [thread] => thread
                            .as_str()
                            .map(|thread| format!("{runtime_id}:{thread}")),
                        _ => None,
                    },
                }],
            )
        }
        "plan" => (
            MessageRole::System,
            vec![MessageBlock::Notice {
                text: item["text"].as_str().unwrap_or_default().into(),
            }],
        ),
        _ => return None,
    };
    Some(Message {
        turn_settings: None,
        turn_usage: None,
        branch_point: None,
        completed: true,
        id: id.into(),
        role: kind,
        timestamp_unix_ms: None,
        blocks,
    })
}

/// An MCP call's result as text: its structured content when the server gave
/// one, else its text content, or the error message.
fn mcp_output(item: &Value) -> Option<String> {
    let result = &item["result"];
    if !result["structuredContent"].is_null() {
        return Some(result["structuredContent"].to_string());
    }
    let text = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|content| content["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty())
        .then_some(text)
        .or_else(|| item["error"]["message"].as_str().map(str::to_owned))
}

/// A user message as Codex input items; Codex reads attached images from
/// their paths itself.
fn codex_input(input: &crate::TurnInput) -> Vec<Value> {
    (!input.text.is_empty())
        .then(|| json!({"type":"text", "text":input.text}))
        .into_iter()
        .chain(
            input
                .images
                .iter()
                .map(|image| json!({"type":"localImage", "path":image.path})),
        )
        .collect()
}

/// Another thread of the conversation's app-server while it works.
pub(super) struct CodexThread {
    pub(super) state: BackgroundState,
    /// Its running turn, which stopping it interrupts.
    turn: Option<String>,
}
impl Default for CodexThread {
    fn default() -> Self {
        Self {
            state: BackgroundState::Running,
            turn: None,
        }
    }
}
