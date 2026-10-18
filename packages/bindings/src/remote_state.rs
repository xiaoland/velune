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
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingDeviceLocator {
    pub relay_urls: Vec<String>,
    pub direct_addresses: Vec<String>,
    pub lookup_url: Option<String>,
}
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct BindingDeviceContact {
    pub peer_id: String,
    pub name: String,
    pub locator: BindingDeviceLocator,
    pub confirmation: BindingContactConfirmation,
    pub paused: bool,
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
    pub reset_notice: Option<BindingConfigurationResetNotice>,
}

#[derive(Clone, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
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
    #[serde(rename = "list")]
    ListConfiguration,
    SetOutlinePresentation,
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
