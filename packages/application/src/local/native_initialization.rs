//! Native creation captures configuration on the application owner, but creates
//! HOME/resources and the new adapter only on the bounded execution worker.
use super::*;
use velune_agent_runtime::creation::{DataRetention, Retained};

impl CoreRuntime {
    pub(super) fn initialize_codex_conversation(
        &mut self,
        runtime: RuntimeInstance,
        cwd: Option<String>,
        selection: Option<crate::ModelSelection>,
    ) -> Result<crate::SnapshotResult, RuntimeError> {
        self.validate_model_selection(&runtime, selection.as_ref())?;
        let cwd = cwd.map(PathBuf::from);
        if cwd.as_ref().is_some_and(|cwd| !cwd.is_absolute()) {
            return Err(RuntimeError::invalid("会话工作目录必须为绝对路径"));
        }
        let source = self.runtime_source_key(&runtime)?;
        let (context, preparation) = self.execution_preparation(&runtime)?;
        let config = NativeConfig {
            kind: NativeKind::Codex166,
            binary: setting_path(&runtime, "binary")?,
            node_binary: setting_path_optional(&runtime, "nodeBinary"),
            agent_dir: context.agent_dir,
            resources_directory: context.resources,
            projection_directory: context.projection,
            execution: context.target,
            os_home: context.os_home,
            gateway: None,
            mcp_servers: self.collaboration_servers(),
            session_agents: Vec::new(),
        };
        self.capture_execution_configuration(&runtime, None);
        self.session.execution_runtime_id = Some(runtime.id.clone());
        self.session.model_selection = Some(selection.clone());
        let owner = self.telemetry.clone();
        self.start_live_operation_with_outcome("native_create", move |state| {
            let create = || -> Result<(NativeSession, Option<String>), RuntimeError> {
                let prepared = preparation.prepare()?;
                if let Some(cwd) = &cwd
                    && !config
                        .execution
                        .is_directory(cwd)
                        .map_err(|error| RuntimeError::context("会话工作目录检查", error))?
                {
                    return Err(RuntimeError::invalid(
                        "会话工作目录必须是执行机器上的已有目录",
                    ));
                }
                let mut native = telemetry::observe(owner.as_ref(), "harness.start", || {
                    NativeSession::connect(config)
                })
                .map_err(|error| RuntimeError::context("原生运行时连接", error))?;
                if let Some(crate::ModelSelection::Native {
                    provider_id,
                    model_id,
                }) = selection
                {
                    native
                        .select_native_model(Retained::from_parts(
                            (provider_id, model_id),
                            DataRetention::default(),
                        ))
                        .map_err(|error| RuntimeError::context("原生模型选择", error))?;
                }
                native
                    .create(cwd.as_deref(), &runtime.id)
                    .map_err(|error| {
                        RuntimeError::runtime_operation("原生会话创建", Box::new(error))
                    })?;
                Ok((native, prepared))
            };
            match telemetry::observe(owner.as_ref(), "harness.prepare", create) {
                Ok((native, prepared)) => {
                    *state = ActiveState::Native(Box::new(native));
                    execution_workers::OperationOutcome::after_creation(prepared)
                }
                Err(error) => Err(error).into(),
            }
        })?;
        if let ActiveState::Pending(pending) = &mut self.session.active_state {
            pending.mark_initialization(source);
        }
        self.snapshot_result()
    }

    pub(super) fn finish_native_creation(
        &mut self,
        prepared_resources: Option<String>,
    ) -> Result<(), RuntimeError> {
        let native = self
            .session
            .projected()
            .ok_or_else(|| {
                RuntimeError::operation_completed("原生会话已创建，但没有返回可显示的身份")
            })?
            .conversation
            .clone();
        if let Some(key) = prepared_resources {
            self.prepared_remote_resources.borrow_mut().insert(key);
        }
        self.session.logical_projection = None;
        self.session.pending_native_fork = None;
        self.session.model_record_key = None;
        self.session.unsubmitted_draft = Some(UnsubmittedDraft {
            public_id: native.id.clone(),
            original_runtime_id: native.runtime_id.clone(),
            runtime_id: native.runtime_id,
            native_id: native.id,
            creation_token: None,
        });
        // Create already performed preparation on its worker and installed a
        // loaded thread. This captures memory-only ownership proof, not a new
        // preparation/provider RPC on the application owner.
        let creation_token = self.capture_created_draft();
        self.session
            .unsubmitted_draft
            .as_mut()
            .expect("created draft retained")
            .creation_token = creation_token;
        Ok(())
    }
}
