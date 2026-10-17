use crate::config::GatewayProtocol;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub use velune_application_api::authentication::*;
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
