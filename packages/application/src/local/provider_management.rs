//! Provider-owned configuration operations with one atomic commit.
use super::*;
use crate::provider_configuration::ProviderEdit;
impl CoreRuntime {
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
        if self.execution_active()
            || self
                .authentication
                .as_ref()
                .is_some_and(authentication::Login::is_running)
            || self
                .subscription_login
                .as_ref()
                .is_some_and(crate::provider_login::Login::is_running)
        {
            return Err(RuntimeError::invalid("runtime or login is busy"));
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
        let mut cleanup_failed = false;
        for old in previous.iter().flat_map(|g| &g.providers) {
            if let crate::provider_authentication::AuthenticationMaterial::CodexSubscription { credential_id, .. } = &old.authentication.material
                && !self.gateways.iter().flat_map(|g| &g.providers).any(|p| matches!(&p.authentication.material, crate::provider_authentication::AuthenticationMaterial::CodexSubscription { credential_id: current, .. } if current == credential_id))
            { cleanup_failed |= crate::subscription_auth::remove(&self.options.home_directory, credential_id).is_err(); }
        }
        for old in previous.iter().flat_map(|g| &g.providers) {
            if let crate::provider_authentication::AuthenticationMaterial::ClaudeCli { credential_id, configured, .. } = &old.authentication.material
                && !self.gateways.iter().flat_map(|g| &g.providers).any(|p| matches!(&p.authentication.material, crate::provider_authentication::AuthenticationMaterial::ClaudeCli { credential_id: current, .. } if current == credential_id)) {
                if *configured && let crate::config::ExecutionBackend::ClaudeCli { binary } = &old.execution_backend {
                    let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build()?;
                    cleanup_failed |= runtime.block_on(crate::claude_cli::logout(binary,&crate::claude_cli::directory(&self.options.home_directory,credential_id))).is_err();
                }
                cleanup_failed |= crate::claude_cli::remove(&self.options.home_directory,credential_id).is_err();
            }
        }
        let reconnect = self.next_turn_runtime_id.as_ref().is_some_and(|id| {
            let before = previous_runtimes.iter().find(|runtime| &runtime.id == id);
            let after = self
                .runtime_instances
                .iter()
                .find(|runtime| &runtime.id == id);
            before != after
                || before.is_some_and(|runtime| {
                    previous
                        .iter()
                        .find(|gateway| gateway.id == runtime.gateway_id)
                        != self
                            .gateways
                            .iter()
                            .find(|gateway| gateway.id == runtime.gateway_id)
                })
        });
        if reconnect {
            self.for_each_session(Self::invalidate_execution)?;
        }
        if cleanup_failed {
            tracing::warn!(event = "subscription_cleanup_failed");
            return Err(RuntimeError::invalid(
                "提供商配置已保存，但旧订阅认证清理失败；请检查应用配置目录的写入权限。",
            ));
        }
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
