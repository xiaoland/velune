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
            if resource.protocol != protocol
                || resource.endpoint != target.endpoint
                || resource.execution_backend != target.execution_backend
                || resource.request_profile != target.request_profile
            {
                return Err(CredentialResolutionError::UnauthorizedTarget);
            }
            let command = match &resource.authentication.material {
                AuthenticationMaterial::None => return Err(CredentialResolutionError::Unavailable),
                AuthenticationMaterial::ApiKey { value } => {
                    return Ok(ResolvedCredential {
                        material: velune_gateway::CredentialMaterial::Http {
                            authorization: http_authorization(&protocol, value.clone(), false),
                            account_id: None,
                        },
                        recovery: velune_gateway::CredentialRecovery::None,
                        request_capabilities: Default::default(),
                    });
                }
                AuthenticationMaterial::CodexSubscription { credential_id, .. } => {
                    if target.endpoint != crate::subscription_auth::ENDPOINT {
                        return Err(CredentialResolutionError::UnauthorizedTarget);
                    }
                    return crate::subscription_auth::resolve(
                        options.home_directory,
                        credential_id.clone(),
                        resource.authentication.generation,
                        None,
                    )
                    .await;
                }
                AuthenticationMaterial::ClaudeCli {
                    credential_id,
                    configured,
                    ..
                } => {
                    if !configured
                        || !matches!(
                            resource.execution_backend,
                            crate::config::ExecutionBackend::ClaudeCli { .. }
                        )
                    {
                        return Err(CredentialResolutionError::Unavailable);
                    }
                    return Ok(ResolvedCredential {
                        material: velune_gateway::CredentialMaterial::LocalSession {
                            config_dir: crate::claude_cli::directory(
                                &options.home_directory,
                                credential_id,
                            )
                            .to_string_lossy()
                            .into_owned(),
                        },
                        recovery: velune_gateway::CredentialRecovery::None,
                        request_capabilities: Default::default(),
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
                        .env_clear()
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
                    crate::credential_helper::CredentialError::Exit { .. } => {
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
                material: velune_gateway::CredentialMaterial::Http {
                    authorization: http_authorization(
                        &protocol,
                        token,
                        !matches!(
                            &resource.authentication.material,
                            AuthenticationMaterial::RuntimeProvider {
                                method: AuthenticationMethod::ApiKey,
                                ..
                            }
                        ),
                    ),
                    account_id: None,
                },
                recovery: velune_gateway::CredentialRecovery::None,
                request_capabilities: velune_gateway::RequestCapabilities {
                    explicit_output_cap: value["capabilities"]["explicitOutputCap"].as_bool(),
                },
            })
        })
    }
    fn refresh(
        &self,
        id: String,
        target: CredentialTarget,
        rejected_token: String,
    ) -> Pin<Box<dyn Future<Output = Result<ResolvedCredential, CredentialResolutionError>> + Send>>
    {
        let resource = self.providers.get(&id).cloned();
        let home = self.options.home_directory.clone();
        Box::pin(async move {
            let resource = resource.ok_or(CredentialResolutionError::Unavailable)?;
            if resource.execution_backend != target.execution_backend
                || resource.request_profile != target.request_profile
                || resource.endpoint != target.endpoint
                || !matches!(
                    target.protocol,
                    velune_gateway::GatewayProtocol::ResponsesV1
                )
            {
                return Err(CredentialResolutionError::UnauthorizedTarget);
            }
            let AuthenticationMaterial::CodexSubscription { credential_id, .. } =
                resource.authentication.material
            else {
                return Err(CredentialResolutionError::Unavailable);
            };
            crate::subscription_auth::resolve(
                home,
                credential_id,
                resource.authentication.generation,
                Some(rejected_token),
            )
            .await
        })
    }
    fn invalidate(
        &self,
        id: String,
        target: CredentialTarget,
        rejected_token: String,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let resource = self.providers.get(&id).cloned();
        let home = self.options.home_directory.clone();
        Box::pin(async move {
            if let Some(resource) = resource
                && resource.endpoint == target.endpoint
                && matches!(
                    target.protocol,
                    velune_gateway::GatewayProtocol::ResponsesV1
                )
                && let AuthenticationMaterial::CodexSubscription { credential_id, .. } =
                    resource.authentication.material
            {
                crate::subscription_auth::invalidate(
                    home,
                    credential_id,
                    resource.authentication.generation,
                    rejected_token,
                )
                .await;
            }
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
                routes: Vec::new(),
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
                        execution_backend: provider.execution_backend.clone(),
                        request_profile: provider.request_profile,
                    },
                ))
                .map_err(|_| {
                    crate::Error::invalid("无法读取此提供商的 API key；请检查来源配置。")
                })?;
            result
                .token()
                .map(str::to_owned)
                .ok_or_else(|| crate::Error::invalid("来源未返回 HTTP 认证"))
        }
        _ => Err(crate::Error::invalid("此提供商没有可编辑的 API key")),
    }
}

fn http_authorization(
    protocol: &crate::config::GatewayProtocol,
    value: String,
    bearer: bool,
) -> velune_gateway::HttpAuthorization {
    if !bearer && *protocol == crate::config::GatewayProtocol::MessagesV1 {
        velune_gateway::HttpAuthorization::ApiKey(value)
    } else {
        velune_gateway::HttpAuthorization::Bearer(value)
    }
}
