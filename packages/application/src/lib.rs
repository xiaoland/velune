//! Shared application use cases, ordinary configuration and lifecycle.
mod acp;
pub use acp::{
    AcpAgentInfo, AcpAuthMethod, AcpCapabilities, AcpConfigChoice, AcpConfigOption, AcpConfigValue,
    AcpLaunch, AcpRuntimeInfo,
};
mod analytics;
mod commands;
mod service;
pub use service::ApplicationService;
mod api;
#[cfg(feature = "local-runtime")]
mod authentication_resolver;
#[cfg(feature = "local-runtime")]
mod claude_cli;
#[cfg(feature = "local-runtime")]
mod credential_helper;
mod provider_authentication;
#[cfg(feature = "local-runtime")]
mod provider_login;
#[cfg(feature = "local-runtime")]
mod subscription_auth;
#[cfg(feature = "local-runtime")]
mod subscription_quota;
pub use provider_authentication::{
    AuthenticationEdit, AuthenticationMethod, AuthenticationProvenance,
    CODEX_SUBSCRIPTION_ENDPOINT, CODEX_SUBSCRIPTION_PROTOCOL, ProviderAuthenticationDescription,
    ProviderAuthenticationSource, SubscriptionDisclosure,
};
pub use velune_application_api::{ProviderTemplate, ProviderTemplateKind, provider_templates};
pub mod config;
#[cfg(feature = "local-runtime")]
mod local;
pub mod model_catalog;
#[cfg(not(feature = "local-runtime"))]
mod portable;
mod provider_configuration;
#[cfg(feature = "local-runtime")]
mod provider_import;
mod repository;
mod runtime_execution;
pub use runtime_execution::{RuntimeExecution, RuntimeHome, RuntimeTransport};
mod resource_library;
pub use resource_library::{
    BindingResolution, BoundEntry, ResourceBinding, ResourceDriftFile, ResourceSaveResult,
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
#[cfg(feature = "local-runtime")]
pub use velune_agent_runtime::management::ErrorCode as RuntimeManagementErrorCode;
#[cfg(feature = "local-runtime")]
mod runtime_discovery;
pub use api::*;
pub use velune_application_api::{
    ApplicationReply, AttachmentContent, AttachmentReference, ClientViewId, Command,
    ExecutionContext, Failure, FailureKind, Response, Revision,
};
mod transcript_delivery;
#[cfg(feature = "local-runtime")]
use local as implementation;
#[cfg(not(feature = "local-runtime"))]
use portable as implementation;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
pub use transcript_delivery::{
    DeliveredBlock, DeliveredImageSource, DeliveredMessage, DeliveredSnapshot, MessagePayload,
    SnapshotUpdate, TranscriptCursor,
};
pub use velune_conversation as conversation;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
    #[cfg(feature = "local-runtime")]
    #[error("{0}")]
    ProviderImport(#[from] velune_agent_runtime::provider_source::SourceReadError),
    #[cfg(feature = "local-runtime")]
    #[error("{0}")]
    History(#[from] velune_agent_runtime::history::HistoryError),
    #[cfg(feature = "local-runtime")]
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
    #[error("application I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("application JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}
impl Error {
    /// Retain the actual boundary failure in local diagnostics rather than replacing
    /// it with the operation's summary.
    #[cfg(feature = "local-runtime")]
    pub(crate) fn context(message: &str, cause: impl std::fmt::Display) -> Self {
        Self::Invalid(format!("{message}：{cause}"))
    }

    pub(crate) fn invalid(message: &'static str) -> Self {
        Self::Invalid(message.into())
    }
}
pub struct Application {
    runtime: implementation::CoreRuntime,
    analytics: std::sync::Arc<analytics::AnalyticsStore>,
    attachment_directory: PathBuf,
    uploaded_attachments: std::collections::BTreeMap<String, (ClientViewId, PathBuf)>,
}
impl Application {
    pub fn open(options: Options) -> Result<Self, Error> {
        options.validate()?;
        let analytics = std::sync::Arc::new(
            analytics::AnalyticsStore::open(&options.home_directory).unwrap_or_else(|error| {
                analytics::AnalyticsStore::unavailable(&options.home_directory, error)
            }),
        );
        let attachment_directory = options.home_directory.join("remote-attachments");
        Ok(Self {
            attachment_directory,
            uploaded_attachments: Default::default(),
            runtime: implementation::CoreRuntime::open(options, analytics.clone())?,
            analytics,
        })
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
            Error::Io(error) => Failure::new(Kind::Io, error.to_string()),
            Error::Json(error) => Failure::new(Kind::Contract, error.to_string()),
            #[cfg(feature = "local-runtime")]
            Error::History(error) => {
                let mut f = Failure::new(Kind::History, error.full_message());
                f.code = error.code().into();
                f.phase = error.phase().into();
                return f;
            }
            #[cfg(feature = "local-runtime")]
            Error::ProviderImport(error) => {
                let mut f = Failure::new(Kind::ProviderImport, error.to_string());
                f.code = error.code().into();
                f.phase = error.phase().into();
                return f;
            }
            #[cfg(feature = "local-runtime")]
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
