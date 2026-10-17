//! Typed recovery results for a remote request whose acknowledgement was lost.
use crate::*;
use velune_application_api::Response;

/// Shared payload variants retain result types without duplicating identical operation results.
/// Explicitly requested secrets and message content are never printed through Debug.
#[derive(uniffi::Enum)]
pub enum BindingApplicationResponse {
    Empty,
    RuntimeResourceCatalog {
        value: BindingRuntimeResourceCatalog,
    },
    RuntimeResourceDocument {
        value: BindingRuntimeResourceDocument,
    },
    ResourceTemplateSummaryList {
        value: Vec<BindingResourceTemplateSummary>,
    },
    ResourceTemplate {
        value: BindingResourceTemplate,
    },
    ResourceSaveResult {
        value: BindingResourceSaveResult,
    },
    ResourceTemplateImport {
        value: BindingResourceTemplateImport,
    },
    ResourceTemplateSummary {
        value: BindingResourceTemplateSummary,
    },
    ResourceSyncStatusList {
        value: Vec<BindingResourceSyncStatus>,
    },
    ResourceDriftFileList {
        value: Vec<BindingResourceDriftFile>,
    },
    UnmanagedRuntimeResourceList {
        value: Vec<BindingUnmanagedRuntimeResource>,
    },
    TemplateApplyReportList {
        value: Vec<BindingTemplateApplyReport>,
    },
    AnalyticsReport {
        value: BindingAnalyticsReport,
    },
    RuntimeDiscoveryHintList {
        value: Vec<BindingRuntimeDiscoveryHint>,
    },
    RuntimeDiscoveryCandidateList {
        value: Vec<BindingRuntimeDiscoveryCandidate>,
    },
    ConfigurationSnapshot {
        value: BindingConfigurationSnapshot,
    },
    OutlinePresentation {
        value: BindingOutlinePresentation,
    },
    StatusLineItemList {
        value: Vec<BindingStatusLineItem>,
    },
    ComposerDraftList {
        value: Vec<BindingComposerDraft>,
    },
    ConversationBrowserLimits {
        value: BindingConversationBrowserLimits,
    },
    ConversationSearch {
        value: BindingConversationSearch,
    },
    OptionalListedConversation {
        value: Option<BindingListedConversation>,
    },
    RuntimeUpdate {
        value: BindingRuntimeUpdate,
    },
    AcpLaunch {
        value: BindingAcpLaunch,
    },
    AcpRuntimeInfo {
        value: BindingAcpRuntimeInfo,
    },
    SnapshotResult {
        value: BindingSnapshotResult,
    },
    AcpConfigOptionList {
        value: Vec<BindingAcpConfigOption>,
    },
    RuntimeExecution {
        value: BindingRuntimeExecution,
    },
    OptionalString {
        value: Option<String>,
    },
    RuntimeModelList {
        value: Vec<BindingRuntimeModel>,
    },
    OptionalModelSelection {
        value: Option<BindingModelSelection>,
    },
    SnapshotUpdate {
        value: BindingSnapshotUpdate,
    },
    MessageList {
        value: Vec<BindingMessage>,
    },
    Strings {
        value: Vec<String>,
    },
    ConversationActivity {
        value: BindingConversationActivity,
    },
    WorkingChanges {
        value: BindingWorkingChanges,
    },
    FileDiff {
        value: BindingFileDiff,
    },
    ProviderImportPreview {
        value: BindingProviderImportPreview,
    },
    ImportResult {
        value: BindingImportResult,
    },
    ProviderQuotaStatus {
        value: BindingProviderQuotaStatus,
    },
    GatewayUpdate {
        value: BindingGatewayUpdate,
    },
    ProviderApiKey {
        value: String,
    },
    ModelTemplateList {
        value: Vec<BindingModelTemplate>,
    },
    AuthenticationMetadata {
        value: BindingAuthenticationMetadata,
    },
    AuthenticationProgress {
        value: BindingAuthenticationProgress,
    },
    CatalogModelList {
        value: Vec<BindingCatalogModel>,
    },
    AttachmentReference {
        value: BindingAttachmentReference,
    },
    AttachmentContent {
        value: BindingAttachmentContent,
    },
}

