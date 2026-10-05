//! Resolver captures a registry snapshot; unknown IDs never fall back to a secret backend.
use crate::{Options, authentication_resources::*};
use std::{future::Future, pin::Pin, sync::Arc};
use velune_gateway::{
    CredentialResolutionError, CredentialResolver, CredentialTarget, ResolvedCredential,
};
pub(crate) struct RegistryResolver {
    manager: AuthenticationManager,
    options: Options,
}
impl RegistryResolver {
    pub(crate) fn capture(
        manager: &AuthenticationManager,
        options: &Options,
    ) -> Arc<dyn CredentialResolver> {
        Arc::new(Self {
            manager: manager.clone(),
            options: options.clone(),
        })
    }
}
impl CredentialResolver for RegistryResolver {
    fn resolve(
        &self,
        id: String,
        target: CredentialTarget,
    ) -> Pin<Box<dyn Future<Output = Result<ResolvedCredential, CredentialResolutionError>> + Send>>
    {
        let resource = self.manager.get(&id).cloned();
        let options = self.options.clone();
        Box::pin(async move {
            let resource = resource.map_err(|_| CredentialResolutionError::Unavailable)?;
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
            let (command, delegated) = match &resource.locator {
                AuthenticationLocator::Keychain { reference, .. } => {
                    let mut command = tokio::process::Command::new(
                        options
                            .credential_resolver
                            .ok_or(CredentialResolutionError::Unavailable)?,
                    );
                    command.arg(reference);
                    (command, false)
                }
                AuthenticationLocator::RuntimeProvider { source } => {
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
                    (command, true)
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
            if !delegated {
                if output.trim().is_empty() {
                    return Err(CredentialResolutionError::InvalidContract);
                }
                return Ok(ResolvedCredential {
                    token: output.trim().to_owned(),
                    explicit_output_cap: None,
                    subscription: false,
                });
            }
            let value: serde_json::Value = serde_json::from_str(&output)
                .map_err(|_| CredentialResolutionError::InvalidContract)?;
            let expected = match protocol {
                crate::config::GatewayProtocol::ChatCompletionsV1 => "chatCompletionsV1",
                crate::config::GatewayProtocol::ResponsesV1 => "responsesV1",
                crate::config::GatewayProtocol::MessagesV1 => {
                    return Err(CredentialResolutionError::InvalidContract);
                }
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
