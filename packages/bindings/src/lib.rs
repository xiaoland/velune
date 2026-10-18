//! Typed foreign-language entry point for the shared application unit.
//!
//! Local objects retain one execution owner; remote objects retain only their
//! connection. Closing a controller never ends the execution owner or its work.
#[cfg(all(target_os = "android", feature = "remote"))]
mod android;
mod diagnostics;
#[cfg(feature = "remote")]
mod remote;
#[cfg(feature = "remote")]
mod remote_state;
#[cfg(feature = "remote")]
pub use remote_state::*;
#[cfg(feature = "remote")]
mod remote_command;
#[cfg(feature = "remote")]
mod remote_response;
#[cfg(feature = "remote-diagnostics")]
mod transport_probe;
#[cfg(feature = "remote-diagnostics")]
mod transport_trace;
mod types;
#[cfg(feature = "remote")]
pub use remote::*;
#[cfg(feature = "remote")]
pub use remote_command::*;
#[cfg(feature = "remote")]
pub use remote_response::*;
#[cfg(feature = "remote-diagnostics")]
pub use transport_probe::*;
#[cfg(feature = "remote-diagnostics")]
pub use transport_trace::*;
pub use types::*;

use serde::{Serialize, de::DeserializeOwned};
#[cfg(any(feature = "local-runtime", feature = "remote"))]
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard};
#[cfg(feature = "local-runtime")]
use velune_application::{ApplicationService, Error, Options};
#[cfg(any(feature = "local-runtime", feature = "remote"))]
use velune_application_api::ClientViewId;
use velune_application_api::{Command, Failure, Response};

uniffi::setup_scaffolding!();

macro_rules! request {
    ($owner:ident, $name:literal, $variant:ident $( { $($field:ident),* } )?) => {
        match $owner.request($name, Command::$variant $( { $($field),* } )?)? {
            Response::$variant(value) => convert(value),
            _ => Err(BindingError::Contract { detail: "应用返回了不匹配的具名结果".into() }),
        }
    };
    ($owner:ident, $name:literal, $variant:ident $( { $($field:ident),* } )?; unit) => {
        match $owner.request($name, Command::$variant $( { $($field),* } )?)? {
            Response::$variant => Ok(()),
            _ => Err(BindingError::Contract { detail: "应用返回了不匹配的具名结果".into() }),
        }
    };
}

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
    Conflict,
    Unauthorized,
    Connection,
    OutcomeUnknown,
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
    #[cfg(feature = "remote")]
    #[error("{detail}")]
    RemoteRequest {
        kind: BindingFailureKind,
        detail: String,
        code: String,
        request_id: Option<BindingRemoteRequestId>,
    },
    #[error("application has been closed")]
    Closed,
    #[error("application state is unavailable")]
    Unavailable,
}

#[cfg(feature = "local-runtime")]
impl From<Error> for BindingError {
    fn from(error: Error) -> Self {
        Failure::from(error).into()
    }
}

impl From<Failure> for BindingError {
    fn from(error: Failure) -> Self {
        Self::Diagnostic {
            kind: failure_kind(error.kind),
            detail: error.message,
            code: error.code,
            phase: error.phase,
            operation_id: String::new(),
        }
    }
}

