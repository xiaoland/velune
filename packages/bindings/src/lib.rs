//! Typed foreign-language entry point for the shared application unit.
//!
//! Local objects retain one execution owner; remote objects retain only their
//! connection. Closing a controller never ends the execution owner or its work.
#[cfg(all(target_os = "android", feature = "remote"))]
mod android;
mod diagnostics;
mod telemetry;
pub use telemetry::*;
#[cfg(feature = "remote")]
mod remote;
#[cfg(feature = "remote")]
mod remote_state;
#[cfg(feature = "remote")]
pub use remote_state::*;
#[cfg(feature = "remote-diagnostics")]
mod transport_probe;
#[cfg(feature = "remote-diagnostics")]
mod transport_trace;
#[cfg(feature = "remote")]
pub use remote::*;
#[cfg(feature = "remote-diagnostics")]
pub use transport_probe::*;
#[cfg(feature = "remote-diagnostics")]
pub use transport_trace::*;

use std::sync::Arc;
use std::sync::{Mutex, MutexGuard};
#[cfg(feature = "local-runtime")]
use velune_application::{ApplicationService, Error, Options};
#[cfg(any(feature = "local-runtime", feature = "remote"))]
use velune_application_api::ClientViewId;
use velune_application_api::conversation::{EditPlacement, RuntimeInteractionReply};
use velune_application_api::*;

uniffi::setup_scaffolding!();

