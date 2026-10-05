//! Authentication uses a registry ID; only the source adapter receives private coordinates.
use super::*;
use crate::authentication_resources::*;
impl CoreRuntime {
    pub(super) fn authentication_action(&mut self, payload: &Value) -> Result<Value, RuntimeError> {
        let operation = payload["operation"].as_str().unwrap_or("poll");
        if operation == "start" && self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let mut request = payload.clone();
        if matches!(operation, "start" | "inspect") {
            let id = payload["bindingID"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("authentication resource id"))?;
            let resource = self.authentication_resources.get(id)?;
            match &resource.locator {
                AuthenticationLocator::Keychain { .. } => {
                    if operation == "start" {
                        return Err(RuntimeError::invalid(
                            "API key 认证请在认证设置中替换；此资源没有登录流程。",
                        ));
                    }
                    return Ok(
                        json!({"metadata":{"configured":resource.configured,"capabilities":{"protocol":resource.protocol,"endpoint":resource.endpoint,"explicitOutputCap":true,"temperature":true},"actions":[]}}),
                    );
                }
                AuthenticationLocator::RuntimeProvider { source } => {
                    request["source"] = Value::String(serde_json::to_string(source)?)
                }
            }
            if operation == "start" {
                self.authentication_binding_id = Some(id.into());
            }
        }
        let mut data = authentication::handle(
            &mut self.authentication,
            &self.options.resources_directory,
            &self.options.home_directory,
            &request,
        )
        .map_err(RuntimeError::Invalid)?;
        let succeeded = data["events"].as_array().is_some_and(|events| {
            events
                .iter()
                .any(|event| event["type"] == "result" && event["ok"] == true)
        });
        if succeeded {
            let previous = self.authentication_resources.clone();
            let id = self.authentication_binding_id.take().ok_or_else(|| {
                RuntimeError::invalid("authentication session resource is missing")
            })?;
            let resource = self
                .authentication_resources
                .resources
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or_else(|| RuntimeError::invalid("authentication resource not found"))?;
            resource.method = AuthenticationMethod::OAuth;
            resource.configured = true;
            resource.generation = resource
                .generation
                .checked_add(1)
                .ok_or_else(|| RuntimeError::invalid("authentication revision overflow"))?;
            if let AuthenticationLocator::RuntimeProvider { source } = &mut resource.locator {
                source
                    .settings
                    .insert("credentialKind".into(), "oauth".into());
            }
            if self.persist().is_err() {
                self.authentication_resources = previous;
                return Err(RuntimeError::invalid(
                    "来源登录已完成，但认证资源保存失败；原来源保持登录，请检查配置目录后重试登记。",
                ));
            }
            self.shutdown_active()?;
            data["gateways"] = serde_json::to_value(&self.gateways)?;
            data["requiresReconnect"] = Value::Bool(true);
        }
        Ok(data)
    }
}
