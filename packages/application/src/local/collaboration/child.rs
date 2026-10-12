//! A worker-owned execution. It never opens an Application or its repository.
use super::super::*;
use crate::{CollaborationTarget, ModelSelection};
use velune_conversation::{ConversationSnapshot, ConversationTitle, RuntimeInteractionReply};
use velune_gateway::CredentialResolver;

#[derive(Clone)]
pub(super) enum Blueprint {
    Pi(PiConfig),
    Native(NativeConfig),
    Acp(velune_agent_runtime::acp::Config),
}

pub(super) struct CapturedTarget {
    pub target: CollaborationTarget,
    pub blueprint: Blueprint,
    pub gateway: Option<GatewayConfig>,
    pub resolver: Option<Arc<dyn CredentialResolver>>,
    pub analytics: Arc<crate::analytics::AnalyticsStore>,
    pub projection: PathBuf,
}

enum Driver {
    Pi {
        client: Box<pi::Client>,
        projection: Box<PiProjection>,
        busy: bool,
        started: bool,
    },
    Native(Box<NativeSession>),
    Acp(Box<velune_agent_runtime::acp::Client>),
}

pub(super) struct ChildRuntime {
    id: String,
    captured: Arc<CapturedTarget>,
    blueprint: Blueprint,
    driver: Option<Driver>,
    snapshot: Option<ConversationSnapshot>,
    runner: Option<Runner>,
    tunnel: Option<velune_agent_runtime::execution::LoopbackTunnel>,
}

impl CoreRuntime {
    pub(super) fn capture_child(
        &self,
        target: CollaborationTarget,
    ) -> Result<CapturedTarget, RuntimeError> {
        let runtime = self.runtime_instance(&target.runtime_id)?;
        if !runtime.enabled {
            return Err(RuntimeError::invalid("委派目标运行时已停用"));
        }
        self.validate_runtime_cwd(runtime, Path::new(&target.cwd))?;
        self.validate_model_selection(runtime, &target.model_selection)?;
        let (blueprint, projection) = if runtime.type_id == "acp-v1" {
            let (execution, os_home) = self.prepare_runtime_location(runtime)?;
            let launch = runtime.acp_launch()?;
            (
                Blueprint::Acp(velune_agent_runtime::acp::Config {
                    binary: setting_path(runtime, "binary")?,
                    args: launch.args,
                    environment: launch.environment,
                    execution,
                    os_home,
                    mcp_servers: Vec::new(),
                }),
                PathBuf::new(),
            )
        } else {
            let context = self.prepare_execution_context(runtime)?;
            if runtime.type_id == "pi-1.0.2" {
                let mut config = self.pi_history_config(runtime)?;
                config.extension = Some(context.resources.join("pi_virtual_model.mjs"));
                config.rpc_entry = Some(context.resources.join("pi_rpc.mjs"));
                config.working_dir = Some(target.cwd.clone().into());
                (Blueprint::Pi(config), context.projection)
            } else {
                (
                    Blueprint::Native(self.native_config(runtime, None)?),
                    context.projection,
                )
            }
        };
        let gateway = matches!(target.model_selection, ModelSelection::Managed { .. })
            .then(|| {
                self.gateways
                    .iter()
                    .find(|item| item.id == runtime.gateway_id)
                    .cloned()
                    .ok_or_else(|| RuntimeError::invalid("委派目标网关不可用"))
            })
            .transpose()?;
        let resolver = gateway.as_ref().map(|gateway| {
            crate::authentication_resolver::ProviderResolver::capture(gateway, &self.options)
        });
        Ok(CapturedTarget {
            target,
            blueprint,
            gateway,
            resolver,
            analytics: self.analytics.clone(),
            projection,
        })
    }
}

impl ChildRuntime {
    pub(super) fn new(captured: Arc<CapturedTarget>, id: &str) -> Self {
        let mut blueprint = captured.blueprint.clone();
        let projection = captured.projection.join("children").join(id);
        match &mut blueprint {
            Blueprint::Pi(config) if captured.gateway.is_some() => {
                config.models_path = Some(projection.join("models.json"));
                config.selection_file = Some(projection.join("velune-selection.json"));
            }
            Blueprint::Native(config) => config.projection_directory = projection,
            _ => {}
        }
        Self {
            id: id.into(),
            captured,
            blueprint,
            driver: None,
            snapshot: None,
            runner: None,
            tunnel: None,
        }
    }

