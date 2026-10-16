//! Application-owned provider configuration and private adapter integration metadata.
#[cfg(feature = "local-runtime")]
use velune_agent_runtime::model_projection::PiModelProjection;
#[cfg(not(feature = "local-runtime"))]
type PiModelProjection = serde_json::Value;
use serde::{Deserialize, Serialize};
#[cfg(feature = "local-runtime")]
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

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
    pub pi_projection: Option<PiModelProjection>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderDefinition {
    pub id: String,
    pub name: String,
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    pub(crate) authentication: crate::provider_authentication::ProviderAuthentication,
    pub models: Vec<ProviderModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum FailoverMode {
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailoverPolicy {
    pub mode: FailoverMode,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayConfig {
    pub id: String,
    pub name: String,
    pub providers: Vec<ProviderDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<ModelRoute>,
    pub failover: FailoverPolicy,
}

/// The provider and model are resolved together so callers cannot repeat the
/// lookup and introduce a second, potentially divergent missing-model branch.
pub struct ResolvedProviderModel<'a> {
    provider: &'a ProviderDefinition,
    model: &'a ProviderModel,
}

impl<'a> ResolvedProviderModel<'a> {
    pub fn provider(&self) -> &'a ProviderDefinition {
        self.provider
    }

    pub fn model(&self) -> &'a ProviderModel {
        self.model
    }
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

/// Association metadata only. Native sessions remain the authority for content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg(feature = "local-runtime")]
pub(crate) struct ConversationLink {
    pub id: String,
    pub segments: Vec<ConversationSegmentReference>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg(feature = "local-runtime")]
pub(crate) struct ConversationSegmentReference {
    pub runtime_instance_id: String,
    pub runtime_type_id: String,
    pub native_conversation_id: String,
    pub cwd: Option<String>,
    pub cutoff: Option<ConversationCutoff>,
    pub handoff: Option<ConversationHandoff>,
    pub deleted: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg(feature = "local-runtime")]
pub(crate) struct ConversationCutoff {
    pub message_count: u64,
    pub prefix_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg(feature = "local-runtime")]
pub(crate) struct ConversationHandoff {
    pub marker: String,
    pub payload_digest: String,
    pub user_message_ordinal: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeTypeDescriptor {
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
    pub(crate) fn validate_execution_paths(&self) -> Result<(), &'static str> {
        if !matches!(
            self.type_id.as_str(),
            "pi-1.0.2" | "codex-0.159.3" | "dsh-acp-0.2.0-rc.2" | "claude-code-2.1.286" | "acp-v1"
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
            if key == "nodeBinary" && self.type_id == "claude-code-2.1.286" {
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
            && (self.type_id != "pi-1.0.2"
                || !std::path::Path::new(value).is_absolute()
                || value.contains('\0'))
        {
            return Err("会话存储覆盖只适用于 Pi，且必须是绝对路径");
        }

        Ok(())
    }
}

impl GatewayConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        for provider in &self.providers {
            provider
                .authentication
                .validate_target(&provider.protocol, &provider.endpoint)?;
            let endpoint = url::Url::parse(&provider.endpoint).map_err(|_| "提供商服务地址无效")?;
            if !matches!(endpoint.scheme(), "http" | "https")
                || endpoint.host_str().is_none()
                || !endpoint.username().is_empty()
                || endpoint.password().is_some()
                || endpoint.query().is_some()
                || endpoint.fragment().is_some()
            {
                return Err("提供商服务地址必须是没有认证信息、查询或片段的 HTTP(S) 地址");
            }
        }
        self.to_gateway_config()
            .map_err(|_| "gateway configuration encoding")?
            .validate()
    }

    #[cfg(feature = "local-runtime")]
    pub(crate) fn authentication_revision(&self, key: &str) -> u64 {
        self.resolve_dispatch(key)
            .map_or(0, |resolved| resolved.provider.authentication.generation)
    }
    /// A route's requests must suit every member, so one subscription member
    /// makes the whole route follow the subscription contract.
    #[cfg(feature = "local-runtime")]
    pub(crate) fn subscription(&self, key: &str) -> bool {
        self.members(key).iter().any(|member| {
            self.providers
                .iter()
                .find(|p| p.models.iter().any(|m| &m.record_key == member))
                .is_some_and(|p| p.authentication.subscription())
        })
    }
    /// The provider models a selection key dispatches to: a route's members,
    /// or the provider model itself.
    #[cfg_attr(not(feature = "local-runtime"), allow(dead_code))]
    pub(crate) fn members(&self, key: &str) -> Vec<String> {
        self.route(key)
            .map_or_else(|| vec![key.to_owned()], |route| route.members.clone())
    }
    pub fn route(&self, key: &str) -> Option<&ModelRoute> {
        self.routes.iter().find(|route| route.record_key == key)
    }
    /// Every key a conversation can select: provider models, then routes.
    #[cfg(feature = "local-runtime")]
    pub(crate) fn selectable_keys(&self) -> impl Iterator<Item = &str> {
        self.providers
            .iter()
            .flat_map(|p| &p.models)
            .map(|m| m.record_key.as_str())
            .chain(self.routes.iter().map(|r| r.record_key.as_str()))
    }
    /// Remove route members whose provider model no longer exists, and routes
    /// left without members.
    pub(crate) fn prune_routes(&mut self) {
        let models = self
            .providers
            .iter()
            .flat_map(|p| &p.models)
            .map(|m| m.record_key.clone())
            .collect::<std::collections::BTreeSet<_>>();
        for route in &mut self.routes {
            route.members.retain(|key| models.contains(key));
        }
        self.routes.retain(|route| !route.members.is_empty());
    }
    /// Generate private record keys only for newly-created records. Native provider IDs remain unchanged.
    pub(crate) fn assign_record_keys(&mut self) {
        for model in self.providers.iter_mut().flat_map(|p| &mut p.models) {
            if model.record_key.is_empty() {
                model.record_key = crate::new_record_key();
            }
        }
        for route in &mut self.routes {
            if route.record_key.is_empty() {
                route.record_key = crate::new_record_key();
            }
        }
    }
    /// Return the stable physical model identity used by a Pi catalog.
    ///
    /// The logical model remains the gateway request key. This identity binds
    /// the complete provider target without including bearer material or the
    /// Node executable path, so changing a provider target or account generation cannot
    /// reuse an incompatible Pi model entry.
    #[cfg(feature = "local-runtime")]
    pub fn pi_binding_id(
        &self,
        model_record_key: &str,
        authentication_revision: u64,
    ) -> Result<String, &'static str> {
        let resolved = self.resolve_dispatch(model_record_key)?;
        let provider = resolved.provider();
        let binding = resolved.model();
        let identity = PiBindingIdentity {
            model_record_key,
            provider_id: provider.id.as_str(),
            protocol: provider.protocol.identity_name(),
            endpoint: provider.endpoint.as_str(),
            provider_model_id: binding.provider_model_id.as_str(),
            pi_projection: binding.pi_projection.as_ref(),
            authentication_generation: provider.authentication.generation,
            authentication_revision,
            context_window: binding.context_window,
            max_output_tokens: binding.max_output_tokens,
            reasoning_levels: binding.reasoning_levels.as_ref(),
        };
        let canonical = serde_json::to_vec(&identity).map_err(|_| "binding identity encoding")?;
        let digest = Sha256::digest(canonical);
        Ok(format!("vln_{digest:x}"))
    }

    /// Resolve model ownership from the already validated application
    /// configuration. A route resolves to its first member, whose protocol
    /// and model parameters the runtime is given; members share a protocol.
    pub fn resolve_dispatch(
        &self,
        model_record_key: &str,
    ) -> Result<ResolvedProviderModel<'_>, &'static str> {
        let model_record_key = self
            .route(model_record_key)
            .map_or(model_record_key, |route| route.members[0].as_str());
        self.providers
            .iter()
            .find_map(|provider| {
                provider
                    .models
                    .iter()
                    .find(|model| model.record_key == model_record_key)
                    .map(|model| ResolvedProviderModel { provider, model })
            })
            .ok_or("provider model is not configured")
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg(feature = "local-runtime")]
struct PiBindingIdentity<'a> {
    model_record_key: &'a str,
    provider_id: &'a str,
    protocol: &'static str,
    endpoint: &'a str,
    provider_model_id: &'a str,
    pi_projection: Option<&'a PiModelProjection>,
    authentication_generation: u64,
    authentication_revision: u64,
    context_window: Option<u32>,
    max_output_tokens: Option<u32>,
    reasoning_levels: Option<&'a Vec<String>>,
}

