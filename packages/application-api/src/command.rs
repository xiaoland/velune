use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A host-assigned view key, scoped by the authenticated device at the network boundary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ClientViewId(pub String);
uniffi::custom_newtype!(ClientViewId, String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision(pub u64);
uniffi::custom_newtype!(Revision, u64);

/// A revision is mandatory for configuration and destructive edits over remote connections.
#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
pub struct ExecutionContext {
    pub expected_revision: Option<Revision>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ApplicationReply {
    pub revision: Revision,
    pub response: Response,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct AttachmentReference {
    /// Opaque execution-owner reference; never a controller's filesystem path.
    pub reference: String,
    pub name: String,
    pub bytes: u64,
}
#[derive(Clone, Serialize, Deserialize, uniffi::Record)]
pub struct AttachmentContent {
    pub media_type: Option<String>,
    #[serde(with = "crate::bytes")]
    pub bytes: Vec<u8>,
}

/// Delivery safety is separate from configuration conflict/revision semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryClass {
    Observation,
    Effect,
}

/// Safe operation identity; it contains no command body, attachment, or authentication material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum CommandKind {
    ReleaseView,
    ListResourceTemplates,
    ReadResourceTemplate,
    SaveResourceTemplate,
    DeleteResourceTemplate,
    ImportRuntimeResource,
    AdoptRuntimeResource,
    ReadResourceSyncStatus,
    ReadResourceDrift,
    ListUnmanagedRuntimeResources,
    ApplyResourceTemplate,
    ReadResourceNativeImport,
    ResolveResourceBinding,
    QueryAnalytics,
    ReadRuntimeDiscoveryHints,
    DiscoverRuntimes,
    ReadConfiguration,
    SetOutlinePresentation,
    SetLinkedConversationOpening,
    SetUsageDisplay,
    SetContextUsageDisplay,
    SetStatusLine,
    SetConversationBrowserPreferences,
    ReadComposerDrafts,
    SetComposerDraft,
    ReadComposerChoices,
    SetNextTurnIntent,
    SetRuntimeComposerChoices,
    SetConversationBrowserLimits,
    OrganizeConversations,
    SetProject,
    LoadMoreConversations,
    ShowArchivedConversations,
    SearchConversations,
    FindConversation,
    ResolveAgentUri,
    UpsertRuntime,
    UpsertRuntimeWithExecution,
    UpsertAcpRuntime,
    ReadAcpLaunch,
    ReadAcpRuntimeInfo,
    ReadAcpSessionConfig,
    SetAcpSessionConfig,
    DeleteRuntime,
    SelectRuntime,
    ReadRuntimeExecution,
    ReadRuntimeHomeDirectory,
    CreateConversation,
    ReadRuntimeModels,
    ReadComposerQuota,
    OpenConversation,
    ReadConversation,
    RenameConversation,
    DeleteConversation,
    ReadSnapshot,
    ReadSnapshotUpdate,
    ReadMessagePayloads,
    SendTurn,
    ReadReasoningEfforts,
    EditSendQueue,
    SteerTurn,
    ResumeTurn,
    CompactConversation,
    EditMessage,
    Cancel,
    StopSubAgents,
    ReadConversationActivity,
    ReadWorkingChanges,
    ReadWorkingBranch,
    ReadFileDiff,
    PreviewProviderImport,
    ApplyProviderImport,
    ReadProviderQuota,
    RefreshProviderQuota,
    ReadProviderQuotaHistory,
    ClearProviderQuotaHistory,
    ReadQuotaAlerts,
    SaveProvider,
    DeleteProvider,
    SaveModelRoute,
    DeleteModelRoute,
    ReadProviderApiKey,
    SaveModelTemplate,
    DeleteModelTemplate,
    ReplyRuntimeInteraction,
    InspectAuthentication,
    StartAuthentication,
    LogoutAuthentication,
    PollAuthentication,
    ReplyAuthentication,
    CancelAuthentication,
    FetchPublicModelCatalog,
    UploadAttachment,
    ReadAttachment,
}

/// Variants are verb phrases (`ReadSnapshot`, not `Snapshot`), and `Response`
/// and `CommandKind` repeat them. C# nests each variant as a class inside the
/// enum, so a variant named like a contract type would shadow that type for
/// its sibling variants.
#[derive(Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(
    tag = "command",
    content = "parameters",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum Command {
    /// Release this view's presentation state; running conversations are unaffected.
    ReleaseView,
    ListResourceTemplates,
    ReadResourceTemplate {
        template_id: String,
    },
    SaveResourceTemplate {
        template: crate::ResourceTemplate,
    },
    DeleteResourceTemplate {
        template_id: String,
    },
    ImportRuntimeResource {
        runtime_id: String,
        kind: crate::RuntimeResourceKind,
        path: String,
    },
    AdoptRuntimeResource {
        runtime_id: String,
        kind: crate::RuntimeResourceKind,
        path: String,
    },
    ReadResourceSyncStatus,
    ReadResourceDrift {
        runtime_id: String,
        template_id: String,
    },
    ListUnmanagedRuntimeResources {
        kind: crate::RuntimeResourceKind,
    },
    ApplyResourceTemplate {
        template_id: String,
        targets: Vec<crate::TemplateApplyTarget>,
    },
    ReadResourceNativeImport {
        runtime_id: String,
        template_id: String,
    },
    ResolveResourceBinding {
        runtime_id: String,
        template_id: String,
        resolution: crate::ResourceBindingResolution,
    },
    QueryAnalytics {
        query: AnalyticsQuery,
    },
    ReadRuntimeDiscoveryHints {
        user_home: String,
        overrides: BTreeMap<String, String>,
    },
    DiscoverRuntimes {
        probes: Vec<RuntimeDiscoveryProbe>,
    },
    ReadConfiguration,
    SetOutlinePresentation {
        presentation: OutlinePresentation,
    },
    SetLinkedConversationOpening {
        opening: LinkedConversationOpening,
    },
    SetUsageDisplay {
        display: UsageDisplay,
    },
    SetContextUsageDisplay {
        display: UsageDisplay,
    },
    SetStatusLine {
        items: Vec<StatusLineItem>,
    },
    SetConversationBrowserPreferences {
        preferences: ConversationBrowserPreferences,
    },
    ReadComposerDrafts,
    SetComposerDraft {
        conversation_id: String,
        text: String,
    },
    ReadComposerChoices,
    /// An intent without choices or runtime removes the conversation's.
    SetNextTurnIntent {
        intent: crate::config::NextTurnIntent,
    },
    SetRuntimeComposerChoices {
        choices: crate::config::RuntimeComposerChoices,
    },
    SetConversationBrowserLimits {
        limits: ConversationBrowserLimits,
    },
    OrganizeConversations {
        conversations: Vec<ConversationReference>,
        change: ConversationOrganizationChange,
    },
    SetProject {
        path: String,
        primary_folder: String,
        name: Option<String>,
        folders: Vec<String>,
    },
    LoadMoreConversations {
        group: ConversationGroupKey,
    },
    ShowArchivedConversations {
        archived: bool,
    },
    SearchConversations {
        text: String,
    },
    FindConversation {
        conversation_id: String,
    },
    ResolveAgentUri {
        uri: String,
        runtime_id: Option<String>,
    },
    UpsertRuntime {
        runtime: RuntimeInstance,
    },
    UpsertRuntimeWithExecution {
        runtime: RuntimeInstance,
        execution: crate::RuntimeExecution,
    },
    UpsertAcpRuntime {
        runtime: RuntimeInstance,
        execution: crate::RuntimeExecution,
        launch: crate::AcpLaunch,
    },
    ReadAcpLaunch {
        runtime_id: String,
    },
    ReadAcpRuntimeInfo {
        runtime_id: String,
    },
    ReadAcpSessionConfig {
        runtime_id: String,
        conversation_id: String,
    },
    SetAcpSessionConfig {
        runtime_id: String,
        conversation_id: String,
        config_id: String,
        value: crate::AcpConfigValue,
    },
    DeleteRuntime {
        id: String,
    },
    SelectRuntime {
        id: String,
    },
    ReadRuntimeExecution {
        runtime_id: String,
    },
    ReadRuntimeHomeDirectory {
        runtime_id: String,
    },
    CreateConversation {
        runtime_id: String,
        cwd: Option<String>,
        selection: Option<ModelSelection>,
    },
    ReadRuntimeModels {
        runtime_id: String,
    },
    ReadComposerQuota {
        context: ComposerQuotaContext,
        refresh: bool,
    },
    OpenConversation {
        runtime_id: String,
        conversation_id: String,
    },
    ReadConversation {
        runtime_id: String,
        conversation_id: String,
    },
    RenameConversation {
        runtime_id: String,
        conversation_id: String,
        title: String,
    },
    DeleteConversation {
        runtime_id: String,
        conversation_id: String,
    },
    ReadSnapshot {
        runtime_id: String,
        conversation_id: String,
    },
    ReadSnapshotUpdate {
        runtime_id: String,
        conversation_id: String,
        since: crate::TranscriptCursor,
    },
    ReadMessagePayloads {
        runtime_id: String,
        conversation_id: String,
        message_ids: Vec<String>,
    },
    SendTurn {
        source_runtime_id: String,
        conversation_id: String,
        target_runtime_id: String,
        selection: Option<ModelSelection>,
        turn: ComposedTurn,
    },
    ReadReasoningEfforts {
        runtime_id: String,
        model_record_key: String,
    },
    EditSendQueue {
        runtime_id: String,
        conversation_id: String,
        operation: crate::SendQueueOperation,
    },
    SteerTurn {
        runtime_id: String,
        conversation_id: String,
        turn: ComposedTurn,
    },
    CompactConversation {
        runtime_id: String,
        conversation_id: String,
        selection: Option<ModelSelection>,
    },
    ResumeTurn {
        runtime_id: String,
        conversation_id: String,
        selection: Option<ModelSelection>,
        effort: Option<String>,
        permission_mode: Option<String>,
    },
    EditMessage {
        runtime_id: String,
        conversation_id: String,
        message_id: String,
        placement: crate::conversation::EditPlacement,
        selection: Option<ModelSelection>,
        turn: ComposedTurn,
    },
    Cancel {
        runtime_id: String,
        conversation_id: String,
    },
    StopSubAgents {
        runtime_id: String,
        conversation_id: String,
    },
    ReadConversationActivity {
        since: u64,
    },
    ReadWorkingChanges {
        runtime_id: String,
        conversation_id: String,
    },
    ReadWorkingBranch {
        runtime_id: String,
        conversation_id: String,
    },
    ReadFileDiff {
        runtime_id: String,
        conversation_id: String,
        path: String,
    },
    /// Providers and models the runtime instance's own configuration names.
    PreviewProviderImport {
        gateway_id: String,
        runtime_id: String,
    },
    ApplyProviderImport {
        gateway_id: String,
        runtime_id: String,
        preview_token: String,
        selections: Vec<ImportSelection>,
        replace_existing: bool,
    },
    ReadProviderQuota {
        gateway_id: String,
        provider_id: String,
    },
    RefreshProviderQuota {
        gateway_id: String,
        provider_id: String,
    },
    /// Observations retained since `since_ms` for the current binding.
    ReadProviderQuotaHistory {
        gateway_id: String,
        provider_id: String,
        since_ms: i64,
    },
    ClearProviderQuotaHistory {
        gateway_id: String,
        provider_id: String,
    },
    ReadQuotaAlerts {
        since: u64,
    },
    SaveProvider {
        gateway_id: String,
        provider: ProviderDraft,
        authentication_edit: crate::AuthenticationEdit,
        /// None keeps the provider's quota query credentials.
        #[serde(default)]
        quota_authentication_edit: Option<crate::QuotaAuthenticationEdit>,
    },
    DeleteProvider {
        gateway_id: String,
        provider_id: String,
    },
    SaveModelRoute {
        gateway_id: String,
        route: crate::config::ModelRoute,
    },
    DeleteModelRoute {
        gateway_id: String,
        route_record_key: String,
    },
    ReadProviderApiKey {
        gateway_id: String,
        provider_id: String,
    },
    SaveModelTemplate {
        template: ModelTemplate,
    },
    DeleteModelTemplate {
        id: String,
    },
    ReplyRuntimeInteraction {
        runtime_id: String,
        conversation_id: String,
        interaction_id: String,
        reply: velune_conversation::RuntimeInteractionReply,
    },
    InspectAuthentication {
        gateway_id: String,
        provider_id: String,
    },
    StartAuthentication {
        gateway_id: String,
        provider_id: String,
    },
    LogoutAuthentication {
        gateway_id: String,
        provider_id: String,
    },
    PollAuthentication,
    ReplyAuthentication {
        prompt_id: String,
        value: String,
    },
    CancelAuthentication,
    FetchPublicModelCatalog,
    UploadAttachment {
        name: String,
        #[serde(with = "crate::bytes")]
        bytes: Vec<u8>,
    },
    ReadAttachment {
        runtime_id: String,
        conversation_id: String,
        message_id: String,
        block_index: u32,
    },
}

#[derive(Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "response", content = "value", rename_all = "camelCase")]
pub enum Response {
    ReleaseView,
    ListResourceTemplates(Vec<crate::ResourceTemplateSummary>),
    ReadResourceTemplate(crate::ResourceTemplate),
    SaveResourceTemplate(crate::ResourceSaveResult),
    DeleteResourceTemplate,
    ImportRuntimeResource(crate::ResourceTemplateImport),
    AdoptRuntimeResource(crate::ResourceTemplateSummary),
    ReadResourceSyncStatus(Vec<crate::ResourceSyncStatus>),
    ReadResourceDrift(Vec<crate::ResourceDriftFile>),
    ListUnmanagedRuntimeResources(Vec<crate::UnmanagedRuntimeResource>),
    ApplyResourceTemplate(Vec<crate::TemplateApplyReport>),
    ReadResourceNativeImport(crate::ResourceTemplateImport),
    ResolveResourceBinding,
    QueryAnalytics(AnalyticsReport),
    ReadRuntimeDiscoveryHints(Vec<RuntimeDiscoveryHint>),
    DiscoverRuntimes(Vec<RuntimeDiscoveryCandidate>),
    ReadConfiguration(ConfigurationSnapshot),
    SetOutlinePresentation(OutlinePresentation),
    SetLinkedConversationOpening(LinkedConversationOpening),
    SetUsageDisplay(UsageDisplay),
    SetContextUsageDisplay(UsageDisplay),
    SetStatusLine(Vec<StatusLineItem>),
    SetConversationBrowserPreferences(ConfigurationSnapshot),
    ReadComposerDrafts(Vec<crate::config::ComposerDraft>),
    SetComposerDraft,
    ReadComposerChoices(crate::config::ComposerChoices),
    SetNextTurnIntent,
    SetRuntimeComposerChoices,
    SetConversationBrowserLimits(ConversationBrowserLimits),
    OrganizeConversations(ConfigurationSnapshot),
    SetProject(ConfigurationSnapshot),
    LoadMoreConversations(ConfigurationSnapshot),
    ShowArchivedConversations(ConfigurationSnapshot),
    SearchConversations(ConversationSearch),
    FindConversation(Option<ListedConversation>),
    ResolveAgentUri(AgentUriResolution),
    UpsertRuntime(RuntimeUpdate),
    UpsertRuntimeWithExecution(RuntimeUpdate),
    UpsertAcpRuntime(RuntimeUpdate),
    ReadAcpLaunch(crate::AcpLaunch),
    ReadAcpRuntimeInfo(crate::AcpRuntimeInfo),
    ReadAcpSessionConfig(Vec<crate::AcpConfigOption>),
    SetAcpSessionConfig(Vec<crate::AcpConfigOption>),
    DeleteRuntime(RuntimeUpdate),
    SelectRuntime(ConfigurationSnapshot),
    ReadRuntimeExecution(crate::RuntimeExecution),
    ReadRuntimeHomeDirectory(Option<String>),
    CreateConversation(SnapshotResult),
    ReadRuntimeModels(Vec<RuntimeModel>),
    ReadComposerQuota(Vec<ComposerQuotaEntry>),
    OpenConversation(SnapshotResult),
    ReadConversation(SnapshotResult),
    RenameConversation(ConfigurationSnapshot),
    DeleteConversation(ConfigurationSnapshot),
    ReadSnapshot(SnapshotResult),
    ReadSnapshotUpdate(crate::SnapshotUpdate),
    ReadMessagePayloads(Vec<crate::DeliveredMessage>),
    SendTurn(SnapshotResult),
    ReadReasoningEfforts(Vec<String>),
    EditSendQueue(SnapshotResult),
    SteerTurn(SnapshotResult),
    ResumeTurn(SnapshotResult),
    CompactConversation(SnapshotResult),
    EditMessage(SnapshotResult),
    Cancel(SnapshotResult),
    StopSubAgents(SnapshotResult),
    ReadConversationActivity(ConversationActivity),
    ReadWorkingChanges(WorkingChanges),
    ReadWorkingBranch(Option<String>),
    ReadFileDiff(FileDiff),
    PreviewProviderImport(ProviderImportPreview),
    ApplyProviderImport(ImportResult),
    ReadProviderQuota(ProviderQuotaStatus),
    RefreshProviderQuota(ProviderQuotaStatus),
    ReadProviderQuotaHistory(ProviderQuotaHistory),
    ClearProviderQuotaHistory(ProviderQuotaHistory),
    ReadQuotaAlerts(QuotaAlerts),
    SaveProvider(GatewayUpdate),
    DeleteProvider(GatewayUpdate),
    SaveModelRoute(GatewayUpdate),
    DeleteModelRoute(GatewayUpdate),
    ReadProviderApiKey(String),
    SaveModelTemplate(Vec<ModelTemplate>),
    DeleteModelTemplate(Vec<ModelTemplate>),
    ReplyRuntimeInteraction(SnapshotResult),
    InspectAuthentication(AuthenticationMetadata),
    StartAuthentication(AuthenticationProgress),
    LogoutAuthentication(GatewayUpdate),
    PollAuthentication(AuthenticationProgress),
    ReplyAuthentication(AuthenticationProgress),
    CancelAuthentication(AuthenticationProgress),
    FetchPublicModelCatalog(Vec<crate::CatalogModel>),
    UploadAttachment(crate::AttachmentReference),
    ReadAttachment(crate::AttachmentContent),
}

impl Command {
    /// Exhaustive classification: new commands must declare whether an uncertain delivery may be reissued.
    pub fn delivery_class(&self) -> DeliveryClass {
        match self {
            Self::ListResourceTemplates
            | Self::ReadResourceTemplate { .. }
            | Self::ImportRuntimeResource { .. }
            | Self::ReadResourceSyncStatus
            | Self::ReadResourceDrift { .. }
            | Self::ListUnmanagedRuntimeResources { .. }
            | Self::ReadResourceNativeImport { .. }
            | Self::QueryAnalytics { .. }
            | Self::ReadRuntimeDiscoveryHints { .. }
            | Self::DiscoverRuntimes { .. }
            | Self::ReadConfiguration
            | Self::ReadComposerDrafts
            | Self::ReadComposerChoices
            | Self::SearchConversations { .. }
            | Self::FindConversation { .. }
            | Self::ResolveAgentUri { .. }
            | Self::ReadAcpLaunch { .. }
            | Self::ReadAcpRuntimeInfo { .. }
            | Self::ReadAcpSessionConfig { .. }
            | Self::ReadRuntimeExecution { .. }
            | Self::ReadRuntimeHomeDirectory { .. }
            | Self::ReadRuntimeModels { .. }
            | Self::ReadComposerQuota { .. }
            | Self::ReadConversation { .. }
            | Self::ReadSnapshot { .. }
            | Self::ReadSnapshotUpdate { .. }
            | Self::ReadMessagePayloads { .. }
            | Self::ReadReasoningEfforts { .. }
            | Self::ReadConversationActivity { .. }
            | Self::ReadWorkingChanges { .. }
            | Self::ReadWorkingBranch { .. }
            | Self::ReadFileDiff { .. }
            | Self::PreviewProviderImport { .. }
            | Self::ReadProviderQuota { .. }
            | Self::ReadProviderQuotaHistory { .. }
            | Self::ReadQuotaAlerts { .. }
            | Self::ReadProviderApiKey { .. }
            | Self::FetchPublicModelCatalog
            | Self::ReadAttachment { .. } => DeliveryClass::Observation,
            Self::ReleaseView
            | Self::SaveResourceTemplate { .. }
            | Self::DeleteResourceTemplate { .. }
            | Self::AdoptRuntimeResource { .. }
            | Self::ApplyResourceTemplate { .. }
            | Self::ResolveResourceBinding { .. }
            | Self::SetOutlinePresentation { .. }
            | Self::SetLinkedConversationOpening { .. }
            | Self::SetUsageDisplay { .. }
            | Self::SetContextUsageDisplay { .. }
            | Self::SetStatusLine { .. }
            | Self::SetConversationBrowserPreferences { .. }
            | Self::SetComposerDraft { .. }
            | Self::SetNextTurnIntent { .. }
            | Self::SetRuntimeComposerChoices { .. }
            | Self::SetConversationBrowserLimits { .. }
            | Self::OrganizeConversations { .. }
            | Self::SetProject { .. }
            | Self::LoadMoreConversations { .. }
            | Self::ShowArchivedConversations { .. }
            | Self::UpsertRuntime { .. }
            | Self::UpsertRuntimeWithExecution { .. }
            | Self::UpsertAcpRuntime { .. }
            | Self::SetAcpSessionConfig { .. }
            | Self::DeleteRuntime { .. }
            | Self::SelectRuntime { .. }
            | Self::CreateConversation { .. }
            | Self::OpenConversation { .. }
            | Self::RenameConversation { .. }
            | Self::DeleteConversation { .. }
            | Self::SendTurn { .. }
            | Self::EditSendQueue { .. }
            | Self::SteerTurn { .. }
            | Self::ResumeTurn { .. }
            | Self::CompactConversation { .. }
            | Self::EditMessage { .. }
            | Self::Cancel { .. }
            | Self::StopSubAgents { .. }
            | Self::ApplyProviderImport { .. }
            | Self::RefreshProviderQuota { .. }
            | Self::ClearProviderQuotaHistory { .. }
            | Self::SaveProvider { .. }
            | Self::DeleteProvider { .. }
            | Self::SaveModelRoute { .. }
            | Self::DeleteModelRoute { .. }
            | Self::SaveModelTemplate { .. }
            | Self::DeleteModelTemplate { .. }
            | Self::ReplyRuntimeInteraction { .. }
            | Self::InspectAuthentication { .. }
            | Self::StartAuthentication { .. }
            | Self::LogoutAuthentication { .. }
            | Self::PollAuthentication
            | Self::ReplyAuthentication { .. }
            | Self::CancelAuthentication
            | Self::UploadAttachment { .. } => DeliveryClass::Effect,
        }
    }
    pub fn kind(&self) -> CommandKind {
        match self {
            Self::ReleaseView => CommandKind::ReleaseView,
            Self::ListResourceTemplates => CommandKind::ListResourceTemplates,
            Self::ReadResourceTemplate { .. } => CommandKind::ReadResourceTemplate,
            Self::SaveResourceTemplate { .. } => CommandKind::SaveResourceTemplate,
            Self::DeleteResourceTemplate { .. } => CommandKind::DeleteResourceTemplate,
            Self::ImportRuntimeResource { .. } => CommandKind::ImportRuntimeResource,
            Self::AdoptRuntimeResource { .. } => CommandKind::AdoptRuntimeResource,
            Self::ReadResourceSyncStatus => CommandKind::ReadResourceSyncStatus,
            Self::ReadResourceDrift { .. } => CommandKind::ReadResourceDrift,
            Self::ListUnmanagedRuntimeResources { .. } => {
                CommandKind::ListUnmanagedRuntimeResources
            }
            Self::ApplyResourceTemplate { .. } => CommandKind::ApplyResourceTemplate,
            Self::ReadResourceNativeImport { .. } => CommandKind::ReadResourceNativeImport,
            Self::ResolveResourceBinding { .. } => CommandKind::ResolveResourceBinding,
            Self::QueryAnalytics { .. } => CommandKind::QueryAnalytics,
            Self::ReadRuntimeDiscoveryHints { .. } => CommandKind::ReadRuntimeDiscoveryHints,
            Self::DiscoverRuntimes { .. } => CommandKind::DiscoverRuntimes,
            Self::ReadConfiguration => CommandKind::ReadConfiguration,
            Self::SetOutlinePresentation { .. } => CommandKind::SetOutlinePresentation,
            Self::SetLinkedConversationOpening { .. } => CommandKind::SetLinkedConversationOpening,
            Self::SetUsageDisplay { .. } => CommandKind::SetUsageDisplay,
            Self::SetContextUsageDisplay { .. } => CommandKind::SetContextUsageDisplay,
            Self::SetStatusLine { .. } => CommandKind::SetStatusLine,
            Self::SetConversationBrowserPreferences { .. } => {
                CommandKind::SetConversationBrowserPreferences
            }
            Self::ReadComposerDrafts => CommandKind::ReadComposerDrafts,
            Self::SetComposerDraft { .. } => CommandKind::SetComposerDraft,
            Self::ReadComposerChoices => CommandKind::ReadComposerChoices,
            Self::SetNextTurnIntent { .. } => CommandKind::SetNextTurnIntent,
            Self::SetRuntimeComposerChoices { .. } => CommandKind::SetRuntimeComposerChoices,
            Self::SetConversationBrowserLimits { .. } => CommandKind::SetConversationBrowserLimits,
            Self::OrganizeConversations { .. } => CommandKind::OrganizeConversations,
            Self::SetProject { .. } => CommandKind::SetProject,
            Self::LoadMoreConversations { .. } => CommandKind::LoadMoreConversations,
            Self::ShowArchivedConversations { .. } => CommandKind::ShowArchivedConversations,
            Self::SearchConversations { .. } => CommandKind::SearchConversations,
            Self::FindConversation { .. } => CommandKind::FindConversation,
            Self::ResolveAgentUri { .. } => CommandKind::ResolveAgentUri,
            Self::UpsertRuntime { .. } => CommandKind::UpsertRuntime,
            Self::UpsertRuntimeWithExecution { .. } => CommandKind::UpsertRuntimeWithExecution,
            Self::UpsertAcpRuntime { .. } => CommandKind::UpsertAcpRuntime,
            Self::ReadAcpLaunch { .. } => CommandKind::ReadAcpLaunch,
            Self::ReadAcpRuntimeInfo { .. } => CommandKind::ReadAcpRuntimeInfo,
            Self::ReadAcpSessionConfig { .. } => CommandKind::ReadAcpSessionConfig,
            Self::SetAcpSessionConfig { .. } => CommandKind::SetAcpSessionConfig,
            Self::DeleteRuntime { .. } => CommandKind::DeleteRuntime,
            Self::SelectRuntime { .. } => CommandKind::SelectRuntime,
            Self::ReadRuntimeExecution { .. } => CommandKind::ReadRuntimeExecution,
            Self::ReadRuntimeHomeDirectory { .. } => CommandKind::ReadRuntimeHomeDirectory,
            Self::CreateConversation { .. } => CommandKind::CreateConversation,
            Self::ReadRuntimeModels { .. } => CommandKind::ReadRuntimeModels,
            Self::ReadComposerQuota { .. } => CommandKind::ReadComposerQuota,
            Self::OpenConversation { .. } => CommandKind::OpenConversation,
            Self::ReadConversation { .. } => CommandKind::ReadConversation,
            Self::RenameConversation { .. } => CommandKind::RenameConversation,
            Self::DeleteConversation { .. } => CommandKind::DeleteConversation,
            Self::ReadSnapshot { .. } => CommandKind::ReadSnapshot,
            Self::ReadSnapshotUpdate { .. } => CommandKind::ReadSnapshotUpdate,
            Self::ReadMessagePayloads { .. } => CommandKind::ReadMessagePayloads,
            Self::SendTurn { .. } => CommandKind::SendTurn,
            Self::ReadReasoningEfforts { .. } => CommandKind::ReadReasoningEfforts,
            Self::EditSendQueue { .. } => CommandKind::EditSendQueue,
            Self::SteerTurn { .. } => CommandKind::SteerTurn,
            Self::ResumeTurn { .. } => CommandKind::ResumeTurn,
            Self::CompactConversation { .. } => CommandKind::CompactConversation,
            Self::EditMessage { .. } => CommandKind::EditMessage,
            Self::Cancel { .. } => CommandKind::Cancel,
            Self::StopSubAgents { .. } => CommandKind::StopSubAgents,
            Self::ReadConversationActivity { .. } => CommandKind::ReadConversationActivity,
            Self::ReadWorkingChanges { .. } => CommandKind::ReadWorkingChanges,
            Self::ReadWorkingBranch { .. } => CommandKind::ReadWorkingBranch,
            Self::ReadFileDiff { .. } => CommandKind::ReadFileDiff,
            Self::PreviewProviderImport { .. } => CommandKind::PreviewProviderImport,
            Self::ApplyProviderImport { .. } => CommandKind::ApplyProviderImport,
            Self::ReadProviderQuota { .. } => CommandKind::ReadProviderQuota,
            Self::RefreshProviderQuota { .. } => CommandKind::RefreshProviderQuota,
            Self::ReadProviderQuotaHistory { .. } => CommandKind::ReadProviderQuotaHistory,
            Self::ClearProviderQuotaHistory { .. } => CommandKind::ClearProviderQuotaHistory,
            Self::ReadQuotaAlerts { .. } => CommandKind::ReadQuotaAlerts,
            Self::SaveProvider { .. } => CommandKind::SaveProvider,
            Self::DeleteProvider { .. } => CommandKind::DeleteProvider,
            Self::SaveModelRoute { .. } => CommandKind::SaveModelRoute,
            Self::DeleteModelRoute { .. } => CommandKind::DeleteModelRoute,
            Self::ReadProviderApiKey { .. } => CommandKind::ReadProviderApiKey,
            Self::SaveModelTemplate { .. } => CommandKind::SaveModelTemplate,
            Self::DeleteModelTemplate { .. } => CommandKind::DeleteModelTemplate,
            Self::ReplyRuntimeInteraction { .. } => CommandKind::ReplyRuntimeInteraction,
            Self::InspectAuthentication { .. } => CommandKind::InspectAuthentication,
            Self::StartAuthentication { .. } => CommandKind::StartAuthentication,
            Self::LogoutAuthentication { .. } => CommandKind::LogoutAuthentication,
            Self::PollAuthentication => CommandKind::PollAuthentication,
            Self::ReplyAuthentication { .. } => CommandKind::ReplyAuthentication,
            Self::CancelAuthentication => CommandKind::CancelAuthentication,
            Self::FetchPublicModelCatalog => CommandKind::FetchPublicModelCatalog,
            Self::UploadAttachment { .. } => CommandKind::UploadAttachment,
            Self::ReadAttachment { .. } => CommandKind::ReadAttachment,
        }
    }
    /// Mutations whose stale input could replace or remove another device's work.
    pub fn requires_revision(&self) -> bool {
        matches!(
            self,
            Self::SaveProvider { .. }
                | Self::DeleteProvider { .. }
                | Self::SaveModelRoute { .. }
                | Self::DeleteModelRoute { .. }
                | Self::SaveModelTemplate { .. }
                | Self::DeleteModelTemplate { .. }
                | Self::UpsertRuntime { .. }
                | Self::UpsertRuntimeWithExecution { .. }
                | Self::UpsertAcpRuntime { .. }
                | Self::DeleteRuntime { .. }
                | Self::ApplyProviderImport { .. }
                | Self::RenameConversation { .. }
                | Self::DeleteConversation { .. }
                | Self::EditMessage { .. }
                | Self::SetAcpSessionConfig { .. }
                | Self::SaveResourceTemplate { .. }
                | Self::DeleteResourceTemplate { .. }
                | Self::ApplyResourceTemplate { .. }
                | Self::ResolveResourceBinding { .. }
                | Self::AdoptRuntimeResource { .. }
                | Self::SetProject { .. }
                | Self::OrganizeConversations { .. }
                | Self::LogoutAuthentication { .. }
        )
    }
    pub fn changes_shared_state(&self) -> bool {
        self.requires_revision()
            || matches!(
                self,
                Self::CreateConversation { .. }
                    | Self::SendTurn { .. }
                    | Self::EditSendQueue { .. }
                    | Self::SteerTurn { .. }
                    | Self::ResumeTurn { .. }
                    | Self::CompactConversation { .. }
                    | Self::ReplyRuntimeInteraction { .. }
                    | Self::Cancel { .. }
                    | Self::StopSubAgents { .. }
                    | Self::StartAuthentication { .. }
                    | Self::ReplyAuthentication { .. }
            )
    }
}
