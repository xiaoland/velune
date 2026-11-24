//! Shared application use cases, ordinary configuration and lifecycle.
mod acp;
pub use acp::{
    AcpAgentInfo, AcpAuthMethod, AcpCapabilities, AcpConfigChoice, AcpConfigOption, AcpConfigValue,
    AcpLaunch, AcpRuntimeInfo,
};
mod analytics;
mod commands;
mod composer;
mod scheduled_tasks;
pub use velune_application_api::{
    SchedulePattern, SchedulePreview, ScheduledRun, ScheduledRunPhase, ScheduledTarget,
    ScheduledTask, ScheduledTaskAvailability, ScheduledTaskSource, ScheduledTaskStatus,
    ScheduledTasksOperation, ScheduledTasksState,
};
mod service;
pub use service::ApplicationService;
mod analytics_report;
mod api_account_quota;
mod ark_quota;
mod authentication_resolver;
mod billing;
mod claude_cli;
mod claude_quota;
mod credential_helper;
mod kimi_quota;
mod native_quota_source;
mod provider_account;
mod provider_analytics;
mod provider_authentication;
mod provider_login;
mod query_credential;
mod quota_authentication;
mod quota_history;
mod runtime_quota;
mod subscription_auth;
mod subscription_quota;
pub use provider_authentication::{
    AuthenticationEdit, AuthenticationMethod, AuthenticationProvenance,
    CODEX_SUBSCRIPTION_ENDPOINT, CODEX_SUBSCRIPTION_PROTOCOL, LinkedClaudeLogin,
    ProviderAuthenticationDescription, ProviderAuthenticationSource,
    QuotaAuthenticationDescription, QuotaAuthenticationEdit, SubscriptionDisclosure,
};
pub use velune_application_api::{
    BilledCost, BillingAuthenticationEdit, BillingConfiguration, BillingRateLimit, BillingReport,
    BillingScope, BillingService, BillingSpendLimit, BillingUsage, ProviderBillingStatus,
};
pub use velune_application_api::{
    ProviderAccountCredits, ProviderAccountDailyUsage, ProviderAccountFailure,
    ProviderAccountProfile, ProviderAccountResetGrant, ProviderAccountResets,
    ProviderAccountSource, ProviderAccountState, ProviderAccountStatus,
    ProviderAccountTopInvocation,
};
pub use velune_application_api::{
    ProviderAnalyticsArea, ProviderAnalyticsChatgptProfileTotal, ProviderAnalyticsDailyTotal,
    ProviderAnalyticsFailure, ProviderAnalyticsOrganizationTotal, ProviderAnalyticsReport,
    ProviderAnalyticsScope, ProviderAnalyticsSource, ProviderAnalyticsSourceKind,
    ProviderAnalyticsSourceState, ProviderAnalyticsUsage,
};
pub use velune_application_api::{ProviderTemplate, ProviderTemplateKind, provider_templates};

pub fn default_codex_client_version() -> &'static str {
    velune_agent_runtime::version::CODEX_LATEST_SUPPORTED_VERSION
}

pub mod config;
mod local;
pub mod model_catalog;
mod native_provider_import;
mod persistence_migrations;
mod provider_configuration;
mod provider_import;
mod repository;
mod runtime_execution;
pub use runtime_execution::{RuntimeExecution, RuntimeHome, RuntimeTransport};
mod resource_library;
pub use resource_library::{
    BoundEntry, ResourceBinding, ResourceBindingResolution, ResourceDriftFile, ResourceSaveResult,
    ResourceSyncState, ResourceSyncStatus, ResourceTemplate, ResourceTemplateContent,
    ResourceTemplateImport, ResourceTemplateSummary, SubagentModel, TemplateApplyOutcome,
    TemplateApplyReport, TemplateApplyTarget, TemplateMcpTransport, TemplateSkillFile,
    UnmanagedRuntimeResource,
};
mod runtime_management;
pub use runtime_management::{
    RuntimeResourceAccess, RuntimeResourceCatalog, RuntimeResourceDocument, RuntimeResourceEntry,
    RuntimeResourceKey, RuntimeResourceKind, RuntimeResourceNode, RuntimeResourceRevision,
    RuntimeResourceScope, RuntimeResourceState, RuntimeResourceSupport,
};
pub use velune_agent_runtime::management::ErrorCode as RuntimeManagementErrorCode;
mod runtime_discovery;
pub use velune_application_api::records::*;
pub use velune_application_api::{
    ApplicationReply, AttachmentContent, AttachmentReference, ClientViewId, Command,
    ExecutionContext, Failure, FailureKind, Response, Revision,
};
mod transcript_delivery;

