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
        selection: ModelSelection,
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
        selection: ModelSelection,
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
        selection: ModelSelection,
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
    SetStatusLine(Vec<StatusLineItem>),
    SetConversationBrowserPreferences(ConfigurationSnapshot),
    ComposerDrafts(Vec<crate::config::ComposerDraft>),
    SetComposerDraft,
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
