//! Local application configuration use cases.
use super::*;
impl CoreRuntime {
    pub(super) fn list(&self) -> Result<Value, RuntimeError> {
        Ok(json!({
            "conversations": self.session_summaries()?,
            "connections": self.connections(),
            "gateways": self.public_gateways(),
            "runtimeInstances": self.runtime_instances,
            "runtimeTypes": runtime_types(&self.options.resources_directory),
            "modelTemplates":self.model_templates,
            "providerImportTypes": [provider_import::descriptor()],
            "protocols": [
                {"id":"chatCompletionsV1","name":"OpenAI Chat Completions v1","supported":true},
                {"id":"responsesV1","name":"OpenAI Responses v1","supported":true}
            ],
            "activeRuntimeInstanceID": self.active_runtime_id,
        }))
    }

    pub(super) fn provider_import_action(
        &mut self,
        request: &Value,
    ) -> Result<Value, RuntimeError> {
        if self.busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let operation = request["payload"]["operation"]
            .as_str()
            .unwrap_or("preview");
        let gateway_id = request["payload"]["gatewayID"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("gateway id"))?;
        let empty_default = GatewayConfig {
            id: "default".into(),
            name: "默认网关".into(),
            providers: Vec::new(),
            failover: crate::config::FailoverPolicy {
                mode: crate::config::FailoverMode::Disabled,
            },
        };
        let existing = self.gateways.iter().position(|item| item.id == gateway_id);
        if existing.is_none() && gateway_id != "default" {
            return Err(RuntimeError::invalid("gateway id"));
        }
        match operation {
            "preview" => {
                let gateway = existing
                    .map(|index| &self.gateways[index])
                    .unwrap_or(&empty_default);
                provider_import::preview(
                    &request["payload"],
                    gateway,
                    &self.options,
                    &self.runtime_instances,
                )
                .map(|preview| json!({"preview":preview,"gateway":crate::api::GatewaySummary::from(gateway)}))
            }
            "apply" => {
                let mut imported_gateway = existing
                    .map(|index| self.gateways[index].clone())
                    .unwrap_or(empty_default);
                let previous = imported_gateway.clone();
                let result = provider_import::apply(
                    &request["payload"],
                    &mut imported_gateway,
                    &self.options,
                    &self.runtime_instances,
                )?;
                if imported_gateway.providers.is_empty() {
                    return Err(RuntimeError::invalid(
                        "provider import selected no providers",
                    ));
                }
                let requires_reconnect = (imported_gateway != previous)
                    && self
                        .active_runtime_id
                        .as_ref()
                        .and_then(|id| {
                            self.runtime_instances
                                .iter()
                                .find(|runtime| &runtime.id == id)
                        })
                        .is_some_and(|runtime| runtime.gateway_id == gateway_id);
                let previous_runtimes = self.runtime_instances.clone();
                if let Some(index) = existing {
                    self.gateways[index] = imported_gateway;
                } else {
                    self.gateways.push(imported_gateway);
                }
                crate::provider_configuration::clear_removed_selections(
                    &self.gateways,
                    &mut self.runtime_instances,
                );
                if let Err(error) = self.persist() {
                    self.runtime_instances = previous_runtimes;
                    if let Some(index) = existing {
                        self.gateways[index] = previous;
                    } else {
                        self.gateways.pop();
                    }
                    return Err(error);
                }
                if requires_reconnect {
                    self.shutdown_active()?;
                }
                Ok(
                    json!({"importedProviderIds":result["importedProviderIds"],"skippedProviderIds":result["skippedProviderIds"],"gateways":self.public_gateways(),"requiresReconnect":requires_reconnect}),
                )
            }
            _ => Err(RuntimeError::invalid("provider import operation")),
        }
    }

    pub(super) fn runtime_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if self.busy() {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let previous = self.runtime_instances.clone();
        let previous_gateways = self.gateways.clone();
        match request["payload"]["operation"].as_str().unwrap_or("list") {
            "upsert" => {
                let raw = request["payload"]["runtimeInstance"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("runtime instance"))?;
                let runtime: RuntimeInstance = serde_json::from_str(raw)?;
                if runtime.id.is_empty()
                    || runtime.name.is_empty()
                    || !matches!(
                        runtime.type_id.as_str(),
                        "pi-1.0.2" | "codex-0.159.3" | "dsh-acp-0.2.0-rc.2"
                    )
                    || (runtime.gateway_id != "default"
                        && !self
                            .gateways
                            .iter()
                            .any(|item| item.id == runtime.gateway_id))
                {
                    return Err(RuntimeError::invalid("runtime instance"));
                }
                runtime
                    .validate_execution_paths()
                    .map_err(RuntimeError::invalid)?;
                if runtime.gateway_id == "default"
                    && !self.gateways.iter().any(|g| g.id == "default")
                {
                    self.gateways.push(GatewayConfig {
                        id: "default".into(),
                        name: "默认网关".into(),
                        providers: Vec::new(),
                        failover: crate::config::FailoverPolicy {
                            mode: crate::config::FailoverMode::Disabled,
                        },
                    });
                }
                self.runtime_instances.retain(|item| item.id != runtime.id);
                self.runtime_instances.push(runtime);
                if let Err(error) = self.persist() {
                    self.runtime_instances = previous.clone();
                    self.gateways = previous_gateways.clone();
                    return Err(error);
                }
            }
            "delete" => {
                let id = request["payload"]["runtimeInstanceID"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
                self.runtime_instances.retain(|item| item.id != id);
                if let Err(error) = self.persist() {
                    self.runtime_instances = previous.clone();
                    self.gateways = previous_gateways.clone();
                    return Err(error);
                }
            }
            "list" => {}
            _ => return Err(RuntimeError::invalid("runtime operation")),
        }
        let requires_reconnect = self.active_runtime_id.as_ref().is_some_and(|id| {
            previous.iter().find(|runtime| &runtime.id == id)
                != self
                    .runtime_instances
                    .iter()
                    .find(|runtime| &runtime.id == id)
        });
        if requires_reconnect {
            self.shutdown_active()?;
        }
        Ok(
            json!({"runtimeInstances":self.runtime_instances,"runtimeTypes":runtime_types(&self.options.resources_directory),"requiresReconnect":requires_reconnect}),
        )
    }

    pub(super) fn persist(&self) -> Result<(), RuntimeError> {
        self.repository.store(&crate::repository::PersistedConfig {
            schema_version: 6,
            model_templates: self.model_templates.clone(),
            gateways: self.gateways.clone(),
            runtime_instances: self.runtime_instances.clone(),
        })
    }
}
