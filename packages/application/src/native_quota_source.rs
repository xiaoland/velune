//! Resolve native login sources; CLI and execution details stay outside the cache.
use crate::config::RuntimeKind;
use crate::{QuotaFreshness, config::RuntimeInstance};
use std::path::{Path, PathBuf};
pub(crate) const CLAUDE_RUNTIME: RuntimeKind = RuntimeKind::ClaudeCode;

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Capture {
    Claude(ClaudeLogin),
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ClaudeLogin {
    binary: String,
    directory: Option<PathBuf>,
}
impl Capture {
    pub(crate) fn supported(kind: RuntimeKind) -> bool {
        kind == CLAUDE_RUNTIME
    }
    pub(crate) async fn read(&self, ttl_ms: i64) -> Result<Vec<velune_gateway::QuotaWindow>, ()> {
        match self {
            Self::Claude(login) => {
                crate::claude_quota::read(&login.binary, login.directory.as_deref(), ttl_ms).await
            }
        }
    }
    pub(crate) fn from_runtime(
        runtime: &RuntimeInstance,
    ) -> Result<Self, (QuotaFreshness, String)> {
        if !Self::supported(runtime.type_id) || !runtime.enabled {
            return Err((QuotaFreshness::Unsupported, "此运行时不提供登录额度".into()));
        }
        if runtime
            .execution()
            .map_err(|error| (QuotaFreshness::Unknown, error.to_string()))?
            != crate::RuntimeExecution::default()
        {
            return Err((
                QuotaFreshness::Unsupported,
                "目前仅支持本机、继承 HOME 的 Claude Code 登录额度".into(),
            ));
        }
        let setting = |key| {
            runtime
                .settings
                .get(key)
                .filter(|value| Path::new(value).is_absolute())
                .cloned()
                .ok_or_else(|| {
                    (
                        QuotaFreshness::Unknown,
                        "请先配置运行时入口和运行时目录".into(),
                    )
                })
        };
        let directory = setting("agentDir")?;
        Ok(Self::Claude(ClaudeLogin {
            binary: setting("binary")?,
            directory: velune_agent_runtime::native::claude_login_directory(Path::new(&directory)),
        }))
    }
}
