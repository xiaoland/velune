//! Templates a new provider starts from. A subscription fixes the protocol,
//! address and how requests run, so it is offered as a template rather than as
//! an authentication option on an arbitrary provider.
use crate::authentication::{
    CODEX_SUBSCRIPTION_ENDPOINT, CODEX_SUBSCRIPTION_PROTOCOL, ProviderAuthenticationSource,
    SubscriptionDisclosure,
};
use crate::config::{ExecutionBackend, Protocol, ProviderQuotaSource, RequestProfile};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ProviderTemplateKind {
    /// Uses one subscription account with its model-request composition.
    Subscription,
    /// Reaches an API with a key; protocol and address remain editable.
    Api,
}

/// The whole composition a new provider saves, with what choosing it means.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTemplate {
    pub id: String,
    pub kind: ProviderTemplateKind,
    pub name: String,
    pub summary: String,
    pub protocol: Protocol,
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
            disclosure: SubscriptionDisclosure::for_source(
                ProviderAuthenticationSource::CodexOAuth,
            ),
        },
        ProviderTemplate {
            id: "claudeCodeSubscription".into(),
            kind: ProviderTemplateKind::Subscription,
            name: "Claude Code 订阅".into(),
            summary: "由本机官方 Claude Code CLI 登录一个 Claude 账户并执行请求。".into(),
            protocol: Protocol::Messages,
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
            id: "antigravityCliSubscription".into(),
            kind: ProviderTemplateKind::Subscription,
            name: "Google Antigravity 订阅".into(),
            summary: "通过官方 agy 使用本机或指定 SSH 主机上已有的 Antigravity 登录。".into(),
            protocol: Protocol::Messages,
            endpoint: String::new(),
            execution_backend: ExecutionBackend::AntigravityCli {
                binary: String::new(),
                ssh_destination: None,
            },
            request_profile: RequestProfile::Native,
            authentication_source: ProviderAuthenticationSource::AntigravityCli,
            quota_source: ProviderQuotaSource::None,
            executable_command: Some("agy".into()),
            disclosure: SubscriptionDisclosure::for_source(
                ProviderAuthenticationSource::AntigravityCli,
            ),
        },
        ProviderTemplate {
            id: "kimiCodingPlan".into(),
            kind: ProviderTemplateKind::Subscription,
            name: "Kimi Coding Plan".into(),
            summary: "使用 Kimi Code 订阅的 API key 与额度，与 Moonshot 开放平台分开配置。".into(),
            protocol: Protocol::Messages,
            endpoint: "https://api.kimi.com/coding/".into(),
            execution_backend: ExecutionBackend::Http,
            request_profile: RequestProfile::KimiCoding,
            authentication_source: ProviderAuthenticationSource::ApiKey,
            quota_source: ProviderQuotaSource::KimiUsage,
            executable_command: None,
            disclosure: Some(SubscriptionDisclosure::kimi_coding_plan()),
        },
        ProviderTemplate {
            id: "arkCodingPlan".into(),
            kind: ProviderTemplateKind::Subscription,
            name: "ARK Coding Plan".into(),
            summary: "个人 Coding Plan，模型 API key 与额度查询认证分别配置。".into(),
            protocol: Protocol::Responses,
            endpoint: "https://ark.cn-beijing.volces.com/api/coding/v3".into(),
            execution_backend: ExecutionBackend::Http,
            request_profile: RequestProfile::ArkCoding,
            authentication_source: ProviderAuthenticationSource::ApiKey,
            quota_source: ProviderQuotaSource::ArkCodingUsage,
            executable_command: None,
            disclosure: Some(SubscriptionDisclosure::ark_coding_plan()),
        },
        api_template(
            "api",
            "API 提供商",
            "以 API key 访问兼容 OpenAI Chat Completions、Responses、Anthropic Messages 或 Gemini GenerateContent 协议的服务。",
            Protocol::ChatCompletions,
            "",
            ProviderQuotaSource::None,
        ),
        api_template(
            "openaiApi",
            "OpenAI Platform",
            "以 OpenAI Platform API key 访问官方 API，默认使用 Responses；与 Codex 订阅分开配置。",
            Protocol::Responses,
            "https://api.openai.com/v1",
            ProviderQuotaSource::None,
        ),
        api_template(
            "anthropicApi",
            "Anthropic API",
            "以 Claude Console API key 访问官方 Messages API，与 Claude Code 订阅分开配置。",
            Protocol::Messages,
            "https://api.anthropic.com",
            ProviderQuotaSource::None,
        ),
        api_template(
            "geminiApi",
            "Google Gemini API",
            "以 Google AI Studio 的 Gemini API key 调用原生 GenerateContent。",
            Protocol::Gemini,
            "https://generativelanguage.googleapis.com/v1beta",
            ProviderQuotaSource::None,
        ),
        api_template(
            "deepseekApi",
            "DeepSeek API",
            "以 API key 访问 DeepSeek 开放平台，并读取账户余额。",
            Protocol::ChatCompletions,
            "https://api.deepseek.com",
            ProviderQuotaSource::ApiAccount,
        ),
        api_template(
            "moonshotApi",
            "Moonshot API",
            "以开放平台 API key 按量计费并读取余额，与 Kimi Code 订阅分开配置；国际站地址为 api.moonshot.ai。",
            Protocol::ChatCompletions,
            "https://api.moonshot.cn/v1",
            ProviderQuotaSource::ApiAccount,
        ),
        api_template(
            "openRouterApi",
            "OpenRouter",
            "以 API key 访问 OpenRouter，并读取这个 Key 的花费上限与剩余额度。",
            Protocol::ChatCompletions,
            "https://openrouter.ai/api/v1",
            ProviderQuotaSource::ApiAccount,
        ),
    ]
}

