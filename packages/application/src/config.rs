//! Application-owned provider configuration and private adapter integration metadata.
#[cfg(feature = "local-runtime")]
use velune_agent_runtime::model_projection::PiModelProjection;
#[cfg(not(feature = "local-runtime"))]
type PiModelProjection = serde_json::Value;
use serde::{Deserialize, Serialize};
#[cfg(feature = "local-runtime")]
use sha2::{Digest, Sha256};
pub use velune_application_api::config::*;
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
pub struct ProviderDefinition {
    pub id: String,
    pub name: String,
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    pub execution_backend: ExecutionBackend,
    pub request_profile: RequestProfile,
    pub quota_source: ProviderQuotaSource,
    pub(crate) authentication: crate::provider_authentication::ProviderAuthentication,
    pub models: Vec<ProviderModel>,
    pub quota_limits: Vec<QuotaLimit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayConfig {
    pub id: String,
    pub name: String,
    pub providers: Vec<ProviderDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<ModelRoute>,
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

impl GatewayConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        for provider in &self.providers {
            provider
                .authentication
                .validate_target(&provider.protocol, &provider.endpoint)?;
            provider.validate_composition()?;
            if matches!(
                provider.execution_backend,
                ExecutionBackend::ClaudeCli { .. }
            ) {
                continue;
            }
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
            route.weights.retain(|key, _| route.members.contains(key));
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
            protocol: match provider.protocol {
                GatewayProtocol::ChatCompletionsV1 => "chatCompletionsV1",
                GatewayProtocol::ResponsesV1 => "responsesV1",
                GatewayProtocol::MessagesV1 => "messagesV1",
            },
            execution_backend: &provider.execution_backend,
            request_profile: provider.request_profile,
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
    execution_backend: &'a ExecutionBackend,
    request_profile: RequestProfile,
    endpoint: &'a str,
    provider_model_id: &'a str,
    pi_projection: Option<&'a PiModelProjection>,
    authentication_generation: u64,
    authentication_revision: u64,
    context_window: Option<u32>,
    max_output_tokens: Option<u32>,
    reasoning_levels: Option<&'a Vec<String>>,
}

/// The versioned runtime registry owns support; family IDs never grant capabilities.
#[cfg(feature = "local-runtime")]
pub(crate) fn runtime_protocols(type_id: &str) -> Option<Vec<GatewayProtocol>> {
    use velune_agent_runtime::version::{RuntimeProtocol, variant};
    Some(
        variant(type_id)?
            .supported_protocols
            .iter()
            .map(|protocol| match protocol {
                RuntimeProtocol::ChatCompletionsV1 => GatewayProtocol::ChatCompletionsV1,
                RuntimeProtocol::ResponsesV1 => GatewayProtocol::ResponsesV1,
                RuntimeProtocol::MessagesV1 => GatewayProtocol::MessagesV1,
            })
            .collect(),
    )
}
/// Select the wire protocol the runtime speaks, preferring native passthrough.
#[cfg(feature = "local-runtime")]
pub(crate) fn runtime_ingress(
    type_id: &str,
    upstream: &GatewayProtocol,
) -> Option<GatewayProtocol> {
    let protocols = runtime_protocols(type_id)?;
    protocols
        .iter()
        .find(|protocol| *protocol == upstream)
        .or_else(|| protocols.first())
        .cloned()
}

/// All three provider protocols are reachable through the LLM gateway.
#[cfg(feature = "local-runtime")]
pub(crate) fn runtime_provider_protocols(type_id: &str) -> Option<Vec<GatewayProtocol>> {
    let native = runtime_protocols(type_id)?;
    if native.is_empty() {
        return Some(Vec::new());
    }
    Some(vec![
        GatewayProtocol::ChatCompletionsV1,
        GatewayProtocol::ResponsesV1,
        GatewayProtocol::MessagesV1,
    ])
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
                        execution_backend: gateway_execution_backend(&p.execution_backend),
                        request_profile: gateway_request_profile(p.request_profile),
                        credential_ref: Some(p.id.clone()),
                        quota_limits: p
                            .quota_limits
                            .iter()
                            .map(|limit| velune_gateway::QuotaLimit {
                                window_id: limit.window_id.clone(),
                                max_used_percent: limit.max_used_percent,
                            })
                            .collect(),
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
                    strategy: gateway_routing_strategy(route.strategy),
                    weights: route.weights.clone(),
                    sticky: route.sticky,
                })
                .collect(),
        })
    }
}

