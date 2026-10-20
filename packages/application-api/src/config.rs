use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProviderQuotaSource {
    None,
    CodexUsage,
    ClaudeCliUsage,
    KimiUsage,
    ArkCodingUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderModel {
    pub record_key: String,
    pub provider_model_id: String,
    pub nickname: String,
    pub icon: Option<String>,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelTemplate {
    pub id: String,
    pub name: String,
    pub suggested_provider_model_id: String,
    pub nickname: String,
    pub icon: Option<String>,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning_levels: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum GatewayProtocol {
    ChatCompletionsV1,
    ResponsesV1,
    MessagesV1,
}

/// An ordered set of provider models chosen like a model; the gateway fails
/// over between its members.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelRoute {
    pub record_key: String,
    pub name: String,
    /// Provider model record keys, in priority order.
    pub members: Vec<String>,
    pub strategy: RoutingStrategy,
    pub weights: BTreeMap<String, u32>,
    pub sticky: bool,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeInstance {
    #[serde(default = "runtime_enabled_by_default")]
    pub enabled: bool,
    pub id: String,
    pub name: String,
    pub type_id: String,
    pub gateway_id: String,
    pub settings: BTreeMap<String, String>,
}
impl std::fmt::Debug for RuntimeInstance {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeInstance")
            .field("id", &self.id)
            .field("type_id", &self.type_id)
            .field("enabled", &self.enabled)
            .field("setting_keys", &self.settings.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeTypeDescriptor {
    pub id: String,
    pub family_id: String,
    pub compatible_versions: String,
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
    pub supported_protocols: Vec<GatewayProtocol>,
    /// Provider protocols reachable through native dispatch or gateway conversion.
    pub supported_provider_protocols: Vec<GatewayProtocol>,
    pub name: String,
    pub fields: Vec<crate::conversation::SettingField>,
    /// Modes a turn may request, in the runtime's own vocabulary; empty when
    /// the runtime offers no choice.
    pub permission_modes: Vec<PermissionMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionMode {
    pub id: String,
    pub label: String,
    /// What the mode allows, for help text.
    pub detail: String,
}

fn runtime_enabled_by_default() -> bool {
    true
}

impl RuntimeInstance {
    /// Validate execution-path settings when saving an instance. Existing
    /// configuration remains readable so users can repair incomplete settings.
    pub fn validate_execution_paths(&self) -> Result<(), &'static str> {
        if !matches!(
            self.type_id.as_str(),
            "pi-1.0" | "codex-0.159" | "dsh-acp-0.2" | "claude-code-2.1" | "acp-v1"
        ) {
            return Err("不支持的运行时版本类型");
        }
        let execution = self.execution().map_err(|_| "运行位置或 HOME 配置无效")?;
        if self.type_id == "acp-v1" {
            self.acp_launch().map_err(|_| "ACP 启动配置无效")?;
        }
        for key in ["binary", "agentDir", "nodeBinary"] {
            if self.type_id == "acp-v1" && key != "binary" {
                continue;
            }
            if key == "agentDir" && matches!(execution.home, crate::RuntimeHome::Managed { .. }) {
                continue;
            }
            if key == "nodeBinary" && self.type_id == "claude-code-2.1" {
                continue;
            }
            let value = self
                .settings
                .get(key)
                .filter(|v| !v.is_empty())
                .ok_or("运行时入口、运行时目录与 Node 可执行文件必须配置")?;
            if !std::path::Path::new(value).is_absolute() || value.contains('\0') {
                return Err("运行时路径必须是绝对路径");
            }
        }
        if let Some(value) = self.settings.get("sessionDir").filter(|v| !v.is_empty())
            && (self.type_id != "pi-1.0"
                || !std::path::Path::new(value).is_absolute()
                || value.contains('\0'))
        {
            return Err("会话存储覆盖只适用于 Pi，且必须是绝对路径");
        }

        Ok(())
    }
}

/// What the conversation outline button does; the outline lists user messages.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OutlinePresentation {
    /// A popover anchored to the button.
    #[default]
    Popover,
    /// Collapses the message list in place to its user messages.
    InPlace,
}

/// Whether quota windows and the context window read as used or remaining.
/// This only changes presentation; quota ceilings remain used-percent limits.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UsageDisplay {
    #[default]
    Used,
    Remaining,
}

/// Where a conversation linked from another opens: a native sub-agent's, or
/// one an agent contacted or was contacted by through Velune.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LinkedConversationOpening {
    /// In place of the conversation it is linked from.
    #[default]
    CurrentTab,
    NewTab,
    /// In a sheet over the window, from which it may move to a tab or window.
    Sheet,
    NewWindow,
}

/// What the composer's status line shows; the context usage alone unless the
/// user pins more. The line keeps this order whatever order they were chosen in.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum StatusLineItem {
    ContextUsage,
    SessionTokens,
    WorkingDirectory,
    RateLimits,
    SessionCost,
}

pub fn default_status_line() -> Vec<StatusLineItem> {
    vec![StatusLineItem::ContextUsage]
}

/// The items in display order, each once.
pub fn canonical_status_line(mut items: Vec<StatusLineItem>) -> Vec<StatusLineItem> {
    items.sort();
    items.dedup();
    items
}

/// Unsent composer text of one conversation, keyed by its public identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComposerDraft {
    #[serde(rename = "conversationID")]
    pub conversation_id: String,
    pub text: String,
}

