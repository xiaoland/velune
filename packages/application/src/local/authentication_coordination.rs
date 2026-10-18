//! Interactive authentication stays in its provider context; private source coordinates never reach the UI.
use super::*;
use crate::provider_authentication::*;
impl CoreRuntime {
    /// Re-reads a referenced Claude Code login, which may have been signed in,
    /// switched or moved with its runtime since, and saves what changed.
    fn inspect_linked_claude(
        &mut self,
        gateway_id: &str,
        provider_id: &str,
    ) -> Result<Value, RuntimeError> {
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
        let capabilities = json!({"protocol":provider.protocol,"endpoint":provider.endpoint,"explicitOutputCap":true,"temperature":true});
        if !unchanged {
            crate::claude_cli::reject_duplicate_account(&pending, provider_id)?;
            self.gateways = pending;
            if let Err(error) = self.persist() {
                self.gateways = previous;
                return Err(error);
            }
            self.sync_scheduling(&previous);
        }
        Ok(
            json!({"metadata":{"configured":description.configured,"accountLabel":description.account_label,"capabilities":capabilities,"actions":description.actions}}),
        )
    }
    fn poll_subscription_login(&mut self) -> Result<Value, RuntimeError> {
        let mut data = self
            .subscription_login
            .as_mut()
            .ok_or_else(|| RuntimeError::invalid("subscription login missing"))?
            .poll();
        if data["events"].as_array().is_some_and(|events| {
            events
                .iter()
                .any(|event| event["type"] == "result" && event["ok"] == true)
        }) {
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
            data["gateways"] = serde_json::to_value(self.public_gateways())?;
            data["executionInvalidated"] = Value::Bool(true);
        }
        if data["running"] == false {
            self.subscription_login = None;
            self.authentication_provider = None;
        }
        Ok(data)
    }

    pub(super) fn authentication_action(&mut self, payload: &Value) -> Result<Value, RuntimeError> {
        let operation = payload["operation"].as_str().unwrap_or("poll");
        if operation == "start"
            && (self.execution_active()
                || self
                    .subscription_login
                    .as_ref()
                    .is_some_and(crate::provider_login::Login::is_running)
                || self
                    .authentication
                    .as_ref()
                    .is_some_and(authentication::Login::is_running))
        {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        if self.subscription_login.is_some() && matches!(operation, "poll" | "cancel") {
            if operation == "cancel"
                && let Some(login) = self.subscription_login.as_mut()
            {
                login.cancel();
            }
            return self.poll_subscription_login();
        }
        if operation == "logout" {
            if self.execution_active()
                || self.subscription_login.is_some()
                || self
                    .authentication
                    .as_ref()
                    .is_some_and(authentication::Login::is_running)
            {
                return Err(RuntimeError::invalid("runtime or login is busy"));
            }
            let gateway_id = payload["gatewayID"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("gateway id"))?;
            let provider_id = payload["providerID"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("provider id"))?;
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
            return Ok(json!({"gateways":self.public_gateways(),"executionInvalidated":true}));
        }
        let mut request = payload.clone();
        if matches!(operation, "start" | "inspect") {
            let gateway_id = payload["gatewayID"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("gateway id"))?;
            let provider_id = payload["providerID"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("provider id"))?;
            let provider = self
                .gateways
                .iter()
                .find(|g| g.id == gateway_id)
                .and_then(|g| g.providers.iter().find(|p| p.id == provider_id))
                .ok_or_else(|| RuntimeError::invalid("provider id"))?;
            match &provider.authentication.material {
                AuthenticationMaterial::None | AuthenticationMaterial::ApiKey { .. } => {
                    if operation == "start" {
                        return Err(RuntimeError::invalid("此提供商没有交互式登录流程"));
                    }
                    return Ok(
                        json!({"metadata":{"configured":provider.authentication.description().configured,"capabilities":{"protocol":provider.protocol,"endpoint":provider.endpoint,"explicitOutputCap":true,"temperature":true},"actions":[]}}),
                    );
                }
                AuthenticationMaterial::CodexSubscription { credential_id, .. } => {
                    let (configured, account_label) = crate::subscription_auth::status(
                        &self.options.home_directory,
                        credential_id,
                        provider.authentication.generation,
                    );
                    if operation == "inspect" {
                        return Ok(
                            json!({"metadata":{"configured":configured,"accountLabel":account_label,"capabilities":{"protocol":provider.protocol,"endpoint":provider.endpoint,"explicitOutputCap":false,"temperature":false},"actions":[{"id":"login","label":"登录…"}]}}),
                        );
                    }
                    let next_generation = provider
                        .authentication
                        .generation
                        .checked_add(1)
                        .ok_or_else(|| RuntimeError::invalid("authentication revision overflow"))?;
                    self.subscription_login = Some(crate::provider_login::Login::Codex(
                        crate::subscription_auth::Login::start(
                            self.options.home_directory.clone(),
                            crate::new_record_key(),
                            next_generation,
                        )?,
                    ));
                    self.authentication_provider = Some((gateway_id.into(), provider_id.into()));
                    return self.poll_subscription_login();
                }
                AuthenticationMaterial::ClaudeCli { credential_id, .. } => {
                    let crate::config::ExecutionBackend::ClaudeCli { binary } =
                        &provider.execution_backend
                    else {
                        return Err(RuntimeError::invalid("CLI backend missing"));
                    };
                    if operation == "inspect" {
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
                        return Ok(
                            json!({"metadata":{"configured":status.as_ref().is_ok_and(|v| v.configured),"accountLabel":status.ok().and_then(|v| v.label),"capabilities":{"protocol":provider.protocol,"endpoint":provider.endpoint,"explicitOutputCap":true,"temperature":true},"actions":[{"id":"login","label":"登录…"},{"id":"logout","label":"退出登录"}]}}),
                        );
                    }
                    self.subscription_login = Some(crate::provider_login::Login::Claude(
                        crate::claude_cli::Login::start(
                            self.options.home_directory.clone(),
                            binary.clone(),
                        )?,
                    ));
                    self.authentication_provider = Some((gateway_id.into(), provider_id.into()));
                    return self.poll_subscription_login();
                }
                AuthenticationMaterial::ClaudeCliLinked { .. } => {
                    if operation == "start" {
                        return Err(RuntimeError::invalid(
                            "此提供商沿用既有的 Claude Code 登录；请在 Claude Code 中登录，再检查登录状态。",
                        ));
                    }
                    return self.inspect_linked_claude(gateway_id, provider_id);
                }
                AuthenticationMaterial::RuntimeProvider { source, .. } => {
                    request["source"] = Value::String(serde_json::to_string(source)?);
                }
            }
            if operation == "start" {
                self.authentication_provider = Some((gateway_id.into(), provider_id.into()));
            }
        }
        let mut data = authentication::handle(
            &mut self.authentication,
            &self.options.resources_directory,
            &self.options.home_directory,
            &request,
        )
        .map_err(RuntimeError::Invalid)?;
        if data["events"].as_array().is_some_and(|events| {
            events
                .iter()
                .any(|e| e["type"] == "result" && e["ok"] == true)
        }) {
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
            data["gateways"] = serde_json::to_value(self.public_gateways())?;
            data["executionInvalidated"] = Value::Bool(true);
        }
        Ok(data)
    }
}
