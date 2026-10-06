//! Provider-owned configuration operations with one atomic commit.
use super::*;
impl CoreRuntime {
    pub(super) fn provider_action(&mut self, payload: &Value) -> Result<Value, RuntimeError> {
        if payload["operation"] == "readApiKey" {
            let gateway = self
                .gateways
                .iter()
                .find(|g| g.id == payload["gatewayID"].as_str().unwrap_or(""))
                .ok_or_else(|| RuntimeError::invalid("gateway id"))?;
            let provider = gateway
                .providers
                .iter()
                .find(|p| p.id == payload["providerID"].as_str().unwrap_or(""))
                .ok_or_else(|| RuntimeError::invalid("provider id"))?;
            return crate::authentication_resolver::read_api_key(provider, &self.options)
                .map(Value::String);
        }
        if self.busy()
            || self
                .authentication
                .as_ref()
                .is_some_and(authentication::Login::is_running)
        {
            return Err(RuntimeError::invalid("runtime or login is busy"));
        }
        let previous = self.gateways.clone();
        let previous_runtimes = self.runtime_instances.clone();
        let mut pending = previous.clone();
        crate::provider_configuration::edit_provider(&mut pending, payload)?;
        self.gateways = pending;
        if let Err(error) = self.persist() {
            self.gateways = previous;
            self.runtime_instances = previous_runtimes;
            return Err(error);
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
            self.invalidate_execution()?;
        }
        Ok(json!({"gateways":self.public_gateways(),"executionInvalidated":reconnect}))
    }
    pub(super) fn template_action(&mut self, payload: &Value) -> Result<Value, RuntimeError> {
        let previous = self.model_templates.clone();
        let mut pending = previous.clone();
        crate::provider_configuration::edit_template(&mut pending, payload)?;
        self.model_templates = pending;
        if let Err(error) = self.persist() {
            self.model_templates = previous;
            return Err(error);
        }
        Ok(serde_json::to_value(&self.model_templates)?)
    }
}
