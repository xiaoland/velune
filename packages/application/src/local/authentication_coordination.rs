//! Interactive authentication stays in its provider context; private source coordinates never reach the UI.
use super::*;
use crate::provider_authentication::*;
use velune_conversation::authentication::{AuthenticationEvent, LoginProgress};
impl CoreRuntime {
    /// A runtime-backed login writes the source runtime's credentials as well
    /// as the provider's metadata, so native sessions using that source matter too.
    fn authentication_source_runtime(&self, gateway_id: &str, provider_id: &str) -> Option<String> {
        match &self
            .provider(gateway_id, provider_id)
            .ok()?
            .authentication
            .material
        {
            AuthenticationMaterial::RuntimeProvider { source, .. } => {
                source.source_instance_id.clone()
            }
            _ => None,
        }
    }

    fn ensure_authentication_idle(
        &mut self,
        gateway_id: &str,
        provider_id: &str,
    ) -> Result<(), RuntimeError> {
        let source = self.authentication_source_runtime(gateway_id, provider_id);
        let writes_source = matches!(
            self.provider(gateway_id, provider_id)?
                .authentication
                .material,
            AuthenticationMaterial::RuntimeProvider { .. }
        );
        if (writes_source && self.provider_in_use(gateway_id, provider_id))
            || source.is_some_and(|id| self.runtime_in_use(&id))
        {
            return Err(RuntimeError::invalid(
                "此认证来源仍有会话或后台任务正在使用，请停止后再登录",
            ));
        }
        Ok(())
    }

