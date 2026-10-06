//! Typed foreign-language entry point for the shared application unit.
//!
//! Each object owns one application session. Explicit close refuses a busy
//! runtime and leaves the object usable. Dropping a language wrapper is resource
//! cleanup, not a user-facing request to cancel an Agent turn.
mod diagnostics;
mod types;
pub use types::*;

use serde::{Serialize, de::DeserializeOwned};
use std::sync::{Arc, Mutex, MutexGuard};
use velune_application::{Application, Error, Options};

uniffi::setup_scaffolding!();

#[derive(Debug, Clone, uniffi::Enum)]
pub enum BindingFailureKind {
    Invalid,
    Unsupported,
    Io,
    Contract,
    History,
    Closed,
    Unavailable,
    ProviderImport,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum BindingError {
    #[error("{detail}（诊断编号：{operation_id}）")]
    Diagnostic {
        kind: BindingFailureKind,
        detail: String,
        code: String,
        phase: String,
        operation_id: String,
    },
    #[error("{detail}")]
    Invalid { detail: String },
    #[error("{detail}")]
    Unsupported { detail: String },
    #[error("{detail}")]
    Io { detail: String },
    #[error("{detail}")]
    Contract { detail: String },
    #[error("application has been closed")]
    Closed,
    #[error("application state is unavailable")]
    Unavailable,
}

impl From<Error> for BindingError {
    fn from(error: Error) -> Self {
        // The application error's typed variants are preserved at this boundary.
        match error {
            Error::Invalid(message) => Self::Invalid { detail: message },
            Error::Unsupported(message) => Self::Unsupported { detail: message },
            Error::Io(error) => Self::Io {
                detail: error.to_string(),
            },
            #[cfg(feature = "local-runtime")]
            Error::ProviderImport(error) => Self::Diagnostic {
                kind: BindingFailureKind::ProviderImport,
                detail: format!(
                    "{}（阶段：{}，类别：{}{}{}）",
                    error.message(),
                    error.phase(),
                    error.code(),
                    error
                        .exit_status()
                        .map(|status| format!("，退出状态：{status}"))
                        .unwrap_or_default(),
                    if error.detail().is_empty() {
                        String::new()
                    } else {
                        format!("，原因：{}", error.detail())
                    }
                ),
                code: error.code().into(),
                phase: error.phase().into(),
                operation_id: String::new(),
            },
            #[cfg(feature = "local-runtime")]
            Error::History(error) => Self::Diagnostic {
                kind: BindingFailureKind::History,
                detail: error.full_message(),
                code: error.code().into(),
                phase: error.phase().into(),
                operation_id: String::new(),
            },
            Error::Json(error) => Self::Contract {
                detail: error.to_string(),
            },
        }
    }
}

fn convert<I: Serialize, O: DeserializeOwned>(input: I) -> Result<O, BindingError> {
    let value = serde_json::to_value(input).map_err(|error| BindingError::Contract {
        detail: error.to_string(),
    })?;
    serde_json::from_value(value).map_err(|error| BindingError::Contract {
        detail: error.to_string(),
    })
}

#[derive(Clone, uniffi::Record)]
pub struct BindingOptions {
    pub home_directory: String,
    pub resources_directory: String,
}

#[derive(uniffi::Object)]
pub struct VeluneApplication {
    application: Mutex<Option<Application>>,
    diagnostics: diagnostics::Diagnostics,
}

impl VeluneApplication {
    fn lock(&self) -> Result<MutexGuard<'_, Option<Application>>, BindingError> {
        self.application
            .lock()
            .map_err(|_| BindingError::Unavailable)
    }
    fn with<T>(
        &self,
        name: &'static str,
        operation: impl FnOnce(&mut Application) -> Result<T, Error>,
    ) -> Result<T, BindingError> {
        self.diagnostics.run(name, || {
            let mut state = self.lock()?;
            operation(state.as_mut().ok_or(BindingError::Closed)?).map_err(Into::into)
        })
    }
}

#[uniffi::export]
impl VeluneApplication {
    #[uniffi::constructor]
    pub fn open(options: BindingOptions) -> Result<Arc<Self>, BindingError> {
        if !std::path::Path::new(&options.home_directory).is_absolute() {
            return Err(BindingError::Invalid {
                detail: "应用配置根目录必须是绝对路径".into(),
            });
        }
        let diagnostics =
            diagnostics::Diagnostics::open(std::path::Path::new(&options.home_directory))?;
        let application = diagnostics.run("open", || {
            Application::open(Options {
                home_directory: options.home_directory.into(),
                resources_directory: options.resources_directory.into(),
            })
            .map_err(Into::into)
        })?;
        Ok(Arc::new(Self {
            application: Mutex::new(Some(application)),
            diagnostics,
        }))
    }

