//! Resolver captures a registry snapshot; unknown IDs never fall back to a secret backend.
use crate::{Options, provider_authentication::*};
use std::{future::Future, pin::Pin, sync::Arc};
use velune_gateway::{
    CredentialResolutionError, CredentialResolver, CredentialTarget, ResolvedCredential,
};
pub(crate) struct ProviderResolver {
    providers: std::collections::BTreeMap<String, crate::config::ProviderDefinition>,
    options: Options,
}
impl ProviderResolver {
    pub(crate) fn capture(
        gateway: &crate::config::GatewayConfig,
        options: &Options,
    ) -> Arc<dyn CredentialResolver> {
        Arc::new(Self {
            providers: gateway
                .providers
                .iter()
                .map(|p| (p.id.clone(), p.clone()))
                .collect(),
            options: options.clone(),
        })
    }
}
impl CredentialResolver for ProviderResolver {
    fn resolve(
        &self,
        id: String,
        target: CredentialTarget,
    ) -> Pin<Box<dyn Future<Output = Result<ResolvedCredential, CredentialResolutionError>> + Send>>
    {
        let resource = self.providers.get(&id).cloned();
        let options = self.options.clone();
        Box::pin(async move {
            let resource = resource.ok_or(CredentialResolutionError::Unavailable)?;
            let protocol = match target.protocol {
                velune_gateway::GatewayProtocol::ChatCompletionsV1 => {
                    crate::config::GatewayProtocol::ChatCompletionsV1
                }
                velune_gateway::GatewayProtocol::ResponsesV1 => {
                    crate::config::GatewayProtocol::ResponsesV1
                }
                velune_gateway::GatewayProtocol::MessagesV1 => {
                    crate::config::GatewayProtocol::MessagesV1
                }
            };
            if resource.protocol != protocol || resource.endpoint != target.endpoint {
                return Err(CredentialResolutionError::UnauthorizedTarget);
            }
            let command = match &resource.authentication.material {
                AuthenticationMaterial::None => return Err(CredentialResolutionError::Unavailable),
                AuthenticationMaterial::ApiKey { value } => {
                    return Ok(ResolvedCredential {
                        token: value.clone(),
                        explicit_output_cap: None,
                        subscription: false,
                    });
                }
                AuthenticationMaterial::RuntimeProvider { source, .. } => {
                    let node = source
                        .settings
                        .get("nodeBinary")
                        .ok_or(CredentialResolutionError::Unavailable)?;
                    if !std::path::Path::new(node).is_absolute() {
                        return Err(CredentialResolutionError::InvalidContract);
                    }
                    let helper = if source.settings.contains_key("bindingProtocol") {
                        "pi_provider_import.mjs"
                    } else {
                        "pi_auth.mjs"
                    };
                    let mut command = tokio::process::Command::new(node);
                    command
                        .arg(options.resources_directory.join(helper))
                        .arg("--operation")
                        .arg("resolve")
                        .arg("--source-json")
                        .arg(
                            serde_json::to_string(source)
                                .map_err(|_| CredentialResolutionError::InvalidContract)?,
                        );
                    command
                }
            };
            let output = crate::credential_helper::execute(command)
                .await
                .map_err(|error| match error {
                    crate::credential_helper::CredentialError::Unavailable => {
                        CredentialResolutionError::Unavailable
                    }
                    #[cfg(unix)]
                    crate::credential_helper::CredentialError::Timeout => {
                        CredentialResolutionError::Timeout
                    }
                    #[cfg(unix)]
                    crate::credential_helper::CredentialError::InvalidOutput => {
                        CredentialResolutionError::InvalidContract
                    }
                })?;
            let value: serde_json::Value = serde_json::from_str(&output)
                .map_err(|_| CredentialResolutionError::InvalidContract)?;
            let expected = match protocol {
                crate::config::GatewayProtocol::ChatCompletionsV1 => "chatCompletionsV1",
                crate::config::GatewayProtocol::ResponsesV1 => "responsesV1",
                crate::config::GatewayProtocol::MessagesV1 => "messagesV1",
            };
            if value["contractVersion"] != 1
                || value["capabilities"]["protocol"] != expected
                || value["capabilities"]["endpoint"] != target.endpoint
            {
                return Err(CredentialResolutionError::StaleBinding);
            }
            let token = value["bearer"]
                .as_str()
                .filter(|token| !token.trim().is_empty())
                .ok_or(CredentialResolutionError::InvalidContract)?
                .to_owned();
            Ok(ResolvedCredential {
                token,
                explicit_output_cap: value["capabilities"]["explicitOutputCap"].as_bool(),
                subscription: value["capabilities"]["authentication"] == "subscription",
            })
        })
    }
}

pub(crate) fn read_api_key(
    provider: &crate::config::ProviderDefinition,
    options: &Options,
) -> Result<String, crate::Error> {
    use crate::provider_authentication::{AuthenticationMaterial, AuthenticationMethod};
    match &provider.authentication.material {
        AuthenticationMaterial::ApiKey { value } => Ok(value.clone()),
        AuthenticationMaterial::RuntimeProvider {
            method: AuthenticationMethod::ApiKey,
            ..
        } => {
            let gateway = crate::config::GatewayConfig {
                id: "lookup".into(),
                name: "lookup".into(),
                providers: vec![provider.clone()],
                failover: crate::config::FailoverPolicy {
                    mode: crate::config::FailoverMode::Disabled,
                },
            };
            let protocol = match provider.protocol {
                crate::config::GatewayProtocol::ChatCompletionsV1 => {
                    velune_gateway::GatewayProtocol::ChatCompletionsV1
                }
                crate::config::GatewayProtocol::ResponsesV1 => {
                    velune_gateway::GatewayProtocol::ResponsesV1
                }
                crate::config::GatewayProtocol::MessagesV1 => {
                    velune_gateway::GatewayProtocol::MessagesV1
                }
            };
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let result = runtime
                .block_on(ProviderResolver::capture(&gateway, options).resolve(
                    provider.id.clone(),
                    CredentialTarget {
                        protocol,
                        endpoint: provider.endpoint.clone(),
                    },
                ))
                .map_err(|_| {
                    crate::Error::invalid("无法读取此提供商的 API key；请检查来源配置。")
                })?;
            Ok(result.token)
        }
        _ => Err(crate::Error::invalid("此提供商没有可编辑的 API key")),
    }
}
