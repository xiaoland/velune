//! Local application authentication use cases.
use super::*;
impl CoreRuntime {
    pub(super) fn authentication_action(&mut self, payload: &Value) -> Result<Value, RuntimeError> {
        if payload["operation"].as_str() == Some("start") && self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let source_before = self
            .authentication
            .as_ref()
            .map(|login| login.source().clone());
        let mut data = authentication::handle(
            &mut self.authentication,
            &self.options.resources_directory,
            &self.options.home_directory,
            payload,
        )
        .map_err(RuntimeError::Invalid)?;
        let succeeded = data["events"].as_array().is_some_and(|events| {
            events
                .iter()
                .any(|event| event["type"] == "result" && event["ok"] == true)
        });
        let Some(source) = source_before.filter(|_| succeeded) else {
            return Ok(data);
        };
        let mut changed = false;
        for gateway in &mut self.gateways {
            for provider in &mut gateway.providers {
                let matches = provider
                    .credential_source
                    .as_ref()
                    .is_some_and(|candidate| {
                        candidate.kind == crate::config::CredentialSourceKind::Harness
                            && source.kind == "harness"
                            && candidate.harness_type_id == source.harness_type_id
                            && candidate.provider_id == source.provider_id
                            && candidate.settings.get("authPath") == source.settings.get("authPath")
                    });
                if matches {
                    provider.credential_generation = provider
                        .credential_generation
                        .checked_add(1)
                        .ok_or_else(|| RuntimeError::invalid("credential generation overflow"))?;
                    changed = true;
                }
            }
        }
        if changed {
            self.persist()?;
            if !self.pi_busy {
                self.shutdown_active()?;
            }
            data["gateways"] = serde_json::to_value(&self.gateways)?;
            data["requiresReconnect"] = Value::Bool(true);
        }
        Ok(data)
    }
}
