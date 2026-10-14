//! Native runtime composition maps model ownership into a credential-blind gateway injection.
use super::*;
use velune_agent_runtime::native::SessionAgent;
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
        let resolved = gateway
            .resolve_dispatch(key)
            .map_err(RuntimeError::invalid)?;
        let provider = resolved.provider();
        let model = resolved.model();
        let ingress = GatewayProtocol::runtime_ingress(&runtime.type_id, &provider.protocol)
            .ok_or_else(|| RuntimeError::invalid("不支持的运行时版本类型"))?;
        let protocol = match &ingress {
            GatewayProtocol::ChatCompletionsV1 => "openai-completions",
            GatewayProtocol::ResponsesV1 => "openai-responses",
            GatewayProtocol::MessagesV1 => "anthropic-messages",
        };
        let runner = self
            .session
            .gateway_runner
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid("runtime gateway"))?;
        let endpoint = self
            .session
            .gateway_tunnel
            .as_ref()
            .map_or(runner.endpoint(), |tunnel| tunnel.endpoint());
        Ok(GatewayInjection {
            endpoint: if ingress == GatewayProtocol::MessagesV1 {
                endpoint
                    .strip_suffix("/v1")
                    .expect("gateway base path")
                    .into()
            } else {
                endpoint.into()
            },
            token: runner.token().into(),
            model_alias: model.provider_model_id.clone(),
            protocol: protocol.into(),
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            reasoning_levels: model.reasoning_levels.clone(),
        })
    }
    /// The turn's model under its provider model ID, and each sub-agent
    /// definition enabled on the runtime that names a native model ID, under
    /// that ID, when exactly one Velune model has it. The turn's own model
    /// keeps its name.
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
            .resolve_dispatch(key)
            .map_err(RuntimeError::invalid)?
            .model();
        let mut aliases = BTreeMap::from([(model.provider_model_id.clone(), key.into())]);
        for template in self.enabled_subagents(runtime) {
            let Some(crate::SubagentModel::Native(id)) = template.content.subagent_model() else {
                continue;
            };
            let mut matching = gateway
                .providers
                .iter()
                .flat_map(|provider| &provider.models)
                .filter(|model| model.provider_model_id == id);
            if let (Some(model), None) = (matching.next(), matching.next()) {
                aliases
                    .entry(id.to_owned())
                    .or_insert_with(|| model.record_key.clone());
            }
        }
        Ok(aliases)
    }

    /// The library's sub-agent definitions enabled on a runtime.
    fn enabled_subagents(&self, runtime: &RuntimeInstance) -> Vec<&crate::ResourceTemplate> {
        self.resource_bindings
            .iter()
            .filter(|binding| binding.runtime_id == runtime.id)
            .filter_map(|binding| {
                self.resource_templates
                    .iter()
                    .find(|template| template.id == binding.template_id)
            })
            .filter(|template| template.content.subagent_model().is_some())
            .collect()
    }

    /// The enabled sub-agents on a Velune model, supplied to a turn through
    /// the runtime's gateway, which routes every Velune model by record key.
    pub(super) fn turn_subagents(&self, runtime: &RuntimeInstance) -> Vec<SessionAgent> {
        let Some(gateway) = self.gateways.iter().find(|g| g.id == runtime.gateway_id) else {
            return Vec::new();
        };
        self.enabled_subagents(runtime)
            .into_iter()
            .filter_map(|template| {
                let crate::ResourceTemplateContent::Subagent {
                    description,
                    instructions,
                    velune_model: Some(key),
                    tools,
                    effort,
                    skills,
                    ..
                } = &template.content
                else {
                    return None;
                };
                // A model removed since, or kept by another gateway, leaves the agent out.
                gateway
                    .providers
                    .iter()
                    .flat_map(|provider| &provider.models)
                    .any(|model| &model.record_key == key)
                    .then(|| SessionAgent {
                        name: template.name.clone(),
                        description: description.clone(),
                        instructions: instructions.clone(),
                        model: Some(format!("velune/model/{key}")),
                        effort: effort.clone(),
                        tools: tools.clone(),
                        skills: skills.clone(),
                    })
            })
            .collect()
    }
    pub(super) fn active_instance(&self) -> Result<&RuntimeInstance, RuntimeError> {
        let id = match &self.session.active_state {
            ActiveState::History(snapshot) => &snapshot.conversation.runtime_id,
            ActiveState::Native(_) | ActiveState::Acp(_) | ActiveState::Pi => self
                .session
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
            "claude-code-2.1.286" => "claude-code",
            _ => return Err(RuntimeError::invalid("native history type")),
        };
        let context = self.prepare_execution_context(runtime)?;
        let home = context.agent_dir;
        Ok(HistoryConfig {
            provider: provider.into(),
            binary: setting_path(runtime, "binary")?,
            node_binary: setting_path_optional(runtime, "nodeBinary").unwrap_or_default(),
            resources_directory: context.resources,
            execution: context.target,
            os_home: context.os_home,
            root: home.join(if provider == "claude-code" {
                "projects"
            } else {
                "sessions"
            }),
            home,
        })
    }
    pub(super) fn native_create(
        &mut self,
        cwd: Option<&Path>,
        key: &str,
    ) -> Result<(), RuntimeError> {
        let runtime = self.active_instance()?.clone();
        if let ActiveState::Native(session) = &mut self.session.active_state {
            session
                .create(cwd, &runtime.id)
                .map_err(|error| RuntimeError::context("会话创建失败", error))?;
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
            if let ActiveState::Native(session) = &mut self.session.active_state {
                session.set_conversation(conversation);
            }
        }
        self.session.model_record_key = Some(key.into());
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
        let cwd = snapshot.conversation.cwd.as_deref().map(PathBuf::from);
        if let ActiveState::Native(session) = &mut self.session.active_state {
            session
                .open(
                    native_id,
                    cwd.as_deref(),
                    &runtime.id,
                    snapshot.messages.clone(),
                )
                .map_err(|error| RuntimeError::context("会话恢复失败", error))?;
            session.set_conversation(snapshot.conversation.clone());
        }
        self.session.model_record_key = Some(key.into());
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
        match &mut self.session.active_state {
            ActiveState::Acp(session) => session
                .reply(id, reply)
                .map_err(|error| RuntimeError::context("ACP 请求回复失败", error))?,
            ActiveState::Native(session) => session.reply(id, reply).map_err(|error| {
                RuntimeError::context("运行时请求回复失败；请求可能已取消或所属会话已改变", error)
            })?,
            _ => return Err(RuntimeError::invalid("此运行时没有待回复的请求")),
        };
        self.drain_runtime()?;
        self.snapshot_result()
    }
}
