//! Interactive authentication stays in its provider context; private source coordinates never reach the UI.
use super::*;
use crate::provider_authentication::*;
impl CoreRuntime {
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
            if let Some(login) = self.subscription_login.as_mut() {
                login.commit();
            }
            let cleanup_failed =
                crate::subscription_auth::remove(&self.options.home_directory, &old_credential)
                    .is_err();
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
                    .is_some_and(crate::subscription_auth::Login::is_running)
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
                    self.subscription_login = Some(crate::subscription_auth::Login::start(
                        self.options.home_directory.clone(),
                        crate::new_record_key(),
                        next_generation,
                    )?);
                    self.authentication_provider = Some((gateway_id.into(), provider_id.into()));
                    return self.poll_subscription_login();
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
            self.for_each_session(Self::invalidate_execution)?;
            data["gateways"] = serde_json::to_value(self.public_gateways())?;
            data["executionInvalidated"] = Value::Bool(true);
        }
        Ok(data)
    }
}