fn failure_kind(kind: velune_application_api::FailureKind) -> BindingFailureKind {
    use velune_application_api::FailureKind;
    match kind {
        FailureKind::Invalid | FailureKind::StaleInteraction | FailureKind::OutstandingWork => {
            BindingFailureKind::Invalid
        }
        FailureKind::Unsupported => BindingFailureKind::Unsupported,
        FailureKind::Conflict => BindingFailureKind::Conflict,
        FailureKind::Unavailable | FailureKind::Internal => BindingFailureKind::Unavailable,
        FailureKind::History => BindingFailureKind::History,
        FailureKind::ProviderImport => BindingFailureKind::ProviderImport,
        FailureKind::RuntimeManagement => BindingFailureKind::RuntimeManagement,
        FailureKind::Io => BindingFailureKind::Io,
        FailureKind::Contract => BindingFailureKind::Contract,
        FailureKind::Closed => BindingFailureKind::Closed,
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
    convert(velune_application_api::provider_templates())
}

#[derive(Clone, uniffi::Record)]
pub struct BindingOptions {
    pub home_directory: String,
    pub resources_directory: String,
}

#[derive(Clone)]
enum Backend {
    #[cfg(feature = "local-runtime")]
    Local(ApplicationService),
    #[cfg(feature = "remote")]
    Remote(Arc<remote::RemoteApplication>),
    #[cfg(feature = "remote")]
    ScopedRemote {
        remote: Arc<remote::RemoteApplication>,
        operation: Arc<velune_remote::RemoteOperation>,
    },
}

#[derive(uniffi::Object)]
pub struct VeluneApplication {
    application: Mutex<Option<Backend>>,
    diagnostics: diagnostics::Diagnostics,
    #[cfg(all(feature = "local-runtime", feature = "remote"))]
    remote_service: Mutex<Option<Arc<remote::ServiceOwner>>>,
}

impl VeluneApplication {
    fn lock(&self) -> Result<MutexGuard<'_, Option<Backend>>, BindingError> {
        self.application
            .lock()
            .map_err(|_| BindingError::Unavailable)
    }

    fn request(&self, name: &'static str, command: Command) -> Result<Response, BindingError> {
        self.diagnostics.run(name, || {
            #[cfg(any(feature = "local-runtime", feature = "remote"))]
            {
                let backend = self.lock()?.as_ref().ok_or(BindingError::Closed)?.clone();
                match backend {
                    #[cfg(feature = "local-runtime")]
                    Backend::Local(service) => service
                        .execute(ClientViewId("local".into()), command)
                        .map_err(Into::into),
                    #[cfg(feature = "remote")]
                    Backend::Remote(remote) => remote.request(command, None),
                    #[cfg(feature = "remote")]
                    Backend::ScopedRemote { remote, operation } => {
                        remote.request(command, Some(&operation))
                    }
                }
            }
            #[cfg(not(any(feature = "local-runtime", feature = "remote")))]
            {
                let _ = command;
                Err(BindingError::Unavailable)
            }
        })
    }

    fn close(&self, _stop_work: bool) -> Result<(), BindingError> {
        self.diagnostics.run("shutdown", || {
            let mut state = self.lock()?;
            if let Some(backend) = state.as_mut() {
                match backend {
                    #[cfg(feature = "local-runtime")]
                    Backend::Local(service) => service.close(_stop_work)?,
                    #[cfg(feature = "remote")]
                    Backend::Remote(remote) => remote.close()?,
                    #[cfg(feature = "remote")]
                    Backend::ScopedRemote { .. } => {}
                    #[cfg(not(any(feature = "local-runtime", feature = "remote")))]
                    _ => {}
                }
                *state = None;
                #[cfg(all(feature = "local-runtime", feature = "remote"))]
                if let Some(owner) = self
                    .remote_service
                    .lock()
                    .map_err(|_| BindingError::Unavailable)?
                    .take()
                {
                    owner.close()?;
                }
            }
            Ok(())
        })
    }
}

#[cfg(feature = "local-runtime")]
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
            ApplicationService::open(Options {
                home_directory: options.home_directory.into(),
                resources_directory: options.resources_directory.into(),
            })
            .map_err(Into::into)
        })?;
        Ok(Arc::new(Self {
            application: Mutex::new(Some(Backend::Local(application))),
            diagnostics,
            #[cfg(feature = "remote")]
            remote_service: Mutex::new(None),
        }))
    }
}

#[uniffi::export]
impl VeluneApplication {
    /// Local shutdown retains a busy owner. A remote object only closes its connection.
    pub fn shutdown(&self) -> Result<(), BindingError> {
        self.close(false)
    }

    /// Local shutdown stops work. A remote object only closes its connection;
    /// stopping a remote turn requires its explicit conversation cancellation API.
    pub fn shutdown_stopping_work(&self) -> Result<(), BindingError> {
        self.close(true)
    }

