//! Provider-owned private authentication and non-secret UI descriptions.
use crate::config::GatewayProtocol;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProviderAuthenticationSource {
    None,
    ApiKey,
    CodexOAuth,
    RuntimeProvider,
    ClaudeCli,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AuthenticationMethod {
    ApiKey,
    #[serde(rename = "oauth")]
    OAuth,
    Unconfigured,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationProvenance {
    pub display_name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionDisclosure {
    pub title: String,
    pub detail: String,
}
/// The protocol and endpoint a provider with Codex subscription authentication must use.
pub const CODEX_SUBSCRIPTION_PROTOCOL: GatewayProtocol = GatewayProtocol::ResponsesV1;
pub const CODEX_SUBSCRIPTION_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex";
impl SubscriptionDisclosure {
    /// What choosing Codex subscription authentication means, shown before and after choosing it.
    pub fn codex_subscription() -> Self {
        Self { title: "Codex 订阅协议接入".into(), detail: "当前借用 Codex 客户端协议，非正式 SIWC 接入。多订阅自动轮换与用量池化和 SIWC 禁止规则冲突，不代表供应商认可。使用者应自行决定，并承担账户受限或接入失效的风险。".into() }
    }
    pub fn claude_cli() -> Self {
        Self { title: "Claude Code CLI 订阅接入".into(), detail: "使用未修改的官方 Claude Code CLI 登录和执行，由 CLI 保存并刷新认证。订阅用量与供应商规则仍适用；此组合及自动路由不代表供应商认可，不承诺额度无限或跨工具任意使用。".into() }
    }
    /// What choosing a subscription source means; other sources have nothing to disclose.
    pub fn for_source(source: ProviderAuthenticationSource) -> Option<Self> {
        match source {
            ProviderAuthenticationSource::CodexOAuth => Some(Self::codex_subscription()),
            ProviderAuthenticationSource::ClaudeCli => Some(Self::claude_cli()),
            ProviderAuthenticationSource::None
            | ProviderAuthenticationSource::ApiKey
            | ProviderAuthenticationSource::RuntimeProvider => None,
        }
    }
}
/// The whole composition a provider saves to use a subscription, and what
/// choosing it means, for presenting before the provider exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionTarget {
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    pub execution_backend: crate::config::ExecutionBackend,
    pub request_profile: crate::config::RequestProfile,
    pub authentication_source: ProviderAuthenticationSource,
    pub quota_source: crate::config::ProviderQuotaSource,
    pub disclosure: SubscriptionDisclosure,
}
impl SubscriptionTarget {
    pub fn codex() -> Self {
        Self {
            protocol: CODEX_SUBSCRIPTION_PROTOCOL,
            endpoint: CODEX_SUBSCRIPTION_ENDPOINT.into(),
            execution_backend: crate::config::ExecutionBackend::Http,
            request_profile: crate::config::RequestProfile::CodexResponses,
            authentication_source: ProviderAuthenticationSource::CodexOAuth,
            quota_source: crate::config::ProviderQuotaSource::CodexUsage,
            disclosure: SubscriptionDisclosure::codex_subscription(),
        }
    }
    /// Claude Code runs as the official CLI at `binary`; saving checks the path is absolute.
    pub fn claude_cli(binary: String) -> Self {
        Self {
            protocol: GatewayProtocol::MessagesV1,
            endpoint: String::new(),
            execution_backend: crate::config::ExecutionBackend::ClaudeCli { binary },
            request_profile: crate::config::RequestProfile::Native,
            authentication_source: ProviderAuthenticationSource::ClaudeCli,
            quota_source: crate::config::ProviderQuotaSource::ClaudeCliUsage,
            disclosure: SubscriptionDisclosure::claude_cli(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAuthenticationDescription {
    pub method: AuthenticationMethod,
    pub configured: bool,
    pub account_label: Option<String>,
    pub provenance: Option<AuthenticationProvenance>,
    pub actions: Vec<crate::conversation::SettingAction>,
    pub disclosure: Option<SubscriptionDisclosure>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum AuthenticationEdit {
    Keep,
    SetApiKey { value: String },
    Clear,
}
