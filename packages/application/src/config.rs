//! Application-owned provider configuration and private adapter integration metadata.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use velune_agent_runtime::model_projection::PiModelProjection;
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
    /// Extra fields merged into every upstream request body for this model.
    pub request_parameters: Option<BTreeMap<String, String>>,
    pub pi_projection: Option<PiModelProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderDefinition {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub endpoint: String,
    pub execution_backend: ExecutionBackend,
    pub request_profile: RequestProfile,
    pub quota_source: ProviderQuotaSource,
    pub(crate) authentication: crate::provider_authentication::ProviderAuthentication,
    #[serde(default)]
    pub(crate) quota_authentication: crate::quota_authentication::QuotaAuthentication,
    #[serde(default)]
    pub(crate) billing: crate::billing::Configuration,
    pub models: Vec<ProviderModel>,
    pub quota_limits: Vec<QuotaLimit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_client_version: Option<String>,
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
pub(crate) struct ConversationLink {
    pub id: String,
    pub segments: Vec<ConversationSegmentReference>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ConversationSegmentReference {
    pub runtime_instance_id: String,
    pub runtime_type_id: RuntimeKind,
    pub native_conversation_id: String,
    pub cwd: Option<String>,
    pub cutoff: Option<ConversationCutoff>,
    pub handoff: Option<ConversationHandoff>,
    /// The segment begins with translated effective model context; display
    /// continues to use the original sealed transcript segments.
    #[serde(default)]
    pub imported: Option<ConversationImport>,
    /// Display tombstones for records removed from a rebuilt native context.
    /// The source itself remains immutable for recovery.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_message_ids: Vec<String>,
    /// Stable logical display identities mapped to this imported tail's exact
    /// native prompt identities. Enables repeated deletions after reconstruction.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub message_origins: std::collections::BTreeMap<String, Vec<String>>,
    pub deleted: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ConversationImport {
    /// Native display records seeded by import, hidden from the joined
    /// transcript so compact summaries and retained content are not repeated.
    pub prefix: ConversationCutoff,
    /// A portable-summary maintenance receipt; no summary or prompt content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_work_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ConversationCutoff {
    pub message_count: u64,
    pub prefix_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ConversationHandoff {
    pub marker: String,
    pub payload_digest: String,
    pub user_message_ordinal: u64,
}

impl GatewayConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        let mut antigravity_targets = std::collections::BTreeSet::new();
        for provider in &self.providers {
            provider
                .authentication
                .validate_target(&provider.protocol, &provider.endpoint)?;
            provider.validate_composition()?;
            if let ExecutionBackend::AntigravityCli {
                ssh_destination, ..
            } = &provider.execution_backend
                && !antigravity_targets.insert(ssh_destination.as_deref())
            {
                return Err("同一个 Antigravity 执行目标只能在网关中绑定一次");
            }
            if provider.execution_backend != ExecutionBackend::Http {
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

    pub(crate) fn authentication_revision(&self, key: &str) -> u64 {
        self.resolve_dispatch(key)
            .map_or(0, |resolved| resolved.provider.authentication.generation)
    }
    /// The provider models a selection key dispatches to: a route's members,
    /// or the provider model itself.
    pub(crate) fn members(&self, key: &str) -> Vec<String> {
        self.route(key)
            .map_or_else(|| vec![key.to_owned()], |route| route.members.clone())
    }
    pub fn route(&self, key: &str) -> Option<&ModelRoute> {
        self.routes.iter().find(|route| route.record_key == key)
    }
    /// Every key a conversation can select: provider models, then routes.
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
            protocol: provider.protocol.id(),
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
            request_parameters: binding.request_parameters.as_ref(),
        };
        let canonical = serde_json::to_vec(&identity).map_err(|_| "binding identity encoding")?;
        let digest = Sha256::digest(canonical);
        if provider.protocol == Protocol::Gemini {
            let model_id = binding
                .provider_model_id
                .strip_prefix("models/")
                .unwrap_or(&binding.provider_model_id);
            // Pi infers Gemini's thinking and tool parameter formats from the model name.
            // Its two Flash aliases require an exact ID, so keep the corresponding family
            // marker in the routing alias as well.
            let family_marker = match model_id.to_ascii_lowercase().as_str() {
                "gemini-flash-latest" | "gemini-flash-lite-latest" => "--gemini-3-flash",
                _ => "",
            };
            return Ok(format!("{model_id}{family_marker}--vln_{digest:x}"));
        }
        Ok(format!("vln_{digest:x}"))
    }

    /// Resolve model ownership from the already validated application
    /// configuration. A route resolves to its first member, whose protocol
    /// and model parameters the runtime is given. Each dispatch translates
    /// from that ingress protocol to the selected member's own protocol.
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
    request_parameters: Option<&'a BTreeMap<String, String>>,
}

/// The versioned runtime registry owns support; family IDs never grant capabilities.
pub(crate) fn runtime_protocols(kind: RuntimeKind) -> Vec<Protocol> {
    velune_agent_runtime::version::variant(kind)
        .supported_protocols
        .to_vec()
}
/// Select the wire protocol the runtime speaks, preferring native passthrough.
pub(crate) fn runtime_ingress(kind: RuntimeKind, upstream: &Protocol) -> Option<Protocol> {
    let protocols = runtime_protocols(kind);
    protocols
        .iter()
        .find(|protocol| *protocol == upstream)
        .or_else(|| protocols.first())
        .cloned()
}

/// Runtimes with a gateway ingress can reach all provider protocols.
pub(crate) fn runtime_provider_protocols(kind: RuntimeKind) -> Vec<Protocol> {
    if runtime_protocols(kind).is_empty() {
        return Vec::new();
    }
    Protocol::ALL.to_vec()
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
                        protocol: p.protocol,
                        endpoint: p.endpoint.clone(),
                        execution_backend: gateway_execution_backend(&p.execution_backend),
                        request_profile: gateway_request_profile(p.request_profile),
                        codex_client_version: p.codex_request_version().map(str::to_owned),
                        credential_ref: Some(p.credential_reference()),
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
                                request_parameters: m
                                    .request_parameters
                                    .clone()
                                    .unwrap_or_default(),
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
    /// Application-generated titles for runtimes without native renaming.
    /// A native title always takes precedence.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub generated_titles: BTreeMap<String, String>,
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
    /// Native async questions explicitly answered or dismissed in Velune.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved_async_questions: Vec<String>,
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
            && self.resolved_async_questions.is_empty()
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
    pub(crate) fn effective_quota_source(&self) -> ProviderQuotaSource {
        if self.quota_source == ProviderQuotaSource::None
            && matches!(
                self.execution_backend,
                ExecutionBackend::AntigravityCli { .. }
            )
        {
            // Legacy Antigravity settings predate the manual quota command.
            return ProviderQuotaSource::AntigravityCliUsage;
        }
        self.quota_source
    }

    pub(crate) fn credential_reference(&self) -> String {
        if let ExecutionBackend::AntigravityCli {
            binary,
            ssh_destination,
        } = &self.execution_backend
        {
            // The gateway receives a credential reference without the
            // executable or SSH destination. Reject a reference issued for an
            // earlier binding even when the rest of the route is unchanged.
            let target =
                serde_json::to_vec(&(binary, ssh_destination)).expect("Antigravity target tuple");
            let digest = Sha256::digest(target);
            return format!(
                "antigravity/{}/{digest:x}/{}",
                self.authentication.generation, self.id
            );
        }
        self.id.clone()
    }

    pub(crate) fn effective_codex_client_version(&self) -> &str {
        self.codex_client_version
            .as_deref()
            .unwrap_or(crate::default_codex_client_version())
    }

    pub(crate) fn codex_request_version(&self) -> Option<&str> {
        (self.request_profile == RequestProfile::CodexResponses)
            .then(|| self.effective_codex_client_version())
    }

    pub(crate) fn validate_composition(&self) -> Result<(), &'static str> {
        if let Some(version) = &self.codex_client_version {
            if self.request_profile != RequestProfile::CodexResponses {
                return Err("只有 Codex Responses 提供商支持设置 Codex 客户端版本");
            }
            if !velune_ai_provider::valid_codex_client_version(version) {
                return Err("Codex 客户端版本须为主版本.次版本.补丁版本");
            }
        }
        self.billing.validate(self)?;
        use crate::provider_authentication::ProviderAuthenticationSource as Auth;
        if !self
            .quota_authentication
            .valid(self.authentication.generation)
        {
            return Err("额度查询认证配置无效");
        }
        // Switching collection off preserves private query credentials. Only
        // the selected Ark collector can consume them; switching invalidates
        // the account confirmation in QuotaAuthentication::apply.
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
        // Model request adaptation and quota collection have independent contracts.
        if self.request_profile == RequestProfile::KimiCoding
            && (self.execution_backend != ExecutionBackend::Http
                || self.protocol != Protocol::Messages
                || source != Auth::ApiKey
                || kimi_region(&self.endpoint).is_none())
        {
            return Err("Kimi Coding 请求需要 HTTP、Messages、API key 与官方 Coding 地址");
        }
        if self.quota_source == ProviderQuotaSource::KimiUsage
            && (self.execution_backend != ExecutionBackend::Http
                || source != Auth::ApiKey
                || kimi_quota_region(&self.endpoint).is_none())
        {
            return Err("Kimi Coding 额度查询需要 HTTP、API key 与官方 Coding 地址");
        }
        if self.request_profile == RequestProfile::ArkCoding
            && (self.execution_backend != ExecutionBackend::Http
                || source != Auth::ApiKey
                || !ark_endpoint(&self.endpoint, &self.protocol))
        {
            return Err("ARK Coding 请求需要 HTTP、API key 与官方 Coding 地址");
        }
        if self.quota_source == ProviderQuotaSource::ArkCodingUsage
            && (self.execution_backend != ExecutionBackend::Http
                || source != Auth::ApiKey
                || !ark_endpoint(&self.endpoint, &self.protocol))
        {
            return Err("ARK Coding 额度查询需要 HTTP、API key 与官方 Coding 地址");
        }
        if self.quota_source == ProviderQuotaSource::ApiAccount
            && (self.execution_backend != ExecutionBackend::Http
                || source != Auth::ApiKey
                || velune_application_api::api_account_service(&self.endpoint).is_none())
        {
            return Err("官方账户额度需要 HTTP、API key 与支持的官方服务地址");
        }
        match &self.execution_backend {
            ExecutionBackend::Http => {
                if source == Auth::ClaudeCli {
                    return Err("Claude CLI 认证必须由 Claude CLI 执行后端消费");
                }
                if source == Auth::AntigravityCli {
                    return Err("Antigravity CLI 登录必须绑定到 Antigravity CLI 执行后端");
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
                    || self.protocol != Protocol::Messages
                    || self.request_profile != RequestProfile::Native
                    || !self.endpoint.is_empty()
                {
                    return Err(
                        "Claude CLI 必须组合 Messages、Native、Claude CLI 认证和空 HTTP 地址",
                    );
                }
            }
            ExecutionBackend::AntigravityCli {
                binary,
                ssh_destination,
            } => {
                crate::antigravity_cli::validate_target(binary, ssh_destination.as_deref())?;
                if source != Auth::AntigravityCli
                    || self.protocol != Protocol::Messages
                    || self.request_profile != RequestProfile::Native
                    || !self.endpoint.is_empty()
                    || !matches!(
                        self.quota_source,
                        ProviderQuotaSource::None | ProviderQuotaSource::AntigravityCliUsage
                    )
                    || !self.quota_limits.is_empty()
                {
                    return Err(
                        "Antigravity CLI 需要 Messages、Native 和 CLI 登录绑定，只支持其自带额度查询，服务地址与额度上限留空",
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
            ProviderQuotaSource::AntigravityCliUsage if source != Auth::AntigravityCli => {
                Err("Antigravity CLI 额度只支持 Antigravity 订阅")
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
            request_parameters: model.request_parameters.clone(),
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
            request_parameters: model.request_parameters,
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
        ExecutionBackend::AntigravityCli { .. } => velune_gateway::ExecutionBackend::AntigravityCli,
    }
}
pub(crate) fn gateway_request_profile(value: RequestProfile) -> velune_gateway::RequestProfile {
    match value {
        RequestProfile::Native => velune_gateway::RequestProfile::Native,
        RequestProfile::CodexResponses => velune_gateway::RequestProfile::CodexResponses,
        RequestProfile::KimiCoding => velune_gateway::RequestProfile::KimiCoding,
        RequestProfile::ArkCoding => velune_gateway::RequestProfile::ArkCoding,
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

/// Only first-party Coding targets authorize the subscription metadata requests.
pub(crate) fn kimi_region(endpoint: &str) -> Option<&'static str> {
    match endpoint {
        "https://api.kimi.com/coding/" | "https://api.kimi.com/coding" => Some("mainland-cn"),
        "https://api.kimi.ai/coding/" | "https://api.kimi.ai/coding" => Some("global"),
        _ => None,
    }
}

/// API-key users may configure the protocol's /v1 base explicitly. Quota
/// requests still use a fixed official service, never the supplied URL itself.
pub(crate) fn kimi_quota_region(endpoint: &str) -> Option<&'static str> {
    kimi_region(endpoint).or_else(|| {
        endpoint
            .trim_end_matches('/')
            .strip_suffix("/v1")
            .and_then(kimi_region)
    })
}

pub(crate) fn ark_endpoint(endpoint: &str, protocol: &Protocol) -> bool {
    endpoint.trim_end_matches('/')
        == match protocol {
            Protocol::Messages => "https://ark.cn-beijing.volces.com/api/coding",
            _ => "https://ark.cn-beijing.volces.com/api/coding/v3",
        }
}
