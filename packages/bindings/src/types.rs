//! UniFFI records mirror domain types without adding FFI dependencies to them.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationSummary {
    pub id: String,
    #[serde(deserialize_with = "display_conversation_title")]
    pub title: String,
    /// Unix epoch milliseconds; None means the source did not provide a timestamp.
    pub updated_at_unix_ms: Option<i64>,
    pub created_at_unix_ms: Option<i64>,
    pub runtime_id: String,
    pub cwd: Option<String>,
    pub can_rename: bool,
    pub can_delete: bool,
    /// Set for a native sub-agent's conversation: the conversation whose
    /// agent started it. It opens from the parent's transcript and is read-only.
    pub parent_conversation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum BindingRunState {
    Idle,
    Running,
    Stopping,
    Failed,
    /// Running in a Harness process other than this Velune execution, by the
    /// Harness's own liveness record. Velune cannot stop it.
    Elsewhere,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationActions {
    pub can_send: bool,
    pub can_cancel: bool,
    pub can_switch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct BindingMessage {
    /// Delivery sequence of this content; unchanged content keeps its stamp.
    pub stamp: u64,
    pub completed: bool,
    pub id: String,
    pub role: BindingMessageRole,
    /// Native message time in Unix epoch milliseconds, when available.
    pub timestamp_unix_ms: Option<i64>,
    pub blocks: Vec<BindingMessageBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum BindingMessageRole {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum BindingToolState {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BindingMessageBlock {
    Text {
        text: String,
    },
    /// Reasoning with a summary; `text` is what to show under it.
    Reasoning {
        summary: String,
        text: BindingMessagePayload,
    },
    Tool {
        #[serde(rename = "toolID")]
        tool_id: Option<String>,
        title: String,
        state: BindingToolState,
        output: Option<BindingMessagePayload>,
        /// The conversation a Velune collaboration tool started or addressed.
        #[serde(rename = "linkedConversationID")]
        linked_conversation_id: Option<String>,
    },
    Notice {
        text: String,
    },
    /// An image the user attached.
    Image {
        media_type: Option<String>,
        source: BindingImageSource,
    },
    /// A non-image file the user attached by absolute path.
    File {
        path: String,
    },
    /// The conversation whose agent sent this user-side message through Velune.
    AgentSender {
        #[serde(rename = "conversationID")]
        conversation_id: String,
        runtime_name: String,
        title: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BindingImageSource {
    Path {
        path: String,
    },
    /// Base64 content, normally deferred until read with `message_payloads`.
    Data {
        data: BindingMessagePayload,
    },
}

/// Large reasoning and tool output arrive by size; read them with `message_payloads`.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BindingMessagePayload {
    Inline { text: String },
    Deferred { bytes: u64 },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingTranscriptCursor {
    pub generation: u64,
    pub sequence: u64,
}

// UniFFI enum variants cannot box their records; the size difference is one
// short-lived poll result.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BindingSnapshotUpdate {
    /// No conversation is loaded in the queried runtime context.
    Absent,
    /// The cursor is current.
    Unchanged,
    Changed {
        snapshot: BindingConversationSnapshot,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationSnapshot {
    pub cursor: BindingTranscriptCursor,
    /// None: `messages` is complete. Some: `messages` holds only messages
    /// changed after this cursor, and `message_ids` gives the complete order.
    pub base: Option<BindingTranscriptCursor>,
    pub message_ids: Vec<String>,
    /// Local projection revision; may reset between history and execution preparation.
    /// Do not use it to order snapshots across those lifecycle boundaries.
    pub revision: u64,
    pub conversation: BindingConversationSummary,
    pub context_runtime_id: String,
    pub resource_id: Option<String>,
    pub model_record_key: Option<String>,
    pub run_state: BindingRunState,
    pub messages: Vec<BindingMessage>,
    pub transcript_turns: Vec<BindingTranscriptTurn>,
    pub transcript_items: Vec<BindingTranscriptItem>,
    pub transcript_outline: Vec<BindingTranscriptOutlineEntry>,
    pub transcript_message_identities: Vec<BindingTranscriptMessageIdentity>,
    pub message_identity_confirmations: Vec<BindingMessageIdentityConfirmation>,
    #[serde(default)]
    pub pending_interactions: Vec<BindingRuntimeInteraction>,
    pub actions: BindingConversationActions,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingSettingOption {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingExecutableDiscovery {
    pub command: String,
    pub minimum_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingSettingField {
    pub key: String,
    pub label: String,
    pub kind: BindingSettingKind,
    pub required: bool,
    pub value: String,
    pub options: Vec<BindingSettingOption>,
    pub help: Option<String>,
    pub executable_discovery: Option<BindingExecutableDiscovery>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingSettingAction {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingProviderModel {
    pub record_key: String,
    pub provider_model_id: String,
    pub nickname: String,
    pub icon: Option<String>,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Option<Vec<String>>,
    #[serde(rename = "piProjection", with = "adapter_metadata")]
    pub adapter_metadata_json: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingModelTemplate {
    pub id: String,
    pub name: String,
    pub suggested_provider_model_id: String,
    pub nickname: String,
    pub icon: Option<String>,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Option<Vec<String>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum BindingGatewayProtocol {
    ChatCompletionsV1,
    ResponsesV1,
    MessagesV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingProviderDefinition {
    pub id: String,
    pub name: String,
    pub protocol: BindingGatewayProtocol,
    pub endpoint: String,
    pub authentication: BindingProviderAuthentication,
    pub models: Vec<BindingProviderModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingProviderDraft {
    pub id: String,
    pub name: String,
    pub protocol: BindingGatewayProtocol,
    pub endpoint: String,
    pub models: Vec<BindingProviderModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum BindingFailoverMode {
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingFailoverPolicy {
    pub mode: BindingFailoverMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingGatewayConfig {
    pub id: String,
    pub name: String,
    pub providers: Vec<BindingProviderDefinition>,
    pub failover: BindingFailoverPolicy,
}

#[derive(Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingRuntimeInstance {
    pub enabled: bool,
    pub id: String,
    pub name: String,
    pub type_id: String,
    pub gateway_id: String,
    pub settings: HashMap<String, String>,
}
impl std::fmt::Debug for BindingRuntimeInstance {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BindingRuntimeInstance")
            .field("id", &self.id)
            .field("type_id", &self.type_id)
            .field("enabled", &self.enabled)
            .field("setting_keys", &self.settings.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeDiscoveryHint {
    pub family_id: String,
    pub command: String,
    pub agent_directory: String,
    pub directory_exists: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeDiscoveryProbe {
    pub family_id: String,
    pub binary: String,
    pub node_binary: String,
    pub agent_directory: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeDiscoveryCandidate {
    pub runtime: BindingRuntimeInstance,
    pub version: Option<String>,
    pub supported: bool,
    pub already_configured: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingRuntimeTypeDescriptor {
    pub id: String,
    pub family_id: String,
    pub version_regex: String,
    pub can_rename_conversations: bool,
    pub can_delete_conversations: bool,
    /// Instances take launch arguments and environment (`acp_launch`,
    /// `upsert_acp_runtime`).
    pub launch_options: bool,
    /// `acp_runtime_info` describes the Agent, its capabilities and sign-in methods.
    pub agent_info: bool,
    /// A live conversation has options (`acp_session_config`); the runtime's
    /// own model catalog belongs to that session.
    pub session_options: bool,
    /// Running over SSH needs a helper resources directory on the remote
    /// machine (`RuntimeExecution::remote_resources_directory`).
    pub remote_resources: bool,
    pub supported_protocols: Vec<BindingGatewayProtocol>,
    pub supported_provider_protocols: Vec<BindingGatewayProtocol>,
    pub name: String,
    pub fields: Vec<BindingSettingField>,
    /// Modes a turn may request; empty when the runtime offers no choice.
    pub permission_modes: Vec<BindingPermissionMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingPermissionMode {
    pub id: String,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConfigurationSnapshot {
    /// The conversations the list shows, then open ones it does not.
    pub conversations: Vec<BindingListedConversation>,
    /// The list's groups in display order, including folded and unloaded ones.
    pub conversation_groups: Vec<BindingConversationGroup>,
    /// Every project of the shown conversations, for filtering and moving.
    pub conversation_projects: Vec<BindingConversationProject>,
    /// The list shows archived conversations instead of the others.
    pub shows_archived: bool,
    pub history_failures: Vec<BindingHistoryFailure>,
    pub gateways: Vec<BindingGatewayConfig>,
    pub runtime_instances: Vec<BindingRuntimeInstance>,
    pub runtime_types: Vec<BindingRuntimeTypeDescriptor>,
    pub model_templates: Vec<BindingModelTemplate>,
    pub conversation_browser_limits: BindingConversationBrowserLimits,
    pub outline_presentation: BindingOutlinePresentation,
    pub conversation_browser_preferences: BindingConversationBrowserPreferences,
    /// What the composer's status line shows, in display order.
    pub status_line: Vec<BindingStatusLineItem>,
    pub provider_import_types: Vec<BindingRuntimeTypeDescriptor>,
    pub protocols: Vec<BindingProtocolDescriptor>,
    #[serde(rename = "selectedRuntimeInstanceID")]
    pub selected_runtime_instance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingProtocolDescriptor {
    pub id: BindingGatewayProtocol,
    pub name: String,
    pub supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingGatewayUpdate {
    pub gateways: Vec<BindingGatewayConfig>,
    pub execution_invalidated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingRuntimeUpdate {
    pub runtime_instances: Vec<BindingRuntimeInstance>,
    pub runtime_types: Vec<BindingRuntimeTypeDescriptor>,
    pub execution_invalidated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingComposedTurn {
    pub text: String,
    pub effort: Option<String>,
    /// One of the runtime type's `permission_modes`; `None` leaves the runtime default.
    #[uniffi(default = None)]
    pub permission_mode: Option<String>,
    pub attachments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum BindingModelSelection {
    RuntimeDefault,
    Managed {
        model_record_key: String,
    },
    Native {
        provider_id: Option<String>,
        model_id: String,
    },
}

#[derive(Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAcpLaunch {
    pub args: Vec<String>,
    pub environment: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAcpCapabilities {
    pub load_session: bool,
    pub list_sessions: bool,
    pub resume_session: bool,
    pub close_session: bool,
    pub delete_session: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAcpAgentInfo {
    pub name: String,
    pub title: Option<String>,
    pub version: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAcpAuthMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAcpRuntimeInfo {
    pub capabilities: BindingAcpCapabilities,
    pub agent_info: Option<BindingAcpAgentInfo>,
    pub auth_methods: Vec<BindingAcpAuthMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum BindingAcpConfigValue {
    Select(String),
    Boolean(bool),
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAcpConfigChoice {
    pub value: String,
    pub name: String,
    pub description: Option<String>,
    pub group: Option<String>,
    pub group_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAcpConfigOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub value: BindingAcpConfigValue,
    pub choices: Vec<BindingAcpConfigChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeModel {
    pub selection: BindingModelSelection,
    pub name: String,
    /// Levels the runtime accepts for this model; empty when it takes none.
    pub reasoning_efforts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingSnapshotResult {
    pub snapshot: Option<BindingConversationSnapshot>,
}

/// A conversation that executes or waits for the user, in any open session.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingActiveConversation {
    pub context_runtime_id: String,
    pub conversation_id: String,
    pub run_state: BindingRunState,
    pub pending_interactions: u32,
}

/// Uncommitted changes in a conversation's working directory.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingWorkingChanges {
    pub directory: String,
    /// The git work tree containing the directory; None outside one.
    pub repository: Option<String>,
    /// None when HEAD is detached.
    pub branch: Option<String>,
    pub files: Vec<BindingChangedFile>,
}

/// A file that differs from HEAD or is not tracked, by repository path.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingChangedFile {
    pub path: String,
    pub kind: BindingChangeKind,
    pub staged: bool,
    pub unstaged: bool,
    /// Lines against HEAD; None for binary or untracked files.
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed { from: String },
    Untracked,
    Conflicted,
}

/// One file's diff against HEAD (or as a new file when untracked).
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingFileDiff {
    pub path: String,
    pub lines: Vec<BindingDiffLine>,
    /// The diff was longer than shown.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingDiffLine {
    pub kind: BindingDiffLineKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingDiffLineKind {
    Hunk,
    Context,
    Added,
    Removed,
    /// Binary content, mode changes, missing final newline.
    Notice,
}

/// How a Velune turn ended.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingTurnOutcome {
    Completed,
    Failed,
    /// The user stopped it.
    Cancelled,
}

/// A Velune turn that ended, numbered in the order turns ended.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingFinishedTurn {
    pub sequence: u64,
    pub context_runtime_id: String,
    pub conversation_id: String,
    pub outcome: BindingTurnOutcome,
}

/// Active conversations and turns that ended after the polled cursor;
/// `cursor` is the value to pass next time.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationActivity {
    pub active: Vec<BindingActiveConversation>,
    pub finished: Vec<BindingFinishedTurn>,
    pub cursor: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAnalyticsQuery {
    pub from_ms: i64,
    pub to_ms: i64,
    pub bucket_boundaries_ms: Vec<i64>,
    pub provider_id: Option<String>,
    pub model_record_key: Option<String>,
    pub request_limit: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingAnalyticsProtocol {
    ChatCompletions,
    Responses,
    Messages,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingAnalyticsOutcome {
    Completed,
    Incomplete,
    Failed,
    Cancelled,
    Rejected,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAnalyticsTotals {
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
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAnalyticsBucket {
    pub from_ms: i64,
    pub to_ms: i64,
    pub totals: BindingAnalyticsTotals,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAnalyticsBreakdown {
    pub key: String,
    pub name: String,
    pub provider_id: Option<String>,
    pub model_record_key: Option<String>,
    pub totals: BindingAnalyticsTotals,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAnalyticsRequest {
    pub request_id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model_record_key: String,
    pub provider_model_id: String,
    pub protocol: BindingAnalyticsProtocol,
    pub started_at_ms: i64,
    pub terminal_at_ms: i64,
    pub elapsed_ms: u64,
    pub first_output_ms: Option<u64>,
    pub terminal_elapsed_ms: Option<u64>,
    pub output_tokens_per_second: Option<f64>,
    pub status: Option<u16>,
    pub outcome: BindingAnalyticsOutcome,
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
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingAnalyticsReport {
    pub overview: BindingAnalyticsTotals,
    pub trend: Vec<BindingAnalyticsBucket>,
    pub providers: Vec<BindingAnalyticsBreakdown>,
    pub models: Vec<BindingAnalyticsBreakdown>,
    pub requests: Vec<BindingAnalyticsRequest>,
    pub coverage: String,
    pub storage_warning: Option<String>,
    pub dropped_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingProviderImportSource {
    pub kind: String,
    pub harness_type_id: String,
    pub source_instance_id: Option<String>,
    pub provider_id: Option<String>,
    pub settings: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingImportSelection {
    pub provider_id: String,
    pub candidate_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingProviderImportPreview {
    pub contract_version: u64,
    pub source_fingerprint: String,
    pub target_fingerprint: String,
    pub sdk_version: String,
    pub source: BindingProviderImportSource,
    pub providers: Vec<BindingImportProviderCandidate>,
    pub warnings: Vec<String>,
    pub source_label: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingImportProviderCandidate {
    pub id: String,
    pub source_provider_id: String,
    pub name: String,
    pub protocol: String,
    pub endpoint: Option<String>,
    pub can_import: bool,
    pub already_imported: bool,
    pub credential_status: String,
    pub issues: Vec<String>,
    pub models: Vec<BindingImportModelCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingImportModelCandidate {
    pub candidate_key: String,
    pub provider_model_id: String,
    pub name: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Vec<String>,
    pub can_import: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingImportResult {
    pub imported_provider_ids: Vec<String>,
    pub skipped_provider_ids: Vec<String>,
    pub gateways: Vec<BindingGatewayConfig>,
    pub execution_invalidated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingAuthenticationMetadata {
    pub configured: bool,
    pub capabilities: BindingAuthenticationCapabilities,
    pub actions: Option<Vec<BindingSettingAction>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingAuthenticationCapabilities {
    pub protocol: BindingGatewayProtocol,
    pub endpoint: String,
    pub explicit_output_cap: bool,
    pub temperature: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingAuthenticationProgress {
    pub running: bool,
    pub events: Vec<BindingAuthenticationEvent>,
    pub gateways: Option<Vec<BindingGatewayConfig>>,
    pub execution_invalidated: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingAuthenticationEvent {
    #[serde(rename = "type")]
    pub type_id: String,
    pub id: Option<String>,
    pub prompt: Option<BindingAuthenticationPrompt>,
    pub notification: Option<BindingAuthenticationNotification>,
    pub ok: Option<bool>,
    pub error: Option<String>,
    pub cancelled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingAuthenticationPrompt {
    pub kind: String,
    pub text: String,
    pub options: Option<Vec<BindingSettingOption>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingAuthenticationNotification {
    pub kind: String,
    pub id: Option<String>,
    pub text: Option<String>,
    pub url: Option<String>,
    pub instructions: Option<String>,
    pub user_code: Option<String>,
    pub verification_uri: Option<String>,
}

mod adapter_metadata {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(
        value: &Option<String>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let parsed = value
            .as_ref()
            .map(|text| {
                serde_json::from_str::<serde_json::Value>(text).map_err(serde::ser::Error::custom)
            })
            .transpose()?;
        parsed.serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<String>, D::Error> {
        Option::<serde_json::Value>::deserialize(deserializer)?
            .map(|value| serde_json::to_string(&value).map_err(serde::de::Error::custom))
            .transpose()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum BindingSettingKind {
    Text,
    FilePath,
    DirectoryPath,
    Choice,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum BindingAuthenticationMethod {
    ApiKey,
    #[serde(rename = "oauth")]
    OAuth,
    Unconfigured,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingAuthenticationProvenance {
    pub display_name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingProviderAuthentication {
    pub method: BindingAuthenticationMethod,
    pub configured: bool,
    pub provenance: Option<BindingAuthenticationProvenance>,
    pub actions: Vec<BindingSettingAction>,
}
#[derive(Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum BindingAuthenticationEdit {
    Keep,
    SetApiKey { value: String },
    Clear,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeInteraction {
    pub id: String,
    pub kind: BindingInteractionKind,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BindingInteractionKind {
    Approval {
        title: String,
        detail: String,
        options: Vec<BindingInteractionOption>,
    },
    UserInput {
        questions: Vec<BindingInteractionQuestion>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingInteractionOption {
    pub id: String,
    pub label: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingInteractionQuestion {
    pub id: String,
    pub text: String,
    pub options: Vec<BindingInteractionOption>,
    pub secret: bool,
}
#[derive(Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BindingRuntimeInteractionReply {
    Decision {
        #[serde(rename = "optionId")]
        option_id: String,
    },
    Answers {
        answers: Vec<BindingInteractionAnswer>,
    },
    Cancel,
}
#[derive(Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingInteractionAnswer {
    pub question_id: String,
    pub values: Vec<String>,
}

/// A provider-scoped public catalog entry, used only for editable template copies.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingCatalogModel {
    pub source_provider_id: String,
    pub source_provider_name: String,
    pub model_id: String,
    pub name: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingHistoryFailure {
    pub runtime_id: String,
    pub detail: String,
}

fn display_conversation_title<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let title = velune_application::conversation::ConversationTitle::deserialize(deserializer)?;
    Ok(title.display_text().into())
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingComposerDraft {
    #[serde(rename = "conversationID")]
    pub conversation_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingOutlinePresentation {
    Popover,
    InPlace,
}

/// An item the composer's status line can show.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingStatusLineItem {
    ContextUsage,
    SessionTokens,
    WorkingDirectory,
    RateLimits,
    SessionCost,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingTranscriptTurn {
    pub id: String,
    pub user_message_id: String,
    pub work_message_ids: Vec<String>,
    /// Visible text-only assistant, including streaming content; not a final-answer flag.
    pub last_message_id: Option<String>,
    pub reply_start_block: u32,
    pub duration_ms: Option<u64>,
    pub is_running: bool,
    /// What the turn ran with, as far as the native record and Velune's own
    /// request say.
    pub settings: Option<BindingTurnSettings>,
    /// What the native record says the turn used.
    pub usage: Option<BindingTurnUsage>,
}

/// Each part is absent when nothing says. The runtime is an instance ID; the
/// effort and permission mode are IDs in the runtime type's vocabulary (a
/// permission mode the type does not declare is the native record's own
/// value); the model is display text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingTurnSettings {
    pub runtime_id: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
}

/// Token counts summed over a turn's model requests. Input includes cache
/// reads and writes, output includes reasoning; a count is absent when the
/// harness does not record it. The rest is where the conversation stood when
/// the turn ended: the context its last request left against the model's
/// window (the harness's report, else the managed model's configuration),
/// the harness's own cost estimate for the whole session, and the account
/// limits the harness last reported.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingTurnUsage {
    pub input_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub context_tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub session_cost_micro_usd: Option<u64>,
    pub rate_limits: Vec<BindingRateLimit>,
}

/// One window of an account usage limit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingRateLimit {
    pub window_minutes: Option<u64>,
    pub used_percent: u32,
    pub resets_at_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingMessageIdentityConfirmation {
    pub previous_id: String,
    pub current_id: String,
}

/// Application-owned sidebar choices; these never select an execution runtime.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationBrowserPreferences {
    pub grouping: BindingConversationBrowserGrouping,
    pub sort: BindingConversationBrowserSort,
    pub oldest_first: bool,
    pub runtime_id: Option<String>,
    pub project: BindingConversationBrowserProject,
    /// Groups the user folded.
    #[serde(default)]
    #[uniffi(default = [])]
    pub collapsed: Vec<BindingConversationGroupKey>,
}
/// One group of the conversation list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum BindingConversationGroupKey {
    All,
    Pinned,
    Runtime {
        #[serde(rename = "runtimeID")]
        runtime_id: String,
    },
    /// None for conversations without a working directory.
    Project {
        path: Option<String>,
    },
}
/// One conversation of the list with Velune's organization of it.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingListedConversation {
    pub conversation: BindingConversationSummary,
    /// The identity its runtime resumes it by.
    #[serde(rename = "nativeID")]
    pub native_id: String,
    pub pinned: bool,
    pub unread: bool,
    pub archived: bool,
    /// Where it was moved, else its working directory.
    pub project: Option<String>,
    pub moved: bool,
    /// Its latest turn failed; looked up for unread conversations only.
    pub failed: bool,
    /// The group showing it; none for an open conversation the list does not show.
    pub group: Option<BindingConversationGroupKey>,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationGroup {
    pub key: BindingConversationGroupKey,
    pub alias: Option<String>,
    pub total: u32,
    /// The list holds the group's first `loaded` conversations.
    pub loaded: u32,
    pub latest_activity_unix_ms: Option<i64>,
    pub collapsed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationProject {
    pub path: String,
    pub alias: Option<String>,
    pub latest_activity_unix_ms: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationSearch {
    pub conversations: Vec<BindingListedConversation>,
    pub groups: Vec<BindingConversationGroup>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConversationBrowserLimits {
    /// Conversations loaded per group at a time.
    pub page: u32,
    /// Conversations the whole list loads before groups are expanded on demand.
    pub total: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct BindingConversationReference {
    #[serde(rename = "runtimeInstanceID")]
    pub runtime_id: String,
    #[serde(rename = "conversationID")]
    pub conversation_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", tag = "operation")]
pub enum BindingConversationOrganizationChange {
    Pin,
    Unpin,
    Archive,
    Unarchive,
    MarkRead,
    MarkUnread,
    /// None returns it to its working directory's project.
    MoveToProject {
        path: Option<String>,
    },
}
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingConversationBrowserGrouping {
    #[default]
    None,
    Runtime,
    Project,
}
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingConversationBrowserSort {
    #[default]
    Updated,
    Created,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingConversationBrowserProject {
    #[default]
    All,
    Unspecified,
    Path {
        path: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BindingTranscriptItem {
    Message { id: String, message_id: String },
    Work { turn_id: String },
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingTranscriptOutlineEntry {
    pub id: String,
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingTranscriptMessageIdentity {
    pub id: String,
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum BindingRuntimeTransport {
    Local,
    Ssh {
        binary: String,
        destination: String,
        port: Option<u16>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum BindingRuntimeHome {
    Inherit,
    Existing { path: String },
    Managed { remote_root: Option<String> },
}
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingRuntimeExecution {
    pub transport: BindingRuntimeTransport,
    pub home: BindingRuntimeHome,
    pub remote_resources_directory: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingRuntimeResourceKind {
    UserInstructions,
    McpServers,
    Skills,
    NativeAgentDefinitions,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingRuntimeResourceScope {
    User,
    Project,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeResourceKey {
    pub kind: BindingRuntimeResourceKind,
    pub scope: BindingRuntimeResourceScope,
    /// Relative source path, or an opaque server name for MCP entries.
    pub path: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingRuntimeResourceNode {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BindingRuntimeResourceState {
    Present,
    Missing,
    Invalid { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BindingRuntimeResourceAccess {
    Writable,
    ReadOnly { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeResourceEntry {
    pub key: BindingRuntimeResourceKey,
    pub absolute_path: String,
    pub node: BindingRuntimeResourceNode,
    pub revision: Option<String>,
    pub state: BindingRuntimeResourceState,
    pub access: BindingRuntimeResourceAccess,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BindingRuntimeResourceSupport {
    Supported,
    Unsupported { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeResourceCatalog {
    pub support: BindingRuntimeResourceSupport,
    pub entries: Vec<BindingRuntimeResourceEntry>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingRuntimeResourceDocument {
    pub entry: BindingRuntimeResourceEntry,
    /// Only the requested file, or one MCP server's native JSON object.
    pub bytes: Vec<u8>,
    pub executable: bool,
}
impl std::fmt::Debug for BindingRuntimeResourceDocument {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BindingRuntimeResourceDocument")
            .field("entry", &self.entry)
            .field("bytes", &"[redacted]")
            .field("executable", &self.executable)
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(tag = "kind", content = "revision", rename_all = "camelCase")]
pub enum BindingRuntimeResourceRevision {
    Missing,
    Match(String),
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingTemplateSkillFile {
    pub path: String,
    pub bytes: Vec<u8>,
    pub executable: bool,
}
impl std::fmt::Debug for BindingTemplateSkillFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BindingTemplateSkillFile")
            .field("path", &self.path)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// Environment values and headers may be secrets; Debug prints none of the content.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(tag = "transport", rename_all = "camelCase")]
pub enum BindingTemplateMcpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
        cwd: Option<String>,
    },
    Http {
        url: String,
        headers: HashMap<String, String>,
    },
}
impl std::fmt::Debug for BindingTemplateMcpTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BindingTemplateMcpTransport([redacted])")
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BindingResourceTemplateContent {
    Instructions {
        body: String,
    },
    Skill {
        files: Vec<BindingTemplateSkillFile>,
    },
    Mcp {
        transport: BindingTemplateMcpTransport,
        timeout_seconds: Option<u32>,
    },
    /// `model` is a native model ID written into the definition;
    /// `velune_model` a Velune model record key instead, supplied only to
    /// turns through Velune's gateway. At most one is set.
    Subagent {
        description: String,
        instructions: String,
        model: Option<String>,
        velune_model: Option<String>,
        tools: Vec<String>,
        effort: Option<String>,
        skills: Vec<String>,
    },
}
impl std::fmt::Debug for BindingResourceTemplateContent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BindingResourceTemplateContent([redacted])")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingResourceTemplate {
    /// Empty when saving a new template.
    pub id: String,
    pub name: String,
    pub content: BindingResourceTemplateContent,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingResourceTemplateSummary {
    pub id: String,
    pub name: String,
    pub kind: BindingRuntimeResourceKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BindingResourceSyncState {
    NotEnabled,
    Unsupported { reason: String },
    Synced,
    Pending,
    Drifted { paths: Vec<String> },
    Missing { paths: Vec<String> },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingResourceSyncStatus {
    pub template_id: String,
    pub runtime_id: String,
    pub state: BindingResourceSyncState,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingUnmanagedRuntimeResource {
    pub runtime_id: String,
    pub kind: BindingRuntimeResourceKind,
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingResourceSaveResult {
    pub summary: BindingResourceTemplateSummary,
    pub reports: Vec<BindingTemplateApplyReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BindingTemplateApplyOutcome {
    Create,
    Update,
    Unchanged,
    Conflict {
        unmanaged: Vec<String>,
        drifted: Vec<String>,
    },
    Skipped {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingTemplateApplyReport {
    pub runtime_id: String,
    pub outcome: BindingTemplateApplyOutcome,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingTemplateApplyTarget {
    pub runtime_id: String,
    pub overwrite: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum BindingResolution {
    Overwrite,
    Unmanage,
    Remove,
    TakeNative,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingResourceTemplateImport {
    pub template: BindingResourceTemplate,
    pub notes: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BindingResourceDriftFile {
    pub path: String,
    pub native: Option<String>,
    pub library: Option<String>,
    pub binary: bool,
}
impl std::fmt::Debug for BindingResourceDriftFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BindingResourceDriftFile")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}
