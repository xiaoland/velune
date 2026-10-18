//! UniFFI representation for explicit submission with a preallocated request ID.
use crate::*;
use velune_application_api::Command;

/// Commands can contain secrets or message content and deliberately do not implement Debug.
#[derive(uniffi::Enum)]
pub enum BindingApplicationCommand {
    ReleaseView,
    RuntimeResourceCatalog {
        runtime_id: String,
        cwd: Option<String>,
        kind: BindingRuntimeResourceKind,
        scope: BindingRuntimeResourceScope,
    },
    ReadRuntimeResource {
        runtime_id: String,
        cwd: Option<String>,
        key: BindingRuntimeResourceKey,
    },
    WriteRuntimeResource {
        runtime_id: String,
        cwd: Option<String>,
        key: BindingRuntimeResourceKey,
        expected: BindingRuntimeResourceRevision,
        bytes: Vec<u8>,
        executable: Option<bool>,
    },
    DeleteRuntimeResource {
        runtime_id: String,
        cwd: Option<String>,
        key: BindingRuntimeResourceKey,
        expected: BindingRuntimeResourceRevision,
    },
    ListResourceTemplates,
    ReadResourceTemplate {
        template_id: String,
    },
    SaveResourceTemplate {
        template: BindingResourceTemplate,
    },
    DeleteResourceTemplate {
        template_id: String,
    },
    ImportRuntimeResource {
        runtime_id: String,
        kind: BindingRuntimeResourceKind,
        path: String,
    },
    AdoptRuntimeResource {
        runtime_id: String,
        kind: BindingRuntimeResourceKind,
        path: String,
    },
    ResourceSyncStatus,
    ResourceDrift {
        runtime_id: String,
        template_id: String,
    },
    UnmanagedRuntimeResources {
        kind: BindingRuntimeResourceKind,
    },
    ApplyResourceTemplate {
        template_id: String,
        targets: Vec<BindingTemplateApplyTarget>,
    },
    ResourceNativeImport {
        runtime_id: String,
        template_id: String,
    },
    ResolveResourceBinding {
        runtime_id: String,
        template_id: String,
        resolution: BindingResolution,
    },
    AnalyticsQuery {
        query: BindingAnalyticsQuery,
    },
    RuntimeDiscoveryHints {
        user_home: String,
        overrides: std::collections::HashMap<String, String>,
    },
    DiscoverRuntimes {
        probes: Vec<BindingRuntimeDiscoveryProbe>,
    },
    ListConfiguration,
    SetOutlinePresentation {
        presentation: BindingOutlinePresentation,
    },
    SetLinkedConversationOpening {
        opening: BindingLinkedConversationOpening,
    },
    SetUsageDisplay {
        display: BindingUsageDisplay,
    },
    SetStatusLine {
        items: Vec<BindingStatusLineItem>,
    },
    SetConversationBrowserPreferences {
        preferences: BindingConversationBrowserPreferences,
    },
    ComposerDrafts,
    SetComposerDraft {
        conversation_id: String,
        text: String,
    },
    ComposerChoices,
    SetNextTurnIntent {
        intent: BindingNextTurnIntent,
    },
    SetRuntimeComposerChoices {
        choices: BindingRuntimeComposerChoices,
    },
    SetConversationBrowserLimits {
        limits: BindingConversationBrowserLimits,
    },
    OrganizeConversations {
        conversations: Vec<BindingConversationReference>,
        change: BindingConversationOrganizationChange,
    },
    SetProject {
        path: String,
        primary_folder: String,
        name: Option<String>,
        folders: Vec<String>,
    },
    LoadMoreConversations {
        group: BindingConversationGroupKey,
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
        runtime: BindingRuntimeInstance,
    },
    UpsertRuntimeWithExecution {
        runtime: BindingRuntimeInstance,
        execution: BindingRuntimeExecution,
    },
    UpsertAcpRuntime {
        runtime: BindingRuntimeInstance,
        execution: BindingRuntimeExecution,
        launch: BindingAcpLaunch,
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
        value: BindingAcpConfigValue,
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
        execution: BindingRuntimeExecution,
    },
    CreateConversation {
        runtime_id: String,
        cwd: Option<String>,
        model_record_key: String,
    },
    CreateConversationWithSelection {
        runtime_id: String,
        cwd: Option<String>,
        selection: Option<BindingModelSelection>,
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
        since: BindingTranscriptCursor,
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
        selection: Option<BindingModelSelection>,
        turn: BindingComposedTurn,
    },
    ReasoningEfforts {
        runtime_id: String,
        model_record_key: String,
    },
    SendQueue {
        runtime_id: String,
        conversation_id: String,
        operation: BindingSendQueueOperation,
    },
    SteerTurn {
        runtime_id: String,
        conversation_id: String,
        turn: BindingComposedTurn,
    },
    EditMessage {
        runtime_id: String,
        conversation_id: String,
        message_id: String,
        placement: BindingEditPlacement,
        selection: Option<BindingModelSelection>,
        turn: BindingComposedTurn,
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
        source: BindingProviderImportSource,
    },
    ApplyProviderImport {
        gateway_id: String,
        source: BindingProviderImportSource,
        preview_token: String,
        selections: Vec<BindingImportSelection>,
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
        provider: BindingProviderDraft,
        authentication_edit: BindingAuthenticationEdit,
    },
    DeleteProvider {
        gateway_id: String,
        provider_id: String,
    },
    SaveModelRoute {
        gateway_id: String,
        route: BindingModelRoute,
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
        template: BindingModelTemplate,
    },
    DeleteModelTemplate {
        id: String,
    },
    ReplyRuntimeInteraction {
        runtime_id: String,
        conversation_id: String,
        interaction_id: String,
        reply: BindingRuntimeInteractionReply,
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
        bytes: Vec<u8>,
    },
    ReadAttachment {
        runtime_id: String,
        conversation_id: String,
        message_id: String,
        block_index: u32,
    },
}