    /// Release only this presentation view; running conversations are retained.
    pub fn release_view(&self) -> Result<(), BindingError> {
        request!(self, "release_view", ReleaseView; unit)
    }

    pub fn list(&self) -> Result<BindingConfigurationSnapshot, BindingError> {
        request!(self, "list", List)
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
        request!(
            self,
            "runtime_resource_catalog",
            RuntimeResourceCatalog {
                runtime_id,
                cwd,
                kind,
                scope
            }
        )
    }
    pub fn read_runtime_resource(
        &self,
        runtime_id: String,
        cwd: Option<String>,
        key: BindingRuntimeResourceKey,
    ) -> Result<BindingRuntimeResourceDocument, BindingError> {
        let key = convert(key)?;
        request!(
            self,
            "read_runtime_resource",
            ReadRuntimeResource {
                runtime_id,
                cwd,
                key
            }
        )
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
        request!(
            self,
            "write_runtime_resource",
            WriteRuntimeResource {
                runtime_id,
                cwd,
                key,
                expected,
                bytes,
                executable
            }
        )
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
        request!(self, "delete_runtime_resource", DeleteRuntimeResource { runtime_id, cwd, key, expected }; unit)
    }
    pub fn list_resource_templates(
        &self,
    ) -> Result<Vec<BindingResourceTemplateSummary>, BindingError> {
        request!(self, "list_resource_templates", ListResourceTemplates)
    }
    pub fn read_resource_template(
        &self,
        template_id: String,
    ) -> Result<BindingResourceTemplate, BindingError> {
        request!(
            self,
            "read_resource_template",
            ReadResourceTemplate { template_id }
        )
    }
    pub fn save_resource_template(
        &self,
        template: BindingResourceTemplate,
    ) -> Result<BindingResourceSaveResult, BindingError> {
        let template = convert(template)?;
        request!(
            self,
            "save_resource_template",
            SaveResourceTemplate { template }
        )
    }
    pub fn delete_resource_template(&self, template_id: String) -> Result<(), BindingError> {
        request!(self, "delete_resource_template", DeleteResourceTemplate { template_id }; unit)
    }
    pub fn import_runtime_resource(
        &self,
        runtime_id: String,
        kind: BindingRuntimeResourceKind,
        path: String,
    ) -> Result<BindingResourceTemplateImport, BindingError> {
        let kind = convert(kind)?;
        request!(
            self,
            "import_runtime_resource",
            ImportRuntimeResource {
                runtime_id,
                kind,
                path
            }
        )
    }
    pub fn adopt_runtime_resource(
        &self,
        runtime_id: String,
        kind: BindingRuntimeResourceKind,
        path: String,
    ) -> Result<BindingResourceTemplateSummary, BindingError> {
        let kind = convert(kind)?;
        request!(
            self,
            "adopt_runtime_resource",
            AdoptRuntimeResource {
                runtime_id,
                kind,
                path
            }
        )
    }
    pub fn resource_sync_status(&self) -> Result<Vec<BindingResourceSyncStatus>, BindingError> {
        request!(self, "resource_sync_status", ResourceSyncStatus)
    }
    pub fn resource_drift(
        &self,
        runtime_id: String,
        template_id: String,
    ) -> Result<Vec<BindingResourceDriftFile>, BindingError> {
        request!(
            self,
            "resource_drift",
            ResourceDrift {
                runtime_id,
                template_id
            }
        )
    }
    pub fn unmanaged_runtime_resources(
        &self,
        kind: BindingRuntimeResourceKind,
    ) -> Result<Vec<BindingUnmanagedRuntimeResource>, BindingError> {
        let kind = convert(kind)?;
        request!(
            self,
            "unmanaged_runtime_resources",
            UnmanagedRuntimeResources { kind }
        )
    }
    pub fn apply_resource_template(
        &self,
        template_id: String,
        targets: Vec<BindingTemplateApplyTarget>,
    ) -> Result<Vec<BindingTemplateApplyReport>, BindingError> {
        let targets = convert(targets)?;
        request!(
            self,
            "apply_resource_template",
            ApplyResourceTemplate {
                template_id,
                targets
            }
        )
    }
    pub fn resource_native_import(
        &self,
        runtime_id: String,
        template_id: String,
    ) -> Result<BindingResourceTemplateImport, BindingError> {
        request!(
            self,
            "resource_native_import",
            ResourceNativeImport {
                runtime_id,
                template_id
            }
        )
    }
    pub fn resolve_resource_binding(
        &self,
        runtime_id: String,
        template_id: String,
        resolution: BindingResolution,
    ) -> Result<(), BindingError> {
        let resolution = convert(resolution)?;
        request!(self, "resolve_resource_binding", ResolveResourceBinding { runtime_id, template_id, resolution }; unit)
    }
    pub fn analytics_query(
        &self,
        query: BindingAnalyticsQuery,
    ) -> Result<BindingAnalyticsReport, BindingError> {
        let query = convert(query)?;
        request!(self, "analytics_query", AnalyticsQuery { query })
    }
    pub fn runtime_discovery_hints(
        &self,
        user_home: String,
        overrides: std::collections::HashMap<String, String>,
    ) -> Result<Vec<BindingRuntimeDiscoveryHint>, BindingError> {
        let overrides = overrides.into_iter().collect();
        request!(
            self,
            "runtime_discovery_hints",
            RuntimeDiscoveryHints {
                user_home,
                overrides
            }
        )
    }
    pub fn discover_runtimes(
        &self,
        probes: Vec<BindingRuntimeDiscoveryProbe>,
    ) -> Result<Vec<BindingRuntimeDiscoveryCandidate>, BindingError> {
        let probes = convert(probes)?;
        request!(self, "discover_runtimes", DiscoverRuntimes { probes })
    }
    pub fn set_outline_presentation(
        &self,
        presentation: BindingOutlinePresentation,
    ) -> Result<BindingOutlinePresentation, BindingError> {
        let presentation = convert(presentation)?;
        request!(
            self,
            "set_outline_presentation",
            SetOutlinePresentation { presentation }
        )
    }
    pub fn set_linked_conversation_opening(
        &self,
        opening: BindingLinkedConversationOpening,
    ) -> Result<BindingLinkedConversationOpening, BindingError> {
        let opening = convert(opening)?;
        request!(
            self,
            "set_linked_conversation_opening",
            SetLinkedConversationOpening { opening }
        )
    }
    pub fn set_usage_display(
        &self,
        display: BindingUsageDisplay,
    ) -> Result<BindingUsageDisplay, BindingError> {
        let display = convert(display)?;
        request!(self, "set_usage_display", SetUsageDisplay { display })
    }
    pub fn set_status_line(
        &self,
        items: Vec<BindingStatusLineItem>,
    ) -> Result<Vec<BindingStatusLineItem>, BindingError> {
        let items = convert(items)?;
        request!(self, "set_status_line", SetStatusLine { items })
    }
    pub fn set_conversation_browser_preferences(
        &self,
        preferences: BindingConversationBrowserPreferences,
    ) -> Result<BindingConfigurationSnapshot, BindingError> {
        let preferences = convert(preferences)?;
        request!(
            self,
            "set_conversation_browser_preferences",
            SetConversationBrowserPreferences { preferences }
        )
    }
    pub fn composer_drafts(&self) -> Result<Vec<BindingComposerDraft>, BindingError> {
        request!(self, "composer_drafts", ComposerDrafts)
    }
    pub fn set_composer_draft(
        &self,
        conversation_id: String,
        text: String,
    ) -> Result<(), BindingError> {
        request!(self, "set_composer_draft", SetComposerDraft { conversation_id, text }; unit)
    }
    pub fn set_conversation_browser_limits(
        &self,
        limits: BindingConversationBrowserLimits,
    ) -> Result<BindingConversationBrowserLimits, BindingError> {
        let limits = convert(limits)?;
        request!(
            self,
            "set_conversation_browser_limits",
            SetConversationBrowserLimits { limits }
        )
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
        request!(
            self,
            "organize_conversations",
            OrganizeConversations {
                conversations,
                change
            }
        )
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
        request!(
            self,
            "set_project",
            SetProject {
                path,
                primary_folder,
                name,
                folders
            }
        )
    }
    pub fn load_more_conversations(
        &self,
        group: BindingConversationGroupKey,
    ) -> Result<BindingConfigurationSnapshot, BindingError> {
        let group = convert(group)?;
        request!(
            self,
            "load_more_conversations",
            LoadMoreConversations { group }
        )
    }
    pub fn show_archived_conversations(
        &self,
        archived: bool,
    ) -> Result<BindingConfigurationSnapshot, BindingError> {
        request!(
            self,
            "show_archived_conversations",
            ShowArchivedConversations { archived }
        )
    }
    pub fn search_conversations(
        &self,
        text: String,
    ) -> Result<BindingConversationSearch, BindingError> {
        request!(self, "search_conversations", SearchConversations { text })
    }
    /// One conversation by identity, whether or not the list shows it.
    pub fn find_conversation(
        &self,
        conversation_id: String,
    ) -> Result<Option<BindingListedConversation>, BindingError> {
        request!(
            self,
            "find_conversation",
            FindConversation { conversation_id }
        )
    }

