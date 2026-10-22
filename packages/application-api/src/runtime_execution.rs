//! Typed execution settings are encoded once inside the runtime's extensible settings map.
use crate::{Failure as Error, config::RuntimeInstance};
use serde::{Deserialize, Serialize};

const EXECUTION_SETTING: &str = "execution";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum RuntimeTransport {
    #[default]
    Local,
    Ssh {
        binary: String,
        destination: String,
        port: Option<u16>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum RuntimeHome {
    #[default]
    Inherit,
    Existing {
        path: String,
    },
    /// Local roots are assigned under VELUNE_HOME; SSH roots belong to the remote machine.
    Managed {
        remote_root: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeExecution {
    pub transport: RuntimeTransport,
    pub home: RuntimeHome,
    pub remote_resources_directory: Option<String>,
}

impl RuntimeExecution {
    pub fn validate(&self) -> Result<(), Error> {
        let absolute = |value: &str| value.starts_with('/') && !value.contains('\0');
        if let RuntimeTransport::Ssh {
            binary,
            destination,
            port,
        } = &self.transport
        {
            if !absolute(binary)
                || destination.is_empty()
                || destination.starts_with('-')
                || destination
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control())
                || *port == Some(0)
            {
                return Err(Error::invalid("SSH 入口、目标或端口无效"));
            }
            if self
                .remote_resources_directory
                .as_deref()
                .is_some_and(|path| !absolute(path))
            {
                return Err(Error::invalid("远端辅助资源必须是绝对目录"));
            }
        }
        // A local HOME is a path of the machine this core runs on, by its own
        // rules (C:\… on Windows); an SSH target's is a Unix path.
        let local_absolute =
            |value: &str| std::path::Path::new(value).is_absolute() && !value.contains('\0');
        match (&self.transport, &self.home) {
            (RuntimeTransport::Local, RuntimeHome::Existing { path }) if !local_absolute(path) => {
                return Err(Error::invalid("HOME 必须是执行机器上的绝对目录"));
            }
            (RuntimeTransport::Ssh { .. }, RuntimeHome::Existing { path }) if !absolute(path) => {
                return Err(Error::invalid("HOME 必须是执行机器上的绝对目录"));
            }
            (RuntimeTransport::Ssh { .. }, RuntimeHome::Managed { remote_root })
                if remote_root.as_deref().is_none_or(|path| !absolute(path)) =>
            {
                return Err(Error::invalid("远端 managed HOME 必须指定远端绝对根目录"));
            }
            (
                RuntimeTransport::Local,
                RuntimeHome::Managed {
                    remote_root: Some(_),
                },
            ) => {
                return Err(Error::invalid(
                    "本地 managed HOME 由应用分配，不接受远端根目录",
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

impl RuntimeInstance {
    pub fn execution_settings(&self) -> Result<RuntimeExecution, Error> {
        let execution = self
            .settings
            .get(EXECUTION_SETTING)
            .map(|value| serde_json::from_str(value))
            .transpose()?
            .unwrap_or_default();
        RuntimeExecution::validate(&execution)?;
        Ok(execution)
    }

    pub fn execution(&self) -> Result<RuntimeExecution, Error> {
        let execution = self.execution_settings()?;
        if self.type_id != "acp-v1"
            && matches!(execution.transport, RuntimeTransport::Ssh { .. })
            && execution.remote_resources_directory.is_none()
        {
            return Err(Error::invalid("此运行时必须指定远端辅助资源目录"));
        }
        Ok(execution)
    }

    pub fn set_execution(&mut self, execution: RuntimeExecution) -> Result<(), Error> {
        execution.validate()?;
        self.settings
            .insert(EXECUTION_SETTING.into(), serde_json::to_string(&execution)?);
        Ok(())
    }
}
