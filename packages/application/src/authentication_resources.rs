//! Central application authentication registry. Providers select IDs; locators stay here.
use crate::{
    Error,
    config::{GatewayConfig, GatewayProtocol},
};
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
    pub runtime_instance_id: Option<String>,
    pub runtime_type_id: String,
    pub source_provider_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationBinding {
    pub id: String,
    pub name: String,
    pub method: AuthenticationMethod,
    pub configured: bool,
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    pub provenance: Option<AuthenticationProvenance>,
    pub actions: Vec<crate::conversation::SettingAction>,
    pub owns_secret: bool,
    pub generation: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthenticationSource {
    pub kind: String,
    pub harness_type_id: String,
    #[serde(default)]
    pub source_instance_id: Option<String>,
    pub provider_id: String,
    pub settings: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum AuthenticationLocator {
    Keychain {
        reference: String,
        owns_secret: bool,
    },
    RuntimeProvider {
        source: AuthenticationSource,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AuthenticationResource {
    pub id: String,
    pub name: String,
    pub method: AuthenticationMethod,
    pub configured: bool,
    pub protocol: GatewayProtocol,
    pub endpoint: String,
    pub generation: u64,
    pub locator: AuthenticationLocator,
}
impl AuthenticationResource {
    pub(crate) fn descriptor(&self) -> AuthenticationBinding {
        let provenance = match &self.locator {
            AuthenticationLocator::RuntimeProvider { source } => Some(AuthenticationProvenance {
                display_name: format!("{} · {}", source.harness_type_id, source.provider_id),
                runtime_instance_id: source.source_instance_id.clone(),
                runtime_type_id: source.harness_type_id.clone(),
                source_provider_id: source.provider_id.clone(),
            }),
            AuthenticationLocator::Keychain { .. } => None,
        };
        let actions = match &self.locator {
            AuthenticationLocator::RuntimeProvider { source }
                if source.provider_id == "openai"
                    && self.method != AuthenticationMethod::ApiKey
                    && source
                        .settings
                        .get("credentialLocation")
                        .map(String::as_str)
                        != Some("models")
                    && self.protocol == GatewayProtocol::ResponsesV1
                    && self.endpoint == "https://api.openai.com/v1" =>
            {
                vec![crate::conversation::SettingAction {
                    id: "login".into(),
                    label: "登录…".into(),
                }]
            }
            _ => Vec::new(),
        };
        AuthenticationBinding {
            id: self.id.clone(),
            name: self.name.clone(),
            method: self.method.clone(),
            configured: self.configured,
            protocol: self.protocol.clone(),
            endpoint: self.endpoint.clone(),
            provenance,
            actions,
            generation: self.generation,
            owns_secret: matches!(
                self.locator,
                AuthenticationLocator::Keychain {
                    owns_secret: true,
                    ..
                }
            ),
        }
    }
}
#[derive(Debug, Clone, Default)]
pub(crate) struct AuthenticationManager {
    pub(crate) resources: Vec<AuthenticationResource>,
}
impl AuthenticationManager {
    pub(crate) fn get(&self, id: &str) -> Result<&AuthenticationResource, Error> {
        self.resources
            .iter()
            .find(|resource| resource.id == id)
            .ok_or_else(|| Error::invalid("认证资源不存在；请先在认证设置中登记，再选择它。"))
    }
    pub(crate) fn descriptors(&self) -> Vec<AuthenticationBinding> {
        self.resources
            .iter()
            .map(AuthenticationResource::descriptor)
            .collect()
    }
    pub(crate) fn validate(&self, gateways: &[GatewayConfig]) -> Result<(), Error> {
        let mut ids = std::collections::BTreeSet::new();
        for resource in &self.resources {
            let endpoint = url::Url::parse(&resource.endpoint)
                .map_err(|_| Error::invalid("认证资源服务地址无效。"))?;
            if !matches!(endpoint.scheme(), "http" | "https")
                || endpoint.host_str().is_none()
                || !endpoint.username().is_empty()
                || endpoint.password().is_some()
                || endpoint.query().is_some()
                || endpoint.fragment().is_some()
                || resource.protocol == GatewayProtocol::MessagesV1
            {
                return Err(Error::invalid(
                    "认证资源只支持有效的 OpenAI Chat Completions 或 Responses HTTP 服务地址。",
                ));
            }
            if resource.id.is_empty()
                || resource.name.is_empty()
                || resource.endpoint.is_empty()
                || !ids.insert(&resource.id)
            {
                return Err(Error::invalid("invalid authentication registry"));
            }
            match &resource.locator {
                AuthenticationLocator::Keychain { reference, .. } if reference.is_empty() => {
                    return Err(Error::invalid("invalid registered secret reference"));
                }
                AuthenticationLocator::RuntimeProvider { source }
                    if source.kind != "harness"
                        || source.harness_type_id != "pi"
                        || source.provider_id.is_empty() =>
                {
                    return Err(Error::invalid("invalid delegated authentication resource"));
                }
                _ => {}
            }
        }
        for gateway in gateways {
            gateway.validate().map_err(Error::invalid)?;
            for provider in &gateway.providers {
                if let Some(id) = &provider.authentication_id {
                    let resource = self.get(id)?;
                    if resource.protocol != provider.protocol
                        || resource.endpoint != provider.endpoint
                    {
                        return Err(Error::invalid(
                            "认证资源未授权此协议或服务地址；请登记匹配的认证资源。",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
    #[cfg(feature = "local-runtime")]
    pub(crate) fn revision(&self, gateway: &GatewayConfig, model: &str) -> u64 {
        gateway
            .validate_dispatch(model)
            .ok()
            .and_then(|provider| provider.authentication_id.as_deref())
            .and_then(|id| self.get(id).ok())
            .map_or(0, |resource| resource.generation)
    }
    #[cfg(feature = "local-runtime")]
    pub(crate) fn subscription(&self, gateway: &GatewayConfig, model: &str) -> bool {
        gateway
            .validate_dispatch(model)
            .ok()
            .and_then(|provider| provider.authentication_id.as_deref())
            .and_then(|id| self.get(id).ok())
            .is_some_and(|resource| resource.method == AuthenticationMethod::OAuth)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationMutation {
    pub binding: AuthenticationBinding,
    pub obsolete_owned_keychain_refs: Vec<String>,
    pub requires_reconnect: bool,
    pub warnings: Vec<String>,
}

pub(crate) struct PendingAuthenticationMutation {
    pub(crate) manager: AuthenticationManager,
    pub(crate) result: AuthenticationMutation,
}
impl AuthenticationManager {
    pub(crate) fn prepare_mutation(
        &self,
        payload: &serde_json::Value,
        gateways: &[GatewayConfig],
    ) -> Result<PendingAuthenticationMutation, Error> {
        let id = payload["id"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Error::invalid("authentication resource id"))?;
        let previous = self.clone();
        let index = previous.resources.iter().position(|item| item.id == id);
        let mut next = previous.clone();
        let mut obsolete = Vec::new();
        let binding = match payload["operation"].as_str() {
            Some("configureApiKey") => {
                let expected = payload["expectedGeneration"].as_u64();
                if index.map(|index| previous.resources[index].generation) != expected {
                    return Err(Error::invalid("认证资源已变更，请刷新后再替换。"));
                }
                let name = payload["name"]
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| Error::invalid("authentication resource name"))?;
                let reference = payload["keychainRef"]
                    .as_str()
                    .filter(|value| !value.is_empty() && !value.contains('\0'))
                    .ok_or_else(|| Error::invalid("registered secret reference"))?;
                let protocol: GatewayProtocol =
                    serde_json::from_value(payload["protocol"].clone())?;
                let endpoint = payload["endpoint"]
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| Error::invalid("authentication target endpoint"))?;
                let generation = if let Some(index) = index {
                    let old = &previous.resources[index];
                    if let AuthenticationLocator::Keychain {
                        reference: old_reference,
                        owns_secret: true,
                    } = &old.locator
                        && old_reference != reference
                    {
                        obsolete.push(old_reference.clone());
                    }
                    old.generation
                        .checked_add(1)
                        .ok_or_else(|| Error::invalid("authentication revision overflow"))?
                } else {
                    0
                };
                let resource = AuthenticationResource {
                    id: id.into(),
                    name: name.into(),
                    method: AuthenticationMethod::ApiKey,
                    configured: true,
                    protocol,
                    endpoint: endpoint.into(),
                    generation,
                    locator: AuthenticationLocator::Keychain {
                        reference: reference.into(),
                        owns_secret: payload["ownsSecret"].as_bool().unwrap_or(false),
                    },
                };
                let binding = resource.descriptor();
                next.resources.retain(|item| item.id != id);
                next.resources.push(resource);
                binding
            }
            Some("rename") => {
                let resource = next
                    .resources
                    .iter_mut()
                    .find(|item| item.id == id)
                    .ok_or_else(|| Error::invalid("authentication resource not found"))?;
                resource.name = payload["name"]
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| Error::invalid("authentication resource name"))?
                    .into();
                resource.descriptor()
            }
            Some("delete") => {
                if gateways
                    .iter()
                    .flat_map(|gateway| &gateway.providers)
                    .any(|provider| provider.authentication_id.as_deref() == Some(id))
                {
                    return Err(Error::invalid(
                        "认证资源仍被提供商引用；请先更换或移除提供商的认证选择。",
                    ));
                }
                let resource = next.get(id)?.clone();
                if let AuthenticationLocator::Keychain {
                    reference,
                    owns_secret: true,
                } = &resource.locator
                {
                    obsolete.push(reference.clone());
                }
                next.resources.retain(|item| item.id != id);
                resource.descriptor()
            }
            _ => return Err(Error::invalid("authentication resource operation")),
        };
        next.validate(gateways)?;
        obsolete.retain(|reference| !next.resources.iter().any(|resource| matches!(&resource.locator, AuthenticationLocator::Keychain {reference:live,..} if live == reference)));
        Ok(PendingAuthenticationMutation {
            manager: next,
            result: AuthenticationMutation {
                binding,
                obsolete_owned_keychain_refs: obsolete,
                requires_reconnect: false,
                warnings: Vec::new(),
            },
        })
    }
}