    pub fn provider_quota(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<BindingProviderQuotaStatus, BindingError> {
        request!(
            self,
            "provider_quota",
            ProviderQuota {
                gateway_id,
                provider_id
            }
        )
    }
    pub fn refresh_provider_quota(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<BindingProviderQuotaStatus, BindingError> {
        request!(
            self,
            "refresh_provider_quota",
            RefreshProviderQuota {
                gateway_id,
                provider_id
            }
        )
    }
    pub fn save_provider(
        &self,
        gateway_id: String,
        provider: BindingProviderDraft,
        authentication_edit: BindingAuthenticationEdit,
    ) -> Result<BindingGatewayUpdate, BindingError> {
        let provider = convert(provider)?;
        let authentication_edit = convert(authentication_edit)?;
        request!(
            self,
            "save_provider",
            SaveProvider {
                gateway_id,
                provider,
                authentication_edit
            }
        )
    }
    pub fn delete_provider(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<BindingGatewayUpdate, BindingError> {
        request!(
            self,
            "delete_provider",
            DeleteProvider {
                gateway_id,
                provider_id
            }
        )
    }
    pub fn save_model_route(
        &self,
        gateway_id: String,
        route: BindingModelRoute,
    ) -> Result<BindingGatewayUpdate, BindingError> {
        let route = convert(route)?;
        request!(
            self,
            "save_model_route",
            SaveModelRoute { gateway_id, route }
        )
    }
    pub fn delete_model_route(
        &self,
        gateway_id: String,
        route_record_key: String,
    ) -> Result<BindingGatewayUpdate, BindingError> {
        request!(
            self,
            "delete_model_route",
            DeleteModelRoute {
                gateway_id,
                route_record_key
            }
        )
    }
    pub fn read_provider_api_key(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<String, BindingError> {
        request!(
            self,
            "read_provider_api_key",
            ReadProviderApiKey {
                gateway_id,
                provider_id
            }
        )
    }
    pub fn save_model_template(
        &self,
        template: BindingModelTemplate,
    ) -> Result<Vec<BindingModelTemplate>, BindingError> {
        let template = convert(template)?;
        request!(self, "save_model_template", SaveModelTemplate { template })
    }
    /// Explicit, credential-free retrieval; does not change application configuration.
    pub fn fetch_public_model_catalog(&self) -> Result<Vec<BindingCatalogModel>, BindingError> {
        request!(self, "fetch_public_model_catalog", FetchPublicModelCatalog)
    }
    pub fn delete_model_template(
        &self,
        id: String,
    ) -> Result<Vec<BindingModelTemplate>, BindingError> {
        request!(self, "delete_model_template", DeleteModelTemplate { id })
    }

    pub fn upsert_runtime(
        &self,
        runtime: BindingRuntimeInstance,
    ) -> Result<BindingRuntimeUpdate, BindingError> {
        let runtime = convert(runtime)?;
        request!(self, "upsert_runtime", UpsertRuntime { runtime })
    }

    pub fn delete_runtime(&self, id: String) -> Result<BindingRuntimeUpdate, BindingError> {
        request!(self, "delete_runtime", DeleteRuntime { id })
    }

    pub fn select_runtime(&self, id: String) -> Result<BindingConfigurationSnapshot, BindingError> {
        request!(self, "select_runtime", SelectRuntime { id })
    }

    pub fn create_conversation(
        &self,
        runtime_id: String,
        cwd: Option<String>,
        model_record_key: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        request!(
            self,
            "create_conversation",
            CreateConversation {
                runtime_id,
                cwd,
                model_record_key
            }
        )
    }

    pub fn open_conversation(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        request!(
            self,
            "open_conversation",
            OpenConversation {
                runtime_id,
                conversation_id
            }
        )
    }
    pub fn read_conversation(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        request!(
            self,
            "read_conversation",
            ReadConversation {
                runtime_id,
                conversation_id
            }
        )
    }

    pub fn rename_conversation(
        &self,
        runtime_id: String,
        conversation_id: String,
        title: String,
    ) -> Result<BindingConfigurationSnapshot, BindingError> {
        request!(
            self,
            "rename_conversation",
            RenameConversation {
                runtime_id,
                conversation_id,
                title
            }
        )
    }
    pub fn delete_conversation(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingConfigurationSnapshot, BindingError> {
        request!(
            self,
            "delete_conversation",
            DeleteConversation {
                runtime_id,
                conversation_id
            }
        )
    }
    pub fn snapshot(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        request!(
            self,
            "snapshot",
            Snapshot {
                runtime_id,
                conversation_id
            }
        )
    }
    /// Poll a view relative to the cursor of its latest delivery.
    pub fn snapshot_update(
        &self,
        runtime_id: String,
        conversation_id: String,
        since: BindingTranscriptCursor,
    ) -> Result<BindingSnapshotUpdate, BindingError> {
        let since = convert(since)?;
        request!(
            self,
            "snapshot_update",
            SnapshotUpdate {
                runtime_id,
                conversation_id,
                since
            }
        )
    }
    /// Complete reasoning and tool output of delivered messages.
    pub fn message_payloads(
        &self,
        runtime_id: String,
        conversation_id: String,
        message_ids: Vec<String>,
    ) -> Result<Vec<BindingMessage>, BindingError> {
        request!(
            self,
            "message_payloads",
            MessagePayloads {
                runtime_id,
                conversation_id,
                message_ids
            }
        )
    }

    pub fn send_turn(
        &self,
        source_runtime_id: String,
        conversation_id: String,
        target_runtime_id: String,
        model_record_key: String,
        text: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        request!(
            self,
            "send_turn",
            SendTurn {
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                model_record_key,
                text
            }
        )
    }

    pub fn upsert_runtime_with_execution(
        &self,
        runtime: BindingRuntimeInstance,
        execution: BindingRuntimeExecution,
    ) -> Result<BindingRuntimeUpdate, BindingError> {
        let runtime = convert(runtime)?;
        let execution = convert(execution)?;
        request!(
            self,
            "upsert_runtime_with_execution",
            UpsertRuntimeWithExecution { runtime, execution }
        )
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
        request!(
            self,
            "upsert_acp_runtime",
            UpsertAcpRuntime {
                runtime,
                execution,
                launch
            }
        )
    }
    pub fn acp_launch(&self, runtime_id: String) -> Result<BindingAcpLaunch, BindingError> {
        request!(self, "acp_launch", AcpLaunch { runtime_id })
    }
    pub fn acp_runtime_info(
        &self,
        runtime_id: String,
    ) -> Result<BindingAcpRuntimeInfo, BindingError> {
        request!(self, "acp_runtime_info", AcpRuntimeInfo { runtime_id })
    }
    pub fn open_acp_conversation(
        &self,
        runtime_id: String,
        native_session_id: String,
        cwd: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        request!(
            self,
            "open_acp_conversation",
            OpenAcpConversation {
                runtime_id,
                native_session_id,
                cwd
            }
        )
    }
    pub fn acp_session_config(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<Vec<BindingAcpConfigOption>, BindingError> {
        request!(
            self,
            "acp_session_config",
            AcpSessionConfig {
                runtime_id,
                conversation_id
            }
        )
    }
    pub fn set_acp_session_config(
        &self,
        runtime_id: String,
        conversation_id: String,
        config_id: String,
        value: BindingAcpConfigValue,
    ) -> Result<Vec<BindingAcpConfigOption>, BindingError> {
        let value = convert(value)?;
        request!(
            self,
            "set_acp_session_config",
            SetAcpSessionConfig {
                runtime_id,
                conversation_id,
                config_id,
                value
            }
        )
    }

    pub fn runtime_execution(
        &self,
        runtime_id: String,
    ) -> Result<BindingRuntimeExecution, BindingError> {
        request!(self, "runtime_execution", RuntimeExecution { runtime_id })
    }

    pub fn runtime_home_directory(
        &self,
        runtime_id: String,
    ) -> Result<Option<String>, BindingError> {
        request!(
            self,
            "runtime_home_directory",
            RuntimeHomeDirectory { runtime_id }
        )
    }

    pub fn configure_runtime_execution(
        &self,
        runtime_id: String,
        execution: BindingRuntimeExecution,
    ) -> Result<BindingRuntimeUpdate, BindingError> {
        let execution = convert(execution)?;
        request!(
            self,
            "configure_runtime_execution",
            ConfigureRuntimeExecution {
                runtime_id,
                execution
            }
        )
    }

    pub fn create_conversation_with_selection(
        &self,
        runtime_id: String,
        cwd: Option<String>,
        selection: BindingModelSelection,
    ) -> Result<BindingSnapshotResult, BindingError> {
        let selection = convert(selection)?;
        request!(
            self,
            "create_conversation_with_selection",
            CreateConversationWithSelection {
                runtime_id,
                cwd,
                selection
            }
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
        request!(
            self,
            "send_turn_with_selection",
            SendTurnWithSelection {
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                selection,
                turn
            }
        )
    }

    pub fn reasoning_efforts(
        &self,
        runtime_id: String,
        model_record_key: String,
    ) -> Result<Vec<String>, BindingError> {
        request!(
            self,
            "reasoning_efforts",
            ReasoningEfforts {
                runtime_id,
                model_record_key
            }
        )
    }

    pub fn runtime_models(
        &self,
        runtime_id: String,
    ) -> Result<Vec<BindingRuntimeModel>, BindingError> {
        request!(self, "runtime_models", RuntimeModels { runtime_id })
    }

    pub fn next_turn_model_selection(&self) -> Result<Option<BindingModelSelection>, BindingError> {
        request!(self, "next_turn_model_selection", NextTurnModelSelection)
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
        request!(
            self,
            "send_queue",
            SendQueue {
                runtime_id,
                conversation_id,
                operation
            }
        )
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
        request!(
            self,
            "steer_turn",
            SteerTurn {
                runtime_id,
                conversation_id,
                turn
            }
        )
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
        request!(
            self,
            "edit_message",
            EditMessage {
                runtime_id,
                conversation_id,
                message_id,
                placement,
                selection,
                turn
            }
        )
    }

    pub fn cancel(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        request!(
            self,
            "cancel",
            Cancel {
                runtime_id,
                conversation_id
            }
        )
    }

    pub fn stop_sub_agents(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingSnapshotResult, BindingError> {
        request!(
            self,
            "stop_sub_agents",
            StopSubAgents {
                runtime_id,
                conversation_id
            }
        )
    }

    /// Uncommitted changes in the conversation's working directory.
    pub fn working_changes(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<BindingWorkingChanges, BindingError> {
        request!(
            self,
            "working_changes",
            WorkingChanges {
                runtime_id,
                conversation_id
            }
        )
    }

    /// The branch checked out in the conversation's working directory;
    /// absent outside a repository or on a detached HEAD.
    pub fn working_branch(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<Option<String>, BindingError> {
        request!(
            self,
            "working_branch",
            WorkingBranch {
                runtime_id,
                conversation_id
            }
        )
    }

    /// The diff of one file listed by `working_changes`.
    pub fn file_diff(
        &self,
        runtime_id: String,
        conversation_id: String,
        path: String,
    ) -> Result<BindingFileDiff, BindingError> {
        request!(
            self,
            "file_diff",
            FileDiff {
                runtime_id,
                conversation_id,
                path
            }
        )
    }

    /// Conversations executing or waiting for the user across open
    /// sessions, and turns that ended after `since` (0 for all retained).
    pub fn conversation_activity(
        &self,
        since: u64,
    ) -> Result<BindingConversationActivity, BindingError> {
        request!(
            self,
            "conversation_activity",
            ConversationActivity { since }
        )
    }

    pub fn reply_runtime_interaction(
        &self,
        runtime_id: String,
        conversation_id: String,
        interaction_id: String,
        reply: BindingRuntimeInteractionReply,
    ) -> Result<BindingSnapshotResult, BindingError> {
        let reply = convert(reply)?;
        request!(
            self,
            "reply_runtime_interaction",
            ReplyRuntimeInteraction {
                runtime_id,
                conversation_id,
                interaction_id,
                reply
            }
        )
    }
    pub fn preview_provider_import(
        &self,
        gateway_id: String,
        source: BindingProviderImportSource,
    ) -> Result<BindingProviderImportPreview, BindingError> {
        let source = convert(source)?;
        request!(
            self,
            "preview_provider_import",
            PreviewProviderImport { gateway_id, source }
        )
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
        request!(
            self,
            "apply_provider_import",
            ApplyProviderImport {
                gateway_id,
                source,
                preview_token,
                selections,
                replace_existing
            }
        )
    }

    pub fn authentication_inspect(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<BindingAuthenticationMetadata, BindingError> {
        request!(
            self,
            "authentication_inspect",
            AuthenticationInspect {
                gateway_id,
                provider_id
            }
        )
    }
    pub fn authentication_start(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<BindingAuthenticationProgress, BindingError> {
        request!(
            self,
            "authentication_start",
            AuthenticationStart {
                gateway_id,
                provider_id
            }
        )
    }

    pub fn authentication_logout(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<BindingGatewayUpdate, BindingError> {
        request!(
            self,
            "authentication_logout",
            AuthenticationLogout {
                gateway_id,
                provider_id
            }
        )
    }
    pub fn authentication_poll(&self) -> Result<BindingAuthenticationProgress, BindingError> {
        request!(self, "authentication_poll", AuthenticationPoll)
    }

    pub fn authentication_reply(
        &self,
        prompt_id: String,
        value: String,
    ) -> Result<BindingAuthenticationProgress, BindingError> {
        request!(
            self,
            "authentication_reply",
            AuthenticationReply { prompt_id, value }
        )
    }

    pub fn authentication_cancel(&self) -> Result<BindingAuthenticationProgress, BindingError> {
        request!(self, "authentication_cancel", AuthenticationCancel)
    }
    /// Upload content before using its reference in a remote composed turn.
    pub fn upload_attachment(
        &self,
        name: String,
        bytes: Vec<u8>,
    ) -> Result<BindingAttachmentReference, BindingError> {
        request!(self, "upload_attachment", UploadAttachment { name, bytes })
    }

    /// Only content attached to this delivered message can be retrieved.
    pub fn read_attachment(
        &self,
        runtime_id: String,
        conversation_id: String,
        message_id: String,
        block_index: u32,
    ) -> Result<BindingAttachmentContent, BindingError> {
        request!(
            self,
            "read_attachment",
            ReadAttachment {
                runtime_id,
                conversation_id,
                message_id,
                block_index
            }
        )
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
            #[cfg(feature = "remote")]
            Self::RemoteRequest {
                kind, detail, code, ..
            } => (kind, code, "remote".into(), detail),
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
