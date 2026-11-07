//! Provider-owned configuration operations with one atomic commit.
use super::*;
use crate::provider_configuration::ProviderEdit;
impl CoreRuntime {
    /// A managed session can route to any model in its captured gateway,
    /// including models used by native sub-agents. Native-only sessions do not use it.
    pub(super) fn session_uses_gateway(&self, gateway_id: &str) -> bool {
        self.session.gateway_runner.is_some()
            && self
                .session
                .execution_configuration
                .as_ref()
                .and_then(|configuration| configuration.gateway.as_ref())
                .is_some_and(|gateway| gateway.id == gateway_id)
    }

    pub(super) fn provider_in_use(&self, gateway_id: &str, provider_id: &str) -> bool {
        std::iter::once(&self.session)
            .chain(&self.parked)
            .any(|session| {
                session.executing()
                    && session
                        .execution_configuration
                        .as_ref()
                        .and_then(|configuration| configuration.gateway.as_ref())
                        .is_some_and(|gateway| {
                            gateway.id == gateway_id
                                && gateway
                                    .providers
                                    .iter()
                                    .any(|provider| provider.id == provider_id)
                        })
            })
    }

    /// Owned credentials outlive every execution that captured them.
    pub(super) fn cleanup_retired_provider(&self, provider: &crate::config::ProviderDefinition) {
        use crate::provider_authentication::AuthenticationMaterial;
        let material = &provider.authentication.material;
        let referenced = self
            .gateways
            .iter()
            .flat_map(|gateway| &gateway.providers)
            .any(
                |current| match (material, &current.authentication.material) {
                    (
                        AuthenticationMaterial::CodexSubscription {
                            credential_id: old, ..
                        },
                        AuthenticationMaterial::CodexSubscription {
                            credential_id: current,
                            ..
                        },
                    )
                    | (
                        AuthenticationMaterial::ClaudeCli {
                            credential_id: old, ..
                        },
                        AuthenticationMaterial::ClaudeCli {
                            credential_id: current,
                            ..
                        },
                    ) => old == current,
                    _ => false,
                },
            );
        if referenced {
            return;
        }
        let result = match material {
            AuthenticationMaterial::CodexSubscription { credential_id, .. } => {
                crate::subscription_auth::remove(&self.options.home_directory, credential_id)
            }
            AuthenticationMaterial::ClaudeCli {
                credential_id,
                configured,
                ..
            } => {
                if *configured
                    && let crate::config::ExecutionBackend::ClaudeCli { binary } =
                        &provider.execution_backend
                {
                    let result = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(RuntimeError::from)
                        .and_then(|runtime| {
                            runtime
                                .block_on(crate::claude_cli::logout(
                                    binary,
                                    &crate::claude_cli::directory(
                                        &self.options.home_directory,
                                        credential_id,
                                    ),
                                ))
                                .map_err(|()| RuntimeError::invalid("旧订阅认证清理失败"))
                        });
                    if let Err(error) = result {
                        tracing::warn!(event = "subscription_cleanup_failed", detail = %error);
                    }
                }
                crate::claude_cli::remove(&self.options.home_directory, credential_id)
                    .map_err(RuntimeError::from)
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            tracing::warn!(event = "subscription_cleanup_failed", detail = %error);
        }
    }

    pub(super) fn invalidate_gateway_execution(
        &mut self,
        gateway_id: &str,
    ) -> Result<bool, RuntimeError> {
        let mut invalidated = false;
        self.for_each_session(|core| {
            if core.session_uses_gateway(gateway_id) {
                if core.session.executing() {
                    core.session.configuration_changed = true;
                } else {
                    core.invalidate_execution()?;
                    invalidated = true;
                }
            }
            Ok(())
        })?;
        Ok(invalidated)
    }

    pub(crate) fn provider_billing(
        &self,
        gateway_id: String,
        provider_id: String,
        refresh: bool,
    ) -> Result<crate::ProviderBillingStatus, RuntimeError> {
        let provider = self
            .gateways
            .iter()
            .find(|g| g.id == gateway_id)
            .and_then(|g| g.providers.iter().find(|p| p.id == provider_id))
            .ok_or_else(|| RuntimeError::invalid("提供商不存在"))?;
        Ok(self.billing.query(&gateway_id, provider, refresh))
    }
    pub(crate) fn save_provider_billing(
        &mut self,
        gateway_id: String,
        provider_id: String,
        configuration: Option<crate::BillingConfiguration>,
        authentication_edit: crate::BillingAuthenticationEdit,
        confirmed: bool,
    ) -> Result<crate::ProviderBillingStatus, RuntimeError> {
        let gateway_index = self
            .gateways
            .iter()
            .position(|g| g.id == gateway_id)
            .ok_or_else(|| RuntimeError::invalid("网关不存在"))?;
        let provider_index = self.gateways[gateway_index]
            .providers
            .iter()
            .position(|p| p.id == provider_id)
            .ok_or_else(|| RuntimeError::invalid("提供商不存在"))?;
        let provider = &self.gateways[gateway_index].providers[provider_index];
        let previous = provider.billing.clone();
        let mut pending = previous.clone();
        pending.apply(provider, configuration, authentication_edit, confirmed)?;
        self.gateways[gateway_index].providers[provider_index].billing = pending;
        if let Err(error) = self.persist() {
            self.gateways[gateway_index].providers[provider_index].billing = previous;
            return Err(error);
        }
        self.billing.configure(&self.gateways);
        self.provider_billing(gateway_id, provider_id, false)
    }
    pub(crate) fn provider_quota(
        &self,
        gateway_id: String,
        provider_id: String,
        refresh: bool,
    ) -> Result<crate::ProviderQuotaStatus, RuntimeError> {
        self.subscription_quota
            .query(&gateway_id, &provider_id, refresh)
    }

    pub(crate) fn quota_alerts(&self, since: u64) -> Result<crate::QuotaAlerts, RuntimeError> {
        Ok(self.subscription_quota.alerts(since))
    }

    pub(crate) fn provider_quota_history(
        &self,
        gateway_id: String,
        provider_id: String,
        since_ms: i64,
    ) -> Result<crate::ProviderQuotaHistory, RuntimeError> {
        self.subscription_quota
            .history(&gateway_id, &provider_id, since_ms)
    }

    pub(crate) fn clear_provider_quota_history(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<crate::ProviderQuotaHistory, RuntimeError> {
        self.subscription_quota
            .clear_history(&gateway_id, &provider_id)
    }

    pub(crate) fn read_provider_api_key(
        &self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<String, RuntimeError> {
        let provider = self
            .gateways
            .iter()
            .find(|g| g.id == gateway_id)
            .ok_or_else(|| RuntimeError::invalid("gateway id"))?
            .providers
            .iter()
            .find(|p| p.id == provider_id)
            .ok_or_else(|| RuntimeError::invalid("provider id"))?;
        crate::authentication_resolver::read_api_key(provider, &self.options)
    }

    pub(crate) fn save_provider(
        &mut self,
        gateway_id: String,
        provider: crate::ProviderDraft,
        authentication: crate::AuthenticationEdit,
        quota_authentication: Option<crate::QuotaAuthenticationEdit>,
    ) -> Result<crate::GatewayUpdate, RuntimeError> {
        self.edit_gateway(
            &gateway_id,
            ProviderEdit::Save {
                provider,
                authentication,
                quota_authentication,
            },
        )
    }

    pub(crate) fn delete_provider(
        &mut self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<crate::GatewayUpdate, RuntimeError> {
        self.edit_gateway(&gateway_id, ProviderEdit::Delete { provider_id })
    }

    pub(crate) fn save_model_route(
        &mut self,
        gateway_id: String,
        route: crate::config::ModelRoute,
    ) -> Result<crate::GatewayUpdate, RuntimeError> {
        self.edit_gateway(&gateway_id, ProviderEdit::SaveRoute { route })
    }

    pub(crate) fn delete_model_route(
        &mut self,
        gateway_id: String,
        route_record_key: String,
    ) -> Result<crate::GatewayUpdate, RuntimeError> {
        self.edit_gateway(&gateway_id, ProviderEdit::DeleteRoute { route_record_key })
    }

    fn edit_gateway(
        &mut self,
        gateway_id: &str,
        edit: ProviderEdit,
    ) -> Result<crate::GatewayUpdate, RuntimeError> {
        if self
            .authentication
            .as_ref()
            .is_some_and(authentication::Login::is_running)
            || self
                .subscription_login
                .as_ref()
                .is_some_and(crate::provider_login::Login::is_running)
        {
            return Err(RuntimeError::invalid(
                "请先完成登录，再修改提供商或模型配置",
            ));
        }
        let previous = self.gateways.clone();
        let previous_runtimes = self.runtime_instances.clone();
        let mut pending = previous.clone();
        let runtimes = self.runtime_instances.clone();
        crate::provider_configuration::edit_provider(&mut pending, gateway_id, edit, &|login| {
            crate::claude_cli::linked_directory(login, &runtimes)
        })?;
        link_claude_logins(&previous, &mut pending)?;
        self.gateways = pending;
        if let Err(error) = self.persist() {
            self.gateways = previous;
            self.runtime_instances = previous_runtimes;
            return Err(error);
        }
        self.sync_scheduling(&previous);
        let reconnect =
            previous != self.gateways && self.invalidate_gateway_execution(gateway_id)?;
        Ok(crate::GatewayUpdate {
            gateways: self.public_gateways(),
            execution_invalidated: reconnect,
        })
    }

    pub(crate) fn save_model_template(
        &mut self,
        template: crate::config::ModelTemplate,
    ) -> Result<Vec<crate::config::ModelTemplate>, RuntimeError> {
        let mut pending = self.model_templates.clone();
        crate::provider_configuration::save_template(&mut pending, template)?;
        self.store_model_templates(pending)
    }

    pub(crate) fn delete_model_template(
        &mut self,
        id: String,
    ) -> Result<Vec<crate::config::ModelTemplate>, RuntimeError> {
        let mut pending = self.model_templates.clone();
        pending.retain(|template| template.id != id);
        self.store_model_templates(pending)
    }

    fn store_model_templates(
        &mut self,
        templates: Vec<crate::config::ModelTemplate>,
    ) -> Result<Vec<crate::config::ModelTemplate>, RuntimeError> {
        let previous = std::mem::replace(&mut self.model_templates, templates);
        if let Err(error) = self.persist() {
            self.model_templates = previous;
            return Err(error);
        }
        Ok(self.model_templates.clone())
    }
}

/// Reads the status of each newly referenced Claude Code login, and keeps one
/// Claude account bound to one provider whether signed in here or referenced.
fn link_claude_logins(
    previous: &[crate::config::GatewayConfig],
    pending: &mut [crate::config::GatewayConfig],
) -> Result<(), RuntimeError> {
    use crate::provider_authentication::AuthenticationMaterial;
    let mut linked = Vec::new();
    for provider in pending.iter_mut().flat_map(|g| g.providers.iter_mut()) {
        let unchanged = previous.iter().flat_map(|g| &g.providers).any(|p| {
            p.id == provider.id && p.authentication.material == provider.authentication.material
        });
        if !unchanged
            && matches!(
                provider.authentication.material,
                AuthenticationMaterial::ClaudeCliLinked { .. }
            )
            && let crate::config::ExecutionBackend::ClaudeCli { binary } =
                &provider.execution_backend
        {
            let binary = binary.clone();
            crate::claude_cli::refresh_linked(&mut provider.authentication.material, &binary)?;
            linked.push(provider.id.clone());
        }
    }
    for id in &linked {
        crate::claude_cli::reject_duplicate_account(pending, id)?;
    }
    Ok(())
}
