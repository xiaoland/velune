//! Typed application operations and non-secret result records.
use crate::config::*;
use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;
use velune_conversation::{ConversationSummary, SettingAction, SettingOption};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDiscoveryHint {
    pub family_id: String,
    pub command: String,
    pub agent_directory: String,
    pub directory_exists: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDiscoveryProbe {
    pub family_id: String,
    pub binary: String,
    pub node_binary: String,
    pub agent_directory: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDiscoveryCandidate {
    pub runtime: RuntimeInstance,
    pub version: Option<String>,
    pub supported: bool,
    pub already_configured: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSummary {
    pub id: String,
    pub name: String,
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    pub execution_backend: ExecutionBackend,
    pub request_profile: RequestProfile,
    pub authentication_source: crate::ProviderAuthenticationSource,
    pub quota_source: ProviderQuotaSource,
    pub authentication: crate::ProviderAuthenticationDescription,
    pub quota_limits: Vec<crate::config::QuotaLimit>,
    pub models: Vec<ProviderModel>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewaySummary {
    pub id: String,
    pub name: String,
    pub providers: Vec<ProviderSummary>,
    pub routes: Vec<crate::config::ModelRoute>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderDraft {
    pub id: String,
    pub name: String,
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    pub execution_backend: ExecutionBackend,
    pub request_profile: RequestProfile,
    pub authentication_source: crate::ProviderAuthenticationSource,
    pub quota_source: ProviderQuotaSource,
    pub models: Vec<ProviderModel>,
    pub quota_limits: Vec<crate::config::QuotaLimit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationSnapshot {
    /// The conversations the list shows, then open ones it does not.
    pub conversations: Vec<ListedConversation>,
    /// The list's groups in display order, including folded and unloaded ones.
    pub conversation_groups: Vec<ConversationGroup>,
    /// Every project of the shown conversations (archived or not), for
    /// filtering and moving.
    pub conversation_projects: Vec<ConversationProject>,
    /// The list shows archived conversations instead of the others.
    pub shows_archived: bool,
    pub history_failures: Vec<HistoryFailure>,
    pub gateways: Vec<GatewaySummary>,
    pub runtime_instances: Vec<RuntimeInstance>,
    pub runtime_types: Vec<RuntimeTypeDescriptor>,
    pub model_templates: Vec<ModelTemplate>,
    pub conversation_browser_limits: ConversationBrowserLimits,
    pub outline_presentation: OutlinePresentation,
    pub linked_conversation_opening: LinkedConversationOpening,
    pub usage_display: UsageDisplay,
    pub conversation_browser_preferences: ConversationBrowserPreferences,
    pub status_line: Vec<StatusLineItem>,
    pub provider_import_types: Vec<RuntimeTypeDescriptor>,
    pub protocols: Vec<ProtocolDescriptor>,
    #[serde(rename = "selectedRuntimeInstanceID")]
    pub selected_runtime_instance_id: Option<String>,
}

/// One conversation of the list with Velune's organization of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedConversation {
    pub conversation: ConversationSummary,
    /// The identity its runtime resumes it by; for a continued conversation,
    /// that of its latest segment.
    #[serde(rename = "nativeID")]
    pub native_id: String,
    pub pinned: bool,
    pub unread: bool,
    pub archived: bool,
    /// The project it is listed under, by its primary folder: where it was
    /// moved, else the project its working directory belongs to (a Git
    /// worktree's is its repository's), unless its runtime started it there
    /// without a project.
    pub project: Option<String>,
    /// Moved by the user away from its working directory's project.
    pub moved: bool,
    /// Its latest turn failed, as its Harness recorded it on this machine;
    /// looked up for unread conversations only.
    pub failed: bool,
    /// The group showing it; none for an open conversation the list does not show.
    pub group: Option<ConversationGroupKey>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationGroup {
    pub key: ConversationGroupKey,
    /// The user's name for a project group.
    pub name: Option<String>,
    /// Conversations in the group; the list holds the first `loaded`.
    pub total: u32,
    pub loaded: u32,
    pub latest_activity_unix_ms: Option<i64>,
    pub collapsed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationProject {
    /// Its primary folder, which identifies it.
    pub path: String,
    pub name: Option<String>,
    /// Its other folders, as the user gave them.
    pub folders: Vec<String>,
    /// Git worktrees its listed conversations ran in, which belong to it by
    /// their repository.
    pub worktrees: Vec<String>,
    pub latest_activity_unix_ms: Option<i64>,
}
/// Matching conversations, grouped like the list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSearch {
    pub conversations: Vec<ListedConversation>,
    pub groups: Vec<ConversationGroup>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationBrowserLimits {
    /// Conversations loaded per group at a time.
    pub page: u32,
    /// Conversations the whole list loads before groups are expanded on demand.
    pub total: u32,
}
/// A conversation as the list identifies it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationReference {
    #[serde(rename = "runtimeInstanceID")]
    pub runtime_id: String,
    #[serde(rename = "conversationID")]
    pub conversation_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "operation")]
pub enum ConversationOrganizationChange {
    Pin,
    Unpin,
    Archive,
    Unarchive,
    MarkRead,
    MarkUnread,
    /// Into the project at `path`; none returns it to its working directory's.
    MoveToProject {
        path: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolDescriptor {
    pub id: String,
    pub name: String,
    pub supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayUpdate {
    pub gateways: Vec<GatewaySummary>,
    pub execution_invalidated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeUpdate {
    pub runtime_instances: Vec<RuntimeInstance>,
    pub runtime_types: Vec<RuntimeTypeDescriptor>,
    pub execution_invalidated: bool,
}

/// The selected model belongs either to Velune's gateway or to the target runtime.
/// Native identifiers are never resolved through the gateway catalog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ModelSelection {
    /// Keep the ACP Agent's own session default; no model identifier is invented.
    RuntimeDefault,
    Managed {
        model_record_key: String,
    },
    Native {
        provider_id: Option<String>,
        model_id: String,
    },
}

/// What the user composed for one turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposedTurn {
    pub text: String,
    /// One of `reasoning_efforts` for the turn's runtime and model; `None`
    /// leaves the level to the runtime's default.
    pub effort: Option<String>,
    /// One of the runtime type's `permission_modes`; `None` leaves the
    /// runtime's own default.
    pub permission_mode: Option<String>,
    /// Local calls use execution-host paths; remote calls use this view's UploadAttachment references.
    /// Images use the Harness image input; other files become execution-host path references.
    pub attachments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeModel {
    pub selection: ModelSelection,
    pub name: String,
    /// Levels the runtime accepts for this model; empty when it takes none.
    pub reasoning_efforts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotResult {
    pub snapshot: Option<crate::DeliveredSnapshot>,
}

/// A conversation that executes or waits for the user, in any open session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveConversation {
    pub context_runtime_id: String,
    pub conversation_id: String,
    pub run_state: crate::conversation::RunState,
    pub pending_interactions: u32,
    /// The conversation this one is a native sub-agent of.
    pub parent_conversation_id: Option<String>,
}

/// Uncommitted changes in a conversation's working directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingChanges {
    pub directory: String,
    /// The git work tree containing the directory; None outside one.
    pub repository: Option<String>,
    /// None when HEAD is detached.
    pub branch: Option<String>,
    pub files: Vec<ChangedFile>,
}

/// A file that differs from HEAD or is not tracked, by repository path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    pub kind: ChangeKind,
    pub staged: bool,
    pub unstaged: bool,
    /// Lines against HEAD; None for binary or untracked files.
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed { from: String },
    Untracked,
    Conflicted,
}

/// One file's diff against HEAD (or as a new file when untracked).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    pub lines: Vec<DiffLine>,
    /// The diff was longer than shown.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiffLineKind {
    Hunk,
    Context,
    Added,
    Removed,
    /// Binary content, mode changes, missing final newline.
    Notice,
}

/// How a Velune turn ended.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TurnOutcome {
    Completed,
    Failed,
    /// The user stopped it.
    Cancelled,
}

/// A Velune turn that ended, numbered in the order turns ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishedTurn {
    pub sequence: u64,
    pub context_runtime_id: String,
    pub conversation_id: String,
    pub outcome: TurnOutcome,
}

/// Conversations executing or waiting for the user, and turns that ended
/// after a cursor. `cursor` is the sequence to pass next time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationActivity {
    pub active: Vec<ActiveConversation>,
    pub finished: Vec<FinishedTurn>,
    pub cursor: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsQuery {
    pub from_ms: i64,
    pub to_ms: i64,
    pub bucket_boundaries_ms: Vec<i64>,
    pub provider_id: Option<String>,
    pub model_record_key: Option<String>,
    pub request_limit: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AnalyticsProtocol {
    ChatCompletions,
    Responses,
    Messages,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AnalyticsOutcome {
    Completed,
    Incomplete,
    Failed,
    Cancelled,
    Rejected,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsTotals {
    pub request_count: u64,
    pub completed_count: u64,
    pub failed_count: u64,
    pub incomplete_count: u64,
    pub rejected_count: u64,
    pub cancelled_count: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub usage_reported_count: u64,
    pub usage_complete_count: u64,
    pub eligible_speed_count: u64,
    pub output_tokens_per_second: Option<f64>,
    pub total_tokens: Option<u64>,
    pub total_reported_count: u64,
    pub input_reported_count: u64,
    pub output_reported_count: u64,
    pub reasoning_reported_count: u64,
    pub cache_reported_count: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsBucket {
    pub from_ms: i64,
    pub to_ms: i64,
    pub totals: AnalyticsTotals,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsBreakdown {
    pub key: String,
    pub name: String,
    pub provider_id: Option<String>,
    pub model_record_key: Option<String>,
    pub totals: AnalyticsTotals,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsRequest {
    pub request_id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model_record_key: String,
    pub provider_model_id: String,
    pub protocol: AnalyticsProtocol,
    pub started_at_ms: i64,
    pub terminal_at_ms: i64,
    pub elapsed_ms: u64,
    pub first_output_ms: Option<u64>,
    pub terminal_elapsed_ms: Option<u64>,
    pub output_tokens_per_second: Option<f64>,
    pub status: Option<u16>,
    pub outcome: AnalyticsOutcome,
    pub input_tokens: Option<u64>,
    pub uncached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub usage_reported: bool,
    pub usage_complete: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsReport {
    pub overview: AnalyticsTotals,
    pub trend: Vec<AnalyticsBucket>,
    pub providers: Vec<AnalyticsBreakdown>,
    pub models: Vec<AnalyticsBreakdown>,
    pub requests: Vec<AnalyticsRequest>,
    pub coverage: String,
    pub storage_warning: Option<String>,
    pub dropped_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderImportSource {
    pub kind: String,
    pub harness_type_id: String,
    pub source_instance_id: Option<String>,
    pub provider_id: Option<String>,
    pub settings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSelection {
    pub provider_id: String,
    pub candidate_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderImportPreview {
    pub contract_version: u64,
    pub source_fingerprint: String,
    pub target_fingerprint: String,
    pub sdk_version: String,
    pub source: ProviderImportSource,
    pub providers: Vec<ImportProviderCandidate>,
    pub warnings: Vec<String>,
    pub source_label: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProviderCandidate {
    pub id: String,
    pub source_provider_id: String,
    pub name: String,
    pub protocol: String,
    pub endpoint: Option<String>,
    pub can_import: bool,
    pub already_imported: bool,
    pub credential_status: String,
    pub issues: Vec<String>,
    pub models: Vec<ImportModelCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportModelCandidate {
    pub candidate_key: String,
    pub provider_model_id: String,
    pub name: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Vec<String>,
    pub can_import: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported_provider_ids: Vec<String>,
    pub skipped_provider_ids: Vec<String>,
    pub gateways: Vec<GatewaySummary>,
    pub execution_invalidated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationMetadata {
    pub configured: bool,
    pub account_label: Option<String>,
    pub capabilities: AuthenticationCapabilities,
    pub actions: Option<Vec<SettingAction>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationCapabilities {
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    pub explicit_output_cap: bool,
    pub temperature: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationProgress {
    pub running: bool,
    pub events: Vec<AuthenticationEvent>,
    pub gateways: Option<Vec<GatewaySummary>>,
    pub execution_invalidated: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationEvent {
    #[serde(rename = "type")]
    pub type_id: String,
    pub id: Option<String>,
    pub prompt: Option<AuthenticationPrompt>,
    pub notification: Option<AuthenticationNotification>,
    pub ok: Option<bool>,
    pub error: Option<String>,
    pub cancelled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationPrompt {
    pub kind: String,
    pub text: String,
    pub options: Option<Vec<SettingOption>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationNotification {
    pub kind: String,
    pub id: Option<String>,
    pub text: Option<String>,
    pub url: Option<String>,
    pub instructions: Option<String>,
    pub user_code: Option<String>,
    pub verification_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryFailure {
    pub runtime_id: String,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum SendQueueOperation {
    /// Queue a turn as `send_turn_with_selection` would send it.
    #[serde(rename_all = "camelCase")]
    Add {
        source_runtime_id: String,
        target_runtime_id: String,
        selection: crate::ModelSelection,
        turn: crate::ComposedTurn,
    },
    Edit {
        id: String,
        text: String,
    },
    Remove {
        id: String,
    },
    /// Move to `index` in the queue as it is after removing the message.
    Move {
        id: String,
        index: u32,
    },
    /// Send again after a pause, once no turn runs.
    Resume,
    /// Deliver the message into the running turn instead of queueing it.
    Steer {
        id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuotaFreshness {
    Fresh,
    Stale,
    Unknown,
    Unsupported,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderQuotaWindow {
    pub id: String,
    pub model_ids: Option<Vec<String>>,
    pub used_percent: f64,
    pub reset_at_ms: Option<i64>,
    pub observed_at_ms: i64,
    pub expires_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderQuotaStatus {
    pub gateway_id: String,
    pub provider_id: String,
    pub freshness: QuotaFreshness,
    pub windows: Vec<ProviderQuotaWindow>,
    pub available: Option<bool>,
    pub refreshing: bool,
    pub problem: Option<String>,
}
