//! Interactive authentication stays in its provider context; private source coordinates never reach the UI.
use super::*;
use crate::provider_authentication::*;
impl CoreRuntime {
    pub(super) fn authentication_action(&mut self, payload: &Value) -> Result<Value, RuntimeError> {
        let operation = payload["operation"].as_str().unwrap_or("poll");
        if operation == "start" && self.busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
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
            self.invalidate_execution()?;
            data["gateways"] = serde_json::to_value(self.public_gateways())?;
            data["executionInvalidated"] = Value::Bool(true);
        }
        Ok(data)
    }
}
