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
    RuntimeManagement,
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
            error @ Error::OutstandingWork => Self::Invalid {
                detail: error.to_string(),
            },
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
            #[cfg(feature = "local-runtime")]
            Error::RuntimeManagement(error) => Self::Diagnostic {
                kind: BindingFailureKind::RuntimeManagement,
                detail: error.to_string(),
                code: match error.code() {
                    velune_application::RuntimeManagementErrorCode::Conflict => "conflict",
                    velune_application::RuntimeManagementErrorCode::Missing => "missing",
                    velune_application::RuntimeManagementErrorCode::Invalid => "invalid",
                    velune_application::RuntimeManagementErrorCode::Unsupported => "unsupported",
                    velune_application::RuntimeManagementErrorCode::Unavailable => "unavailable",
                }
                .into(),
                phase: "runtime_management".into(),
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

/// What a new provider starts from: each subscription, then an API provider.
/// Present them before the provider exists, with each template's disclosure.
#[uniffi::export]
pub fn provider_templates() -> Result<Vec<BindingProviderTemplate>, BindingError> {
    convert(velune_application::provider_templates())
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
    /// Refused while a conversation's turn or its sub-agents run.
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

    /// Ends running turns and sub-agents with their runtime processes.
    pub fn shutdown_stopping_work(&self) -> Result<(), BindingError> {
        self.diagnostics.run("shutdown_stopping_work", || {
            let mut state = self.lock()?;
            if let Some(application) = state.as_mut() {
                application.close_stopping_work()?;
                *state = None;
            }
            Ok(())
        })
    }

    pub fn list(&self) -> Result<BindingConfigurationSnapshot, BindingError> {
        convert(self.with("list", |application| application.list())?)
    }
    pub fn runtime_resource_catalog(
        &self,
        runtime_id: String,
        cwd: Option<String>,
        kind: BindingRuntimeResourceKind,
        scope: BindingRuntimeResourceScope,
    ) -> Result<BindingRuntimeResourceCatalog, BindingError> {
        let kind = convert(kind)?;
        let scope = convert(scope)?;
        convert(self.with("runtime_resource_catalog", |application| {
            application.runtime_resource_catalog(runtime_id, cwd, kind, scope)
        })?)
    }
    pub fn read_runtime_resource(
        &self,
        runtime_id: String,
        cwd: Option<String>,
        key: BindingRuntimeResourceKey,
    ) -> Result<BindingRuntimeResourceDocument, BindingError> {
        let key = convert(key)?;
        convert(self.with("read_runtime_resource", |application| {
            application.read_runtime_resource(runtime_id, cwd, key)
        })?)
    }
    pub fn write_runtime_resource(
        &self,
        runtime_id: String,
        cwd: Option<String>,
        key: BindingRuntimeResourceKey,
        expected: BindingRuntimeResourceRevision,
        bytes: Vec<u8>,
        executable: Option<bool>,
    ) -> Result<BindingRuntimeResourceDocument, BindingError> {
        let key = convert(key)?;
        let expected = convert(expected)?;
        convert(self.with("write_runtime_resource", |application| {
            application.write_runtime_resource(runtime_id, cwd, key, expected, bytes, executable)
        })?)
    }
    pub fn delete_runtime_resource(
        &self,
        runtime_id: String,
        cwd: Option<String>,
        key: BindingRuntimeResourceKey,
        expected: BindingRuntimeResourceRevision,
    ) -> Result<(), BindingError> {
        let key = convert(key)?;
        let expected = convert(expected)?;
        self.with("delete_runtime_resource", |application| {
            application.delete_runtime_resource(runtime_id, cwd, key, expected)
        })
    }
    pub fn list_resource_templates(
        &self,
    ) -> Result<Vec<BindingResourceTemplateSummary>, BindingError> {
        convert(self.with("list_resource_templates", |application| {
            application.list_resource_templates()
        })?)
    }
    pub fn read_resource_template(
        &self,
        template_id: String,
    ) -> Result<BindingResourceTemplate, BindingError> {
        convert(self.with("read_resource_template", |application| {
            application.read_resource_template(template_id)
        })?)
    }
    pub fn save_resource_template(
        &self,
        template: BindingResourceTemplate,
    ) -> Result<BindingResourceSaveResult, BindingError> {
        let template = convert(template)?;
        convert(self.with("save_resource_template", |application| {
            application.save_resource_template(template)
        })?)
    }
    pub fn delete_resource_template(&self, template_id: String) -> Result<(), BindingError> {
        self.with("delete_resource_template", |application| {
            application.delete_resource_template(template_id)
        })
    }
    pub fn import_runtime_resource(
        &self,
        runtime_id: String,
        kind: BindingRuntimeResourceKind,
        path: String,
    ) -> Result<BindingResourceTemplateImport, BindingError> {
        let kind = convert(kind)?;
        convert(self.with("import_runtime_resource", |application| {
            application.import_runtime_resource(runtime_id, kind, path)
        })?)
    }
    pub fn adopt_runtime_resource(
        &self,
        runtime_id: String,
        kind: BindingRuntimeResourceKind,
        path: String,
    ) -> Result<BindingResourceTemplateSummary, BindingError> {
        let kind = convert(kind)?;
        convert(self.with("adopt_runtime_resource", |application| {
            application.adopt_runtime_resource(runtime_id, kind, path)
        })?)
    }
    pub fn resource_sync_status(&self) -> Result<Vec<BindingResourceSyncStatus>, BindingError> {
        convert(self.with("resource_sync_status", |application| {
            application.resource_sync_status()
        })?)
    }
    pub fn resource_drift(
        &self,
        runtime_id: String,
        template_id: String,
    ) -> Result<Vec<BindingResourceDriftFile>, BindingError> {
        convert(self.with("resource_drift", |application| {
            application.resource_drift(runtime_id, template_id)
        })?)
    }
    pub fn unmanaged_runtime_resources(
        &self,
        kind: BindingRuntimeResourceKind,
    ) -> Result<Vec<BindingUnmanagedRuntimeResource>, BindingError> {
        let kind = convert(kind)?;
        convert(self.with("unmanaged_runtime_resources", |application| {
            application.unmanaged_runtime_resources(kind)
        })?)
    }
    pub fn apply_resource_template(
        &self,
        template_id: String,
        targets: Vec<BindingTemplateApplyTarget>,
    ) -> Result<Vec<BindingTemplateApplyReport>, BindingError> {
        let targets = convert(targets)?;
        convert(self.with("apply_resource_template", |application| {
            application.apply_resource_template(template_id, targets)
        })?)
    }
    pub fn resource_native_import(
        &self,
        runtime_id: String,
        template_id: String,
    ) -> Result<BindingResourceTemplateImport, BindingError> {
        convert(self.with("resource_native_import", |application| {
            application.resource_native_import(runtime_id, template_id)
        })?)
    }
    pub fn resolve_resource_binding(
        &self,
        runtime_id: String,
        template_id: String,
        resolution: BindingResolution,
    ) -> Result<(), BindingError> {
        let resolution = convert(resolution)?;
        self.with("resolve_resource_binding", |application| {
            application.resolve_resource_binding(runtime_id, template_id, resolution)
        })
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
    pub fn set_outline_presentation(
        &self,
        presentation: BindingOutlinePresentation,
    ) -> Result<BindingOutlinePresentation, BindingError> {
        let presentation = convert(presentation)?;
        convert(self.with("set_outline_presentation", |app| {
            app.set_outline_presentation(presentation)
        })?)
    }
    pub fn set_linked_conversation_opening(
        &self,
        opening: BindingLinkedConversationOpening,
    ) -> Result<BindingLinkedConversationOpening, BindingError> {
        let opening = convert(opening)?;
        convert(self.with("set_linked_conversation_opening", |app| {
            app.set_linked_conversation_opening(opening)
        })?)
    }
    pub fn set_usage_display(
        &self,
        display: BindingUsageDisplay,
    ) -> Result<BindingUsageDisplay, BindingError> {
        let display = convert(display)?;
        convert(self.with("set_usage_display", |app| app.set_usage_display(display))?)
    }
    pub fn set_status_line(
        &self,
        items: Vec<BindingStatusLineItem>,
    ) -> Result<Vec<BindingStatusLineItem>, BindingError> {
        let items = convert(items)?;
        convert(self.with("set_status_line", |app| app.set_status_line(items))?)
    }
    pub fn set_conversation_browser_preferences(
        &self,
        preferences: BindingConversationBrowserPreferences,
    ) -> Result<BindingConfigurationSnapshot, BindingError> {
        let preferences = convert(preferences)?;
        convert(self.with("set_conversation_browser_preferences", |app| {
            app.set_conversation_browser_preferences(preferences)
        })?)
    }
    pub fn composer_drafts(&self) -> Result<Vec<BindingComposerDraft>, BindingError> {
        convert(self.with("composer_drafts", |app| app.composer_drafts())?)
    }
    pub fn set_composer_draft(
        &self,
        conversation_id: String,
        text: String,
    ) -> Result<(), BindingError> {
        self.with("set_composer_draft", |app| {
            app.set_composer_draft(conversation_id, text)
        })
    }
    pub fn set_conversation_browser_limits(
        &self,
        limits: BindingConversationBrowserLimits,
    ) -> Result<BindingConversationBrowserLimits, BindingError> {
        let limits = convert(limits)?;
        convert(self.with("set_conversation_browser_limits", |app| {
            app.set_conversation_browser_limits(limits)
        })?)
    }
    /// Changes Velune's organization of the conversations; the runtime's own
    /// archive is used where it has one.
    pub fn organize_conversations(
        &self,
        conversations: Vec<BindingConversationReference>,
        change: BindingConversationOrganizationChange,
    ) -> Result<BindingConfigurationSnapshot, BindingError> {
        let conversations = convert(conversations)?;
        let change = convert(change)?;
        convert(self.with("organize_conversations", |app| {
            app.organize_conversations(conversations, change)
        })?)
    }
    /// Shapes the project whose primary folder is `path`: its name (none
    /// shows the folder's), primary folder, and other folders.
    pub fn set_project(
        &self,
        path: String,
        primary_folder: String,
        name: Option<String>,
        folders: Vec<String>,
    ) -> Result<BindingConfigurationSnapshot, BindingError> {
        convert(self.with("set_project", |app| {
            app.set_project(path, primary_folder, name, folders)
        })?)
    }
    pub fn load_more_conversations(
        &self,
        group: BindingConversationGroupKey,
    ) -> Result<BindingConfigurationSnapshot, BindingError> {
        let group = convert(group)?;
        convert(self.with("load_more_conversations", |app| {
            app.load_more_conversations(group)
        })?)
    }
    pub fn show_archived_conversations(
        &self,
        archived: bool,
    ) -> Result<BindingConfigurationSnapshot, BindingError> {
        convert(self.with("show_archived_conversations", |app| {
            app.show_archived_conversations(archived)
        })?)
    }
    pub fn search_conversations(
        &self,
        text: String,
    ) -> Result<BindingConversationSearch, BindingError> {
        convert(self.with("search_conversations", |app| app.search_conversations(text))?)
    }
    /// One conversation by identity, whether or not the list shows it.
    pub fn find_conversation(
        &self,
        conversation_id: String,
    ) -> Result<Option<BindingListedConversation>, BindingError> {
        convert(self.with("find_conversation", |app| {
            app.find_conversation(conversation_id)
        })?)
    }

    pub fn provider_quota(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<BindingProviderQuotaStatus, BindingError> {
        convert(self.with("provider_quota", |app| {
            app.provider_quota(gateway_id, provider_id)
        })?)
    }
    pub fn refresh_provider_quota(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<BindingProviderQuotaStatus, BindingError> {
        convert(self.with("refresh_provider_quota", |app| {
            app.refresh_provider_quota(gateway_id, provider_id)
        })?)
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
    pub fn save_model_route(
        &self,
        gateway_id: String,
        route: BindingModelRoute,
    ) -> Result<BindingGatewayUpdate, BindingError> {
        let route = convert(route)?;
        convert(self.with("save_model_route", |app| {
            app.save_model_route(gateway_id, route)
        })?)
    }
    pub fn delete_model_route(
        &self,
        gateway_id: String,
        route_record_key: String,
    ) -> Result<BindingGatewayUpdate, BindingError> {
        convert(self.with("delete_model_route", |app| {
            app.delete_model_route(gateway_id, route_record_key)
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
        cwd: Option<String>,
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
    pub fn read_conversation(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with("read_conversation", |application| {
            application.read_conversation(runtime_id, conversation_id)
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
    pub fn snapshot(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with("snapshot", |application| {
            application.snapshot(runtime_id, conversation_id)
        })?)
    }
    /// Poll a view relative to the cursor of its latest delivery.
    pub fn snapshot_update(
        &self,
        runtime_id: String,
        conversation_id: String,
        since: BindingTranscriptCursor,
    ) -> Result<BindingSnapshotUpdate, BindingError> {
        let since = convert(since)?;
        convert(self.with("snapshot_update", |application| {
            application.snapshot_update(runtime_id, conversation_id, since)
        })?)
    }
    /// Complete reasoning and tool output of delivered messages.
    pub fn message_payloads(
        &self,
        runtime_id: String,
        conversation_id: String,
        message_ids: Vec<String>,
    ) -> Result<Vec<BindingMessage>, BindingError> {
        convert(self.with("message_payloads", |application| {
            application.message_payloads(runtime_id, conversation_id, message_ids)
        })?)
    }

    pub fn send_turn(
        &self,
        source_runtime_id: String,
        conversation_id: String,
        target_runtime_id: String,
        model_record_key: String,
        text: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with("send_turn", |application| {
            application.send_turn(
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                model_record_key,
                text,
            )
        })?)
    }

    pub fn upsert_runtime_with_execution(
        &self,
        runtime: BindingRuntimeInstance,
        execution: BindingRuntimeExecution,
    ) -> Result<BindingRuntimeUpdate, BindingError> {
        let runtime = convert(runtime)?;
        let execution = convert(execution)?;
        convert(self.with("upsert_runtime_with_execution", |application| {
            application.upsert_runtime_with_execution(runtime, execution)
        })?)
    }

    pub fn upsert_acp_runtime(
        &self,
        runtime: BindingRuntimeInstance,
        execution: BindingRuntimeExecution,
        launch: BindingAcpLaunch,
    ) -> Result<BindingRuntimeUpdate, BindingError> {
        let runtime = convert(runtime)?;
        let execution = convert(execution)?;
        let launch = convert(launch)?;
        convert(self.with("upsert_acp_runtime", |application| {
            application.upsert_acp_runtime(runtime, execution, launch)
        })?)
    }
    pub fn acp_launch(&self, runtime_id: String) -> Result<BindingAcpLaunch, BindingError> {
        convert(self.with("acp_launch", |application| {
            application.acp_launch(runtime_id)
        })?)
    }
    pub fn acp_runtime_info(
        &self,
        runtime_id: String,
    ) -> Result<BindingAcpRuntimeInfo, BindingError> {
        convert(self.with("acp_runtime_info", |application| {
            application.acp_runtime_info(runtime_id)
        })?)
    }
    pub fn open_acp_conversation(
        &self,
        runtime_id: String,
        native_session_id: String,
        cwd: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with("open_acp_conversation", |application| {
            application.open_acp_conversation(runtime_id, native_session_id, cwd)
        })?)
    }
    pub fn acp_session_config(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<Vec<BindingAcpConfigOption>, BindingError> {
        convert(self.with("acp_session_config", |application| {
            application.acp_session_config(runtime_id, conversation_id)
        })?)
    }
    pub fn set_acp_session_config(
        &self,
        runtime_id: String,
        conversation_id: String,
        config_id: String,
        value: BindingAcpConfigValue,
    ) -> Result<Vec<BindingAcpConfigOption>, BindingError> {
        let value = convert(value)?;
        convert(self.with("set_acp_session_config", |application| {
            application.set_acp_session_config(runtime_id, conversation_id, config_id, value)
        })?)
    }

    pub fn runtime_execution(
        &self,
        runtime_id: String,
    ) -> Result<BindingRuntimeExecution, BindingError> {
        convert(self.with("runtime_execution", |application| {
            application.runtime_execution(runtime_id)
        })?)
    }

    pub fn runtime_home_directory(
        &self,
        runtime_id: String,
    ) -> Result<Option<String>, BindingError> {
        self.with("runtime_home_directory", |application| {
            application.runtime_home_directory(runtime_id)
        })
    }

    pub fn configure_runtime_execution(
        &self,
        runtime_id: String,
        execution: BindingRuntimeExecution,
    ) -> Result<BindingRuntimeUpdate, BindingError> {
        let execution = convert(execution)?;
        convert(self.with("configure_runtime_execution", |application| {
            application.configure_runtime_execution(runtime_id, execution)
        })?)
    }

    pub fn create_conversation_with_selection(
        &self,
        runtime_id: String,
        cwd: Option<String>,
        selection: BindingModelSelection,
    ) -> Result<BindingSnapshotResult, BindingError> {
        let selection = convert(selection)?;
        convert(
            self.with("create_conversation_with_selection", |application| {
                application.create_conversation_with_selection(runtime_id, cwd, selection)
            })?,
        )
    }

    pub fn send_turn_with_selection(
        &self,
        source_runtime_id: String,
        conversation_id: String,
        target_runtime_id: String,
        selection: BindingModelSelection,
        turn: BindingComposedTurn,
    ) -> Result<BindingSnapshotResult, BindingError> {
        let selection = convert(selection)?;
        let turn = convert(turn)?;
        convert(self.with("send_turn_with_selection", |application| {
            application.send_turn_with_selection(
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                selection,
                turn,
            )
        })?)
    }

    pub fn reasoning_efforts(
        &self,
        runtime_id: String,
        model_record_key: String,
    ) -> Result<Vec<String>, BindingError> {
        self.with("reasoning_efforts", |application| {
            application.reasoning_efforts(runtime_id, model_record_key)
        })
    }

    pub fn runtime_models(
        &self,
        runtime_id: String,
    ) -> Result<Vec<BindingRuntimeModel>, BindingError> {
        convert(self.with("runtime_models", |application| {
            application.runtime_models(runtime_id)
        })?)
    }

    pub fn next_turn_model_selection(&self) -> Result<Option<BindingModelSelection>, BindingError> {
        convert(self.with("next_turn_model_selection", |application| {
            application.next_turn_model_selection()
        })?)
    }

    /// Change the conversation's send queue; one added to or resumed while
    /// no turn runs sends its first message at once.
    pub fn send_queue(
        &self,
        runtime_id: String,
        conversation_id: String,
        operation: BindingSendQueueOperation,
    ) -> Result<BindingSnapshotResult, BindingError> {
        let operation = convert(operation)?;
        convert(self.with("send_queue", |application| {
            application.send_queue(runtime_id, conversation_id, operation)
        })?)
    }

    /// Deliver a message into the running turn through the runtime's own
    /// channel; it does not start a turn.
    pub fn steer_turn(
        &self,
        runtime_id: String,
        conversation_id: String,
        turn: BindingComposedTurn,
    ) -> Result<BindingSnapshotResult, BindingError> {
        let turn = convert(turn)?;
        convert(self.with("steer_turn", |application| {
            application.steer_turn(runtime_id, conversation_id, turn)
        })?)
    }

    /// Send `turn` in place of an editable user message; a fork answers
    /// with the new conversation's snapshot.
    pub fn edit_message(
        &self,
        runtime_id: String,
        conversation_id: String,
        message_id: String,
        placement: BindingEditPlacement,
        selection: BindingModelSelection,
        turn: BindingComposedTurn,
    ) -> Result<BindingSnapshotResult, BindingError> {
        let placement = convert(placement)?;
        let selection = convert(selection)?;
        let turn = convert(turn)?;
        convert(self.with("edit_message", |application| {
            application.edit_message(
                runtime_id,
                conversation_id,
                message_id,
                placement,
                selection,
                turn,
            )
        })?)
    }

    pub fn cancel(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with("cancel", |application| {
            application.cancel(runtime_id, conversation_id)
        })?)
    }

    pub fn stop_sub_agents(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        convert(self.with("stop_sub_agents", |application| {
            application.stop_sub_agents(runtime_id, conversation_id)
        })?)
    }

    /// Uncommitted changes in the conversation's working directory.
    pub fn working_changes(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingWorkingChanges, BindingError> {
        convert(self.with("working_changes", |application| {
            application.working_changes(runtime_id, conversation_id)
        })?)
    }

    /// The branch checked out in the conversation's working directory;
    /// absent outside a repository or on a detached HEAD.
    pub fn working_branch(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<Option<String>, BindingError> {
        self.with("working_branch", |application| {
            application.working_branch(runtime_id, conversation_id)
        })
    }

    /// The diff of one file listed by `working_changes`.
    pub fn file_diff(
        &self,
        runtime_id: String,
        conversation_id: String,
        path: String,
    ) -> Result<BindingFileDiff, BindingError> {
        convert(self.with("file_diff", |application| {
            application.file_diff(runtime_id, conversation_id, path)
        })?)
    }

    /// Conversations executing or waiting for the user across open
    /// sessions, and turns that ended after `since` (0 for all retained).
    pub fn conversation_activity(
        &self,
        since: u64,
    ) -> Result<BindingConversationActivity, BindingError> {
        convert(self.with("conversation_activity", |application| {
            application.conversation_activity(since)
        })?)
    }

    pub fn reply_runtime_interaction(
        &self,
        runtime_id: String,
        conversation_id: String,
        interaction_id: String,
        reply: BindingRuntimeInteractionReply,
    ) -> Result<BindingSnapshotResult, BindingError> {
        let reply = convert(reply)?;
        convert(self.with("reply_runtime_interaction", |app| {
            app.reply_runtime_interaction(runtime_id, conversation_id, interaction_id, reply)
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

    pub fn authentication_logout(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<BindingGatewayUpdate, BindingError> {
        convert(self.with("authentication_logout", |app| {
            app.authentication_logout(gateway_id, provider_id)
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
