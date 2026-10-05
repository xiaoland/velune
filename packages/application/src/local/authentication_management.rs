//! Registered authentication mutations; returned errors always precede persistence.
use super::*;
impl CoreRuntime {
    pub(super) fn authentication_resource_action(
        &mut self,
        payload: &Value,
    ) -> Result<Value, RuntimeError> {
        if self.pi_busy
            || self
                .authentication
                .as_ref()
                .is_some_and(authentication::Login::is_running)
        {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let previous = self.authentication_resources.clone();
        let mut pending = self
            .authentication_resources
            .prepare_mutation(payload, &self.gateways)?;
        let id = pending.result.binding.id.clone();
        let requires_reconnect = payload["operation"] != "rename"
            && self
                .active_runtime_id
                .as_ref()
                .and_then(|id| {
                    self.runtime_instances
                        .iter()
                        .find(|runtime| &runtime.id == id)
                })
                .and_then(|runtime| {
                    self.gateways
                        .iter()
                        .find(|gateway| gateway.id == runtime.gateway_id)
                })
                .is_some_and(|gateway| {
                    gateway
                        .providers
                        .iter()
                        .any(|provider| provider.authentication_id.as_deref() == Some(id.as_str()))
                });
        pending.result.requires_reconnect = requires_reconnect;
        let mut result = serde_json::to_value(pending.result)?;
        self.authentication_resources = pending.manager;
        if let Err(error) = self.persist() {
            self.authentication_resources = previous;
            return Err(error);
        }
        if requires_reconnect && self.shutdown_active().is_err() {
            // Commit succeeded. Do not return Err and cause the platform to
            // delete the newly registered secret. Retain old owned refs too.
            self.gateway_runner = None;
            self.active_runtime_id = None;
            result["obsoleteOwnedKeychainRefs"] = json!([]);
            result["warnings"] = json!(["认证已保存，但旧运行时未能完整退出；旧凭据暂时保留。"]);
        }
        Ok(result)
    }
}
