//! Templates a new provider starts from. A subscription fixes the protocol,
//! address and how requests run, so it is offered as a template rather than as
//! an authentication option on an arbitrary provider.
use crate::authentication::{
    CODEX_SUBSCRIPTION_ENDPOINT, CODEX_SUBSCRIPTION_PROTOCOL, ProviderAuthenticationSource,
    SubscriptionDisclosure,
};
use crate::config::{ExecutionBackend, GatewayProtocol, ProviderQuotaSource, RequestProfile};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProviderTemplateKind {
    /// Signs in to one subscription account; the composition stays fixed.
    Subscription,
    /// Reaches an API with a key; protocol and address remain editable.
    Api,
}

/// The whole composition a new provider saves, with what choosing it means.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTemplate {
    pub id: String,
    pub kind: ProviderTemplateKind,
    pub name: String,
    pub summary: String,
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    pub execution_backend: ExecutionBackend,
    pub request_profile: RequestProfile,
    pub authentication_source: ProviderAuthenticationSource,
    pub quota_source: ProviderQuotaSource,
    /// The execution machine's command, located by its platform or supplied by the user.
    /// Fill the CLI backend's empty binary with an absolute path before saving.
    pub executable_command: Option<String>,
    pub disclosure: Option<SubscriptionDisclosure>,
}

/// Subscriptions first, then the editable API provider.
pub fn provider_templates() -> Vec<ProviderTemplate> {
    vec![
        ProviderTemplate {
            id: "codexSubscription".into(),
            kind: ProviderTemplateKind::Subscription,
            name: "Codex 订阅".into(),
            summary: "登录一个 ChatGPT 账户，经 Codex 客户端协议使用其订阅额度。".into(),
            protocol: CODEX_SUBSCRIPTION_PROTOCOL,
            endpoint: CODEX_SUBSCRIPTION_ENDPOINT.into(),
            execution_backend: ExecutionBackend::Http,
            request_profile: RequestProfile::CodexResponses,
            authentication_source: ProviderAuthenticationSource::CodexOAuth,
            quota_source: ProviderQuotaSource::CodexUsage,
            executable_command: None,
            disclosure: SubscriptionDisclosure::for_source(ProviderAuthenticationSource::CodexOAuth),
        },
        ProviderTemplate {
            id: "claudeCodeSubscription".into(),
            kind: ProviderTemplateKind::Subscription,
            name: "Claude Code 订阅".into(),
            summary: "由本机官方 Claude Code CLI 登录一个 Claude 账户并执行请求。".into(),
            protocol: GatewayProtocol::MessagesV1,
            endpoint: String::new(),
            execution_backend: ExecutionBackend::ClaudeCli {
                binary: String::new(),
            },
            request_profile: RequestProfile::Native,
            authentication_source: ProviderAuthenticationSource::ClaudeCli,
            quota_source: ProviderQuotaSource::ClaudeCliUsage,
            executable_command: Some("claude".into()),
            disclosure: SubscriptionDisclosure::for_source(ProviderAuthenticationSource::ClaudeCli),
        },
        ProviderTemplate {
            id: "api".into(),
            kind: ProviderTemplateKind::Api,
            name: "API 提供商".into(),
            summary: "以 API key 访问兼容 OpenAI Chat Completions、Responses 或 Anthropic Messages 协议的服务。".into(),
            protocol: GatewayProtocol::ChatCompletionsV1,
            endpoint: String::new(),
            execution_backend: ExecutionBackend::Http,
            request_profile: RequestProfile::Native,
            authentication_source: ProviderAuthenticationSource::ApiKey,
            quota_source: ProviderQuotaSource::None,
            executable_command: None,
            disclosure: None,
        },
    ]
}
