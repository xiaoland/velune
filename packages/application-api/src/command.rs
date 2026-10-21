use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A host-assigned view key, scoped by the authenticated device at the network boundary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ClientViewId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision(pub u64);

/// A revision is mandatory for configuration and destructive edits over remote connections.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionContext {
    pub expected_revision: Option<Revision>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ApplicationReply {
    pub revision: Revision,
    pub response: Response,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentReference {
    /// Opaque execution-owner reference; never a controller's filesystem path.
    pub reference: String,
    pub name: String,
    pub bytes: u64,
}
#[derive(Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommandKind {
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
    List,
    SetOutlinePresentation,
    SetLinkedConversationOpening,
    SetUsageDisplay,
    SetStatusLine,
    SetConversationBrowserPreferences,
    ComposerDrafts,
    SetComposerDraft,
    ComposerChoices,
    SetNextTurnIntent,
    SetRuntimeComposerChoices,
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

#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "command",
    content = "parameters",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum Command {
    /// Release this view's presentation state; running conversations are unaffected.
    ReleaseView,
    RuntimeResourceCatalog {
        runtime_id: String,
        cwd: Option<String>,
        kind: crate::RuntimeResourceKind,
        scope: crate::RuntimeResourceScope,
    },
    ReadRuntimeResource {
        runtime_id: String,
        cwd: Option<String>,
        key: crate::RuntimeResourceKey,
    },
    WriteRuntimeResource {
        runtime_id: String,
        cwd: Option<String>,
        key: crate::RuntimeResourceKey,
        expected: crate::RuntimeResourceRevision,
        #[serde(with = "crate::bytes")]
        bytes: Vec<u8>,
        executable: Option<bool>,
    },
    DeleteRuntimeResource {
        runtime_id: String,
        cwd: Option<String>,
        key: crate::RuntimeResourceKey,
        expected: crate::RuntimeResourceRevision,
    },
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
    ResourceSyncStatus,
    ResourceDrift {
        runtime_id: String,
        template_id: String,
    },
    UnmanagedRuntimeResources {
        kind: crate::RuntimeResourceKind,
    },
    ApplyResourceTemplate {
        template_id: String,
        targets: Vec<crate::TemplateApplyTarget>,
    },
    ResourceNativeImport {
        runtime_id: String,
        template_id: String,
    },
    ResolveResourceBinding {
        runtime_id: String,
        template_id: String,
        resolution: crate::BindingResolution,
    },
    AnalyticsQuery {
        query: AnalyticsQuery,
    },
    RuntimeDiscoveryHints {
        user_home: String,
        overrides: BTreeMap<String, String>,
    },
    DiscoverRuntimes {
        probes: Vec<RuntimeDiscoveryProbe>,
    },
    List,
    SetOutlinePresentation {
        presentation: OutlinePresentation,
    },
    SetLinkedConversationOpening {
        opening: LinkedConversationOpening,
    },
    SetUsageDisplay {
        display: UsageDisplay,
    },
    SetStatusLine {
        items: Vec<StatusLineItem>,
    },
    SetConversationBrowserPreferences {
        preferences: ConversationBrowserPreferences,
    },
    ComposerDrafts,
    SetComposerDraft {
        conversation_id: String,
        text: String,
    },
    ComposerChoices,
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
    AcpLaunch {
        runtime_id: String,
    },
    AcpRuntimeInfo {
        runtime_id: String,
    },
    OpenAcpConversation {
        runtime_id: String,
        native_session_id: String,
        cwd: String,
    },
    AcpSessionConfig {
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
    RuntimeExecution {
        runtime_id: String,
    },
    RuntimeHomeDirectory {
        runtime_id: String,
    },
    ConfigureRuntimeExecution {
        runtime_id: String,
        execution: crate::RuntimeExecution,
    },
    CreateConversation {
        runtime_id: String,
        cwd: Option<String>,
        model_record_key: String,
    },
    CreateConversationWithSelection {
        runtime_id: String,
        cwd: Option<String>,
        selection: Option<ModelSelection>,
    },
    RuntimeModels {
        runtime_id: String,
    },
    NextTurnModelSelection,
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
    Snapshot {
        runtime_id: String,
        conversation_id: String,
    },
    SnapshotUpdate {
        runtime_id: String,
        conversation_id: String,
        since: crate::TranscriptCursor,
    },
    MessagePayloads {
        runtime_id: String,
        conversation_id: String,
        message_ids: Vec<String>,
    },
    SendTurn {
        source_runtime_id: String,
        conversation_id: String,
        target_runtime_id: String,
        model_record_key: String,
        text: String,
    },
    SendTurnWithSelection {
        source_runtime_id: String,
        conversation_id: String,
        target_runtime_id: String,
        selection: Option<ModelSelection>,
        turn: ComposedTurn,
    },
    ReasoningEfforts {
        runtime_id: String,
        model_record_key: String,
    },
    SendQueue {
        runtime_id: String,
        conversation_id: String,
        operation: crate::SendQueueOperation,
    },
    SteerTurn {
        runtime_id: String,
        conversation_id: String,
        turn: ComposedTurn,
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
    ConversationActivity {
        since: u64,
    },
    WorkingChanges {
        runtime_id: String,
        conversation_id: String,
    },
    WorkingBranch {
        runtime_id: String,
        conversation_id: String,
    },
    FileDiff {
        runtime_id: String,
        conversation_id: String,
        path: String,
    },
    PreviewProviderImport {
        gateway_id: String,
        source: ProviderImportSource,
    },
    ApplyProviderImport {
        gateway_id: String,
        source: ProviderImportSource,
        preview_token: String,
        selections: Vec<ImportSelection>,
        replace_existing: bool,
    },
    ProviderQuota {
        gateway_id: String,
        provider_id: String,
    },
    RefreshProviderQuota {
        gateway_id: String,
        provider_id: String,
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
    AuthenticationInspect {
        gateway_id: String,
        provider_id: String,
    },
    AuthenticationStart {
        gateway_id: String,
        provider_id: String,
    },
    AuthenticationLogout {
        gateway_id: String,
        provider_id: String,
    },
    AuthenticationPoll,
    AuthenticationReply {
        prompt_id: String,
        value: String,
    },
    AuthenticationCancel,
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

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "response", content = "value", rename_all = "camelCase")]
pub enum Response {
    ReleaseView,
    RuntimeResourceCatalog(crate::RuntimeResourceCatalog),
    ReadRuntimeResource(crate::RuntimeResourceDocument),
    WriteRuntimeResource(crate::RuntimeResourceDocument),
    DeleteRuntimeResource,
    ListResourceTemplates(Vec<crate::ResourceTemplateSummary>),
    ReadResourceTemplate(crate::ResourceTemplate),
    SaveResourceTemplate(crate::ResourceSaveResult),
    DeleteResourceTemplate,
    ImportRuntimeResource(crate::ResourceTemplateImport),
    AdoptRuntimeResource(crate::ResourceTemplateSummary),
    ResourceSyncStatus(Vec<crate::ResourceSyncStatus>),
    ResourceDrift(Vec<crate::ResourceDriftFile>),
    UnmanagedRuntimeResources(Vec<crate::UnmanagedRuntimeResource>),
    ApplyResourceTemplate(Vec<crate::TemplateApplyReport>),
    ResourceNativeImport(crate::ResourceTemplateImport),
    ResolveResourceBinding,
    AnalyticsQuery(AnalyticsReport),
    RuntimeDiscoveryHints(Vec<RuntimeDiscoveryHint>),
    DiscoverRuntimes(Vec<RuntimeDiscoveryCandidate>),
    List(ConfigurationSnapshot),
    SetOutlinePresentation(OutlinePresentation),
    SetLinkedConversationOpening(LinkedConversationOpening),
    SetUsageDisplay(UsageDisplay),
    SetStatusLine(Vec<StatusLineItem>),
    SetConversationBrowserPreferences(ConfigurationSnapshot),
    ComposerDrafts(Vec<crate::config::ComposerDraft>),
    SetComposerDraft,
    ComposerChoices(crate::config::ComposerChoices),
    SetNextTurnIntent,
    SetRuntimeComposerChoices,
    SetConversationBrowserLimits(ConversationBrowserLimits),
    OrganizeConversations(ConfigurationSnapshot),
    SetProject(ConfigurationSnapshot),
    LoadMoreConversations(ConfigurationSnapshot),
    ShowArchivedConversations(ConfigurationSnapshot),
    SearchConversations(ConversationSearch),
    FindConversation(Option<ListedConversation>),
    UpsertRuntime(RuntimeUpdate),
    UpsertRuntimeWithExecution(RuntimeUpdate),
    UpsertAcpRuntime(RuntimeUpdate),
    AcpLaunch(crate::AcpLaunch),
    AcpRuntimeInfo(crate::AcpRuntimeInfo),
    OpenAcpConversation(SnapshotResult),
    AcpSessionConfig(Vec<crate::AcpConfigOption>),
    SetAcpSessionConfig(Vec<crate::AcpConfigOption>),
    DeleteRuntime(RuntimeUpdate),
    SelectRuntime(ConfigurationSnapshot),
    RuntimeExecution(crate::RuntimeExecution),
    RuntimeHomeDirectory(Option<String>),
    ConfigureRuntimeExecution(RuntimeUpdate),
    CreateConversation(SnapshotResult),
    CreateConversationWithSelection(SnapshotResult),
    RuntimeModels(Vec<RuntimeModel>),
    NextTurnModelSelection(Option<ModelSelection>),
    OpenConversation(SnapshotResult),
    ReadConversation(SnapshotResult),
    RenameConversation(ConfigurationSnapshot),
    DeleteConversation(ConfigurationSnapshot),
    Snapshot(SnapshotResult),
    SnapshotUpdate(crate::SnapshotUpdate),
    MessagePayloads(Vec<crate::DeliveredMessage>),
    SendTurn(SnapshotResult),
    SendTurnWithSelection(SnapshotResult),
    ReasoningEfforts(Vec<String>),
    SendQueue(SnapshotResult),
    SteerTurn(SnapshotResult),
    EditMessage(SnapshotResult),
    Cancel(SnapshotResult),
    StopSubAgents(SnapshotResult),
    ConversationActivity(ConversationActivity),
    WorkingChanges(WorkingChanges),
    WorkingBranch(Option<String>),
    FileDiff(FileDiff),
    PreviewProviderImport(ProviderImportPreview),
    ApplyProviderImport(ImportResult),
    ProviderQuota(ProviderQuotaStatus),
    RefreshProviderQuota(ProviderQuotaStatus),
    SaveProvider(GatewayUpdate),
    DeleteProvider(GatewayUpdate),
    SaveModelRoute(GatewayUpdate),
    DeleteModelRoute(GatewayUpdate),
    ReadProviderApiKey(String),
    SaveModelTemplate(Vec<ModelTemplate>),
    DeleteModelTemplate(Vec<ModelTemplate>),
    ReplyRuntimeInteraction(SnapshotResult),
    AuthenticationInspect(AuthenticationMetadata),
    AuthenticationStart(AuthenticationProgress),
    AuthenticationLogout(GatewayUpdate),
    AuthenticationPoll(AuthenticationProgress),
    AuthenticationReply(AuthenticationProgress),
    AuthenticationCancel(AuthenticationProgress),
    FetchPublicModelCatalog(Vec<crate::CatalogModel>),
    UploadAttachment(crate::AttachmentReference),
    ReadAttachment(crate::AttachmentContent),
}

impl Command {
    /// Exhaustive classification: new commands must declare whether an uncertain delivery may be reissued.
    pub fn delivery_class(&self) -> DeliveryClass {
        match self {
            Self::RuntimeResourceCatalog { .. }
            | Self::ReadRuntimeResource { .. }
            | Self::ListResourceTemplates
            | Self::ReadResourceTemplate { .. }
            | Self::ImportRuntimeResource { .. }
            | Self::ResourceSyncStatus
            | Self::ResourceDrift { .. }
            | Self::UnmanagedRuntimeResources { .. }
            | Self::ResourceNativeImport { .. }
            | Self::AnalyticsQuery { .. }
            | Self::RuntimeDiscoveryHints { .. }
            | Self::DiscoverRuntimes { .. }
            | Self::List
            | Self::ComposerDrafts
            | Self::ComposerChoices
            | Self::SearchConversations { .. }
            | Self::FindConversation { .. }
            | Self::AcpLaunch { .. }
            | Self::AcpRuntimeInfo { .. }
            | Self::AcpSessionConfig { .. }
            | Self::RuntimeExecution { .. }
            | Self::RuntimeHomeDirectory { .. }
            | Self::RuntimeModels { .. }
            | Self::NextTurnModelSelection
            | Self::ReadConversation { .. }
            | Self::Snapshot { .. }
            | Self::SnapshotUpdate { .. }
            | Self::MessagePayloads { .. }
            | Self::ReasoningEfforts { .. }
            | Self::ConversationActivity { .. }
            | Self::WorkingChanges { .. }
            | Self::WorkingBranch { .. }
            | Self::FileDiff { .. }
            | Self::PreviewProviderImport { .. }
            | Self::ProviderQuota { .. }
            | Self::ReadProviderApiKey { .. }
            | Self::FetchPublicModelCatalog
            | Self::ReadAttachment { .. } => DeliveryClass::Observation,
            Self::ReleaseView
            | Self::WriteRuntimeResource { .. }
            | Self::DeleteRuntimeResource { .. }
            | Self::SaveResourceTemplate { .. }
            | Self::DeleteResourceTemplate { .. }
            | Self::AdoptRuntimeResource { .. }
            | Self::ApplyResourceTemplate { .. }
            | Self::ResolveResourceBinding { .. }
            | Self::SetOutlinePresentation { .. }
            | Self::SetLinkedConversationOpening { .. }
            | Self::SetUsageDisplay { .. }
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
            | Self::OpenAcpConversation { .. }
            | Self::SetAcpSessionConfig { .. }
            | Self::DeleteRuntime { .. }
            | Self::SelectRuntime { .. }
            | Self::ConfigureRuntimeExecution { .. }
            | Self::CreateConversation { .. }
            | Self::CreateConversationWithSelection { .. }
            | Self::OpenConversation { .. }
            | Self::RenameConversation { .. }
            | Self::DeleteConversation { .. }
            | Self::SendTurn { .. }
            | Self::SendTurnWithSelection { .. }
            | Self::SendQueue { .. }
            | Self::SteerTurn { .. }
            | Self::EditMessage { .. }
            | Self::Cancel { .. }
            | Self::StopSubAgents { .. }
            | Self::ApplyProviderImport { .. }
            | Self::RefreshProviderQuota { .. }
            | Self::SaveProvider { .. }
            | Self::DeleteProvider { .. }
            | Self::SaveModelRoute { .. }
            | Self::DeleteModelRoute { .. }
            | Self::SaveModelTemplate { .. }
            | Self::DeleteModelTemplate { .. }
            | Self::ReplyRuntimeInteraction { .. }
            | Self::AuthenticationInspect { .. }
            | Self::AuthenticationStart { .. }
            | Self::AuthenticationLogout { .. }
            | Self::AuthenticationPoll
            | Self::AuthenticationReply { .. }
            | Self::AuthenticationCancel
            | Self::UploadAttachment { .. } => DeliveryClass::Effect,
        }
    }
    pub fn kind(&self) -> CommandKind {
        match self {
            Self::ReleaseView => CommandKind::ReleaseView,
            Self::RuntimeResourceCatalog { .. } => CommandKind::RuntimeResourceCatalog,
            Self::ReadRuntimeResource { .. } => CommandKind::ReadRuntimeResource,
            Self::WriteRuntimeResource { .. } => CommandKind::WriteRuntimeResource,
            Self::DeleteRuntimeResource { .. } => CommandKind::DeleteRuntimeResource,
            Self::ListResourceTemplates => CommandKind::ListResourceTemplates,
            Self::ReadResourceTemplate { .. } => CommandKind::ReadResourceTemplate,
            Self::SaveResourceTemplate { .. } => CommandKind::SaveResourceTemplate,
            Self::DeleteResourceTemplate { .. } => CommandKind::DeleteResourceTemplate,
            Self::ImportRuntimeResource { .. } => CommandKind::ImportRuntimeResource,
            Self::AdoptRuntimeResource { .. } => CommandKind::AdoptRuntimeResource,
            Self::ResourceSyncStatus => CommandKind::ResourceSyncStatus,
            Self::ResourceDrift { .. } => CommandKind::ResourceDrift,
            Self::UnmanagedRuntimeResources { .. } => CommandKind::UnmanagedRuntimeResources,
            Self::ApplyResourceTemplate { .. } => CommandKind::ApplyResourceTemplate,
            Self::ResourceNativeImport { .. } => CommandKind::ResourceNativeImport,
            Self::ResolveResourceBinding { .. } => CommandKind::ResolveResourceBinding,
            Self::AnalyticsQuery { .. } => CommandKind::AnalyticsQuery,
            Self::RuntimeDiscoveryHints { .. } => CommandKind::RuntimeDiscoveryHints,
            Self::DiscoverRuntimes { .. } => CommandKind::DiscoverRuntimes,
            Self::List => CommandKind::List,
            Self::SetOutlinePresentation { .. } => CommandKind::SetOutlinePresentation,
            Self::SetLinkedConversationOpening { .. } => CommandKind::SetLinkedConversationOpening,
            Self::SetUsageDisplay { .. } => CommandKind::SetUsageDisplay,
            Self::SetStatusLine { .. } => CommandKind::SetStatusLine,
            Self::SetConversationBrowserPreferences { .. } => {
                CommandKind::SetConversationBrowserPreferences
            }
            Self::ComposerDrafts => CommandKind::ComposerDrafts,
            Self::SetComposerDraft { .. } => CommandKind::SetComposerDraft,
            Self::ComposerChoices => CommandKind::ComposerChoices,
            Self::SetNextTurnIntent { .. } => CommandKind::SetNextTurnIntent,
            Self::SetRuntimeComposerChoices { .. } => CommandKind::SetRuntimeComposerChoices,
            Self::SetConversationBrowserLimits { .. } => CommandKind::SetConversationBrowserLimits,
            Self::OrganizeConversations { .. } => CommandKind::OrganizeConversations,
            Self::SetProject { .. } => CommandKind::SetProject,
            Self::LoadMoreConversations { .. } => CommandKind::LoadMoreConversations,
            Self::ShowArchivedConversations { .. } => CommandKind::ShowArchivedConversations,
            Self::SearchConversations { .. } => CommandKind::SearchConversations,
            Self::FindConversation { .. } => CommandKind::FindConversation,
            Self::UpsertRuntime { .. } => CommandKind::UpsertRuntime,
            Self::UpsertRuntimeWithExecution { .. } => CommandKind::UpsertRuntimeWithExecution,
            Self::UpsertAcpRuntime { .. } => CommandKind::UpsertAcpRuntime,
            Self::AcpLaunch { .. } => CommandKind::AcpLaunch,
            Self::AcpRuntimeInfo { .. } => CommandKind::AcpRuntimeInfo,
            Self::OpenAcpConversation { .. } => CommandKind::OpenAcpConversation,
            Self::AcpSessionConfig { .. } => CommandKind::AcpSessionConfig,
            Self::SetAcpSessionConfig { .. } => CommandKind::SetAcpSessionConfig,
            Self::DeleteRuntime { .. } => CommandKind::DeleteRuntime,
            Self::SelectRuntime { .. } => CommandKind::SelectRuntime,
            Self::RuntimeExecution { .. } => CommandKind::RuntimeExecution,
            Self::RuntimeHomeDirectory { .. } => CommandKind::RuntimeHomeDirectory,
            Self::ConfigureRuntimeExecution { .. } => CommandKind::ConfigureRuntimeExecution,
            Self::CreateConversation { .. } => CommandKind::CreateConversation,
            Self::CreateConversationWithSelection { .. } => {
                CommandKind::CreateConversationWithSelection
            }
            Self::RuntimeModels { .. } => CommandKind::RuntimeModels,
            Self::NextTurnModelSelection => CommandKind::NextTurnModelSelection,
            Self::OpenConversation { .. } => CommandKind::OpenConversation,
            Self::ReadConversation { .. } => CommandKind::ReadConversation,
            Self::RenameConversation { .. } => CommandKind::RenameConversation,
            Self::DeleteConversation { .. } => CommandKind::DeleteConversation,
            Self::Snapshot { .. } => CommandKind::Snapshot,
            Self::SnapshotUpdate { .. } => CommandKind::SnapshotUpdate,
            Self::MessagePayloads { .. } => CommandKind::MessagePayloads,
            Self::SendTurn { .. } => CommandKind::SendTurn,
            Self::SendTurnWithSelection { .. } => CommandKind::SendTurnWithSelection,
            Self::ReasoningEfforts { .. } => CommandKind::ReasoningEfforts,
            Self::SendQueue { .. } => CommandKind::SendQueue,
            Self::SteerTurn { .. } => CommandKind::SteerTurn,
            Self::EditMessage { .. } => CommandKind::EditMessage,
            Self::Cancel { .. } => CommandKind::Cancel,
            Self::StopSubAgents { .. } => CommandKind::StopSubAgents,
            Self::ConversationActivity { .. } => CommandKind::ConversationActivity,
            Self::WorkingChanges { .. } => CommandKind::WorkingChanges,
            Self::WorkingBranch { .. } => CommandKind::WorkingBranch,
            Self::FileDiff { .. } => CommandKind::FileDiff,
            Self::PreviewProviderImport { .. } => CommandKind::PreviewProviderImport,
            Self::ApplyProviderImport { .. } => CommandKind::ApplyProviderImport,
            Self::ProviderQuota { .. } => CommandKind::ProviderQuota,
            Self::RefreshProviderQuota { .. } => CommandKind::RefreshProviderQuota,
            Self::SaveProvider { .. } => CommandKind::SaveProvider,
            Self::DeleteProvider { .. } => CommandKind::DeleteProvider,
            Self::SaveModelRoute { .. } => CommandKind::SaveModelRoute,
            Self::DeleteModelRoute { .. } => CommandKind::DeleteModelRoute,
            Self::ReadProviderApiKey { .. } => CommandKind::ReadProviderApiKey,
            Self::SaveModelTemplate { .. } => CommandKind::SaveModelTemplate,
            Self::DeleteModelTemplate { .. } => CommandKind::DeleteModelTemplate,
            Self::ReplyRuntimeInteraction { .. } => CommandKind::ReplyRuntimeInteraction,
            Self::AuthenticationInspect { .. } => CommandKind::AuthenticationInspect,
            Self::AuthenticationStart { .. } => CommandKind::AuthenticationStart,
            Self::AuthenticationLogout { .. } => CommandKind::AuthenticationLogout,
            Self::AuthenticationPoll => CommandKind::AuthenticationPoll,
            Self::AuthenticationReply { .. } => CommandKind::AuthenticationReply,
            Self::AuthenticationCancel => CommandKind::AuthenticationCancel,
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
                | Self::ConfigureRuntimeExecution { .. }
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
                | Self::AuthenticationLogout { .. }
        )
    }
    pub fn changes_shared_state(&self) -> bool {
        self.requires_revision()
            || matches!(
                self,
                Self::CreateConversation { .. }
                    | Self::CreateConversationWithSelection { .. }
                    | Self::OpenAcpConversation { .. }
                    | Self::SendTurn { .. }
                    | Self::SendTurnWithSelection { .. }
                    | Self::SendQueue { .. }
                    | Self::SteerTurn { .. }
                    | Self::ReplyRuntimeInteraction { .. }
                    | Self::Cancel { .. }
                    | Self::StopSubAgents { .. }
                    | Self::WriteRuntimeResource { .. }
                    | Self::DeleteRuntimeResource { .. }
                    | Self::AuthenticationStart { .. }
                    | Self::AuthenticationReply { .. }
            )
    }
}