impl TryFrom<Response> for BindingApplicationResponse {
    type Error = BindingError;
    fn try_from(response: Response) -> Result<Self, Self::Error> {
        Ok(match response {
            Response::ReleaseView
            | Response::DeleteRuntimeResource
            | Response::DeleteResourceTemplate
            | Response::ResolveResourceBinding
            | Response::SetComposerDraft => Self::Empty,
            Response::RuntimeResourceCatalog(value) => Self::RuntimeResourceCatalog {
                value: convert(value)?,
            },
            Response::ReadRuntimeResource(value) | Response::WriteRuntimeResource(value) => {
                Self::RuntimeResourceDocument {
                    value: convert(value)?,
                }
            }
            Response::ListResourceTemplates(value) => Self::ResourceTemplateSummaryList {
                value: convert(value)?,
            },
            Response::ReadResourceTemplate(value) => Self::ResourceTemplate {
                value: convert(value)?,
            },
            Response::SaveResourceTemplate(value) => Self::ResourceSaveResult {
                value: convert(value)?,
            },
            Response::ImportRuntimeResource(value) | Response::ResourceNativeImport(value) => {
                Self::ResourceTemplateImport {
                    value: convert(value)?,
                }
            }
            Response::AdoptRuntimeResource(value) => Self::ResourceTemplateSummary {
                value: convert(value)?,
            },
            Response::ResourceSyncStatus(value) => Self::ResourceSyncStatusList {
                value: convert(value)?,
            },
            Response::ResourceDrift(value) => Self::ResourceDriftFileList {
                value: convert(value)?,
            },
            Response::UnmanagedRuntimeResources(value) => Self::UnmanagedRuntimeResourceList {
                value: convert(value)?,
            },
            Response::ApplyResourceTemplate(value) => Self::TemplateApplyReportList {
                value: convert(value)?,
            },
            Response::AnalyticsQuery(value) => Self::AnalyticsReport {
                value: convert(value)?,
            },
            Response::RuntimeDiscoveryHints(value) => Self::RuntimeDiscoveryHintList {
                value: convert(value)?,
            },
            Response::DiscoverRuntimes(value) => Self::RuntimeDiscoveryCandidateList {
                value: convert(value)?,
            },
            Response::List(value)
            | Response::SetConversationBrowserPreferences(value)
            | Response::OrganizeConversations(value)
            | Response::SetProject(value)
            | Response::LoadMoreConversations(value)
            | Response::ShowArchivedConversations(value)
            | Response::SelectRuntime(value)
            | Response::RenameConversation(value)
            | Response::DeleteConversation(value) => Self::ConfigurationSnapshot {
                value: convert(value)?,
            },
            Response::SetOutlinePresentation(value) => Self::OutlinePresentation {
                value: convert(value)?,
            },
            Response::SetStatusLine(value) => Self::StatusLineItemList {
                value: convert(value)?,
            },
            Response::ComposerDrafts(value) => Self::ComposerDraftList {
                value: convert(value)?,
            },
            Response::SetConversationBrowserLimits(value) => Self::ConversationBrowserLimits {
                value: convert(value)?,
            },
            Response::SearchConversations(value) => Self::ConversationSearch {
                value: convert(value)?,
            },
            Response::FindConversation(value) => Self::OptionalListedConversation {
                value: convert(value)?,
            },
            Response::UpsertRuntime(value)
            | Response::UpsertRuntimeWithExecution(value)
            | Response::UpsertAcpRuntime(value)
            | Response::DeleteRuntime(value)
            | Response::ConfigureRuntimeExecution(value) => Self::RuntimeUpdate {
                value: convert(value)?,
            },
            Response::AcpLaunch(value) => Self::AcpLaunch {
                value: convert(value)?,
            },
            Response::AcpRuntimeInfo(value) => Self::AcpRuntimeInfo {
                value: convert(value)?,
            },
            Response::OpenAcpConversation(value)
            | Response::CreateConversation(value)
            | Response::CreateConversationWithSelection(value)
            | Response::OpenConversation(value)
            | Response::ReadConversation(value)
            | Response::Snapshot(value)
            | Response::SendTurn(value)
            | Response::SendTurnWithSelection(value)
            | Response::SendQueue(value)
            | Response::SteerTurn(value)
            | Response::EditMessage(value)
            | Response::Cancel(value)
            | Response::StopSubAgents(value)
            | Response::ReplyRuntimeInteraction(value) => Self::SnapshotResult {
                value: convert(value)?,
            },
            Response::AcpSessionConfig(value) | Response::SetAcpSessionConfig(value) => {
                Self::AcpConfigOptionList {
                    value: convert(value)?,
                }
            }
            Response::RuntimeExecution(value) => Self::RuntimeExecution {
                value: convert(value)?,
            },
            Response::RuntimeHomeDirectory(value) | Response::WorkingBranch(value) => {
                Self::OptionalString {
                    value: convert(value)?,
                }
            }
            Response::RuntimeModels(value) => Self::RuntimeModelList {
                value: convert(value)?,
            },
            Response::NextTurnModelSelection(value) => Self::OptionalModelSelection {
                value: convert(value)?,
            },
            Response::SnapshotUpdate(value) => Self::SnapshotUpdate {
                value: convert(value)?,
            },
            Response::MessagePayloads(value) => Self::MessageList {
                value: convert(value)?,
            },
            Response::ReasoningEfforts(value) => Self::Strings {
                value: convert(value)?,
            },
            Response::ConversationActivity(value) => Self::ConversationActivity {
                value: convert(value)?,
            },
            Response::WorkingChanges(value) => Self::WorkingChanges {
                value: convert(value)?,
            },
            Response::FileDiff(value) => Self::FileDiff {
                value: convert(value)?,
            },
            Response::PreviewProviderImport(value) => Self::ProviderImportPreview {
                value: convert(value)?,
            },
            Response::ApplyProviderImport(value) => Self::ImportResult {
                value: convert(value)?,
            },
            Response::ProviderQuota(value) | Response::RefreshProviderQuota(value) => {
                Self::ProviderQuotaStatus {
                    value: convert(value)?,
                }
            }
            Response::SaveProvider(value)
            | Response::DeleteProvider(value)
            | Response::SaveModelRoute(value)
            | Response::DeleteModelRoute(value)
            | Response::AuthenticationLogout(value) => Self::GatewayUpdate {
                value: convert(value)?,
            },
            Response::ReadProviderApiKey(value) => Self::ProviderApiKey {
                value: convert(value)?,
            },
            Response::SaveModelTemplate(value) | Response::DeleteModelTemplate(value) => {
                Self::ModelTemplateList {
                    value: convert(value)?,
                }
            }
            Response::AuthenticationInspect(value) => Self::AuthenticationMetadata {
                value: convert(value)?,
            },
            Response::AuthenticationStart(value)
            | Response::AuthenticationPoll(value)
            | Response::AuthenticationReply(value)
            | Response::AuthenticationCancel(value) => Self::AuthenticationProgress {
                value: convert(value)?,
            },
            Response::FetchPublicModelCatalog(value) => Self::CatalogModelList {
                value: convert(value)?,
            },
            Response::UploadAttachment(value) => Self::AttachmentReference {
                value: convert(value)?,
            },
            Response::ReadAttachment(value) => Self::AttachmentContent {
                value: convert(value)?,
            },
        })
    }
}