/// Velune's own organization of the conversation list, kept apart from the
/// runtimes' sessions: what the user pinned, archived (where the runtime
/// has no archive), read, or moved, and the projects the user shaped.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationOrganization {
    /// Activity at or before this time counts as read without a mark: the
    /// conversations that existed before read state was first recorded.
    pub read_baseline_unix_ms: i64,
    pub conversations: Vec<ConversationMarks>,
    /// Projects with a name or more folders than their primary one; any
    /// other working directory is a project of its own.
    pub projects: Vec<ProjectRecord>,
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
    /// The project, by its primary folder, the user moved the conversation
    /// to; its working directory is unchanged.
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
pub struct ProjectRecord {
    /// Identifies the project; new conversations in it are offered this folder.
    pub primary_folder: String,
    /// The user's name for it in the list; none shows the folder's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its other folders, whose conversations belong to it as well.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<String>,
}
impl ProjectRecord {
    pub fn all_folders(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.primary_folder.as_str()).chain(self.folders.iter().map(String::as_str))
    }
}
impl ProviderDefinition {
    pub(crate) fn validate_composition(&self) -> Result<(), &'static str> {
        use crate::provider_authentication::ProviderAuthenticationSource as Auth;
        let source = self.authentication.source();
        if let crate::provider_authentication::AuthenticationMaterial::ClaudeCli {
            credential_id,
            ..
        } = &self.authentication.material
            && (credential_id.is_empty()
                || credential_id.len() > 128
                || !credential_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        {
            return Err("CLI 认证引用无效");
        }
        if let crate::provider_authentication::AuthenticationMaterial::ClaudeCliLinked {
            login,
            config_dir,
            ..
        } = &self.authentication.material
            && (config_dir
                .as_deref()
                .is_some_and(|dir| !std::path::Path::new(dir).is_absolute() || dir.contains('\0'))
                || matches!(login, crate::provider_authentication::LinkedClaudeLogin::Runtime { instance_id } if instance_id.is_empty()))
        {
            return Err("沿用的 Claude Code 登录位置无效");
        }
        match &self.execution_backend {
            ExecutionBackend::Http => {
                if source == Auth::ClaudeCli {
                    return Err("Claude CLI 认证必须由 Claude CLI 执行后端消费");
                }
                if source == Auth::CodexOAuth
                    && self.request_profile != RequestProfile::CodexResponses
                {
                    return Err("Codex OAuth 必须显式选择 Codex Responses 请求配置");
                }
                if self.request_profile == RequestProfile::CodexResponses
                    && source != Auth::CodexOAuth
                {
                    return Err("Codex Responses 请求配置需要 Codex OAuth 认证");
                }
            }
            ExecutionBackend::ClaudeCli { binary } => {
                if !std::path::Path::new(binary).is_absolute() || binary.contains('\0') {
                    return Err("Claude CLI 可执行文件必须是绝对路径");
                }
                if source != Auth::ClaudeCli
                    || self.protocol != GatewayProtocol::MessagesV1
                    || self.request_profile != RequestProfile::Native
                    || !self.endpoint.is_empty()
                {
                    return Err(
                        "Claude CLI 必须组合 Messages、Native、Claude CLI 认证和空 HTTP 地址",
                    );
                }
            }
        }
        match self.quota_source {
            ProviderQuotaSource::CodexUsage if source != Auth::CodexOAuth => {
                Err("Codex 额度来源需要 Codex OAuth 认证")
            }
            ProviderQuotaSource::ClaudeCliUsage if source != Auth::ClaudeCli => {
                Err("Claude CLI 额度来源需要 Claude CLI 认证")
            }
            _ => Ok(()),
        }
    }
}

impl From<&ProviderModel> for velune_application_api::ProviderModel {
    fn from(model: &ProviderModel) -> Self {
        Self {
            record_key: model.record_key.clone(),
            provider_model_id: model.provider_model_id.clone(),
            nickname: model.nickname.clone(),
            icon: model.icon.clone(),
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            reasoning_levels: model.reasoning_levels.clone(),
        }
    }
}
impl From<velune_application_api::ProviderModel> for ProviderModel {
    fn from(model: velune_application_api::ProviderModel) -> Self {
        Self {
            record_key: model.record_key,
            provider_model_id: model.provider_model_id,
            nickname: model.nickname,
            icon: model.icon,
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            reasoning_levels: model.reasoning_levels,
            pi_projection: None,
        }
    }
}
pub(crate) fn gateway_execution_backend(
    value: &ExecutionBackend,
) -> velune_gateway::ExecutionBackend {
    match value {
        ExecutionBackend::Http => velune_gateway::ExecutionBackend::Http,
        ExecutionBackend::ClaudeCli { binary } => velune_gateway::ExecutionBackend::ClaudeCli {
            binary: binary.clone(),
        },
    }
}
pub(crate) fn gateway_request_profile(value: RequestProfile) -> velune_gateway::RequestProfile {
    match value {
        RequestProfile::Native => velune_gateway::RequestProfile::Native,
        RequestProfile::CodexResponses => velune_gateway::RequestProfile::CodexResponses,
    }
}
fn gateway_routing_strategy(value: RoutingStrategy) -> velune_gateway::RoutingStrategy {
    match value {
        RoutingStrategy::Smart => velune_gateway::RoutingStrategy::Smart,
        RoutingStrategy::Order => velune_gateway::RoutingStrategy::Order,
        RoutingStrategy::Rotate => velune_gateway::RoutingStrategy::Rotate,
        RoutingStrategy::Weight => velune_gateway::RoutingStrategy::Weight,
    }
}