impl GatewayProtocol {
    /// The versioned runtime registry owns support; family IDs never grant capabilities.
    #[cfg(feature = "local-runtime")]
    pub(crate) fn runtime_protocols(type_id: &str) -> Option<Vec<Self>> {
        use velune_agent_runtime::version::{RuntimeProtocol, variant};
        Some(
            variant(type_id)?
                .supported_protocols
                .iter()
                .map(|protocol| match protocol {
                    RuntimeProtocol::ChatCompletionsV1 => Self::ChatCompletionsV1,
                    RuntimeProtocol::ResponsesV1 => Self::ResponsesV1,
                    RuntimeProtocol::MessagesV1 => Self::MessagesV1,
                })
                .collect(),
        )
    }
    /// Select the wire protocol the runtime speaks, preferring native passthrough.
    #[cfg(feature = "local-runtime")]
    pub(crate) fn runtime_ingress(type_id: &str, upstream: &Self) -> Option<Self> {
        let protocols = Self::runtime_protocols(type_id)?;
        protocols
            .iter()
            .find(|protocol| *protocol == upstream)
            .or_else(|| protocols.first())
            .cloned()
    }

    /// All three provider protocols are reachable through the LLM gateway.
    #[cfg(feature = "local-runtime")]
    pub(crate) fn runtime_provider_protocols(type_id: &str) -> Option<Vec<Self>> {
        let native = Self::runtime_protocols(type_id)?;
        if native.is_empty() {
            return Some(Vec::new());
        }
        Some(vec![
            Self::ChatCompletionsV1,
            Self::ResponsesV1,
            Self::MessagesV1,
        ])
    }