/// API protocol and account metadata support are independent choices.
fn api_template(
    id: &str,
    name: &str,
    summary: &str,
    protocol: Protocol,
    endpoint: &str,
    quota_source: ProviderQuotaSource,
) -> ProviderTemplate {
    ProviderTemplate {
        id: id.into(),
        kind: ProviderTemplateKind::Api,
        name: name.into(),
        summary: summary.into(),
        protocol,
        endpoint: endpoint.into(),
        execution_backend: ExecutionBackend::Http,
        request_profile: RequestProfile::Native,
        authentication_source: ProviderAuthenticationSource::ApiKey,
        quota_source,
        executable_command: None,
        disclosure: None,
    }
}

/// Describe saved model-request metadata using the same catalog as creation.
/// Quota collection is independently editable and never changes this identity.
/// The template is presentation metadata, never a live configuration reference:
/// saved values remain authoritative and application validates their full composition.
/// Neither protocol, endpoint nor API key authentication alone identifies a subscription.
pub fn provider_subscription_template(
    execution_backend: &ExecutionBackend,
    request_profile: RequestProfile,
    authentication_source: ProviderAuthenticationSource,
    _quota_source: ProviderQuotaSource,
) -> Option<ProviderTemplate> {
    provider_templates().into_iter().find(|template| {
        template.kind == ProviderTemplateKind::Subscription
            && std::mem::discriminant(&template.execution_backend)
                == std::mem::discriminant(execution_backend)
            && template.request_profile == request_profile
            && template.authentication_source == authentication_source
    })
}

/// Quota collection is selected independently of the creation template and
/// model request profile. Saving validates the chosen service address/auth.
pub fn provider_quota_sources(
    execution_backend: ExecutionBackend,
    authentication_source: ProviderAuthenticationSource,
) -> Vec<ProviderQuotaSource> {
    use ProviderAuthenticationSource as Auth;
    use ProviderQuotaSource as Quota;
    match (execution_backend, authentication_source) {
        (ExecutionBackend::Http, Auth::ApiKey) => vec![
            Quota::None,
            Quota::KimiUsage,
            Quota::ArkCodingUsage,
            Quota::ApiAccount,
        ],
        (ExecutionBackend::Http, Auth::CodexOAuth) => vec![Quota::None, Quota::CodexUsage],
        (ExecutionBackend::ClaudeCli { .. }, Auth::ClaudeCli) => {
            vec![Quota::None, Quota::ClaudeCliUsage]
        }
        _ => vec![Quota::None],
    }
}

#[cfg(test)]
mod quota_composition_tests {
    use super::*;

    #[test]
    fn disabling_quota_does_not_turn_subscription_request_metadata_into_api_metadata() {
        for template in provider_templates()
            .into_iter()
            .filter(|t| t.kind == ProviderTemplateKind::Subscription)
        {
            let found = provider_subscription_template(
                &template.execution_backend,
                template.request_profile,
                template.authentication_source,
                ProviderQuotaSource::None,
            )
            .unwrap();
            assert_eq!(found.id, template.id);
            assert!(
                provider_quota_sources(template.execution_backend, template.authentication_source)
                    .contains(&template.quota_source)
            );
        }
    }
}
