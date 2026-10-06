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
            GatewayProtocol::MessagesV1 => {
                return Err(RuntimeError::invalid("原生运行时协议适配器未实现"));
            }
        };
        let runner = self
            .gateway_runner
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid("runtime gateway"))?;
        Ok(GatewayInjection {
            endpoint: runner.endpoint().into(),
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
            .or(runtime.model_record_key.as_deref())
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
                "模型选择失败；更改协议需要重新连接运行时",
            ));
        }
        Ok(())
    }
    pub(super) fn active_instance(&self) -> Result<&RuntimeInstance, RuntimeError> {
        self.runtime_instances
            .iter()
            .find(|r| Some(&r.id) == self.active_runtime_id.as_ref())
            .ok_or_else(|| RuntimeError::invalid("active runtime"))
    }
    pub(super) fn history_config(&self) -> Result<HistoryConfig, RuntimeError> {
        let runtime = self.active_instance()?;
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
    pub(super) fn native_create(&mut self, cwd: &Path) -> Result<(), RuntimeError> {
        validate_session_cwd(cwd)?;
        let runtime = self.active_instance()?.clone();
        let key = runtime
            .model_record_key
            .clone()
            .ok_or_else(|| RuntimeError::invalid("请选择初始模型"))?;
        self.select_native_model(&runtime, &key)?;
        if let ActiveState::Native(session) = &mut self.active_state {
            session
                .create(cwd, &runtime.id)
                .map_err(|_| RuntimeError::invalid("会话创建失败"))?;
        }
        self.model_record_key = Some(key);
        Ok(())
    }
    pub(super) fn native_open(&mut self, id: &str) -> Result<(), RuntimeError> {
        let runtime = self.active_instance()?.clone();
        let native_id = id
            .strip_prefix(&format!("{}:", runtime.id))
            .filter(|id| !id.is_empty())
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        let history = history::read(&self.history_config()?, native_id)
            .map_err(|_| RuntimeError::invalid("会话历史读取失败"))?;
        let cwd = history
            .cwd
            .as_deref()
            .map(PathBuf::from)
            .ok_or_else(|| RuntimeError::invalid("会话工作目录不可用"))?;
        validate_session_cwd(&cwd)?;
        let key = runtime
            .model_record_key
            .clone()
            .ok_or_else(|| RuntimeError::invalid("请选择初始模型"))?;
        self.select_native_model(&runtime, &key)?;
        if let ActiveState::Native(session) = &mut self.active_state {
            session
                .open(&history.native_id, &cwd, &runtime.id, history.messages)
                .map_err(|_| RuntimeError::invalid("会话恢复失败"))?;
            session.notice("已恢复历史会话；继续使用此运行时配置的初始模型，可在工具栏重新选择。");
        }
        self.model_record_key = Some(key);
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