impl TryFrom<BindingApplicationCommand> for Command {
    type Error = BindingError;
    fn try_from(command: BindingApplicationCommand) -> Result<Self, Self::Error> {
        Ok(match command {
            BindingApplicationCommand::ReleaseView => Self::ReleaseView,
            BindingApplicationCommand::RuntimeResourceCatalog {
                runtime_id,
                cwd,
                kind,
                scope,
            } => Self::RuntimeResourceCatalog {
                runtime_id,
                cwd,
                kind: convert(kind)?,
                scope: convert(scope)?,
            },
            BindingApplicationCommand::ReadRuntimeResource {
                runtime_id,
                cwd,
                key,
            } => Self::ReadRuntimeResource {
                runtime_id,
                cwd,
                key: convert(key)?,
            },
            BindingApplicationCommand::WriteRuntimeResource {
                runtime_id,
                cwd,
                key,
                expected,
                bytes,
                executable,
            } => Self::WriteRuntimeResource {
                runtime_id,
                cwd,
                key: convert(key)?,
                expected: convert(expected)?,
                bytes,
                executable: convert(executable)?,
            },
            BindingApplicationCommand::DeleteRuntimeResource {
                runtime_id,
                cwd,
                key,
                expected,
            } => Self::DeleteRuntimeResource {
                runtime_id,
                cwd,
                key: convert(key)?,
                expected: convert(expected)?,
            },
            BindingApplicationCommand::ListResourceTemplates => Self::ListResourceTemplates,
            BindingApplicationCommand::ReadResourceTemplate { template_id } => {
                Self::ReadResourceTemplate {
                    template_id: convert(template_id)?,
                }
            }
            BindingApplicationCommand::SaveResourceTemplate { template } => {
                Self::SaveResourceTemplate {
                    template: convert(template)?,
                }
            }
            BindingApplicationCommand::DeleteResourceTemplate { template_id } => {
                Self::DeleteResourceTemplate {
                    template_id: convert(template_id)?,
                }
            }
            BindingApplicationCommand::ImportRuntimeResource {
                runtime_id,
                kind,
                path,
            } => Self::ImportRuntimeResource {
                runtime_id,
                kind: convert(kind)?,
                path,
            },
            BindingApplicationCommand::AdoptRuntimeResource {
                runtime_id,
                kind,
                path,
            } => Self::AdoptRuntimeResource {
                runtime_id,
                kind: convert(kind)?,
                path,
            },
            BindingApplicationCommand::ResourceSyncStatus => Self::ResourceSyncStatus,
            BindingApplicationCommand::ResourceDrift {
                runtime_id,
                template_id,
            } => Self::ResourceDrift {
                runtime_id,
                template_id,
            },
            BindingApplicationCommand::UnmanagedRuntimeResources { kind } => {
                Self::UnmanagedRuntimeResources {
                    kind: convert(kind)?,
                }
            }
            BindingApplicationCommand::ApplyResourceTemplate {
                template_id,
                targets,
            } => Self::ApplyResourceTemplate {
                template_id,
                targets: convert(targets)?,
            },
            BindingApplicationCommand::ResourceNativeImport {
                runtime_id,
                template_id,
            } => Self::ResourceNativeImport {
                runtime_id,
                template_id,
            },
            BindingApplicationCommand::ResolveResourceBinding {
                runtime_id,
                template_id,
                resolution,
            } => Self::ResolveResourceBinding {
                runtime_id,
                template_id,
                resolution: convert(resolution)?,
            },
            BindingApplicationCommand::AnalyticsQuery { query } => Self::AnalyticsQuery {
                query: convert(query)?,
            },
            BindingApplicationCommand::RuntimeDiscoveryHints {
                user_home,
                overrides,
            } => Self::RuntimeDiscoveryHints {
                user_home,
                overrides: overrides.into_iter().collect(),
            },
            BindingApplicationCommand::DiscoverRuntimes { probes } => Self::DiscoverRuntimes {
                probes: convert(probes)?,
            },
            BindingApplicationCommand::ListConfiguration => Self::List,
            BindingApplicationCommand::SetOutlinePresentation { presentation } => {
                Self::SetOutlinePresentation {
                    presentation: convert(presentation)?,
                }
            }
            BindingApplicationCommand::SetLinkedConversationOpening { opening } => {
                Self::SetLinkedConversationOpening {
                    opening: crate::convert(opening)?,
                }
            }
            BindingApplicationCommand::SetUsageDisplay { display } => Self::SetUsageDisplay {
                display: convert(display)?,
            },
            BindingApplicationCommand::SetStatusLine { items } => Self::SetStatusLine {
                items: convert(items)?,
            },
            BindingApplicationCommand::SetConversationBrowserPreferences { preferences } => {
                Self::SetConversationBrowserPreferences {
                    preferences: convert(preferences)?,
                }
            }
            BindingApplicationCommand::ComposerDrafts => Self::ComposerDrafts,
            BindingApplicationCommand::ComposerChoices => Self::ComposerChoices,
            BindingApplicationCommand::SetNextTurnIntent { intent } => Self::SetNextTurnIntent {
                intent: convert(intent)?,
            },
            BindingApplicationCommand::SetRuntimeComposerChoices { choices } => {
                Self::SetRuntimeComposerChoices {
                    choices: convert(choices)?,
                }
            }
            BindingApplicationCommand::SetComposerDraft {
                conversation_id,
                text,
            } => Self::SetComposerDraft {
                conversation_id,
                text,
            },
            BindingApplicationCommand::SetConversationBrowserLimits { limits } => {
                Self::SetConversationBrowserLimits {
                    limits: convert(limits)?,
                }
            }
            BindingApplicationCommand::OrganizeConversations {
                conversations,
                change,
            } => Self::OrganizeConversations {
                conversations: convert(conversations)?,
                change: convert(change)?,
            },
            BindingApplicationCommand::SetProject {
                path,
                primary_folder,
                name,
                folders,
            } => Self::SetProject {
                path,
                primary_folder,
                name,
                folders,
            },
            BindingApplicationCommand::LoadMoreConversations { group } => {
                Self::LoadMoreConversations {
                    group: convert(group)?,
                }
            }
            BindingApplicationCommand::ShowArchivedConversations { archived } => {
                Self::ShowArchivedConversations {
                    archived: convert(archived)?,
                }
            }
            BindingApplicationCommand::SearchConversations { text } => {
                Self::SearchConversations { text }
            }
            BindingApplicationCommand::FindConversation { conversation_id } => {
                Self::FindConversation {
                    conversation_id: convert(conversation_id)?,
                }
            }
            BindingApplicationCommand::UpsertRuntime { runtime } => Self::UpsertRuntime {
                runtime: convert(runtime)?,
            },
            BindingApplicationCommand::UpsertRuntimeWithExecution { runtime, execution } => {
                Self::UpsertRuntimeWithExecution {
                    runtime: convert(runtime)?,
                    execution: convert(execution)?,
                }
            }
            BindingApplicationCommand::UpsertAcpRuntime {
                runtime,
                execution,
                launch,
            } => Self::UpsertAcpRuntime {
                runtime: convert(runtime)?,
                execution: convert(execution)?,
                launch: convert(launch)?,
            },
            BindingApplicationCommand::AcpLaunch { runtime_id } => Self::AcpLaunch { runtime_id },
            BindingApplicationCommand::AcpRuntimeInfo { runtime_id } => {
                Self::AcpRuntimeInfo { runtime_id }
            }
            BindingApplicationCommand::OpenAcpConversation {
                runtime_id,
                native_session_id,
                cwd,
            } => Self::OpenAcpConversation {
                runtime_id,
                native_session_id,
                cwd,
            },
            BindingApplicationCommand::AcpSessionConfig {
                runtime_id,
                conversation_id,
            } => Self::AcpSessionConfig {
                runtime_id,
                conversation_id,
            },
            BindingApplicationCommand::SetAcpSessionConfig {
                runtime_id,
                conversation_id,
                config_id,
                value,
            } => Self::SetAcpSessionConfig {
                runtime_id,
                conversation_id,
                config_id,
                value: convert(value)?,
            },
            BindingApplicationCommand::DeleteRuntime { id } => {
                Self::DeleteRuntime { id: convert(id)? }
            }
            BindingApplicationCommand::SelectRuntime { id } => {
                Self::SelectRuntime { id: convert(id)? }
            }
            BindingApplicationCommand::RuntimeExecution { runtime_id } => {
                Self::RuntimeExecution { runtime_id }
            }
            BindingApplicationCommand::RuntimeHomeDirectory { runtime_id } => {
                Self::RuntimeHomeDirectory {
                    runtime_id: convert(runtime_id)?,
                }
            }
            BindingApplicationCommand::ConfigureRuntimeExecution {
                runtime_id,
                execution,
            } => Self::ConfigureRuntimeExecution {
                runtime_id,
                execution: convert(execution)?,
            },
            BindingApplicationCommand::CreateConversation {
                runtime_id,
                cwd,
                model_record_key,
            } => Self::CreateConversation {
                runtime_id,
                cwd,
                model_record_key,
            },
            BindingApplicationCommand::CreateConversationWithSelection {
                runtime_id,
                cwd,
                selection,
            } => Self::CreateConversationWithSelection {
                runtime_id,
                cwd,
                selection: convert(selection)?,
            },
            BindingApplicationCommand::RuntimeModels { runtime_id } => {
                Self::RuntimeModels { runtime_id }
            }
            BindingApplicationCommand::NextTurnModelSelection => Self::NextTurnModelSelection,
            BindingApplicationCommand::OpenConversation {
                runtime_id,
                conversation_id,
            } => Self::OpenConversation {
                runtime_id,
                conversation_id,
            },
            BindingApplicationCommand::ReadConversation {
                runtime_id,
                conversation_id,
            } => Self::ReadConversation {
                runtime_id,
                conversation_id,
            },
            BindingApplicationCommand::RenameConversation {
                runtime_id,
                conversation_id,
                title,
            } => Self::RenameConversation {
                runtime_id,
                conversation_id,
                title,
            },
            BindingApplicationCommand::DeleteConversation {
                runtime_id,
                conversation_id,
            } => Self::DeleteConversation {
                runtime_id,
                conversation_id,
            },
            BindingApplicationCommand::Snapshot {
                runtime_id,
                conversation_id,
            } => Self::Snapshot {
                runtime_id,
                conversation_id,
            },
            BindingApplicationCommand::SnapshotUpdate {
                runtime_id,
                conversation_id,
                since,
            } => Self::SnapshotUpdate {
                runtime_id,
                conversation_id,
                since: convert(since)?,
            },
            BindingApplicationCommand::MessagePayloads {
                runtime_id,
                conversation_id,
                message_ids,
            } => Self::MessagePayloads {
                runtime_id,
                conversation_id,
                message_ids,
            },
            BindingApplicationCommand::SendTurn {
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                model_record_key,
                text,
            } => Self::SendTurn {
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                model_record_key,
                text,
            },
            BindingApplicationCommand::SendTurnWithSelection {
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                selection,
                turn,
            } => Self::SendTurnWithSelection {
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                selection: convert(selection)?,
                turn: convert(turn)?,
            },
            BindingApplicationCommand::ReasoningEfforts {
                runtime_id,
                model_record_key,
            } => Self::ReasoningEfforts {
                runtime_id,
                model_record_key,
            },
            BindingApplicationCommand::SendQueue {
                runtime_id,
                conversation_id,
                operation,
            } => Self::SendQueue {
                runtime_id,
                conversation_id,
                operation: convert(operation)?,
            },
            BindingApplicationCommand::SteerTurn {
                runtime_id,
                conversation_id,
                turn,
            } => Self::SteerTurn {
                runtime_id,
                conversation_id,
                turn: convert(turn)?,
            },
            BindingApplicationCommand::EditMessage {
                runtime_id,
                conversation_id,
                message_id,
                placement,
                selection,
                turn,
            } => Self::EditMessage {
                runtime_id,
                conversation_id,
                message_id,
                placement: convert(placement)?,
                selection: convert(selection)?,
                turn: convert(turn)?,
            },
            BindingApplicationCommand::Cancel {
                runtime_id,
                conversation_id,
            } => Self::Cancel {
                runtime_id,
                conversation_id,
            },
            BindingApplicationCommand::StopSubAgents {
                runtime_id,
                conversation_id,
            } => Self::StopSubAgents {
                runtime_id,
                conversation_id,
            },
            BindingApplicationCommand::ConversationActivity { since } => {
                Self::ConversationActivity {
                    since: convert(since)?,
                }
            }
            BindingApplicationCommand::WorkingChanges {
                runtime_id,
                conversation_id,
            } => Self::WorkingChanges {
                runtime_id,
                conversation_id,
            },
            BindingApplicationCommand::WorkingBranch {
                runtime_id,
                conversation_id,
            } => Self::WorkingBranch {
                runtime_id,
                conversation_id,
            },
            BindingApplicationCommand::FileDiff {
                runtime_id,
                conversation_id,
                path,
            } => Self::FileDiff {
                runtime_id,
                conversation_id,
                path,
            },
            BindingApplicationCommand::PreviewProviderImport { gateway_id, source } => {
                Self::PreviewProviderImport {
                    gateway_id: convert(gateway_id)?,
                    source: convert(source)?,
                }
            }
            BindingApplicationCommand::ApplyProviderImport {
                gateway_id,
                source,
                preview_token,
                selections,
                replace_existing,
            } => Self::ApplyProviderImport {
                gateway_id,
                source: convert(source)?,
                preview_token,
                selections: convert(selections)?,
                replace_existing,
            },
            BindingApplicationCommand::ProviderQuota {
                gateway_id,
                provider_id,
            } => Self::ProviderQuota {
                gateway_id,
                provider_id,
            },
            BindingApplicationCommand::RefreshProviderQuota {
                gateway_id,
                provider_id,
            } => Self::RefreshProviderQuota {
                gateway_id,
                provider_id,
            },
            BindingApplicationCommand::SaveProvider {
                gateway_id,
                provider,
                authentication_edit,
            } => Self::SaveProvider {
                gateway_id,
                provider: convert(provider)?,
                authentication_edit: convert(authentication_edit)?,
            },
            BindingApplicationCommand::DeleteProvider {
                gateway_id,
                provider_id,
            } => Self::DeleteProvider {
                gateway_id,
                provider_id,
            },
            BindingApplicationCommand::SaveModelRoute { gateway_id, route } => {
                Self::SaveModelRoute {
                    gateway_id: convert(gateway_id)?,
                    route: convert(route)?,
                }
            }
            BindingApplicationCommand::DeleteModelRoute {
                gateway_id,
                route_record_key,
            } => Self::DeleteModelRoute {
                gateway_id,
                route_record_key,
            },
            BindingApplicationCommand::ReadProviderApiKey {
                gateway_id,
                provider_id,
            } => Self::ReadProviderApiKey {
                gateway_id,
                provider_id,
            },
            BindingApplicationCommand::SaveModelTemplate { template } => Self::SaveModelTemplate {
                template: convert(template)?,
            },
            BindingApplicationCommand::DeleteModelTemplate { id } => {
                Self::DeleteModelTemplate { id: convert(id)? }
            }
            BindingApplicationCommand::ReplyRuntimeInteraction {
                runtime_id,
                conversation_id,
                interaction_id,
                reply,
            } => Self::ReplyRuntimeInteraction {
                runtime_id,
                conversation_id,
                interaction_id,
                reply: convert(reply)?,
            },
            BindingApplicationCommand::AuthenticationInspect {
                gateway_id,
                provider_id,
            } => Self::AuthenticationInspect {
                gateway_id,
                provider_id,
            },
            BindingApplicationCommand::AuthenticationStart {
                gateway_id,
                provider_id,
            } => Self::AuthenticationStart {
                gateway_id,
                provider_id,
            },
            BindingApplicationCommand::AuthenticationLogout {
                gateway_id,
                provider_id,
            } => Self::AuthenticationLogout {
                gateway_id,
                provider_id,
            },
            BindingApplicationCommand::AuthenticationPoll => Self::AuthenticationPoll,
            BindingApplicationCommand::AuthenticationReply { prompt_id, value } => {
                Self::AuthenticationReply {
                    prompt_id: convert(prompt_id)?,
                    value: convert(value)?,
                }
            }
            BindingApplicationCommand::AuthenticationCancel => Self::AuthenticationCancel,
            BindingApplicationCommand::FetchPublicModelCatalog => Self::FetchPublicModelCatalog,
            BindingApplicationCommand::UploadAttachment { name, bytes } => {
                Self::UploadAttachment { name, bytes }
            }
            BindingApplicationCommand::ReadAttachment {
                runtime_id,
                conversation_id,
                message_id,
                block_index,
            } => Self::ReadAttachment {
                runtime_id,
                conversation_id,
                message_id,
                block_index,
            },
        })
    }
}
