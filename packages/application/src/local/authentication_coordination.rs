//! Interactive authentication stays in its provider context; private source coordinates never reach the UI.
use super::*;
use crate::provider_authentication::*;
use velune_conversation::authentication::{AuthenticationEvent, LoginProgress};
impl CoreRuntime {
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
            let is_claude = claude_status.is_some();
            let old_credential = match &mut provider.authentication.material {
                AuthenticationMaterial::CodexSubscription {
                    credential_id,
                    configured,
                } => {
                    let old = credential_id.clone();
                    *credential_id = self
                        .subscription_login
                        .as_ref()
                        .ok_or_else(|| RuntimeError::invalid("login missing"))?
                        .credential_id()
                        .to_owned();
                    *configured = true;
                    old
                }
                AuthenticationMaterial::ClaudeCli {
                    credential_id,
                    configured,
                    account_label,
                    identity,
                } => {
                    let old = credential_id.clone();
                    let status = claude_status
                        .as_ref()
                        .ok_or_else(|| RuntimeError::invalid("CLI login status missing"))?;
                    *credential_id = staged_id.to_owned();
                    *configured = status.configured;
                    *account_label = status.label.clone();
                    *identity = status.identity.clone();
                    old
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
            let cleanup_failed = if is_claude {
                let mut failed = false;
                if let Some(old) = previous
                    .iter()
                    .find(|g| g.id == gateway_id)
                    .and_then(|g| g.providers.iter().find(|p| p.id == provider_id))
                    && let (
                        crate::config::ExecutionBackend::ClaudeCli { binary },
                        AuthenticationMaterial::ClaudeCli {
                            configured: true, ..
                        },
                    ) = (&old.execution_backend, &old.authentication.material)
                {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    failed |= runtime
                        .block_on(crate::claude_cli::logout(
                            binary,
                            &crate::claude_cli::directory(
                                &self.options.home_directory,
                                &old_credential,
                            ),
                        ))
                        .is_err();
                }
                failed
                    | crate::claude_cli::remove(&self.options.home_directory, &old_credential)
                        .is_err()
            } else {
                crate::subscription_auth::remove(&self.options.home_directory, &old_credential)
                    .is_err()
            };
            self.for_each_session(Self::invalidate_execution)?;
            if cleanup_failed {
                tracing::warn!(event = "subscription_cleanup_failed");
                return Err(RuntimeError::invalid(
                    "订阅登录已保存，但旧认证清理失败；请检查应用配置目录的写入权限。",
                ));
            }
            progress.gateways = Some(self.public_gateways());
            progress.execution_invalidated = Some(true);
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
        if self.execution_active() || self.login_busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
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
        if self.execution_active()
            || self.subscription_login.is_some()
            || self
                .authentication
                .as_ref()
                .is_some_and(authentication::Login::is_running)
        {
            return Err(RuntimeError::invalid("runtime or login is busy"));
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
        self.for_each_session(Self::invalidate_execution)?;
        persisted?;
        Ok(crate::GatewayUpdate {
            gateways: self.public_gateways(),
            execution_invalidated: true,
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
        self.for_each_session(Self::invalidate_execution)?;
        progress.gateways = Some(self.public_gateways());
        progress.execution_invalidated = Some(true);
        Ok(progress)
    }
}

fn capabilities(
    provider: &crate::config::ProviderDefinition,
    sampling: bool,
) -> crate::AuthenticationCapabilities {
    crate::AuthenticationCapabilities {
        protocol: provider.protocol.clone(),
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
