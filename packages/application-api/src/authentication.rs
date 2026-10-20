//! Provider-owned authentication edits and non-secret UI descriptions.
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
    pub fn kimi_coding_plan() -> Self {
        Self { title: "Kimi Coding Plan 接入".into(), detail: "使用个人 Coding API key；同一账户的多个 key 共用额度。仅限个人交互式使用，不伪造客户端身份。加油包开启后，服务可能在订阅额度耗尽时继续扣充值余额；额度准入不保证账单硬上限。".into() }
    }
    pub fn ark_coding_plan() -> Self {
        Self { title: "ARK Coding Plan 接入".into(), detail: "仅用于个人交互式编程。模型 API key 与额度查询 AK/SK 分别配置；额度来源由用户确认关联，未自动核验同账户。一订阅只关联一个提供商。Coding 额度耗尽后不自动转为按量计费地址。".into() }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAuthenticationDescription {
    pub method: AuthenticationMethod,
    pub configured: bool,
    pub account_label: Option<String>,
    pub provenance: Option<AuthenticationProvenance>,
    pub actions: Vec<crate::conversation::SettingAction>,
    pub disclosure: Option<SubscriptionDisclosure>,
    /// The existing Claude Code login the provider references, if it does.
    pub linked_login: Option<LinkedClaudeLogin>,
}
/// An existing Claude Code login referenced by its owner, without credentials or local paths.
/// The official CLI keeps saving and renewing it; Velune never signs out this login.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum LinkedClaudeLogin {
    /// The execution machine user's own default login.
    UserDefault,
    /// The login of a local Claude Code runtime on the execution machine.
    Runtime { instance_id: String },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum AuthenticationEdit {
    Keep,
    SetApiKey {
        value: String,
    },
    Clear,
    /// Reference an existing Claude Code login instead of signing in anew.
    UseClaudeCodeLogin {
        login: LinkedClaudeLogin,
    },
}

/// Credentials used only for the provider's fixed quota query, never model requests.
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum QuotaAuthenticationEdit {
    SetAccessKey {
        access_key_id: String,
        secret_access_key: String,
        confirmed: bool,
    },
    ConfirmBinding,
    Clear,
}
impl std::fmt::Debug for QuotaAuthenticationEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("QuotaAuthenticationEdit([redacted])")
    }
}
/// Confirmation is a user assertion, not server verification of model-key ownership.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaAuthenticationDescription {
    pub configured: bool,
    pub binding_confirmed: bool,
}
