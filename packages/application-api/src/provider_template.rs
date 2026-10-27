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

/// Subscriptions first, then the editable API provider and official API services.
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
            id: "kimiCodingPlan".into(), kind: ProviderTemplateKind::Subscription,
            name: "Kimi Coding Plan".into(),
            summary: "使用 Kimi Coding API key，按账户验证订阅归属并读取实际额度窗口。".into(),
            protocol: GatewayProtocol::MessagesV1, endpoint: "https://api.kimi.com/coding/".into(),
            execution_backend: ExecutionBackend::Http, request_profile: RequestProfile::KimiCoding,
            authentication_source: ProviderAuthenticationSource::ApiKey, quota_source: ProviderQuotaSource::KimiUsage,
            executable_command: None,
            disclosure: Some(SubscriptionDisclosure::kimi_coding_plan()),
        },
        ProviderTemplate {
            id: "arkCodingPlan".into(), kind: ProviderTemplateKind::Subscription,
            name: "ARK Coding Plan".into(), summary: "个人 Coding Plan，模型 API key 与额度查询认证分别配置。".into(),
            protocol: GatewayProtocol::ResponsesV1, endpoint: "https://ark.cn-beijing.volces.com/api/coding/v3".into(),
            execution_backend: ExecutionBackend::Http, request_profile: RequestProfile::ArkCoding,
            authentication_source: ProviderAuthenticationSource::ApiKey, quota_source: ProviderQuotaSource::ArkCodingUsage,
            executable_command: None, disclosure: Some(SubscriptionDisclosure::ark_coding_plan()),
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
        api_account_template(
            "deepseekApi",
            "DeepSeek API",
            "以 API key 访问 DeepSeek 开放平台，并读取账户余额。",
            "https://api.deepseek.com",
        ),
        api_account_template(
            "moonshotApi",
            "Moonshot API",
            "以 API key 访问 Moonshot（Kimi）开放平台，并读取账户可用余额；国际站地址为 api.moonshot.ai。",
            "https://api.moonshot.cn/v1",
        ),
        api_account_template(
            "openRouterApi",
            "OpenRouter",
            "以 API key 访问 OpenRouter，并读取这个 Key 的花费上限与剩余额度。",
            "https://openrouter.ai/api/v1",
        ),
    ]
}

/// An API provider at an official address whose account quota is read with its key.
fn api_account_template(id: &str, name: &str, summary: &str, endpoint: &str) -> ProviderTemplate {
    ProviderTemplate {
        id: id.into(),
        kind: ProviderTemplateKind::Api,
        name: name.into(),
        summary: summary.into(),
        protocol: GatewayProtocol::ChatCompletionsV1,
        endpoint: endpoint.into(),
        execution_backend: ExecutionBackend::Http,
        request_profile: RequestProfile::Native,
        authentication_source: ProviderAuthenticationSource::ApiKey,
        quota_source: ProviderQuotaSource::ApiAccount,
        executable_command: None,
        disclosure: None,
    }
}
