//! Provider-owned private authentication and non-secret UI descriptions.
use crate::config::GatewayProtocol;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
pub struct ProviderAuthenticationDescription {
    pub method: AuthenticationMethod,
    pub configured: bool,
    pub provenance: Option<AuthenticationProvenance>,
    pub actions: Vec<crate::conversation::SettingAction>,
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
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum AuthenticationMaterial {
    #[default]
    None,
    ApiKey {
        value: String,
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
        }
    }
    #[cfg(feature = "local-runtime")]
    pub(crate) fn subscription(&self) -> bool {
        matches!(
            &self.material,
            AuthenticationMaterial::RuntimeProvider {
                method: AuthenticationMethod::OAuth,
                ..
            }
        )
    }
    pub(crate) fn validate_target(
        &self,
        protocol: &GatewayProtocol,
        endpoint: &str,
    ) -> Result<(), &'static str> {
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