    /// Close only when the application allows it. A failed close retains all
    /// resources and remains available for polling or an explicit Agent cancel.
    pub fn shutdown(&self) -> Result<(), BindingError> {
        self.diagnostics.run("shutdown", || {
            let mut state = self.lock()?;
            if let Some(application) = state.as_mut() {
                application.close()?;
                *state = None;
            }
            Ok(())
        })
    }

    pub fn list(&self) -> Result<BindingConfigurationSnapshot, BindingError> {
        convert(self.with("list", |application| application.list())?)
    }
    pub fn analytics_query(
        &self,
        query: BindingAnalyticsQuery,
    ) -> Result<BindingAnalyticsReport, BindingError> {
        let query = convert(query)?;
        convert(self.with("analytics_query", |application| {
            application.analytics_query(query)
        })?)
    }
    pub fn runtime_discovery_hints(
        &self,
        user_home: String,
        overrides: std::collections::HashMap<String, String>,
    ) -> Result<Vec<BindingRuntimeDiscoveryHint>, BindingError> {
        convert(self.with("runtime_discovery_hints", |app| {
            app.runtime_discovery_hints(user_home, overrides.into_iter().collect())
        })?)
    }
    pub fn discover_runtimes(
        &self,
        probes: Vec<BindingRuntimeDiscoveryProbe>,
    ) -> Result<Vec<BindingRuntimeDiscoveryCandidate>, BindingError> {
        let probes = convert(probes)?;
        convert(self.with("discover_runtimes", |app| app.discover_runtimes(probes))?)
    }
    pub fn set_transcript_presentation(
        &self,
        presentation: BindingTranscriptPresentation,
    ) -> Result<BindingTranscriptPresentation, BindingError> {
        let presentation = convert(presentation)?;
        convert(self.with("set_transcript_presentation", |app| {
            app.set_transcript_presentation(presentation)
        })?)
    }
    pub fn set_conversation_browser_group_limit(&self, limit: u32) -> Result<u32, BindingError> {
        self.with("set_conversation_browser_group_limit", |app| {
            app.set_conversation_browser_group_limit(limit)
        })
    }

    pub fn save_provider(
        &self,
        gateway_id: String,
        provider: BindingProviderDraft,
        authentication_edit: BindingAuthenticationEdit,
    ) -> Result<BindingGatewayUpdate, BindingError> {
        let provider = convert(provider)?;
        let authentication_edit = convert(authentication_edit)?;
        convert(self.with("save_provider", |app| {
            app.save_provider(gateway_id, provider, authentication_edit)
        })?)
    }
    pub fn delete_provider(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<BindingGatewayUpdate, BindingError> {
        convert(self.with("delete_provider", |app| {
            app.delete_provider(gateway_id, provider_id)
        })?)
    }
    pub fn read_provider_api_key(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<String, BindingError> {
        self.with("read_provider_api_key", |app| {
            app.read_provider_api_key(gateway_id, provider_id)
        })
    }
    pub fn save_model_template(
        &self,
        template: BindingModelTemplate,
    ) -> Result<Vec<BindingModelTemplate>, BindingError> {
        let template = convert(template)?;
        convert(self.with("save_model_template", |app| {
            app.save_model_template(template)
        })?)
    }
    /// Explicit, credential-free retrieval; does not change application configuration.
    pub fn fetch_public_model_catalog(&self) -> Result<Vec<BindingCatalogModel>, BindingError> {
        convert(self.with("fetch_public_model_catalog", |_app| {
            velune_application::model_catalog::fetch_models_dev()
        })?)
    }
    pub fn delete_model_template(
        &self,
        id: String,
    ) -> Result<Vec<BindingModelTemplate>, BindingError> {
        convert(self.with("delete_model_template", |app| app.delete_model_template(id))?)
    }

    pub fn upsert_runtime(
        &self,
        runtime: BindingRuntimeInstance,
    ) -> Result<BindingRuntimeUpdate, BindingError> {
        let runtime = convert(runtime)?;
        convert(self.with("upsert_runtime", |application| {
            application.upsert_runtime(runtime)
        })?)
    }

    pub fn delete_runtime(&self, id: String) -> Result<BindingRuntimeUpdate, BindingError> {
        convert(self.with("delete_runtime", |application| {
            application.delete_runtime(id)
        })?)
    }

    pub fn select_runtime(&self, id: String) -> Result<BindingConfigurationSnapshot, BindingError> {
        convert(self.with("select_runtime", |application| {
            application.select_runtime(id)
        })?)
    }

    pub fn create_conversation(
        &self,
        runtime_id: String,
        cwd: String,
        model_record_key: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with("create_conversation", |application| {
            application.create_conversation(runtime_id, cwd, model_record_key)
        })?)
    }

    pub fn open_conversation(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with("open_conversation", |application| {
            application.open_conversation(runtime_id, conversation_id)
        })?)
    }

