//! Native runtime composition maps model ownership into a credential-blind gateway injection.
use super::*;
impl CoreRuntime {
    pub(super) fn native_injection(
        &self,
        runtime: &RuntimeInstance,
        key: &str,
    ) -> Result<GatewayInjection, RuntimeError> {
        let gateway = self
            .gateways
            .iter()
            .find(|g| g.id == runtime.gateway_id)
            .ok_or_else(|| RuntimeError::invalid("runtime gateway"))?;
        let provider = gateway
            .validate_dispatch(key)
            .map_err(RuntimeError::invalid)?;
        let model = gateway
            .model(key)
            .ok_or_else(|| RuntimeError::invalid("runtime model"))?;
        let supported = GatewayProtocol::runtime_protocols(&runtime.type_id)
            .ok_or_else(|| RuntimeError::invalid("不支持的运行时版本类型"))?;
        if !supported.contains(&provider.protocol) {
            return Err(RuntimeError::invalid("所选模型协议不适用于此运行时版本"));
        }
        let protocol = match &provider.protocol {
            GatewayProtocol::ChatCompletionsV1 => "openai-completions",
            GatewayProtocol::ResponsesV1 => "openai-responses",
            GatewayProtocol::MessagesV1 => "anthropic-messages",
        };
        let runner = self
            .gateway_runner
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid("runtime gateway"))?;
        Ok(GatewayInjection {
            endpoint: if provider.protocol == GatewayProtocol::MessagesV1 {
                runner
                    .endpoint()
                    .strip_suffix("/v1")
                    .expect("gateway base path")
                    .into()
            } else {
                runner.endpoint().into()
            },
            token: runner.token().into(),
            model_alias: model.provider_model_id.clone(),
            protocol: protocol.into(),
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            reasoning_levels: model.reasoning_levels.clone(),
        })
    }
    pub(super) fn native_aliases(
        &self,
        runtime: &RuntimeInstance,
        key: &str,
    ) -> Result<BTreeMap<String, String>, RuntimeError> {
        let gateway = self
            .gateways
            .iter()
            .find(|g| g.id == runtime.gateway_id)
            .ok_or_else(|| RuntimeError::invalid("runtime gateway"))?;
        let model = gateway
            .model(key)
            .ok_or_else(|| RuntimeError::invalid("runtime model"))?;
        Ok(BTreeMap::from([(
            model.provider_model_id.clone(),
            key.into(),
        )]))
    }
    pub(super) fn select_native_model(
        &mut self,
        runtime: &RuntimeInstance,
        key: &str,
    ) -> Result<(), RuntimeError> {
        let injection = self.native_injection(runtime, key)?;
        let previous = self
            .model_record_key
            .as_deref()
            .ok_or_else(|| RuntimeError::invalid("runtime model"))?;
        let previous_aliases = self.native_aliases(runtime, previous)?;
        let aliases = self.native_aliases(runtime, key)?;
        self.gateway_runner
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid("runtime gateway"))?
            .replace_aliases(aliases)
            .map_err(|_| RuntimeError::invalid("模型路由更新失败"))?;
        let result = match &mut self.active_state {
            ActiveState::Native(session) => session.select_model(injection),
            _ => return Err(RuntimeError::invalid("native runtime")),
        };
        if result.is_err() {
            if self
                .gateway_runner
                .as_ref()
                .expect("active gateway")
                .replace_aliases(previous_aliases)
                .is_err()
            {
                self.shutdown_active()?;
                return Err(RuntimeError::invalid("模型路由恢复失败，运行时已断开"));
            }
            return Err(RuntimeError::invalid(
                "模型选择失败；已保留当前会话与原模型选择",
            ));
        }
        Ok(())
    }
    pub(super) fn active_instance(&self) -> Result<&RuntimeInstance, RuntimeError> {
        let id = match &self.active_state {
            ActiveState::History(snapshot) => &snapshot.conversation.runtime_id,
            ActiveState::Native(_) | ActiveState::Pi => self
                .execution_runtime_id
                .as_ref()
                .ok_or_else(|| RuntimeError::invalid("active runtime"))?,
            ActiveState::Empty => return Err(RuntimeError::invalid("active runtime")),
        };
        self.runtime_instances
            .iter()
            .find(|r| &r.id == id)
            .ok_or_else(|| RuntimeError::invalid("active runtime"))
    }
    pub(super) fn history_config(
        &self,
        runtime: &RuntimeInstance,
    ) -> Result<HistoryConfig, RuntimeError> {
        let provider = match runtime.type_id.as_str() {
            "codex-0.159.3" => "codex",
            "dsh-acp-0.2.0-rc.2" => "deepseek",
            _ => return Err(RuntimeError::invalid("native history type")),
        };
        let home = setting_path(runtime, "agentDir")?;
        Ok(HistoryConfig {
            provider: provider.into(),
            node_binary: setting_path(runtime, "nodeBinary")?,
            resources_directory: self.options.resources_directory.clone(),
            root: home.join("sessions"),
            home,
        })
    }
    pub(super) fn native_create(&mut self, cwd: &Path, key: &str) -> Result<(), RuntimeError> {
        let runtime = self.active_instance()?.clone();
        if let ActiveState::Native(session) = &mut self.active_state {
            session
                .create(cwd, &runtime.id)
                .map_err(|_| RuntimeError::invalid("会话创建失败"))?;
        }
        if runtime.type_id == "dsh-acp-0.2.0-rc.2" {
            // ACP new flushes its native header before replying, but does not
            // return its date. Read the existing source adapter rather than
            // inventing a timestamp or reimplementing the persistence format.
            let id = self
                .current_snapshot()
                .expect("created native session")
                .conversation
                .id;
            let conversation = self
                .summaries_for(&runtime)?
                .into_iter()
                .find(|conversation| conversation.id == id)
                .ok_or_else(|| RuntimeError::invalid("新建会话的原生元数据不可用"))?;
            if let ActiveState::Native(session) = &mut self.active_state {
                session.set_conversation(conversation);
            }
        }
        self.model_record_key = Some(key.into());
        Ok(())
    }
    pub(super) fn native_resume(
        &mut self,
        snapshot: &crate::conversation::ConversationSnapshot,
        key: &str,
    ) -> Result<(), RuntimeError> {
        let runtime = self.active_instance()?.clone();
        let native_id = snapshot
            .conversation
            .id
            .strip_prefix(&format!("{}:", runtime.id))
            .filter(|id| !id.is_empty())
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        let cwd = snapshot
            .conversation
            .cwd
            .as_deref()
            .map(PathBuf::from)
            .ok_or_else(|| RuntimeError::invalid("会话工作目录不可用"))?;
        if let ActiveState::Native(session) = &mut self.active_state {
            session
                .open(native_id, &cwd, &runtime.id, snapshot.messages.clone())
                .map_err(|_| RuntimeError::invalid("会话恢复失败"))?;
            session.set_conversation(snapshot.conversation.clone());
        }
        self.model_record_key = Some(key.into());
        Ok(())
    }
    pub(super) fn reply_runtime_interaction(
        &mut self,
        request: &Value,
    ) -> Result<Value, RuntimeError> {
        self.ensure_active(request)?;
        let id = request["payload"]["interactionID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("interaction id"))?;
        let reply = serde_json::from_value(request["payload"]["reply"].clone())?;
        match &mut self.active_state {
            ActiveState::Native(session) => session.reply(id, reply).map_err(|_| {
                RuntimeError::invalid("运行时请求回复失败；请求可能已取消或所属会话已改变")
            })?,
            _ => return Err(RuntimeError::invalid("此运行时没有待回复的请求")),
        };
        self.drain_runtime()?;
        Ok(json!({"snapshot":self.current_snapshot()}))
    }
}
