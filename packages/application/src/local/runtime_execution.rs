//! Resolve execution-machine paths before dispatching any process or filesystem operation.
use super::*;
use crate::{RuntimeExecution, RuntimeHome, RuntimeTransport};
use sha2::Digest;
use velune_agent_runtime::execution::{ExecutionTarget, SshTarget};

pub(super) struct ExecutionContext {
    pub target: ExecutionTarget,
    pub os_home: Option<PathBuf>,
    pub agent_dir: PathBuf,
    pub resources: PathBuf,
    pub projection: PathBuf,
}

impl CoreRuntime {
    pub(super) fn runtime_location(
        &self,
        runtime: &RuntimeInstance,
    ) -> Result<(ExecutionTarget, Option<PathBuf>), RuntimeError> {
        let execution = runtime.execution_settings()?;
        let target = match &execution.transport {
            RuntimeTransport::Local => ExecutionTarget::Local,
            RuntimeTransport::Ssh {
                binary,
                destination,
                port,
            } => ExecutionTarget::Ssh(SshTarget {
                binary: binary.into(),
                destination: destination.clone(),
                port: *port,
            }),
        };
        target
            .validate()
            .map_err(|error| RuntimeError::context("运行位置配置", error))?;
        let key = format!("{:x}", sha2::Sha256::digest(runtime.id.as_bytes()));
        let os_home = match &execution.home {
            RuntimeHome::Inherit => None,
            RuntimeHome::Existing { path } => Some(path.into()),
            RuntimeHome::Managed { remote_root } => Some(match &target {
                ExecutionTarget::Local => {
                    self.options.home_directory.join("runtime-homes").join(&key)
                }
                ExecutionTarget::Ssh(_) => {
                    PathBuf::from(remote_root.as_ref().expect("validated remote HOME")).join(&key)
                }
            }),
        };
        Ok((target, os_home))
    }

    pub(super) fn prepare_runtime_location(
        &self,
        runtime: &RuntimeInstance,
    ) -> Result<(ExecutionTarget, Option<PathBuf>), RuntimeError> {
        let (target, home) = self.runtime_location(runtime)?;
        if matches!(runtime.execution()?.home, RuntimeHome::Managed { .. }) {
            target
                .create_dir_all(home.as_deref().expect("managed HOME"))
                .map_err(|error| RuntimeError::context("managed HOME 初始化", error))?;
        }
        Ok((target, home))
    }

    pub(super) fn execution_context(
        &self,
        runtime: &RuntimeInstance,
    ) -> Result<ExecutionContext, RuntimeError> {
        let execution = runtime.execution()?;
        let (target, os_home) = self.runtime_location(runtime)?;
        let key = format!("{:x}", sha2::Sha256::digest(runtime.id.as_bytes()));
        let agent_dir = self
            .native_agent_directory(runtime, os_home.as_deref())?
            .ok_or_else(|| RuntimeError::invalid("此运行时没有原生配置目录"))?;
        let resources = match &target {
            ExecutionTarget::Local => self.options.resources_directory.clone(),
            ExecutionTarget::Ssh(_) => PathBuf::from(
                execution
                    .remote_resources_directory
                    .expect("validated remote resources"),
            ),
        };
        let projection = match &target {
            ExecutionTarget::Local => self
                .options
                .home_directory
                .join("runtime-projections")
                .join(&key),
            ExecutionTarget::Ssh(_) => resources.join("runtime-projections").join(&key),
        };
        Ok(ExecutionContext {
            target,
            os_home,
            agent_dir,
            resources,
            projection,
        })
    }

    /// Resolve native roots without creating HOME or preparing execution resources.
    pub(super) fn native_agent_directory(
        &self,
        runtime: &RuntimeInstance,
        os_home: Option<&Path>,
    ) -> Result<Option<PathBuf>, RuntimeError> {
        if runtime.type_id == "acp-v1" {
            return Ok(None);
        }
        if matches!(
            runtime.execution_settings()?.home,
            RuntimeHome::Managed { .. }
        ) {
            let directory = match runtime.type_id.as_str() {
                "pi-1.0" => ".pi/agent",
                "codex-0.159" => ".codex",
                "claude-code-2.1" => ".claude",
                "dsh-acp-0.2" => ".dsh",
                _ => return Err(RuntimeError::invalid("不支持的运行时版本类型")),
            };
            Ok(Some(
                os_home
                    .ok_or_else(|| RuntimeError::invalid("managed HOME 尚未解析"))?
                    .join(directory),
            ))
        } else {
            Ok(setting_path_optional(runtime, "agentDir"))
        }
    }

    pub(super) fn prepare_execution_context(
        &self,
        runtime: &RuntimeInstance,
    ) -> Result<ExecutionContext, RuntimeError> {
        let context = self.execution_context(runtime)?;
        if matches!(runtime.execution()?.home, RuntimeHome::Managed { .. }) {
            context
                .target
                .create_dir_all(context.os_home.as_deref().expect("managed HOME"))
                .and_then(|()| context.target.create_dir_all(&context.agent_dir))
                .map_err(|error| RuntimeError::context("managed HOME 初始化", error))?;
        }
        if !context.target.is_local() {
            if context.resources.starts_with(&context.agent_dir)
                || context
                    .os_home
                    .as_ref()
                    .is_some_and(|home| context.resources == *home)
            {
                return Err(RuntimeError::invalid(
                    "远端辅助资源目录必须独立于运行时 HOME 与原生配置目录",
                ));
            }
            let key = serde_json::to_string(&runtime.execution()?)?;
            if !self.prepared_remote_resources.borrow().contains(&key) {
                velune_agent_runtime::execution::install_resources(
                    &context.target,
                    &self.options.resources_directory,
                    &context.resources,
                )
                .map_err(|error| RuntimeError::context("远端辅助资源安装", error))?;
                self.prepared_remote_resources.borrow_mut().insert(key);
            }
        }
        Ok(context)
    }

    pub(super) fn runtime_execution_action(
        &mut self,
        request: &Value,
        update: bool,
    ) -> Result<Value, RuntimeError> {
        let runtime_id = request["payload"]["runtimeInstanceID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
        let mut runtime = self.runtime_instance(runtime_id)?.clone();
        if !update {
            return Ok(serde_json::to_value(runtime.execution()?)?);
        }
        let execution: RuntimeExecution =
            serde_json::from_value(request["payload"]["execution"].clone())?;
        runtime.set_execution(execution)?;
        self.runtime_action(&json!({"payload":{"operation":"upsert","runtimeInstance":serde_json::to_string(&runtime)?}}))
    }

    pub(super) fn validate_runtime_cwd(
        &self,
        runtime: &RuntimeInstance,
        cwd: &Path,
    ) -> Result<(), RuntimeError> {
        if !cwd.is_absolute()
            || !self
                .runtime_location(runtime)?
                .0
                .is_directory(cwd)
                .map_err(|error| RuntimeError::context("会话工作目录检查", error))?
        {
            return Err(RuntimeError::invalid(
                "会话工作目录必须是执行机器上的已有绝对目录",
            ));
        }
        Ok(())
    }
}