    pub fn rename_conversation(
        &self,
        runtime_id: String,
        conversation_id: String,
        title: String,
    ) -> Result<BindingConfigurationSnapshot, BindingError> {
        convert(self.with("rename_conversation", |application| {
            application.rename_conversation(runtime_id, conversation_id, title)
        })?)
    }
    pub fn delete_conversation(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingConfigurationSnapshot, BindingError> {
        convert(self.with("delete_conversation", |application| {
            application.delete_conversation(runtime_id, conversation_id)
        })?)
    }
    pub fn snapshot(&self, runtime_id: String) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with("snapshot", |application| application.snapshot(runtime_id))?)
    }

    pub fn send_turn(
        &self,
        runtime_id: String,
        model_record_key: String,
        text: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with("send_turn", |application| {
            application.send_turn(runtime_id, model_record_key, text)
        })?)
    }

    pub fn cancel(&self, runtime_id: String) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with("cancel", |application| application.cancel(runtime_id))?)
    }

    pub fn reply_runtime_interaction(
        &self,
        runtime_id: String,
        interaction_id: String,
        reply: BindingRuntimeInteractionReply,
    ) -> Result<BindingSnapshotResult, BindingError> {
        let reply = convert(reply)?;
        convert(self.with("reply_runtime_interaction", |app| {
            app.reply_runtime_interaction(runtime_id, interaction_id, reply)
        })?)
    }
    pub fn preview_provider_import(
        &self,
        gateway_id: String,
        source: BindingProviderImportSource,
    ) -> Result<BindingProviderImportPreview, BindingError> {
        let source = convert(source)?;
        convert(self.with("preview_provider_import", |application| {
            application.preview_provider_import(gateway_id, source)
        })?)
    }

    pub fn apply_provider_import(
        &self,
        gateway_id: String,
        source: BindingProviderImportSource,
        preview_token: String,
        selections: Vec<BindingImportSelection>,
        replace_existing: bool,
    ) -> Result<BindingImportResult, BindingError> {
        let source = convert(source)?;
        let selections = convert(selections)?;
        convert(self.with("apply_provider_import", |application| {
            application.apply_provider_import(
                gateway_id,
                source,
                preview_token,
                selections,
                replace_existing,
            )
        })?)
    }

    pub fn authentication_inspect(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<BindingAuthenticationMetadata, BindingError> {
        convert(self.with("authentication_inspect", |app| {
            app.authentication_inspect(gateway_id, provider_id)
        })?)
    }
    pub fn authentication_start(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<BindingAuthenticationProgress, BindingError> {
        convert(self.with("authentication_start", |app| {
            app.authentication_start(gateway_id, provider_id)
        })?)
    }

    pub fn authentication_poll(&self) -> Result<BindingAuthenticationProgress, BindingError> {
        convert(self.with("authentication_poll", |application| {
            application.authentication_poll()
        })?)
    }

    pub fn authentication_reply(
        &self,
        prompt_id: String,
        value: String,
    ) -> Result<BindingAuthenticationProgress, BindingError> {
        convert(self.with("authentication_reply", |application| {
            application.authentication_reply(prompt_id, value)
        })?)
    }

    pub fn authentication_cancel(&self) -> Result<BindingAuthenticationProgress, BindingError> {
        convert(self.with("authentication_cancel", |application| {
            application.authentication_cancel()
        })?)
    }
}

impl BindingError {
    fn diagnostic_parts(self) -> (BindingFailureKind, String, String, String) {
        let phase = "application".to_owned();
        match self {
            Self::Diagnostic {
                kind,
                code,
                phase,
                detail,
                ..
            } => (kind, code, phase, detail),
            Self::Invalid { detail } => (
                BindingFailureKind::Invalid,
                "invalid_operation".into(),
                phase,
                detail,
            ),
            Self::Unsupported { detail } => (
                BindingFailureKind::Unsupported,
                "unsupported_operation".into(),
                phase,
                detail,
            ),
            Self::Io { detail } => (BindingFailureKind::Io, "io_failed".into(), phase, detail),
            Self::Contract { detail } => (
                BindingFailureKind::Contract,
                "contract_failed".into(),
                phase,
                detail,
            ),
            Self::Closed => (
                BindingFailureKind::Closed,
                "application_closed".into(),
                phase,
                "本地核心已关闭".into(),
            ),
            Self::Unavailable => (
                BindingFailureKind::Unavailable,
                "application_unavailable".into(),
                phase,
                "本地核心不可用".into(),
            ),
        }
    }
}
