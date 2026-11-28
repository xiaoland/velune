//! Generic ACP uses negotiated protocol features, never Harness-specific files or overlays.
use super::*;
use crate::conversation::ConversationSnapshot;
use velune_agent_runtime::acp::{Client, Config};
use velune_conversation::AutoCompactionSetting;

impl CoreRuntime {
    fn acp_client(&self, runtime: &RuntimeInstance) -> Result<Client, RuntimeError> {
        self.acp_client_with_servers(runtime, Vec::new())
    }
    fn acp_client_with_servers(
        &self,
        runtime: &RuntimeInstance,
        mcp_servers: Vec<velune_agent_runtime::mcp::Server>,
    ) -> Result<Client, RuntimeError> {
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用，请先启用"));
        }
        let config = self.acp_configuration(runtime, mcp_servers)?;
        self.prepare_runtime_location(runtime)?;
        let (client, capabilities) = connect_acp(config, self.telemetry.as_ref())?;
        self.acp_capability_cache
            .borrow_mut()
            .insert(runtime.id.clone(), capabilities);
        Ok(client)
    }

    pub(super) fn acp_configuration(
        &self,
        runtime: &RuntimeInstance,
        mcp_servers: Vec<velune_agent_runtime::mcp::Server>,
    ) -> Result<Config, RuntimeError> {
        let launch = runtime.acp_launch()?;
        let (execution, os_home) = self.runtime_location(runtime)?;
        Ok(Config {
            binary: setting_path(runtime, "binary")?,
            args: launch.args,
            environment: launch.environment,
            execution,
            os_home,
            mcp_servers,
        })
    }

    pub(crate) fn acp_launch(&self, runtime_id: String) -> Result<crate::AcpLaunch, RuntimeError> {
        Ok(self.runtime_instance(&runtime_id)?.acp_launch()?)
    }

    pub(crate) fn acp_runtime_info(
        &self,
        runtime_id: String,
    ) -> Result<crate::AcpRuntimeInfo, RuntimeError> {
        let runtime = self.runtime_instance(&runtime_id)?;
        if self.session.execution_runtime_id() == Some(&runtime.id)
            && let ActiveState::Acp(session) = &self.session.active_state
        {
            return Ok(acp_runtime_info(session));
        }
        let mut session = self.acp_client(runtime)?;
        let info = acp_runtime_info(&session);
        session
            .shutdown()
            .map_err(|error| RuntimeError::context("ACP 关闭", error))?;
        Ok(info)
    }

    pub(super) fn validate_acp_handoff_target(
        &self,
        runtime: &RuntimeInstance,
    ) -> Result<(), RuntimeError> {
        if runtime.type_id != RuntimeKind::Acp {
            return Ok(());
        }
        let mut session = self.acp_client(runtime)?;
        let supported = session.capabilities().load_session;
        session
            .shutdown()
            .map_err(|error| RuntimeError::context("ACP 关闭", error))?;
        if !supported {
            return Err(RuntimeError::Unsupported(
                "目标 ACP Agent 不提供历史加载，无法建立重启后可恢复的跨运行时会话".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn refresh_acp_capabilities(
        &self,
        runtime: &RuntimeInstance,
    ) -> Result<(), RuntimeError> {
        if runtime.type_id == RuntimeKind::Acp {
            self.acp_client(runtime)?
                .shutdown()
                .map_err(|error| RuntimeError::context("ACP 关闭", error))?;
        }
        Ok(())
    }

    pub(super) fn read_acp_history(
        &self,
        runtime: &RuntimeInstance,
        native_id: &str,
        cwd: &Path,
    ) -> Result<browsing::HistorySnapshot, RuntimeError> {
        let request = self.prepare_acp_history(runtime, native_id, cwd)?;
        self.finish_history_read(request.load()?)
    }

    pub(super) fn prepare_acp_snapshot(
        &mut self,
        snapshot: &ConversationSnapshot,
        selection: Option<&crate::ModelSelection>,
        is_new: bool,
    ) -> Result<(), RuntimeError> {
        let runtime = self
            .runtime_instance(&snapshot.conversation.runtime_id)?
            .clone();
        self.validate_model_selection(&runtime, selection)?;
        let native_id = (!is_new)
            .then(|| {
                snapshot
                    .conversation
                    .id
                    .strip_prefix(&format!("{}:", runtime.id))
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| RuntimeError::invalid("conversation id"))
            })
            .transpose()?;
        if !is_new
            && self.session.execution_runtime_id() == Some(&runtime.id)
            && let ActiveState::Acp(session) = &mut self.session.active_state
            && session
                .snapshot()
                .is_some_and(|active| active.conversation.id == snapshot.conversation.id)
        {
            if let Some(crate::ModelSelection::Native { model_id, .. }) = selection {
                session
                    .select_model(model_id)
                    .map_err(|error| RuntimeError::context("ACP 模型选择", error))?;
            }
            return Ok(());
        }
        let cwd = snapshot
            .conversation
            .cwd
            .as_deref()
            .map(Path::new)
            .ok_or_else(|| RuntimeError::invalid("ACP 会话必须指定执行机器上的工作目录"))?;
        self.validate_runtime_cwd(&runtime, cwd)?;
        let servers = self.collaboration_servers();
        let mut session = self.acp_client_with_servers(&runtime, servers)?;
        if let Some(native_id) = native_id {
            if !session.capabilities().load_session {
                return Err(RuntimeError::Unsupported(
                    "此 ACP Agent 不支持重新加载会话，请保留当前活动进程".into(),
                ));
            }
            session.load(native_id, cwd, &runtime.id)
        } else {
            session.create(cwd, &runtime.id)
        }
        .map_err(|error| RuntimeError::context("ACP 会话准备", error))?;
        if let Some(crate::ModelSelection::Native { model_id, .. }) = selection {
            session
                .select_model(model_id)
                .map_err(|error| RuntimeError::context("ACP 模型选择", error))?;
        }
        self.shutdown_active()?;
        self.session.install_execution(
            ActiveState::Acp(Box::new(session)),
            execution::SessionExecution::native(runtime),
        );
        Ok(())
    }

    pub(super) fn apply_acp_model_auto_compaction(
        &mut self,
        runtime_id: &str,
    ) -> Result<(), RuntimeError> {
        let ActiveState::Acp(session) = &mut self.session.active_state else {
            return Ok(());
        };
        let runtime = self
            .runtime_instances
            .iter()
            .find(|runtime| runtime.id == runtime_id)
            .ok_or_else(|| RuntimeError::invalid("ACP 运行时不存在"))?;
        session
            .poll()
            .map_err(|error| RuntimeError::context("读取 ACP 会话设置", error))?;
        let desired = session
            .current_model_id()
            .map(|model_id| {
                runtime.model_auto_compaction(&crate::ModelSelection::Native {
                    provider_id: None,
                    model_id: model_id.into(),
                })
            })
            .transpose()?
            .flatten();
        let desired = match &desired {
            Some(AutoCompactionSetting::Acp { option_id, value }) => {
                Some((option_id.as_str(), value))
            }
            _ => None,
        };
        session
            .reconcile_auto_compaction(desired)
            .map_err(|error| RuntimeError::context("ACP 模型自动压缩设置", error))
    }

    /// The loaded ACP conversation's session configuration, after setting
    /// `update` (a config id and its value) when given.
    pub(crate) fn acp_session_config(
        &mut self,
        runtime_id: &str,
        id: &str,
        update: Option<(&str, crate::AcpConfigValue)>,
    ) -> Result<Vec<crate::AcpConfigOption>, RuntimeError> {
        self.ensure_active(runtime_id)?;
        if self
            .current_snapshot()
            .is_none_or(|snapshot| snapshot.conversation.id != id)
        {
            return Err(RuntimeError::invalid("ACP 配置不属于当前会话"));
        }
        let ActiveState::Acp(session) = &mut self.session.active_state else {
            return Err(RuntimeError::Unsupported("当前会话不是通用 ACP".into()));
        };
        if let Some((config_id, value)) = update {
            session
                .set_config_option(config_id, value)
                .map_err(|error| RuntimeError::context("ACP 配置选择", error))?;
            self.session.model_selection = Some(None);
        }
        Ok(session.config_options().to_vec())
    }

    pub(super) fn delete_acp_conversation(
        &mut self,
        runtime: &RuntimeInstance,
        id: &str,
    ) -> Result<crate::ConfigurationSnapshot, RuntimeError> {
        let native_id = id
            .strip_prefix(&format!("{}:", runtime.id))
            .filter(|id| !id.is_empty())
            .ok_or_else(|| RuntimeError::invalid("conversation id"))?;
        let mut session = self.acp_client(runtime)?;
        if !session.capabilities().delete_session {
            return Err(RuntimeError::Unsupported(
                "此 ACP Agent 不支持删除会话".into(),
            ));
        }
        session
            .delete_session(native_id)
            .map_err(|error| RuntimeError::context("ACP 会话删除", error))?;
        session
            .shutdown()
            .map_err(|error| RuntimeError::context("ACP 关闭", error))?;
        if self
            .native_snapshot()
            .is_some_and(|snapshot| snapshot.conversation.id == id)
        {
            self.shutdown_active()?;
            self.session.unsubmitted_draft = None;
        }
        self.list()
    }
}

pub(super) fn connect_acp(
    config: Config,
    telemetry: Option<&Arc<velune_telemetry::ClientTelemetry>>,
) -> Result<(Client, crate::AcpCapabilities), RuntimeError> {
    let client = telemetry::observe(telemetry, "harness.start", || Client::connect(config))
        .map_err(|error| RuntimeError::context("ACP 初始化", error))?;
    let capabilities = acp_capabilities(&client);
    Ok((client, capabilities))
}

pub(super) fn list_acp_sessions(
    mut session: Client,
    runtime_id: &str,
) -> Result<velune_agent_runtime::creation::Retained<Vec<ConversationSummary>>, RuntimeError> {
    if !session.capabilities().list_sessions {
        return Err(RuntimeError::Unsupported(
            "此 ACP Agent 不提供会话列表；当前会话仍可使用，已知会话可按 load 能力显式打开".into(),
        ));
    }
    let summaries = session
        .list_sessions(None, runtime_id)
        .map_err(|error| RuntimeError::context("ACP 会话列表", error));
    let closed = session
        .shutdown()
        .map_err(|error| RuntimeError::context("ACP 关闭", error));
    let summaries = summaries?;
    closed?;
    Ok(summaries)
}

/// Perform the ACP history request without touching application state.
pub(super) fn load_acp_history(
    config: Config,
    runtime_id: &str,
    native_id: &str,
    cwd: &Path,
) -> Result<
    (
        velune_agent_runtime::creation::Retained<ConversationSnapshot>,
        crate::AcpCapabilities,
    ),
    RuntimeError,
> {
    if !cwd.is_absolute()
        || !config
            .execution
            .is_directory(cwd)
            .map_err(|error| RuntimeError::context("会话工作目录检查", error))?
    {
        return Err(RuntimeError::invalid(
            "会话工作目录必须是执行机器上的已有绝对目录",
        ));
    }
    let mut session =
        Client::connect(config).map_err(|error| RuntimeError::context("ACP 初始化", error))?;
    let capabilities = acp_capabilities(&session);
    let snapshot = if capabilities.load_session {
        session
            .load(native_id, cwd, runtime_id)
            .map_err(|error| RuntimeError::context("ACP 原生历史加载", error))
    } else {
        Err(RuntimeError::Unsupported(
            "此 ACP Agent 不提供会话历史加载；持久跨运行时接续需要可重读的原生历史".into(),
        ))
    };
    let closed = session
        .shutdown()
        .map_err(|error| RuntimeError::context("ACP 关闭", error));
    let snapshot = snapshot?;
    closed?;
    Ok((snapshot, capabilities))
}

fn acp_capabilities(session: &Client) -> crate::AcpCapabilities {
    let capabilities = session.capabilities();
    crate::AcpCapabilities {
        load_session: capabilities.load_session,
        list_sessions: capabilities.list_sessions,
        resume_session: capabilities.resume_session,
        close_session: capabilities.close_session,
        delete_session: capabilities.delete_session,
    }
}

fn acp_runtime_info(session: &Client) -> crate::AcpRuntimeInfo {
    crate::AcpRuntimeInfo {
        capabilities: acp_capabilities(session),
        agent_info: session.agent_info().map(|info| crate::AcpAgentInfo {
            name: info.name.clone(),
            title: info.title.clone(),
            version: info.version.clone(),
        }),
        auth_methods: session
            .auth_methods()
            .iter()
            .map(|method| crate::AcpAuthMethod {
                id: method.id.clone(),
                name: method.name.clone(),
                description: method.description.clone(),
            })
            .collect(),
    }
}