use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
pub use transcript_delivery::{
    DeliveredBlock, DeliveredImageSource, DeliveredMessage, DeliveredSnapshot, MessagePayload,
    SnapshotUpdate, TranscriptCursor,
};
pub use velune_conversation as conversation;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub home_directory: PathBuf,
    pub resources_directory: PathBuf,
}
impl Options {
    fn validate(&self) -> Result<(), Error> {
        for path in [Some(&self.home_directory), Some(&self.resources_directory)]
            .into_iter()
            .flatten()
        {
            if !path.is_absolute() {
                return Err(Error::invalid("application paths must be absolute"));
            }
        }
        fs::create_dir_all(&self.home_directory)
            .map_err(|_| Error::invalid("application home directory"))?;
        if !self.resources_directory.is_dir() {
            return Err(Error::invalid("application resources directory"));
        }
        Ok(())
    }
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Contract(#[from] velune_application_api::Failure),
    #[error("{0}")]
    ProviderImport(#[from] velune_agent_runtime::provider_source::SourceReadError),
    #[error("{0}")]
    History(#[from] velune_agent_runtime::history::HistoryError),
    #[error("{0}")]
    RuntimeManagement(#[from] velune_agent_runtime::management::Error),
    #[error("invalid application operation: {0}")]
    Invalid(String),
    #[error("unsupported application operation: {0}")]
    Unsupported(String),
    /// What was asked would end a runtime process whose native sub-agents
    /// or background tasks still run.
    #[error("子 Agent 仍在运行；请等待其结束，或先停止子 Agent")]
    OutstandingWork,
    /// A requested operation conflicts with the conversation's active turn.
    /// Copying a completed prefix into an independent fork is not a conflict.
    #[error("会话正在运行；请等当前 turn 结束后重试")]
    Busy,
    #[error("application I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("application JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}
impl Error {
    /// Retain the actual boundary failure in local diagnostics rather than replacing
    /// it with the operation's summary.
    pub(crate) fn context(message: &str, cause: impl std::fmt::Display) -> Self {
        Self::Invalid(format!("{message}：{cause}"))
    }

    pub(crate) fn after_delivery(cause: impl std::fmt::Display) -> Self {
        let mut failure = Failure::new(
            FailureKind::Unavailable,
            format!("输入已受理，但刷新结果失败；请先查看会话和队列，不要重复发送：{cause}"),
        );
        failure.code = "message_delivery_accepted".into();
        Self::Contract(failure)
    }
    pub(crate) fn runtime_operation(message: &str, cause: velune_agent_runtime::LiveError) -> Self {
        let indeterminate = cause.downcast_ref::<velune_agent_runtime::native::Error>()
            .is_some_and(|error| matches!(error.code(), "timeout" | "transport"))
            || cause.downcast_ref::<velune_agent_runtime::Error>().is_some_and(|error| matches!(
                error,
                velune_agent_runtime::Error::Timeout(_) | velune_agent_runtime::Error::ChildExited
                    | velune_agent_runtime::Error::Transport(_)
                    | velune_agent_runtime::Error::Io(_) | velune_agent_runtime::Error::Json(_)
            ));
        if indeterminate {
            Self::operation_indeterminate(format!("{message}：{cause}"))
        } else {
            Self::context(message, cause)
        }
    }
    pub(crate) fn operation_indeterminate(cause: impl std::fmt::Display) -> Self {
        let mut failure = Failure::new(
            FailureKind::Unavailable,
            format!("运行时操作结果不确定，请先查看会话，不要重复提交：{cause}"),
        );
        failure.code = "runtime_operation_indeterminate".into();
        Self::Contract(failure)
    }
    pub(crate) fn operation_completed(cause: impl std::fmt::Display) -> Self {
        let mut failure = Failure::new(
            FailureKind::Unavailable,
            format!("运行时操作已完成，但刷新结果失败；请先查看会话，不要重复提交：{cause}"),
        );
        failure.code = "runtime_operation_completed".into();
        Self::Contract(failure)
    }
    pub(crate) fn delivery_accepted(&self) -> bool {
        matches!(self, Self::Contract(failure) if failure.code == "message_delivery_accepted")
    }
    pub(crate) fn invalid(message: &'static str) -> Self {
        Self::Invalid(message.into())
    }
}
pub struct Application {
    composer: composer::ComposerInputs,
    runtime: local::CoreRuntime,
    analytics: std::sync::Arc<analytics::AnalyticsStore>,
    attachment_directory: PathBuf,
    uploaded_attachments: std::collections::BTreeMap<String, (ClientViewId, PathBuf)>,
}
impl Application {
    pub fn open(options: Options) -> Result<Self, Error> {
        options.validate()?;
        let home_lock = repository::Repository::lock_home(&options.home_directory)?;
        persistence_migrations::prepare(&options.home_directory)?;
        let analytics = std::sync::Arc::new(
            analytics::AnalyticsStore::open(&options.home_directory).unwrap_or_else(|error| {
                analytics::AnalyticsStore::unavailable(&options.home_directory, error)
            }),
        );
        let attachment_directory = options.home_directory.join("remote-attachments");
        let composer_home = options.home_directory.clone();
        let runtime = local::CoreRuntime::open(options, analytics.clone(), home_lock)?;
        let composer = composer::ComposerInputs::open(&composer_home)?;
        Ok(Self {
            composer,
            attachment_directory,
            uploaded_attachments: Default::default(),
            runtime,
            analytics,
        })
    }
    /// Read a cached ChatGPT provider account report or request a background refresh.
    /// This information is informational and never affects model admission.
    pub fn provider_account(
        &self,
        gateway_id: &str,
        provider_id: &str,
        refresh: bool,
    ) -> Result<ProviderAccountStatus, Error> {
        self.runtime
            .provider_account(gateway_id, provider_id, refresh)
    }
    /// Close unless a conversation's turn or its sub-agents still run.
    pub fn close(&mut self) -> Result<(), Error> {
        self.runtime.close_if_idle(false)?;
        let _ = self.analytics.barrier();
        Ok(())
    }
    /// Close, ending running turns and sub-agents with their runtime processes.
    pub fn close_stopping_work(&mut self) -> Result<(), Error> {
        self.runtime.close_if_idle(true)?;
        let _ = self.analytics.barrier();
        Ok(())
    }
}

fn new_record_key() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    format!(
        "model_record_{time:x}_{:x}",
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        use FailureKind as Kind;
        let mut failure = match error {
            Error::Contract(failure) => return failure,
            Error::Invalid(message) => Failure::new(Kind::Invalid, message),
            Error::Unsupported(message) => Failure::new(Kind::Unsupported, message),
            Error::OutstandingWork => Failure::new(
                Kind::OutstandingWork,
                "子 Agent 仍在运行；请等待其结束，或先停止子 Agent",
            ),
            Error::Busy => {
                let mut failure = Failure::new(
                    Kind::Invalid,
                    "会话正在运行；请等当前 turn 结束后重试".to_owned(),
                );
                failure.code = "busy".into();
                return failure;
            }
            Error::Io(error) => Failure::new(Kind::Io, error.to_string()),
            Error::Json(error) => Failure::new(Kind::Contract, error.to_string()),
            Error::History(error) => {
                let mut f = Failure::new(Kind::History, error.full_message());
                f.code = error.code().into();
                f.phase = error.phase().into();
                return f;
            }
            Error::ProviderImport(error) => {
                let mut f = Failure::new(Kind::ProviderImport, error.to_string());
                f.code = error.code().into();
                f.phase = error.phase().into();
                return f;
            }
            Error::RuntimeManagement(error) => {
                let mut f = Failure::new(Kind::RuntimeManagement, error.to_string());
                f.code = match error.code() {
                    RuntimeManagementErrorCode::Conflict => "conflict",
                    RuntimeManagementErrorCode::Missing => "missing",
                    RuntimeManagementErrorCode::Invalid => "invalid",
                    RuntimeManagementErrorCode::Unsupported => "unsupported",
                    RuntimeManagementErrorCode::Unavailable => "unavailable",
                }
                .into();
                f.phase = "runtime_management".into();
                return f;
            }
        };
        failure.code = match failure.kind {
            Kind::Invalid => "invalid",
            Kind::Unsupported => "unsupported",
            Kind::OutstandingWork => "outstanding_work",
            Kind::Io => "io",
            Kind::Contract => "contract",
            _ => "application_failed",
        }
        .into();
        failure
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;

    #[test]
    fn busy_preserves_its_code_instead_of_generic_invalid() {
        let busy = Failure::from(Error::Busy);
        assert_eq!(busy.kind, FailureKind::Invalid);
        assert_eq!(busy.code, "busy");
        assert_eq!(Failure::from(Error::invalid("local validation")).code, "invalid");
    }
}