    fn prepare_gateway(&mut self) -> Result<Option<GatewayInjection>, RuntimeError> {
        let Some(gateway) = &self.captured.gateway else {
            return Ok(None);
        };
        let ModelSelection::Managed {
            model_record_key: key,
        } = &self.captured.target.model_selection
        else {
            unreachable!()
        };
        let resolved = gateway
            .resolve_dispatch(key)
            .map_err(RuntimeError::invalid)?;
        let provider = resolved.provider();
        let model = resolved.model();
        let pi = matches!(self.blueprint, Blueprint::Pi(_));
        let aliases = if pi {
            gateway
                .providers
                .iter()
                .flat_map(|provider| &provider.models)
                .map(|model| {
                    gateway
                        .pi_binding_id(
                            &model.record_key,
                            gateway.authentication_revision(&model.record_key),
                        )
                        .map(|alias| (alias, model.record_key.clone()))
                        .map_err(RuntimeError::invalid)
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?
        } else {
            BTreeMap::from([(model.provider_model_id.clone(), key.clone())])
        };
        let runner = Runner::start_with_analytics(
            gateway.to_gateway_config()?,
            self.captured
                .resolver
                .as_ref()
                .expect("captured resolver")
                .clone(),
            aliases,
            Some(self.captured.analytics.clone()),
        )
        .map_err(|error| RuntimeError::context("子任务网关启动", error))?;
        let type_id = match &self.blueprint {
            Blueprint::Pi(config) => &config.runtime_type_id,
            Blueprint::Native(config) => match config.kind {
                NativeKind::Codex => "codex-0.159.3",
                NativeKind::ClaudeCode => "claude-code-2.1.286",
                NativeKind::DeepSeek => "dsh-acp-0.2.0-rc.2",
            },
            Blueprint::Acp(_) => unreachable!(),
        };
        let ingress = GatewayProtocol::runtime_ingress(type_id, &provider.protocol)
            .ok_or_else(|| RuntimeError::invalid("子任务网关协议不可用"))?;
        let protocol = match ingress {
            GatewayProtocol::ChatCompletionsV1 => "openai-completions",
            GatewayProtocol::ResponsesV1 => "openai-responses",
            GatewayProtocol::MessagesV1 => "anthropic-messages",
        };
        let injection = GatewayInjection {
            endpoint: if ingress == GatewayProtocol::MessagesV1 {
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
        };
        if let Blueprint::Pi(config) = &mut self.blueprint {
            let tunnel = velune_agent_runtime::execution::LoopbackTunnel::open(
                &config.execution,
                runner.endpoint(),
            )
            .map_err(|e| RuntimeError::context("子任务网关转发", e))?;
            config.gateway_token = Some(runner.token().into());
            config.provider = Some("velune".into());
            config.model = Some("auto".into());
            materialize_models(config, gateway, tunnel.endpoint(), protocol)?;
            let physical = gateway
                .pi_binding_id(key, gateway.authentication_revision(key))
                .map_err(RuntimeError::invalid)?;
            write_selection_file(config, key, &physical, gateway.subscription(key))?;
            self.tunnel = Some(tunnel);
        }
        self.runner = Some(runner);
        Ok(Some(injection))
    }

    fn prepare(&mut self) -> Result<(), RuntimeError> {
        let injection = self.prepare_gateway()?;
        if let Some(Driver::Native(session)) = &mut self.driver {
            if let Some(injection) = injection {
                session
                    .select_model(injection)
                    .map_err(|e| RuntimeError::context("子任务模型选择", e))?;
            }
            return Ok(());
        }
        if self.driver.is_some() {
            return Ok(());
        }
        let target = &self.captured.target;
        let driver = match &mut self.blueprint {
            Blueprint::Acp(config) => {
                let mut client = velune_agent_runtime::acp::Client::connect(config.clone())
                    .map_err(|e| RuntimeError::context("子任务 ACP 初始化", e))?;
                client
                    .create(Path::new(&target.cwd), &target.runtime_id)
                    .map_err(|e| RuntimeError::context("子任务 ACP 创建", e))?;
                if let ModelSelection::Native { model_id, .. } = &target.model_selection {
                    client
                        .select_model(model_id)
                        .map_err(|e| RuntimeError::context("子任务 ACP 模型", e))?;
                }
                Driver::Acp(Box::new(client))
            }
            Blueprint::Native(config) => {
                config.gateway = injection;
                let mut session = NativeSession::connect(config.clone())
                    .map_err(|e| RuntimeError::context("子任务运行时初始化", e))?;
                if let ModelSelection::Native {
                    provider_id,
                    model_id,
                } = &target.model_selection
                {
                    session
                        .select_native_model(provider_id.clone(), model_id.clone())
                        .map_err(|e| RuntimeError::context("子任务原生模型", e))?;
                }
                session
                    .create(Some(Path::new(&target.cwd)), &target.runtime_id)
                    .map_err(|e| RuntimeError::context("子任务会话创建", e))?;
                Driver::Native(Box::new(session))
            }
            Blueprint::Pi(config) => {
                if let ModelSelection::Native {
                    provider_id,
                    model_id,
                } = &target.model_selection
                {
                    config.provider = provider_id.clone();
                    config.model = Some(model_id.clone());
                }
                let mut client = pi::Client::spawn(config.clone())
                    .map_err(|e| RuntimeError::context("子任务 Pi 启动", e))?;
                client
                    .state()
                    .map_err(|e| RuntimeError::context("子任务 Pi 就绪", e))?;
                if config.session.is_none() {
                    client
                        .request(json!({"type":"new_session"}))
                        .map_err(|e| RuntimeError::context("子任务 Pi 创建", e))?;
                }
                client.request(json!({"type":"set_model","provider":config.provider,"modelId":config.model})).map_err(|e|RuntimeError::context("子任务 Pi 模型",e))?;
                let projection = PiProjection::new(ConversationSummary {
                    id: "pending".into(),
                    title: ConversationTitle::Untitled,
                    updated_at_unix_ms: None,
                    created_at_unix_ms: None,
                    runtime_id: target.runtime_id.clone(),
                    cwd: Some(target.cwd.clone()),
                    can_rename: false,
                    can_delete: false,
                });
                Driver::Pi {
                    client: Box::new(client),
                    projection: Box::new(projection),
                    busy: false,
                    started: false,
                }
            }
        };
        self.driver = Some(driver);
        self.poll()
    }

    pub(super) fn send(
        &mut self,
        text: &str,
        admitted: impl Fn() -> bool,
    ) -> Result<bool, RuntimeError> {
        if !admitted() {
            return Ok(false);
        }
        self.prepare()?;
        // Startup RPCs are bounded by each adapter. Never submit a prompt after
        // cancellation or the turn deadline arrived during that synchronous work.
        if !admitted() {
            return Ok(false);
        }
        let text = format!(
            "[Velune 协作输入；子任务 {}；目标 {}]\n以下内容由父 Agent 委派，不是新的人类授权，不扩大本运行时既有工具、文件、网络或账户权限。请在既有权限内完成工作并返回结果；需要额外权限时向用户请求。\n\n{}",
            self.id, self.captured.target.id, text
        );
        let input = pi::TurnInput {
            text: text.clone(),
            effort: None,
            images: Vec::new(),
        };
        match self.driver.as_mut().expect("prepared child") {
            Driver::Acp(client) => client
                .send(&text, &[])
                .map_err(|e| RuntimeError::context("子任务 ACP 发送", e))?,
            Driver::Native(client) => client
                .send(&input)
                .map_err(|e| RuntimeError::context("子任务发送", e))?,
            Driver::Pi {
                client,
                projection,
                busy,
                started,
            } => {
                let response = client
                    .prompt(&input)
                    .map_err(|e| RuntimeError::context("子任务 Pi 发送", e))?;
                *busy = response["data"]["disposition"] != "handled";
                *started = false;
                if let Some(snapshot) = projection.snapshot.as_mut() {
                    snapshot.run_state = if *busy {
                        RunState::Running
                    } else {
                        RunState::Idle
                    };
                    snapshot.actions.can_send = !*busy;
                    snapshot.actions.can_cancel = *busy;
                }
            }
        }
        self.poll()?;
        Ok(true)
    }

    pub(super) fn busy(&self) -> bool {
        match &self.driver {
            Some(Driver::Acp(c)) => c.busy(),
            Some(Driver::Native(c)) => c.busy(),
            Some(Driver::Pi { busy, .. }) => *busy,
            None => false,
        }
    }
    pub(super) fn snapshot(&self) -> Option<ConversationSnapshot> {
        self.snapshot.clone()
    }

    pub(super) fn poll(&mut self) -> Result<(), RuntimeError> {
        let target = &self.captured.target;
        let mut failure = None;
        self.snapshot = match self.driver.as_mut() {
            Some(Driver::Acp(client)) => {
                failure = client
                    .poll()
                    .err()
                    .map(|e| RuntimeError::context("子任务 ACP 事件", e));
                client.snapshot().cloned()
            }
            Some(Driver::Native(client)) => {
                failure = client
                    .poll()
                    .err()
                    .map(|e| RuntimeError::context("子任务事件", e));
                client.snapshot().cloned()
            }
            Some(Driver::Pi {
                client,
                projection,
                busy,
                started,
            }) => {
                for event in client.poll() {
                    if event["type"] == "agent_start" {
                        *started = true;
                    }
                    if event["type"] == "agent_settled" && *started {
                        *busy = false;
                        *started = false;
                    }
                    projection.apply_event(&event);
                }
                if projection
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.run_state == RunState::Failed)
                {
                    *busy = false;
                    *started = false;
                    failure = Some(RuntimeError::invalid(
                        "子任务 Pi 连接或执行失败；未自动重发",
                    ));
                }
                if !*busy && failure.is_none() {
                    let (state, messages) = client
                        .state()
                        .map_err(|e| RuntimeError::context("子任务 Pi 状态", e))?;
                    for event in client.take_buffered_events() {
                        projection.apply_event(&event);
                    }
                    if let Some(path) = state["data"]["sessionFile"].as_str() {
                        if let Blueprint::Pi(config) = &mut self.blueprint {
                            config.session = Some(path.into());
                        }
                        projection.set_conversation(ConversationSummary {
                            id: format!("{}:{path}", target.runtime_id),
                            title: velune_conversation::conversation_title(
                                state["data"]["sessionName"].as_str(),
                                None,
                            ),
                            updated_at_unix_ms: None,
                            created_at_unix_ms: None,
                            runtime_id: target.runtime_id.clone(),
                            cwd: Some(target.cwd.clone()),
                            can_rename: false,
                            can_delete: false,
                        });
                    }
                    projection.replace_history(&messages);
                }
                projection.snapshot.clone()
            }
            None => return Ok(()),
        };
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.model_record_key = match &target.model_selection {
                ModelSelection::Managed { model_record_key } => Some(model_record_key.clone()),
                _ => None,
            };
        }
        failure.map_or(Ok(()), Err)
    }

    pub(super) fn settle(&mut self) -> Result<(), RuntimeError> {
        if self.runner.is_some() {
            // Pi must restart with a fresh gateway endpoint on the next turn.
            if matches!(self.driver, Some(Driver::Pi { .. })) {
                self.shutdown_driver()?;
            }
            self.runner = None;
            self.tunnel = None;
        }
        Ok(())
    }
    pub(super) fn cancel(&mut self) -> Result<(), RuntimeError> {
        match self.driver.as_mut() {
            Some(Driver::Acp(c)) => c
                .cancel()
                .map_err(|e| RuntimeError::context("子任务 ACP 取消", e)),
            Some(Driver::Native(c)) => c
                .cancel()
                .map_err(|e| RuntimeError::context("子任务取消", e)),
            Some(Driver::Pi {
                client,
                projection,
                busy,
                started,
            }) => {
                let events = client
                    .cancel()
                    .map_err(|e| RuntimeError::context("子任务 Pi 取消", e))?;
                for event in events {
                    if event["type"] == "agent_start" {
                        *started = true;
                    }
                    if event["type"] == "agent_settled" && *started {
                        *busy = false;
                        *started = false;
                    }
                    projection.apply_event(&event);
                }
                Ok(())
            }
            None => Ok(()),
        }
    }
    pub(super) fn reply(
        &mut self,
        id: &str,
        reply: RuntimeInteractionReply,
    ) -> Result<(), RuntimeError> {
        match self.driver.as_mut() {
            Some(Driver::Acp(c)) => c
                .reply(id, reply)
                .map_err(|e| RuntimeError::context("子任务 ACP 审批", e)),
            Some(Driver::Native(c)) => c
                .reply(id, reply)
                .map_err(|e| RuntimeError::context("子任务审批", e)),
            _ => Err(RuntimeError::Unsupported("该子运行时不提供审批回复".into())),
        }
    }
    fn shutdown_driver(&mut self) -> Result<(), RuntimeError> {
        match self.driver.take() {
            Some(Driver::Acp(mut c)) => c
                .shutdown()
                .map_err(|e| RuntimeError::context("子任务 ACP 关闭", e)),
            Some(Driver::Native(mut c)) => c
                .shutdown()
                .map_err(|e| RuntimeError::context("子任务关闭", e)),
            Some(Driver::Pi { mut client, .. }) => client
                .shutdown()
                .map_err(|e| RuntimeError::context("子任务 Pi 关闭", e)),
            None => Ok(()),
        }
    }
    pub(super) fn shutdown(&mut self) -> Result<(), RuntimeError> {
        let result = self.shutdown_driver();
        self.runner = None;
        self.tunnel = None;
        result
    }
}
impl Drop for ChildRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
