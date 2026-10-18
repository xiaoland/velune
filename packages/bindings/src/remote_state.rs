//! Safe projections of the Rust-owned remote state; response bodies use explicit query only.
use crate::*;

#[derive(Clone, uniffi::Record)]
pub struct BindingPairingOfferInfo {
    pub issuer: String,
    pub name: String,
    pub expires_at_unix_seconds: u64,
    pub locator: BindingDeviceLocator,
}

/// Bounded offline parsing; no service, socket, identity, or token persistence is required.
#[uniffi::export]
pub fn decode_remote_offer(encoded: String) -> Result<BindingPairingOfferInfo, BindingError> {
    let offer = velune_remote::PairingOffer::decode(&encoded)?;
    Ok(BindingPairingOfferInfo {
        issuer: offer.issuer,
        name: offer.name,
        expires_at_unix_seconds: offer.expires_at_unix_seconds,
        locator: convert(offer.locator)?,
    })
}

#[derive(Clone, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingPairingPhase {
    AwaitingReady,
    AwaitingApproval,
    Granted,
    Rejected,
    Expired,
    Disconnected,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingPairingStatus {
    pub attempt_id: String,
    pub peer_id: String,
    pub name: String,
    pub phase: BindingPairingPhase,
    pub verification_code: Option<String>,
    pub expires_at_unix_seconds: u64,
}
#[derive(Clone, serde::Deserialize, uniffi::Enum)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum BindingContactConfirmation {
    Pending { attempt_id: String },
    Confirmed { attempt_id: String },
}
#[derive(Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct BindingDeviceLocator {
    pub relay_urls: Vec<String>,
    pub direct_addresses: Vec<String>,
    pub lookup_urls: Vec<String>,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingDeviceContact {
    pub peer_id: String,
    pub name: String,
    pub locator: BindingDeviceLocator,
    pub confirmation: BindingContactConfirmation,
    pub paused: bool,
    pub locator_source: BindingLocatorSource,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingIncomingControlGrant {
    pub peer_id: String,
    pub name: String,
    pub attempt_id: String,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingConfigurationResetNotice {
    pub id: String,
}
#[derive(Clone, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingPendingStatus {
    Recorded,
    Running,
    Completed,
    BusinessFailed,
    TransportFailed,
    Unknown,
    ResultExpired,
    ServerRestarted,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingRemoteExecutionContext {
    pub expected_revision: Option<u64>,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingPendingOperation {
    pub peer_id: String,
    pub request_id: BindingRemoteRequestId,
    pub view: String,
    pub execution: BindingRemoteExecutionContext,
    pub kind: BindingRemoteOperationKind,
    pub runtime_id: Option<String>,
    pub conversation_id: Option<String>,
    pub status: BindingPendingStatus,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingRemoteServiceSnapshot {
    pub version: u64,
    pub device_id: String,
    pub control_ingress: bool,
    pub descriptor: Option<BindingRemoteEndpointDescriptor>,
    pub contacts: Vec<BindingDeviceContact>,
    pub incoming_grants: Vec<BindingIncomingControlGrant>,
    pub incoming_pairings: Vec<BindingPairingStatus>,
    pub outgoing_pairings: Vec<BindingPairingStatus>,
    pub pending: Vec<BindingPendingOperation>,
    pub controllers: Vec<BindingControllerStatus>,
    pub network: BindingNetworkState,
    pub reset_notice: Option<BindingConfigurationResetNotice>,
}

#[derive(Clone, serde::Deserialize, uniffi::Enum)]
#[serde(from = "velune_application_api::CommandKind")]
pub enum BindingRemoteOperationKind {
    ReleaseView,
    RuntimeResourceCatalog,
    ReadRuntimeResource,
    WriteRuntimeResource,
    DeleteRuntimeResource,
    ListResourceTemplates,
    ReadResourceTemplate,
    SaveResourceTemplate,
    DeleteResourceTemplate,
    ImportRuntimeResource,
    AdoptRuntimeResource,
    ResourceSyncStatus,
    ResourceDrift,
    UnmanagedRuntimeResources,
    ApplyResourceTemplate,
    ResourceNativeImport,
    ResolveResourceBinding,
    AnalyticsQuery,
    RuntimeDiscoveryHints,
    DiscoverRuntimes,
    ListConfiguration,
    SetOutlinePresentation,
    SetLinkedConversationOpening,
    SetUsageDisplay,
    SetStatusLine,
    SetConversationBrowserPreferences,
    ComposerDrafts,
    SetComposerDraft,
    SetConversationBrowserLimits,
    OrganizeConversations,
    SetProject,
    LoadMoreConversations,
    ShowArchivedConversations,
    SearchConversations,
    FindConversation,
    UpsertRuntime,
    UpsertRuntimeWithExecution,
    UpsertAcpRuntime,
    AcpLaunch,
    AcpRuntimeInfo,
    OpenAcpConversation,
    AcpSessionConfig,
    SetAcpSessionConfig,
    DeleteRuntime,
    SelectRuntime,
    RuntimeExecution,
    RuntimeHomeDirectory,
    ConfigureRuntimeExecution,
    CreateConversation,
    CreateConversationWithSelection,
    RuntimeModels,
    NextTurnModelSelection,
    OpenConversation,
    ReadConversation,
    RenameConversation,
    DeleteConversation,
    Snapshot,
    SnapshotUpdate,
    MessagePayloads,
    SendTurn,
    SendTurnWithSelection,
    ReasoningEfforts,
    SendQueue,
    SteerTurn,
    EditMessage,
    Cancel,
    StopSubAgents,
    ConversationActivity,
    WorkingChanges,
    WorkingBranch,
    FileDiff,
    PreviewProviderImport,
    ApplyProviderImport,
    ProviderQuota,
    RefreshProviderQuota,
    SaveProvider,
    DeleteProvider,
    SaveModelRoute,
    DeleteModelRoute,
    ReadProviderApiKey,
    SaveModelTemplate,
    DeleteModelTemplate,
    ReplyRuntimeInteraction,
    AuthenticationInspect,
    AuthenticationStart,
    AuthenticationLogout,
    AuthenticationPoll,
    AuthenticationReply,
    AuthenticationCancel,
    FetchPublicModelCatalog,
    UploadAttachment,
    ReadAttachment,
}
impl From<velune_application_api::CommandKind> for BindingRemoteOperationKind {
    fn from(kind: velune_application_api::CommandKind) -> Self {
        use velune_application_api::CommandKind;
        match kind {
            CommandKind::ReleaseView => Self::ReleaseView,
            CommandKind::RuntimeResourceCatalog => Self::RuntimeResourceCatalog,
            CommandKind::ReadRuntimeResource => Self::ReadRuntimeResource,
            CommandKind::WriteRuntimeResource => Self::WriteRuntimeResource,
            CommandKind::DeleteRuntimeResource => Self::DeleteRuntimeResource,
            CommandKind::ListResourceTemplates => Self::ListResourceTemplates,
            CommandKind::ReadResourceTemplate => Self::ReadResourceTemplate,
            CommandKind::SaveResourceTemplate => Self::SaveResourceTemplate,
            CommandKind::DeleteResourceTemplate => Self::DeleteResourceTemplate,
            CommandKind::ImportRuntimeResource => Self::ImportRuntimeResource,
            CommandKind::AdoptRuntimeResource => Self::AdoptRuntimeResource,
            CommandKind::ResourceSyncStatus => Self::ResourceSyncStatus,
            CommandKind::ResourceDrift => Self::ResourceDrift,
            CommandKind::UnmanagedRuntimeResources => Self::UnmanagedRuntimeResources,
            CommandKind::ApplyResourceTemplate => Self::ApplyResourceTemplate,
            CommandKind::ResourceNativeImport => Self::ResourceNativeImport,
            CommandKind::ResolveResourceBinding => Self::ResolveResourceBinding,
            CommandKind::AnalyticsQuery => Self::AnalyticsQuery,
            CommandKind::RuntimeDiscoveryHints => Self::RuntimeDiscoveryHints,
            CommandKind::DiscoverRuntimes => Self::DiscoverRuntimes,
            CommandKind::List => Self::ListConfiguration,
            CommandKind::SetOutlinePresentation => Self::SetOutlinePresentation,
            CommandKind::SetLinkedConversationOpening => Self::SetLinkedConversationOpening,
            CommandKind::SetUsageDisplay => Self::SetUsageDisplay,
            CommandKind::SetStatusLine => Self::SetStatusLine,
            CommandKind::SetConversationBrowserPreferences => {
                Self::SetConversationBrowserPreferences
            }
            CommandKind::ComposerDrafts => Self::ComposerDrafts,
            CommandKind::SetComposerDraft => Self::SetComposerDraft,
            CommandKind::SetConversationBrowserLimits => Self::SetConversationBrowserLimits,
            CommandKind::OrganizeConversations => Self::OrganizeConversations,
            CommandKind::SetProject => Self::SetProject,
            CommandKind::LoadMoreConversations => Self::LoadMoreConversations,
            CommandKind::ShowArchivedConversations => Self::ShowArchivedConversations,
            CommandKind::SearchConversations => Self::SearchConversations,
            CommandKind::FindConversation => Self::FindConversation,
            CommandKind::UpsertRuntime => Self::UpsertRuntime,
            CommandKind::UpsertRuntimeWithExecution => Self::UpsertRuntimeWithExecution,
            CommandKind::UpsertAcpRuntime => Self::UpsertAcpRuntime,
            CommandKind::AcpLaunch => Self::AcpLaunch,
            CommandKind::AcpRuntimeInfo => Self::AcpRuntimeInfo,
            CommandKind::OpenAcpConversation => Self::OpenAcpConversation,
            CommandKind::AcpSessionConfig => Self::AcpSessionConfig,
            CommandKind::SetAcpSessionConfig => Self::SetAcpSessionConfig,
            CommandKind::DeleteRuntime => Self::DeleteRuntime,
            CommandKind::SelectRuntime => Self::SelectRuntime,
            CommandKind::RuntimeExecution => Self::RuntimeExecution,
            CommandKind::RuntimeHomeDirectory => Self::RuntimeHomeDirectory,
            CommandKind::ConfigureRuntimeExecution => Self::ConfigureRuntimeExecution,
            CommandKind::CreateConversation => Self::CreateConversation,
            CommandKind::CreateConversationWithSelection => Self::CreateConversationWithSelection,
            CommandKind::RuntimeModels => Self::RuntimeModels,
            CommandKind::NextTurnModelSelection => Self::NextTurnModelSelection,
            CommandKind::OpenConversation => Self::OpenConversation,
            CommandKind::ReadConversation => Self::ReadConversation,
            CommandKind::RenameConversation => Self::RenameConversation,
            CommandKind::DeleteConversation => Self::DeleteConversation,
            CommandKind::Snapshot => Self::Snapshot,
            CommandKind::SnapshotUpdate => Self::SnapshotUpdate,
            CommandKind::MessagePayloads => Self::MessagePayloads,
            CommandKind::SendTurn => Self::SendTurn,
            CommandKind::SendTurnWithSelection => Self::SendTurnWithSelection,
            CommandKind::ReasoningEfforts => Self::ReasoningEfforts,
            CommandKind::SendQueue => Self::SendQueue,
            CommandKind::SteerTurn => Self::SteerTurn,
            CommandKind::EditMessage => Self::EditMessage,
            CommandKind::Cancel => Self::Cancel,
            CommandKind::StopSubAgents => Self::StopSubAgents,
            CommandKind::ConversationActivity => Self::ConversationActivity,
            CommandKind::WorkingChanges => Self::WorkingChanges,
            CommandKind::WorkingBranch => Self::WorkingBranch,
            CommandKind::FileDiff => Self::FileDiff,
            CommandKind::PreviewProviderImport => Self::PreviewProviderImport,
            CommandKind::ApplyProviderImport => Self::ApplyProviderImport,
            CommandKind::ProviderQuota => Self::ProviderQuota,
            CommandKind::RefreshProviderQuota => Self::RefreshProviderQuota,
            CommandKind::SaveProvider => Self::SaveProvider,
            CommandKind::DeleteProvider => Self::DeleteProvider,
            CommandKind::SaveModelRoute => Self::SaveModelRoute,
            CommandKind::DeleteModelRoute => Self::DeleteModelRoute,
            CommandKind::ReadProviderApiKey => Self::ReadProviderApiKey,
            CommandKind::SaveModelTemplate => Self::SaveModelTemplate,
            CommandKind::DeleteModelTemplate => Self::DeleteModelTemplate,
            CommandKind::ReplyRuntimeInteraction => Self::ReplyRuntimeInteraction,
            CommandKind::AuthenticationInspect => Self::AuthenticationInspect,
            CommandKind::AuthenticationStart => Self::AuthenticationStart,
            CommandKind::AuthenticationLogout => Self::AuthenticationLogout,
            CommandKind::AuthenticationPoll => Self::AuthenticationPoll,
            CommandKind::AuthenticationReply => Self::AuthenticationReply,
            CommandKind::AuthenticationCancel => Self::AuthenticationCancel,
            CommandKind::FetchPublicModelCatalog => Self::FetchPublicModelCatalog,
            CommandKind::UploadAttachment => Self::UploadAttachment,
            CommandKind::ReadAttachment => Self::ReadAttachment,
        }
    }
}

#[derive(Clone, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingControllerPhase {
    Disconnected,
    Suspended,
    Discovering,
    Connecting,
    Connected,
    Recovering,
    Blocked,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingControllerStatus {
    pub peer_id: String,
    pub phase: BindingControllerPhase,
    pub selected: bool,
    pub foreground: bool,
    pub paused: bool,
    pub intent_revision: u64,
    pub connection_epoch: u64,
    pub error_code: Option<String>,
}

#[derive(Clone, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingLocatorSource {
    PairingOffer,
    AuthenticatedPeer,
    ExplicitImport,
}
#[derive(Clone, uniffi::Record)]
pub struct BindingLocatorUpdate {
    pub peer_id: String,
    pub locator: BindingDeviceLocator,
}

#[derive(Clone, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingNetworkApplicationPhase {
    Unapplied,
    Applying,
    Applied,
    Failed,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingRelayCredentialSummary {
    pub url: String,
    pub configured: bool,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingNetworkSettingsSummary {
    pub revision: u64,
    pub relay: BindingRelayConfig,
    pub discovery: BindingDiscoveryConfig,
    pub local_discovery: bool,
    pub path_policy: BindingRemotePathPolicy,
    pub bind_addresses: Vec<String>,
    pub request_timeout_ms: u64,
    pub credentials: Vec<BindingRelayCredentialSummary>,
    pub withdrawn_lookup_urls: Vec<String>,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingNetworkState {
    pub saved: Option<BindingNetworkSettingsSummary>,
    pub phase: BindingNetworkApplicationPhase,
    pub applied_revision: Option<u64>,
    pub generation: Option<u64>,
    pub error_code: Option<String>,
    pub facilities: Vec<BindingFacilityStatus>,
}
#[derive(Clone, serde::Serialize, uniffi::Enum)]
pub enum BindingCredentialEdit {
    Keep,
    Set { value: String },
    Remove,
}
#[derive(Clone, serde::Serialize, uniffi::Record)]
pub struct BindingRelayCredentialEdit {
    pub url: String,
    pub credential: BindingCredentialEdit,
}
#[derive(Clone, uniffi::Record)]
pub struct BindingNetworkSettingsUpdate {
    pub expected_revision: u64,
    pub network: BindingRemoteNetworkOptions,
    pub credentials: Vec<BindingRelayCredentialEdit>,
    pub restore_lookup_urls: Vec<String>,
}

#[derive(Clone, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingFacilityKind {
    HomeRelay,
    Lookup,
}
#[derive(Clone, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingFacilityReachability {
    Unknown,
    Connecting,
    Reachable,
    Unavailable,
    AuthenticationDenied,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingFacilityStatus {
    pub kind: BindingFacilityKind,
    pub url: String,
    pub reachability: BindingFacilityReachability,
}