    fn invalidate_authentication_execution(
        &mut self,
        gateway_id: &str,
        provider_id: &str,
    ) -> Result<bool, RuntimeError> {
        let source_runtime = self.authentication_source_runtime(gateway_id, provider_id);
        let mut invalidated = false;
        self.for_each_session(|core| {
            if core.session_uses_gateway(gateway_id)
                || source_runtime
                    .as_ref()
                    .is_some_and(|id| core.session.uses_runtime(id))
            {
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

    /// Re-reads a referenced Claude Code login, which may have been signed in,
    /// switched or moved with its runtime since, and saves what changed.
    fn inspect_linked_claude(
        &mut self,
        gateway_id: &str,
        provider_id: &str,
    ) -> Result<crate::AuthenticationMetadata, RuntimeError> {
        let previous = self.gateways.clone();
        let runtimes = self.runtime_instances.clone();
        let mut pending = previous.clone();
        let provider = pending
            .iter_mut()
            .find(|g| g.id == gateway_id)
            .and_then(|g| g.providers.iter_mut().find(|p| p.id == provider_id))
            .ok_or_else(|| RuntimeError::invalid("provider id"))?;
        let crate::config::ExecutionBackend::ClaudeCli { binary } = &provider.execution_backend
        else {
            return Err(RuntimeError::invalid("CLI backend missing"));
        };
        let binary = binary.clone();
        if let AuthenticationMaterial::ClaudeCliLinked {
            login, config_dir, ..
        } = &mut provider.authentication.material
        {
            *config_dir = crate::claude_cli::linked_directory(login, &runtimes)?;
        }
        crate::claude_cli::refresh_linked(&mut provider.authentication.material, &binary)?;
        let unchanged = previous
            .iter()
            .flat_map(|g| &g.providers)
            .any(|p| p.id == provider_id && p.authentication == provider.authentication);
        if !unchanged {
            provider.authentication.generation = provider
                .authentication
                .generation
                .checked_add(1)
                .ok_or_else(|| RuntimeError::invalid("authentication revision overflow"))?;
        }
        let description = provider.authentication.description();
        let metadata = crate::AuthenticationMetadata {
            configured: description.configured,
            account_label: description.account_label,
            capabilities: capabilities(provider, true),
            actions: Some(description.actions),
        };
        if !unchanged {
            crate::claude_cli::reject_duplicate_account(&pending, provider_id)?;
            self.gateways = pending;
            if let Err(error) = self.persist() {
                self.gateways = previous;
                return Err(error);
            }
            self.sync_scheduling(&previous);
            self.invalidate_gateway_execution(gateway_id)?;
        }
        Ok(metadata)
    }
    fn poll_subscription_login(&mut self) -> Result<crate::AuthenticationProgress, RuntimeError> {
        let login = self
            .subscription_login
            .as_mut()
            .ok_or_else(|| RuntimeError::invalid("subscription login missing"))?
            .poll();
        let mut progress = progress(login);
        if progress.events.iter().any(AuthenticationEvent::succeeded) {
            let staged_id = self
                .subscription_login
                .as_ref()
                .ok_or_else(|| RuntimeError::invalid("subscription login missing"))?
                .credential_id();
            let owner = self
                .authentication_provider
                .as_ref()
                .ok_or_else(|| RuntimeError::invalid("login provider missing"))?;
            let claude_status = self
                .subscription_login
                .as_ref()
                .and_then(crate::provider_login::Login::claude_status)
                .cloned();
            let duplicate = self.gateways.iter().any(|gateway| {
                gateway.providers.iter().any(|provider| {
                    if gateway.id == owner.0 && provider.id == owner.1 {
                        return false;
                    }
                    match (&provider.authentication.material, &claude_status) {
                        (AuthenticationMaterial::CodexSubscription { credential_id, .. }, None) => {
                            crate::subscription_auth::same_subscription(
                                &self.options.home_directory,
                                staged_id,
                                credential_id,
                            )
                        }
                        (
                            AuthenticationMaterial::ClaudeCli {
                                identity: Some(identity),
                                ..
                            }
                            | AuthenticationMaterial::ClaudeCliLinked {
                                identity: Some(identity),
                                ..
                            },
                            Some(status),
                        ) => status.identity.as_ref() == Some(identity),
                        _ => false,
                    }
                })
            });
            if duplicate {
                self.subscription_login = None;
                self.authentication_provider = None;
                return Err(RuntimeError::invalid(
                    "此订阅已绑定另一个提供商；请使用该提供商的模型或登录其他订阅。",
                ));
            }
            let previous = self.gateways.clone();
            let (gateway_id, provider_id) = self
                .authentication_provider
                .take()
                .ok_or_else(|| RuntimeError::invalid("login provider missing"))?;
            let provider = self
                .gateways
                .iter_mut()
                .find(|g| g.id == gateway_id)
                .and_then(|g| g.providers.iter_mut().find(|p| p.id == provider_id))
                .ok_or_else(|| RuntimeError::invalid("login provider missing"))?;
            match &mut provider.authentication.material {
                AuthenticationMaterial::CodexSubscription {
                    credential_id,
                    configured,
                } => {
                    *credential_id = self
                        .subscription_login
                        .as_ref()
                        .ok_or_else(|| RuntimeError::invalid("login missing"))?
                        .credential_id()
                        .to_owned();
                    *configured = true;
                }
                AuthenticationMaterial::ClaudeCli {
                    credential_id,
                    configured,
                    account_label,
                    identity,
                } => {
                    let status = claude_status
                        .as_ref()
                        .ok_or_else(|| RuntimeError::invalid("CLI login status missing"))?;
                    *credential_id = staged_id.to_owned();
                    *configured = status.configured;
                    *account_label = status.label.clone();
                    *identity = status.identity.clone();
                }
                _ => return Err(RuntimeError::invalid("subscription binding changed")),
            };
            provider.authentication.generation = provider
                .authentication
                .generation
                .checked_add(1)
                .ok_or_else(|| RuntimeError::invalid("authentication revision overflow"))?;
            if let Err(error) = self.persist() {
                self.gateways = previous;
                self.subscription_login = None;
                return Err(error);
            }
            self.sync_scheduling(&previous);
            if let Some(login) = self.subscription_login.as_mut() {
                login.commit();
            }
            // sync_scheduling retains the old credential until every execution
            // that captured it has ended, then performs the owned cleanup.
            let invalidated =
                self.invalidate_authentication_execution(&gateway_id, &provider_id)?;
            progress.gateways = Some(self.public_gateways());
            progress.execution_invalidated = Some(invalidated);
        }
        if !progress.running {
            self.subscription_login = None;
            self.authentication_provider = None;
        }
        Ok(progress)
    }

    fn login_busy(&self) -> bool {
        self.subscription_login
            .as_ref()
            .is_some_and(crate::provider_login::Login::is_running)
            || self
                .authentication
                .as_ref()
                .is_some_and(authentication::Login::is_running)
    }

    fn provider(
        &self,
        gateway_id: &str,
        provider_id: &str,
    ) -> Result<&crate::config::ProviderDefinition, RuntimeError> {
        self.gateways
            .iter()
            .find(|g| g.id == gateway_id)
            .and_then(|g| g.providers.iter().find(|p| p.id == provider_id))
            .ok_or_else(|| RuntimeError::invalid("provider id"))
    }

    pub(crate) fn authentication_inspect(
        &mut self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<crate::AuthenticationMetadata, RuntimeError> {
        let provider = self.provider(&gateway_id, &provider_id)?;
        let login = |id: &str, label: &str| crate::conversation::SettingAction {
            id: id.into(),
            label: label.into(),
        };
        match &provider.authentication.material {
            AuthenticationMaterial::None | AuthenticationMaterial::ApiKey { .. } => {
                let (configured, account_label) =
                    if provider.request_profile == crate::config::RequestProfile::KimiCoding {
                        let _ = self
                            .subscription_quota
                            .query(&gateway_id, &provider_id, false);
                        self.subscription_quota
                            .identity_status(&gateway_id, &provider_id)
                    } else {
                        (provider.authentication.description().configured, None)
                    };
                Ok(crate::AuthenticationMetadata {
                    configured,
                    account_label,
                    capabilities: capabilities(provider, true),
                    actions: Some(Vec::new()),
                })
            }
            AuthenticationMaterial::CodexSubscription { credential_id, .. } => {
                let (configured, account_label) = crate::subscription_auth::status(
                    &self.options.home_directory,
                    credential_id,
                    provider.authentication.generation,
                );
                Ok(crate::AuthenticationMetadata {
                    configured,
                    account_label,
                    capabilities: capabilities(provider, false),
                    actions: Some(vec![login("login", "登录…")]),
                })
            }
            AuthenticationMaterial::ClaudeCli { credential_id, .. } => {
                let crate::config::ExecutionBackend::ClaudeCli { binary } =
                    &provider.execution_backend
                else {
                    return Err(RuntimeError::invalid("CLI backend missing"));
                };
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                let status = runtime.block_on(crate::claude_cli::status(
                    binary,
                    Some(&crate::claude_cli::directory(
                        &self.options.home_directory,
                        credential_id,
                    )),
                ));
                Ok(crate::AuthenticationMetadata {
                    configured: status.as_ref().is_ok_and(|v| v.configured),
                    account_label: status.ok().and_then(|v| v.label),
                    capabilities: capabilities(provider, true),
                    actions: Some(vec![login("login", "登录…"), login("logout", "退出登录")]),
                })
            }
            AuthenticationMaterial::ClaudeCliLinked { .. } => {
                self.inspect_linked_claude(&gateway_id, &provider_id)
            }
            AuthenticationMaterial::RuntimeProvider { source, .. } => {
                let metadata = authentication::inspect(&self.options.resources_directory, source)
                    .map_err(RuntimeError::Invalid)?;
                serde_json::from_value(metadata)
                    .map_err(|error| RuntimeError::context("运行时来源返回的认证描述无效", error))
            }
        }
    }

    pub(crate) fn authentication_start(
        &mut self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<crate::AuthenticationProgress, RuntimeError> {
        if self.login_busy() {
            return Err(RuntimeError::invalid("请先完成当前登录"));
        }
        self.ensure_authentication_idle(&gateway_id, &provider_id)?;
        let provider = self.provider(&gateway_id, &provider_id)?;
        let login = match &provider.authentication.material {
            AuthenticationMaterial::None | AuthenticationMaterial::ApiKey { .. } => {
                return Err(RuntimeError::invalid("此提供商没有交互式登录流程"));
            }
            AuthenticationMaterial::ClaudeCliLinked { .. } => {
                return Err(RuntimeError::invalid(
                    "此提供商沿用既有的 Claude Code 登录；请在 Claude Code 中登录，再检查登录状态。",
                ));
            }
            AuthenticationMaterial::CodexSubscription { .. } => {
                let next_generation = provider
                    .authentication
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| RuntimeError::invalid("authentication revision overflow"))?;
                crate::provider_login::Login::Codex(crate::subscription_auth::Login::start(
                    self.options.home_directory.clone(),
                    crate::new_record_key(),
                    next_generation,
                )?)
            }
            AuthenticationMaterial::ClaudeCli { .. } => {
                let crate::config::ExecutionBackend::ClaudeCli { binary } =
                    &provider.execution_backend
                else {
                    return Err(RuntimeError::invalid("CLI backend missing"));
                };
                crate::provider_login::Login::Claude(crate::claude_cli::Login::start(
                    self.options.home_directory.clone(),
                    binary.clone(),
                )?)
            }
            AuthenticationMaterial::RuntimeProvider { source, .. } => {
                let source = source.clone();
                self.authentication_provider = Some((gateway_id, provider_id));
                let started = authentication::start(
                    &mut self.authentication,
                    &self.options.resources_directory,
                    &self.options.home_directory,
                    source,
                )
                .map_err(RuntimeError::Invalid)?;
                return self.finish_source_login(started);
            }
        };
        self.subscription_login = Some(login);
        self.authentication_provider = Some((gateway_id, provider_id));
        self.poll_subscription_login()
    }

    pub(crate) fn authentication_poll(
        &mut self,
    ) -> Result<crate::AuthenticationProgress, RuntimeError> {
        if self.subscription_login.is_some() {
            return self.poll_subscription_login();
        }
        let polled =
            authentication::poll(&mut self.authentication).map_err(RuntimeError::Invalid)?;
        self.finish_source_login(polled)
    }

    pub(crate) fn authentication_reply(
        &mut self,
        prompt_id: String,
        value: String,
    ) -> Result<crate::AuthenticationProgress, RuntimeError> {
        let replied = authentication::reply(&mut self.authentication, &prompt_id, &value)
            .map_err(RuntimeError::Invalid)?;
        self.finish_source_login(replied)
    }

    pub(crate) fn authentication_cancel(
        &mut self,
    ) -> Result<crate::AuthenticationProgress, RuntimeError> {
        if let Some(login) = self.subscription_login.as_mut() {
            login.cancel();
            return self.poll_subscription_login();
        }
        let cancelled = authentication::cancel(&mut self.authentication);
        self.finish_source_login(cancelled)
    }

    pub(crate) fn authentication_logout(
        &mut self,
        gateway_id: String,
        provider_id: String,
    ) -> Result<crate::GatewayUpdate, RuntimeError> {
        self.ensure_authentication_idle(&gateway_id, &provider_id)?;
        // CLI logout mutates the captured directory itself, so it cannot be
        // staged beside an execution that still needs those credentials.
        if self.provider_in_use(&gateway_id, &provider_id) {
            return Err(RuntimeError::invalid(
                "此订阅仍有会话或后台任务正在使用，请停止后再退出登录",
            ));
        }
        if self
            .subscription_login
            .as_ref()
            .is_some_and(crate::provider_login::Login::is_running)
            || self
                .authentication
                .as_ref()
                .is_some_and(authentication::Login::is_running)
        {
            return Err(RuntimeError::invalid("请先完成当前登录，再退出登录"));
        }
        let previous = self.gateways.clone();
        let provider = self
            .gateways
            .iter_mut()
            .find(|g| g.id == gateway_id)
            .and_then(|g| g.providers.iter_mut().find(|p| p.id == provider_id))
            .ok_or_else(|| RuntimeError::invalid("provider id"))?;
        let (
            crate::config::ExecutionBackend::ClaudeCli { binary },
            AuthenticationMaterial::ClaudeCli {
                credential_id,
                configured,
                ..
            },
        ) = (
            &provider.execution_backend,
            &mut provider.authentication.material,
        )
        else {
            return Err(RuntimeError::invalid("此认证来源不支持 CLI 退出登录"));
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime
            .block_on(crate::claude_cli::logout(
                binary,
                &crate::claude_cli::directory(&self.options.home_directory, credential_id),
            ))
            .map_err(|_| RuntimeError::invalid("Claude Code 退出登录失败"))?;
        *configured = false;
        provider.authentication.generation = provider
            .authentication
            .generation
            .checked_add(1)
            .ok_or_else(|| RuntimeError::invalid("authentication revision overflow"))?;
        let persisted = self.persist();
        self.sync_scheduling(&previous);
        let invalidated = self.invalidate_authentication_execution(&gateway_id, &provider_id)?;
        persisted?;
        Ok(crate::GatewayUpdate {
            gateways: self.public_gateways(),
            execution_invalidated: invalidated,
        })
    }

    /// A runtime source's login that finished writes its credential to the
    /// source; the provider then uses it as an OAuth login.
    fn finish_source_login(
        &mut self,
        login: LoginProgress,
    ) -> Result<crate::AuthenticationProgress, RuntimeError> {
        let mut progress = progress(login);
        if !progress.events.iter().any(AuthenticationEvent::succeeded) {
            return Ok(progress);
        }
        let previous = self.gateways.clone();
        let (gateway_id, provider_id) = self
            .authentication_provider
            .take()
            .ok_or_else(|| RuntimeError::invalid("login provider missing"))?;
        let provider = self
            .gateways
            .iter_mut()
            .find(|g| g.id == gateway_id)
            .and_then(|g| g.providers.iter_mut().find(|p| p.id == provider_id))
            .ok_or_else(|| RuntimeError::invalid("login provider missing"))?;
        if let AuthenticationMaterial::RuntimeProvider {
            source,
            method,
            configured,
            ..
        } = &mut provider.authentication.material
        {
            *method = AuthenticationMethod::OAuth;
            *configured = true;
            source
                .settings
                .insert("credentialKind".into(), "oauth".into());
        }
        provider.authentication.generation = provider
            .authentication
            .generation
            .checked_add(1)
            .ok_or_else(|| RuntimeError::invalid("authentication revision overflow"))?;
        if self.persist().is_err() {
            self.gateways = previous;
            return Err(RuntimeError::invalid(
                "来源已完成登录，但提供商配置保存失败；请检查配置目录后重试。",
            ));
        }
        self.sync_scheduling(&previous);
        let invalidated = self.invalidate_authentication_execution(&gateway_id, &provider_id)?;
        progress.gateways = Some(self.public_gateways());
        progress.execution_invalidated = Some(invalidated);
        Ok(progress)
    }
}

fn capabilities(
    provider: &crate::config::ProviderDefinition,
    sampling: bool,
) -> crate::AuthenticationCapabilities {
    crate::AuthenticationCapabilities {
        protocol: provider.protocol,
        endpoint: provider.endpoint.clone(),
        explicit_output_cap: sampling,
        temperature: sampling,
    }
}

fn progress(login: LoginProgress) -> crate::AuthenticationProgress {
    crate::AuthenticationProgress {
        running: login.running,
        events: login.events,
        gateways: None,
        execution_invalidated: None,
    }
}
