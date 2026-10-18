//! Generic ACP launch data and negotiated capabilities, independent of the local executor.
use crate::{Failure as Error, config::RuntimeInstance};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const LAUNCH_SETTING: &str = "acpLaunch";

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcpLaunch {
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
}

impl std::fmt::Debug for AcpLaunch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AcpLaunch")
            .field("argument_count", &self.args.len())
            .field(
                "environment_keys",
                &self.environment.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl AcpLaunch {
    pub fn validate(&self) -> Result<(), Error> {
        if self.args.iter().any(|argument| argument.contains('\0'))
            || self.environment.iter().any(|(key, value)| {
                key.is_empty() || key.contains(['=', '\0']) || value.contains('\0')
            })
        {
            return Err(Error::invalid("ACP 参数或环境配置无效"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpCapabilities {
    pub load_session: bool,
    pub list_sessions: bool,
    pub resume_session: bool,
    pub close_session: bool,
    pub delete_session: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpAgentInfo {
    pub name: String,
    pub title: Option<String>,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpAuthMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpRuntimeInfo {
    pub capabilities: AcpCapabilities,
    pub agent_info: Option<AcpAgentInfo>,
    pub auth_methods: Vec<AcpAuthMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum AcpConfigValue {
    Select(String),
    Boolean(bool),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpConfigChoice {
    pub value: String,
    pub name: String,
    pub description: Option<String>,
    pub group: Option<String>,
    pub group_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpConfigOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub value: AcpConfigValue,
    pub choices: Vec<AcpConfigChoice>,
}

impl RuntimeInstance {
    pub fn acp_launch(&self) -> Result<AcpLaunch, Error> {
        if self.type_id != "acp-v1" {
            return Err(Error::invalid("此实例不是通用 ACP 运行时"));
        }
        let launch: AcpLaunch = self
            .settings
            .get(LAUNCH_SETTING)
            .map(|value| serde_json::from_str(value))
            .transpose()?
            .unwrap_or_default();
        launch.validate()?;
        Ok(launch)
    }

    pub fn set_acp_launch(&mut self, launch: AcpLaunch) -> Result<(), Error> {
        if self.type_id != "acp-v1" {
            return Err(Error::invalid("此实例不是通用 ACP 运行时"));
        }
        launch.validate()?;
        self.settings
            .insert(LAUNCH_SETTING.into(), serde_json::to_string(&launch)?);
        Ok(())
    }
}