macro_rules! request {
    ($owner:ident, $name:literal, $variant:ident $( { $($field:ident),* } )?) => {
        match $owner.request(concat!("bindings.", $name), Command::$variant $( { $($field),* } )?)? {
            Response::$variant(value) => Ok(value),
            _ => Err(BindingError::Contract { detail: "应用返回了不匹配的具名结果".into() }),
        }
    };
    ($owner:ident, $name:literal, $variant:ident $( { $($field:ident),* } )?; unit) => {
        match $owner.request(concat!("bindings.", $name), Command::$variant $( { $($field),* } )?)? {
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
        request_id: Option<velune_remote::RequestId>,
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

/// What a new provider starts from: each subscription, then an API provider.
/// Present them before the provider exists, with each template's disclosure.
#[uniffi::export]
pub fn provider_templates() -> Result<Vec<ProviderTemplate>, BindingError> {
    Ok(velune_application_api::provider_templates())
}

/// The official API service an address belongs to, whose account balance or
/// key limit the `apiAccount` quota source reads; None for any other address.
#[uniffi::export]
pub fn api_account_service_name(endpoint: String) -> Option<String> {
    velune_application_api::api_account_service(&endpoint).map(|service| service.name().into())
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
}

#[derive(uniffi::Object)]
pub struct VeluneApplication {
    application: Arc<Mutex<Option<Backend>>>,
    trace_carrier: Option<velune_telemetry::TraceCarrier>,
    diagnostics: diagnostics::Diagnostics,
    #[cfg(all(feature = "local-runtime", feature = "remote"))]
    remote_service: Arc<Mutex<Option<Arc<remote::ServiceOwner>>>>,
}

impl VeluneApplication {
    fn lock(&self) -> Result<MutexGuard<'_, Option<Backend>>, BindingError> {
        self.application
            .lock()
            .map_err(|_| BindingError::Unavailable)
    }

    fn request(&self, name: &'static str, command: Command) -> Result<Response, BindingError> {
        self.diagnostics
            .run_context(name, self.trace_carrier.as_ref(), || {
                #[cfg(any(feature = "local-runtime", feature = "remote"))]
                {
                    let backend = self.lock()?.as_ref().ok_or(BindingError::Closed)?.clone();
                    match backend {
                        #[cfg(feature = "local-runtime")]
                        Backend::Local(service) => service
                            .execute(ClientViewId("local".into()), command)
                            .map_err(Into::into),
                        #[cfg(feature = "remote")]
                        Backend::Remote(remote) => remote.request(command),
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
        self.diagnostics.run("bindings.shutdown", || {
            let mut state = self.lock()?;
            if let Some(backend) = state.as_mut() {
                match backend {
                    #[cfg(feature = "local-runtime")]
                    Backend::Local(service) => service.close(_stop_work)?,
                    #[cfg(feature = "remote")]
                    Backend::Remote(remote) => remote.close()?,
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
        Self::open_with_diagnostics(options, diagnostics)
    }

    #[uniffi::constructor]
    pub fn open_with_telemetry(
        options: BindingOptions,
        telemetry: Arc<VeluneTelemetry>,
    ) -> Result<Arc<Self>, BindingError> {
        Self::open_with_diagnostics(
            options,
            diagnostics::Diagnostics::with_telemetry(telemetry.owner.clone()),
        )
    }
}

#[cfg(feature = "local-runtime")]
impl VeluneApplication {
    fn open_with_diagnostics(
        options: BindingOptions,
        diagnostics: diagnostics::Diagnostics,
    ) -> Result<Arc<Self>, BindingError> {
        let application = diagnostics.run("bindings.open", || {
            ApplicationService::open_with_telemetry(
                Options {
                    home_directory: options.home_directory.into(),
                    resources_directory: options.resources_directory.into(),
                },
                diagnostics.telemetry.clone(),
            )
            .map_err(Into::into)
        })?;
        Ok(Arc::new(Self {
            application: Arc::new(Mutex::new(Some(Backend::Local(application)))),
            trace_carrier: None,
            diagnostics,
            #[cfg(feature = "remote")]
            remote_service: Arc::new(Mutex::new(None)),
        }))
    }
}

#[uniffi::export]
impl VeluneApplication {
    /// An immutable per-call view. Dropping it does not close the shared application session.
    pub fn with_trace_context(
        &self,
        carrier: Option<velune_telemetry::TraceCarrier>,
    ) -> Result<Arc<Self>, BindingError> {
        if let Some(carrier) = &carrier {
            carrier.validate()?;
        }
        if self.lock()?.is_none() {
            return Err(BindingError::Closed);
        }
        Ok(Arc::new(Self {
            application: self.application.clone(),
            diagnostics: self.diagnostics.clone(),
            trace_carrier: carrier,
            #[cfg(all(feature = "local-runtime", feature = "remote"))]
            remote_service: self.remote_service.clone(),
        }))
    }
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

    pub fn list(&self) -> Result<ConfigurationSnapshot, BindingError> {
        request!(self, "list", ReadConfiguration)
    }
    pub fn runtime_resource_catalog(
        &self,
        runtime_id: String,
        cwd: Option<String>,
        kind: RuntimeResourceKind,
        scope: RuntimeResourceScope,
    ) -> Result<RuntimeResourceCatalog, BindingError> {
        request!(
            self,
            "runtime_resource_catalog",
            ReadRuntimeResourceCatalog {
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
        key: RuntimeResourceKey,
    ) -> Result<RuntimeResourceDocument, BindingError> {
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
        key: RuntimeResourceKey,
        expected: RuntimeResourceRevision,
        bytes: Vec<u8>,
        executable: Option<bool>,
    ) -> Result<RuntimeResourceDocument, BindingError> {
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
        key: RuntimeResourceKey,
        expected: RuntimeResourceRevision,
    ) -> Result<(), BindingError> {
        request!(self, "delete_runtime_resource", DeleteRuntimeResource { runtime_id, cwd, key, expected }; unit)
    }
    pub fn list_resource_templates(&self) -> Result<Vec<ResourceTemplateSummary>, BindingError> {
        request!(self, "list_resource_templates", ListResourceTemplates)
    }
    pub fn read_resource_template(
        &self,
        template_id: String,
    ) -> Result<ResourceTemplate, BindingError> {
        request!(
            self,
            "read_resource_template",
            ReadResourceTemplate { template_id }
        )
    }
    pub fn save_resource_template(
        &self,
        template: ResourceTemplate,
    ) -> Result<ResourceSaveResult, BindingError> {
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
        kind: RuntimeResourceKind,
        path: String,
    ) -> Result<ResourceTemplateImport, BindingError> {
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
        kind: RuntimeResourceKind,
        path: String,
    ) -> Result<ResourceTemplateSummary, BindingError> {
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
    pub fn resource_sync_status(&self) -> Result<Vec<ResourceSyncStatus>, BindingError> {
        request!(self, "resource_sync_status", ReadResourceSyncStatus)
    }
    pub fn resource_drift(
        &self,
        runtime_id: String,
        template_id: String,
    ) -> Result<Vec<ResourceDriftFile>, BindingError> {
        request!(
            self,
            "resource_drift",
            ReadResourceDrift {
                runtime_id,
                template_id
            }
        )
    }
    pub fn unmanaged_runtime_resources(
        &self,
        kind: RuntimeResourceKind,
    ) -> Result<Vec<UnmanagedRuntimeResource>, BindingError> {
        request!(
            self,
            "unmanaged_runtime_resources",
            ListUnmanagedRuntimeResources { kind }
        )
    }
    pub fn apply_resource_template(
        &self,
        template_id: String,
        targets: Vec<TemplateApplyTarget>,
    ) -> Result<Vec<TemplateApplyReport>, BindingError> {
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
    ) -> Result<ResourceTemplateImport, BindingError> {
        request!(
            self,
            "resource_native_import",
            ReadResourceNativeImport {
                runtime_id,
                template_id
            }
        )
    }
    pub fn resolve_resource_binding(
        &self,
        runtime_id: String,
        template_id: String,
        resolution: ResourceBindingResolution,
    ) -> Result<(), BindingError> {
        request!(self, "resolve_resource_binding", ResolveResourceBinding { runtime_id, template_id, resolution }; unit)
    }
    pub fn analytics_query(&self, query: AnalyticsQuery) -> Result<AnalyticsReport, BindingError> {
        request!(self, "analytics_query", QueryAnalytics { query })
    }
    pub fn runtime_discovery_hints(
        &self,
        user_home: String,
        overrides: std::collections::HashMap<String, String>,
    ) -> Result<Vec<RuntimeDiscoveryHint>, BindingError> {
        let overrides = overrides.into_iter().collect();
        request!(
            self,
            "runtime_discovery_hints",
            ReadRuntimeDiscoveryHints {
                user_home,
                overrides
            }
        )
    }
    pub fn discover_runtimes(
        &self,
        probes: Vec<RuntimeDiscoveryProbe>,
    ) -> Result<Vec<RuntimeDiscoveryCandidate>, BindingError> {
        request!(self, "discover_runtimes", DiscoverRuntimes { probes })
    }
    pub fn set_outline_presentation(
        &self,
        presentation: OutlinePresentation,
    ) -> Result<OutlinePresentation, BindingError> {
        request!(
            self,
            "set_outline_presentation",
            SetOutlinePresentation { presentation }
        )
    }
    pub fn set_linked_conversation_opening(
        &self,
        opening: LinkedConversationOpening,
    ) -> Result<LinkedConversationOpening, BindingError> {
        request!(
            self,
            "set_linked_conversation_opening",
            SetLinkedConversationOpening { opening }
        )
    }
    pub fn set_usage_display(&self, display: UsageDisplay) -> Result<UsageDisplay, BindingError> {
        request!(self, "set_usage_display", SetUsageDisplay { display })
    }
    pub fn set_context_usage_display(
        &self,
        display: UsageDisplay,
    ) -> Result<UsageDisplay, BindingError> {
        request!(
            self,
            "set_context_usage_display",
            SetContextUsageDisplay { display }
        )
    }
    pub fn set_status_line(
        &self,
        items: Vec<StatusLineItem>,
    ) -> Result<Vec<StatusLineItem>, BindingError> {
        request!(self, "set_status_line", SetStatusLine { items })
    }
    pub fn set_conversation_browser_preferences(
        &self,
        preferences: ConversationBrowserPreferences,
    ) -> Result<ConfigurationSnapshot, BindingError> {
        request!(
            self,
            "set_conversation_browser_preferences",
            SetConversationBrowserPreferences { preferences }
        )
    }
    pub fn composer_choices(&self) -> Result<ComposerChoices, BindingError> {
        request!(self, "composer_choices", ReadComposerChoices)
    }
    /// An intent without choices or runtime removes the conversation's.
    pub fn set_next_turn_intent(&self, intent: NextTurnIntent) -> Result<(), BindingError> {
        request!(self, "set_next_turn_intent", SetNextTurnIntent { intent }; unit)
    }
    pub fn set_runtime_composer_choices(
        &self,
        choices: RuntimeComposerChoices,
    ) -> Result<(), BindingError> {
        request!(self, "set_runtime_composer_choices", SetRuntimeComposerChoices { choices }; unit)
    }
    pub fn composer_drafts(&self) -> Result<Vec<ComposerDraft>, BindingError> {
        request!(self, "composer_drafts", ReadComposerDrafts)
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
        limits: ConversationBrowserLimits,
    ) -> Result<ConversationBrowserLimits, BindingError> {
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
        conversations: Vec<ConversationReference>,
        change: ConversationOrganizationChange,
    ) -> Result<ConfigurationSnapshot, BindingError> {
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
    ) -> Result<ConfigurationSnapshot, BindingError> {
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
        group: ConversationGroupKey,
    ) -> Result<ConfigurationSnapshot, BindingError> {
        request!(
            self,
            "load_more_conversations",
            LoadMoreConversations { group }
        )
    }
    pub fn show_archived_conversations(
        &self,
        archived: bool,
    ) -> Result<ConfigurationSnapshot, BindingError> {
        request!(
            self,
            "show_archived_conversations",
            ShowArchivedConversations { archived }
        )
    }
    pub fn search_conversations(&self, text: String) -> Result<ConversationSearch, BindingError> {
        request!(self, "search_conversations", SearchConversations { text })
    }
    /// One conversation by identity, whether or not the list shows it.
    pub fn resolve_agent_uri(
        &self,
        uri: String,
        runtime_id: Option<String>,
    ) -> Result<AgentUriResolution, BindingError> {
        request!(
            self,
            "resolve_agent_uri",
            ResolveAgentUri { uri, runtime_id }
        )
    }
    pub fn find_conversation(
        &self,
        conversation_id: String,
    ) -> Result<Option<ListedConversation>, BindingError> {
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
    ) -> Result<ProviderQuotaStatus, BindingError> {
        request!(
            self,
            "provider_quota",
            ReadProviderQuota {
                gateway_id,
                provider_id
            }
        )
    }
    pub fn refresh_provider_quota(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<ProviderQuotaStatus, BindingError> {
        request!(
            self,
            "refresh_provider_quota",
            RefreshProviderQuota {
                gateway_id,
                provider_id
            }
        )
    }
    /// Retained observations of the provider's current binding since
    /// `since_ms`, oldest first. Reading never samples the provider.
    pub fn provider_quota_history(
        &self,
        gateway_id: String,
        provider_id: String,
        since_ms: i64,
    ) -> Result<ProviderQuotaHistory, BindingError> {
        request!(
            self,
            "provider_quota_history",
            ReadProviderQuotaHistory {
                gateway_id,
                provider_id,
                since_ms
            }
        )
    }
    pub fn clear_provider_quota_history(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<ProviderQuotaHistory, BindingError> {
        request!(
            self,
            "clear_provider_quota_history",
            ClearProviderQuotaHistory {
                gateway_id,
                provider_id
            }
        )
    }
    /// Quota alerts raised after `since` (0 for all retained).
    pub fn quota_alerts(&self, since: u64) -> Result<QuotaAlerts, BindingError> {
        request!(self, "quota_alerts", ReadQuotaAlerts { since })
    }
    /// `quota_authentication_edit` edits a provider's own quota query
    /// credentials, saved atomically with the model credentials; None keeps
    /// them.
    #[uniffi::method(default(quota_authentication_edit = None))]
    pub fn save_provider(
        &self,
        gateway_id: String,
        provider: ProviderDraft,
        authentication_edit: AuthenticationEdit,
        quota_authentication_edit: Option<QuotaAuthenticationEdit>,
    ) -> Result<GatewayUpdate, BindingError> {
        request!(
            self,
            "save_provider",
            SaveProvider {
                gateway_id,
                provider,
                authentication_edit,
                quota_authentication_edit
            }
        )
    }
    pub fn delete_provider(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<GatewayUpdate, BindingError> {
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
        route: ModelRoute,
    ) -> Result<GatewayUpdate, BindingError> {
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
    ) -> Result<GatewayUpdate, BindingError> {
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
        template: ModelTemplate,
    ) -> Result<Vec<ModelTemplate>, BindingError> {
        request!(self, "save_model_template", SaveModelTemplate { template })
    }
    /// Explicit, credential-free retrieval; does not change application configuration.
    pub fn fetch_public_model_catalog(&self) -> Result<Vec<CatalogModel>, BindingError> {
        request!(self, "fetch_public_model_catalog", FetchPublicModelCatalog)
    }
    pub fn delete_model_template(&self, id: String) -> Result<Vec<ModelTemplate>, BindingError> {
        request!(self, "delete_model_template", DeleteModelTemplate { id })
    }

    pub fn upsert_runtime(&self, runtime: RuntimeInstance) -> Result<RuntimeUpdate, BindingError> {
        request!(self, "upsert_runtime", UpsertRuntime { runtime })
    }

    pub fn delete_runtime(&self, id: String) -> Result<RuntimeUpdate, BindingError> {
        request!(self, "delete_runtime", DeleteRuntime { id })
    }

    pub fn select_runtime(&self, id: String) -> Result<ConfigurationSnapshot, BindingError> {
        request!(self, "select_runtime", SelectRuntime { id })
    }

    pub fn open_conversation(
        &self,
        runtime_id: String,
        conversation_id: String,
    ) -> Result<SnapshotResult, BindingError> {
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
    ) -> Result<SnapshotResult, BindingError> {
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
    ) -> Result<ConfigurationSnapshot, BindingError> {
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
    ) -> Result<ConfigurationSnapshot, BindingError> {
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
    ) -> Result<SnapshotResult, BindingError> {
        request!(
            self,
            "snapshot",
            ReadSnapshot {
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
        since: TranscriptCursor,
    ) -> Result<SnapshotUpdate, BindingError> {
        request!(
            self,
            "snapshot_update",
            ReadSnapshotUpdate {
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
    ) -> Result<Vec<DeliveredMessage>, BindingError> {
        request!(
            self,
            "message_payloads",
            ReadMessagePayloads {
                runtime_id,
                conversation_id,
                message_ids
            }
        )
    }

    pub fn upsert_runtime_with_execution(
        &self,
        runtime: RuntimeInstance,
        execution: RuntimeExecution,
    ) -> Result<RuntimeUpdate, BindingError> {
        request!(
            self,
            "upsert_runtime_with_execution",
            UpsertRuntimeWithExecution { runtime, execution }
        )
    }

    pub fn upsert_acp_runtime(
        &self,
        runtime: RuntimeInstance,
        execution: RuntimeExecution,
        launch: AcpLaunch,
    ) -> Result<RuntimeUpdate, BindingError> {
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
    pub fn acp_launch(&self, runtime_id: String) -> Result<AcpLaunch, BindingError> {
        request!(self, "acp_launch", ReadAcpLaunch { runtime_id })
    }
    pub fn acp_runtime_info(&self, runtime_id: String) -> Result<AcpRuntimeInfo, BindingError> {
        request!(self, "acp_runtime_info", ReadAcpRuntimeInfo { runtime_id })
    }
    pub fn open_acp_conversation(
        &self,
        runtime_id: String,
        native_session_id: String,
        cwd: String,
    ) -> Result<SnapshotResult, BindingError> {
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
    ) -> Result<Vec<AcpConfigOption>, BindingError> {
        request!(
            self,
            "acp_session_config",
            ReadAcpSessionConfig {
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
        value: AcpConfigValue,
    ) -> Result<Vec<AcpConfigOption>, BindingError> {
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

    pub fn runtime_execution(&self, runtime_id: String) -> Result<RuntimeExecution, BindingError> {
        request!(
            self,
            "runtime_execution",
            ReadRuntimeExecution { runtime_id }
        )
    }

    pub fn runtime_home_directory(
        &self,
        runtime_id: String,
    ) -> Result<Option<String>, BindingError> {
        request!(
            self,
            "runtime_home_directory",
            ReadRuntimeHomeDirectory { runtime_id }
        )
    }

    pub fn configure_runtime_execution(
        &self,
        runtime_id: String,
        execution: RuntimeExecution,
    ) -> Result<RuntimeUpdate, BindingError> {
        request!(
            self,
            "configure_runtime_execution",
            ConfigureRuntimeExecution {
                runtime_id,
                execution
            }
        )
    }

    pub fn create_conversation(
        &self,
        runtime_id: String,
        cwd: Option<String>,
        selection: Option<ModelSelection>,
    ) -> Result<SnapshotResult, BindingError> {
        request!(
            self,
            "create_conversation",
            CreateConversation {
                runtime_id,
                cwd,
                selection
            }
        )
    }

    pub fn send_turn(
        &self,
        source_runtime_id: String,
        conversation_id: String,
        target_runtime_id: String,
        selection: Option<ModelSelection>,
        turn: ComposedTurn,
    ) -> Result<SnapshotResult, BindingError> {
        request!(
            self,
            "send_turn",
            SendTurn {
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
            ReadReasoningEfforts {
                runtime_id,
                model_record_key
            }
        )
    }

    pub fn composer_quota(
        &self,
        context: ComposerQuotaContext,
        refresh: bool,
    ) -> Result<Vec<ComposerQuotaEntry>, BindingError> {
        request!(
            self,
            "composer_quota",
            ReadComposerQuota { context, refresh }
        )
    }

    pub fn runtime_quota(
        &self,
        runtime_id: String,
        refresh: bool,
    ) -> Result<RuntimeQuotaStatus, BindingError> {
        request!(
            self,
            "runtime_quota",
            ReadRuntimeQuota {
                runtime_id,
                refresh
            }
        )
    }

    pub fn runtime_models(&self, runtime_id: String) -> Result<Vec<RuntimeModel>, BindingError> {
        request!(self, "runtime_models", ReadRuntimeModels { runtime_id })
    }

    pub fn next_turn_model_selection(&self) -> Result<Option<ModelSelection>, BindingError> {
        request!(
            self,
            "next_turn_model_selection",
            ReadNextTurnModelSelection
        )
    }

    /// Change the conversation's send queue; one added to or resumed while
    /// no turn runs sends its first message at once.
    pub fn send_queue(
        &self,
        runtime_id: String,
        conversation_id: String,
        operation: SendQueueOperation,
    ) -> Result<SnapshotResult, BindingError> {
        request!(
            self,
            "send_queue",
            EditSendQueue {
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
        turn: ComposedTurn,
    ) -> Result<SnapshotResult, BindingError> {
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

    /// Start native context compaction; observe progress through snapshots.
    pub fn compact_conversation(
        &self,
        runtime_id: String,
        conversation_id: String,
        selection: Option<ModelSelection>,
    ) -> Result<SnapshotResult, BindingError> {
        request!(
            self,
            "compact_conversation",
            CompactConversation {
                runtime_id,
                conversation_id,
                selection
            }
        )
    }

    /// Start a native turn without a user message, only when `can_resume`
    /// is true. Errors never send fallback text or resume the send queue.
    pub fn resume_turn(
        &self,
        runtime_id: String,
        conversation_id: String,
        selection: Option<ModelSelection>,
        effort: Option<String>,
        permission_mode: Option<String>,
    ) -> Result<SnapshotResult, BindingError> {
        request!(
            self,
            "resume_turn",
            ResumeTurn {
                runtime_id,
                conversation_id,
                selection,
                effort,
                permission_mode
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
        placement: EditPlacement,
        selection: Option<ModelSelection>,
        turn: ComposedTurn,
    ) -> Result<SnapshotResult, BindingError> {
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
    ) -> Result<SnapshotResult, BindingError> {
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
    ) -> Result<SnapshotResult, BindingError> {
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
    ) -> Result<WorkingChanges, BindingError> {
        request!(
            self,
            "working_changes",
            ReadWorkingChanges {
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
            ReadWorkingBranch {
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
    ) -> Result<FileDiff, BindingError> {
        request!(
            self,
            "file_diff",
            ReadFileDiff {
                runtime_id,
                conversation_id,
                path
            }
        )
    }

    /// Conversations executing or waiting for the user across open
    /// sessions, and turns that ended after `since` (0 for all retained).
    pub fn conversation_activity(&self, since: u64) -> Result<ConversationActivity, BindingError> {
        request!(
            self,
            "conversation_activity",
            ReadConversationActivity { since }
        )
    }

    pub fn reply_runtime_interaction(
        &self,
        runtime_id: String,
        conversation_id: String,
        interaction_id: String,
        reply: RuntimeInteractionReply,
    ) -> Result<SnapshotResult, BindingError> {
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
        runtime_id: String,
    ) -> Result<ProviderImportPreview, BindingError> {
        request!(
            self,
            "preview_provider_import",
            PreviewProviderImport {
                gateway_id,
                runtime_id
            }
        )
    }

    pub fn apply_provider_import(
        &self,
        gateway_id: String,
        runtime_id: String,
        preview_token: String,
        selections: Vec<ImportSelection>,
        replace_existing: bool,
    ) -> Result<ImportResult, BindingError> {
        request!(
            self,
            "apply_provider_import",
            ApplyProviderImport {
                gateway_id,
                runtime_id,
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
    ) -> Result<AuthenticationMetadata, BindingError> {
        request!(
            self,
            "authentication_inspect",
            InspectAuthentication {
                gateway_id,
                provider_id
            }
        )
    }
    pub fn authentication_start(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<AuthenticationProgress, BindingError> {
        request!(
            self,
            "authentication_start",
            StartAuthentication {
                gateway_id,
                provider_id
            }
        )
    }

    pub fn authentication_logout(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<GatewayUpdate, BindingError> {
        request!(
            self,
            "authentication_logout",
            LogoutAuthentication {
                gateway_id,
                provider_id
            }
        )
    }
    pub fn authentication_poll(&self) -> Result<AuthenticationProgress, BindingError> {
        request!(self, "authentication_poll", PollAuthentication)
    }

    pub fn authentication_reply(
        &self,
        prompt_id: String,
        value: String,
    ) -> Result<AuthenticationProgress, BindingError> {
        request!(
            self,
            "authentication_reply",
            ReplyAuthentication { prompt_id, value }
        )
    }

    pub fn authentication_cancel(&self) -> Result<AuthenticationProgress, BindingError> {
        request!(self, "authentication_cancel", CancelAuthentication)
    }
    /// Upload content before using its reference in a remote composed turn.
    pub fn upload_attachment(
        &self,
        name: String,
        bytes: Vec<u8>,
    ) -> Result<AttachmentReference, BindingError> {
        request!(self, "upload_attachment", UploadAttachment { name, bytes })
    }

    /// Only content attached to this delivered message can be retrieved.
    pub fn read_attachment(
        &self,
        runtime_id: String,
        conversation_id: String,
        message_id: String,
        block_index: u32,
    ) -> Result<AttachmentContent, BindingError> {
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
