//! Provider-owned private authentication and non-secret UI descriptions.
use crate::config::GatewayProtocol;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AuthenticationSource {
    pub kind: String,
    pub harness_type_id: String,
    pub source_instance_id: Option<String>,
    pub provider_id: String,
    pub settings: BTreeMap<String, String>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum AuthenticationMaterial {
    #[default]
    None,
    ApiKey {
        value: String,
    },
    CodexSubscription {
        credential_id: String,
        configured: bool,
    },
    ClaudeCli {
        credential_id: String,
        configured: bool,
        account_label: Option<String>,
        identity: Option<String>,
    },
    RuntimeProvider {
        source: AuthenticationSource,
        method: AuthenticationMethod,
        configured: bool,
        protocol: GatewayProtocol,
        endpoint: String,
    },
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderAuthentication {
    pub material: AuthenticationMaterial,
    pub generation: u64,
}
impl std::fmt::Debug for ProviderAuthentication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderAuthentication")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl ProviderAuthentication {
    pub(crate) fn source(&self) -> ProviderAuthenticationSource {
        match self.material {
            AuthenticationMaterial::None => ProviderAuthenticationSource::None,
            AuthenticationMaterial::ApiKey { .. } => ProviderAuthenticationSource::ApiKey,
            AuthenticationMaterial::CodexSubscription { .. } => {
                ProviderAuthenticationSource::CodexOAuth
            }
            AuthenticationMaterial::RuntimeProvider { .. } => {
                ProviderAuthenticationSource::RuntimeProvider
            }
            AuthenticationMaterial::ClaudeCli { .. } => ProviderAuthenticationSource::ClaudeCli,
        }
    }
    pub(crate) fn description(&self) -> ProviderAuthenticationDescription {
        let (method, configured, provenance, actions) = match &self.material {
            AuthenticationMaterial::None => {
                (AuthenticationMethod::Unconfigured, false, None, Vec::new())
            }
            AuthenticationMaterial::ApiKey { value } => (
                AuthenticationMethod::ApiKey,
                !value.is_empty(),
                None,
                Vec::new(),
            ),
            AuthenticationMaterial::CodexSubscription { configured, .. } => (
                AuthenticationMethod::OAuth,
                *configured,
                Some(AuthenticationProvenance {
                    display_name: "Velune · Codex 订阅".into(),
                }),
                vec![crate::conversation::SettingAction {
                    id: "login".into(),
                    label: if *configured {
                        "重新登录…".into()
                    } else {
                        "登录…".into()
                    },
                }],
            ),
            AuthenticationMaterial::ClaudeCli { configured, .. } => (
                AuthenticationMethod::OAuth,
                *configured,
                Some(AuthenticationProvenance {
                    display_name: "Claude Code CLI".into(),
                }),
                vec![
                    crate::conversation::SettingAction {
                        id: "login".into(),
                        label: "登录…".into(),
                    },
                    crate::conversation::SettingAction {
                        id: "logout".into(),
                        label: "退出登录".into(),
                    },
                ],
            ),
            AuthenticationMaterial::RuntimeProvider {
                source,
                method,
                configured,
                protocol,
                endpoint,
            } => {
                let login = source.provider_id == "openai"
                    && method != &AuthenticationMethod::ApiKey
                    && source
                        .settings
                        .get("credentialLocation")
                        .map(String::as_str)
                        != Some("models")
                    && protocol == &GatewayProtocol::ResponsesV1
                    && endpoint == "https://api.openai.com/v1";
                (
                    method.clone(),
                    *configured,
                    Some(AuthenticationProvenance {
                        display_name: format!(
                            "{} · {}",
                            source.harness_type_id, source.provider_id
                        ),
                    }),
                    if login {
                        vec![crate::conversation::SettingAction {
                            id: "login".into(),
                            label: "登录…".into(),
                        }]
                    } else {
                        Vec::new()
                    },
                )
            }
        };
        ProviderAuthenticationDescription {
            method,
            configured,
            provenance,
            actions,
            account_label: match &self.material {
                AuthenticationMaterial::ClaudeCli { account_label, .. } => account_label.clone(),
                _ => None,
            },
            disclosure: SubscriptionDisclosure::for_source(self.source()),
        }
    }
    pub(crate) fn validate_target(
        &self,
        protocol: &GatewayProtocol,
        endpoint: &str,
    ) -> Result<(), &'static str> {
        if matches!(
            &self.material,
            AuthenticationMaterial::CodexSubscription { .. }
        ) && (protocol != &CODEX_SUBSCRIPTION_PROTOCOL
            || endpoint != CODEX_SUBSCRIPTION_ENDPOINT)
        {
            return Err("Codex 订阅认证仅用于原生订阅 Responses 地址；修改目标前请更换认证。");
        }
        if let AuthenticationMaterial::RuntimeProvider {
            protocol: authorized,
            endpoint: target,
            ..
        } = &self.material
            && (authorized != protocol || target != endpoint)
        {
            return Err("来源认证仅授权原协议与地址；修改目标时请明确换用 API key 或清除认证。");
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum AuthenticationEdit {
    Keep,
    SetApiKey { value: String },
    Clear,
}