/// What the user chose for a turn on one runtime. Each part that is absent
/// follows the conversation's current one, and without one the runtime's own
/// default; Velune never names a default of its own.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TurnChoice {
    pub model: Option<crate::ModelSelection>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
}
impl TurnChoice {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// One runtime's choice in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeTurnChoice {
    #[serde(rename = "runtimeID")]
    pub runtime_id: String,
    pub choice: TurnChoice,
}

/// A conversation's choices for its next turn, kept until it is deleted:
/// each runtime keeps its own, and an unsent conversation also the runtime
/// chosen for it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NextTurnIntent {
    #[serde(rename = "conversationID")]
    pub conversation_id: String,
    #[serde(rename = "runtimeID")]
    pub runtime_id: Option<String>,
    pub choices: Vec<RuntimeTurnChoice>,
}

/// What a runtime instance's new conversations start with.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum NewConversationChoice {
    /// The user's last choice on the runtime.
    #[default]
    LastChosen,
    /// These; an absent part is the runtime's own default.
    Chosen { choice: TurnChoice },
}

/// A runtime instance's composer choices across conversations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeComposerChoices {
    #[serde(rename = "runtimeID")]
    pub runtime_id: String,
    /// The user's last choice of each part on the runtime.
    pub last_chosen: TurnChoice,
    pub new_conversation: NewConversationChoice,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComposerChoices {
    pub conversations: Vec<NextTurnIntent>,
    pub runtimes: Vec<RuntimeComposerChoices>,
}

/// Application-owned sidebar choices; these never select an execution runtime.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationBrowserPreferences {
    pub grouping: ConversationBrowserGrouping,
    pub sort: ConversationBrowserSort,
    pub oldest_first: bool,
    pub runtime_id: Option<String>,
    pub project: ConversationBrowserProject,
    /// Groups the user folded; remembered with the other choices.
    #[serde(default)]
    pub collapsed: Vec<ConversationGroupKey>,
    /// How groups follow each other; `sort` orders the rows within them.
    #[serde(default)]
    pub group_sort: ConversationGroupSort,
    /// Groups in the order the user put them, each grouping's own; kept
    /// while groups follow recent activity, for when they no longer do.
    #[serde(default)]
    pub group_order: Vec<ConversationGroupKey>,
}
/// One group of the conversation list. Keys of other groupings may stay in
/// the folded set; they apply again when that grouping returns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ConversationGroupKey {
    /// The single group when the list is not grouped.
    All,
    Pinned,
    Runtime {
        #[serde(rename = "runtimeID")]
        runtime_id: String,
    },
    /// A conversation's project by its primary folder: where it was moved,
    /// else the project its working directory belongs to; none when the
    /// runtime recorded no directory or started it without a project.
    Project {
        path: Option<String>,
    },
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConversationBrowserGrouping {
    #[default]
    None,
    Runtime,
    Project,
}
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConversationBrowserSort {
    #[default]
    Updated,
    Created,
}
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConversationGroupSort {
    /// The group with the latest activity first.
    #[default]
    Activity,
    /// In `group_order`, which no time moves.
    Manual,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConversationBrowserProject {
    #[default]
    All,
    Unspecified,
    Path {
        path: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ExecutionBackend {
    Http,
    ClaudeCli { binary: String },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuotaLimit {
    pub window_id: String,
    pub max_used_percent: u8,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RoutingStrategy {
    Smart,
    Order,
    Rotate,
    Weight,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequestProfile {
    Native,
    CodexResponses,
    KimiCoding,
    ArkCoding,
}