    #[cfg(feature = "local-runtime")]
    fn identity_name(&self) -> &'static str {
        match self {
            Self::ChatCompletionsV1 => "chatCompletionsV1",
            Self::ResponsesV1 => "responsesV1",
            Self::MessagesV1 => "messagesV1",
        }
    }
}

impl GatewayConfig {
    /// Strip adapter metadata at the application-to-gateway boundary.
    pub fn to_gateway_config(&self) -> Result<velune_gateway::GatewayConfig, crate::Error> {
        Ok(velune_gateway::GatewayConfig {
            id: self.id.clone(),
            name: self.name.clone(),
            providers: self
                .providers
                .iter()
                .map(|p| {
                    Ok(velune_gateway::ProviderDefinition {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        protocol: match p.protocol {
                            GatewayProtocol::ChatCompletionsV1 => {
                                velune_gateway::GatewayProtocol::ChatCompletionsV1
                            }
                            GatewayProtocol::ResponsesV1 => {
                                velune_gateway::GatewayProtocol::ResponsesV1
                            }
                            GatewayProtocol::MessagesV1 => {
                                velune_gateway::GatewayProtocol::MessagesV1
                            }
                        },
                        endpoint: p.endpoint.clone(),
                        credential_ref: Some(p.id.clone()),
                        models: p
                            .models
                            .iter()
                            .map(|m| velune_gateway::ProviderModel {
                                record_key: m.record_key.clone(),
                                nickname: m.nickname.clone(),
                                icon: m.icon.clone(),
                                provider_model_id: m.provider_model_id.clone(),
                                context_window: m.context_window,
                                max_output_tokens: m.max_output_tokens,
                                reasoning_levels: m.reasoning_levels.clone(),
                            })
                            .collect(),
                    })
                })
                .collect::<Result<Vec<_>, crate::Error>>()?,
            routes: self
                .routes
                .iter()
                .map(|route| velune_gateway::ModelRoute {
                    record_key: route.record_key.clone(),
                    name: route.name.clone(),
                    members: route.members.clone(),
                })
                .collect(),
            failover: velune_gateway::FailoverPolicy {
                mode: velune_gateway::FailoverMode::Disabled,
            },
        })
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
    /// A conversation's project: its working directory unless moved; none
    /// when the runtime recorded no directory.
    Project {
        path: Option<String>,
    },
}

/// Velune's own organization of the conversation list, kept apart from the
/// runtimes' sessions: what the user pinned, archived (where the runtime
/// has no archive), read, or moved, and the names given to projects.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationOrganization {
    /// Activity at or before this time counts as read without a mark: the
    /// conversations that existed before read state was first recorded.
    pub read_baseline_unix_ms: i64,
    pub conversations: Vec<ConversationMarks>,
    pub project_aliases: Vec<ProjectAlias>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationMarks {
    /// The conversation's public identity in the list.
    #[serde(rename = "conversationID")]
    pub conversation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_at_unix_ms: Option<i64>,
    /// Archived by Velune; a runtime's own archive is not recorded here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at_unix_ms: Option<i64>,
    /// The conversation's last activity when the user last saw it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_activity_unix_ms: Option<i64>,
    /// Marked unread by the user, whatever the activity.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub marked_unread: bool,
    /// The project the user moved the conversation to; its working directory
    /// is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}
impl ConversationMarks {
    pub fn is_empty(&self) -> bool {
        self.pinned_at_unix_ms.is_none()
            && self.archived_at_unix_ms.is_none()
            && self.seen_activity_unix_ms.is_none()
            && !self.marked_unread
            && self.project.is_none()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectAlias {
    pub path: String,
    pub alias: String,
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
