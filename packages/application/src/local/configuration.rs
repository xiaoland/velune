//! Local application configuration use cases.
use super::*;
impl CoreRuntime {
    pub(super) fn list(&self) -> Result<Value, RuntimeError> {
        Ok(json!({
            "conversations": self.session_summaries()?,
            "connections": self.connections(),
            "models": self.gateways.iter().flat_map(|item| item.models.iter()).collect::<Vec<_>>(),
            "gateways": self.gateways,
            "runtimeInstances": self.runtime_instances,
            "runtimeTypes": runtime_types(&self.options.resources_directory),
            "credentialSourceTypes": [{
                "id":"pi",
                "name":"Pi Agent 认证来源",
                "fields":[
                    {"key":"providerId","label":"认证提供商","kind":"text","required":true,"value":"","options":[],"help":"Pi provider ID；认证动作会根据指定来源能力显示。"},
                    {"key":"authPath","label":"认证文件","kind":"filePath","required":true,"value":"","options":[],"help":"保留 Pi 原生认证来源文件路径；Velune 不复制凭据。"},
                    {"key":"nodeBinary","label":"Node 可执行文件","kind":"filePath","required":true,"value":"","options":[],"executableDiscovery":{"command":"node","minimumVersion":"22.19.0"},"help":"Node 22.19+ 的绝对路径。"}
                ],
                "actions":[{"id":"login","label":"登录…"}],
                "capability":"读取指定 Pi 认证来源；Velune 不复制认证资料。"
            }],
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
        if self.pi_busy {
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
            models: Vec::new(),
            providers: Vec::new(),
            routes: Vec::new(),
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
                .map(|preview| json!({"preview":preview,"gateway":gateway}))
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
                if let Some(index) = existing {
                    self.gateways[index] = imported_gateway;
                } else {
                    self.gateways.push(imported_gateway);
                }
                if let Err(error) = self.persist() {
                    if let Some(index) = existing {
                        self.gateways[index] = previous;
                    } else {
                        self.gateways.pop();
                    }
                    return Err(error);
                }
                Ok(
                    json!({"importedProviderIds":result["importedProviderIds"],"skippedProviderIds":[],"gateways":self.gateways,"requiresReconnect":self.active_runtime_id.is_some()}),
                )
            }
            _ => Err(RuntimeError::invalid("provider import operation")),
        }
    }

    pub(super) fn gateway_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let previous = self.gateways.clone();
        let mut changed = false;
        match request["payload"]["operation"].as_str().unwrap_or("list") {
            "upsert" => {
                let raw = request["payload"]["gateway"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("gateway"))?;
                let gateway: GatewayConfig = serde_json::from_str(raw)?;
                gateway.validate().map_err(RuntimeError::invalid)?;
                self.gateways.retain(|item| item.id != gateway.id);
                self.gateways.push(gateway);
                changed = true;
                if let Err(error) = self.persist() {
                    self.gateways = previous.clone();
                    return Err(error);
                }
            }
            "delete" => {
                let id = request["payload"]["gatewayID"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("gateway id"))?;
                if self
                    .runtime_instances
                    .iter()
                    .any(|item| item.gateway_id == id)
                {
                    return Err(RuntimeError::invalid("gateway is used by a runtime"));
                }
                self.gateways.retain(|item| item.id != id);
                changed = true;
                if let Err(error) = self.persist() {
                    self.gateways = previous.clone();
                    return Err(error);
                }
            }
            "list" => {}
            _ => return Err(RuntimeError::invalid("gateway operation")),
        }
        if changed && self.active_runtime_id.is_some() {
            self.shutdown_active()?;
        }
        Ok(
            json!({"gateways":self.gateways,"models":self.gateways.iter().flat_map(|item| item.models.iter()).collect::<Vec<_>>(),"requiresReconnect":changed}),
        )
    }

    pub(super) fn runtime_action(&mut self, request: &Value) -> Result<Value, RuntimeError> {
        if self.pi_busy {
            return Err(RuntimeError::invalid("runtime is busy"));
        }
        let previous = self.runtime_instances.clone();
        let mut changed = false;
        match request["payload"]["operation"].as_str().unwrap_or("list") {
            "upsert" => {
                let raw = request["payload"]["runtimeInstance"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("runtime instance"))?;
                let runtime: RuntimeInstance = serde_json::from_str(raw)?;
                if runtime.id.is_empty()
                    || runtime.name.is_empty()
                    || runtime.type_id != "pi"
                    || !self
                        .gateways
                        .iter()
                        .any(|item| item.id == runtime.gateway_id)
                {
                    return Err(RuntimeError::invalid("runtime instance"));
                }
                runtime
                    .validate_execution_paths()
                    .map_err(RuntimeError::invalid)?;
                self.runtime_instances.retain(|item| item.id != runtime.id);
                self.runtime_instances.push(runtime);
                changed = true;
                if let Err(error) = self.persist() {
                    self.runtime_instances = previous.clone();
                    return Err(error);
                }
            }
            "delete" => {
                let id = request["payload"]["runtimeInstanceID"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("runtime instance id"))?;
                self.runtime_instances.retain(|item| item.id != id);
                changed = true;
                if let Err(error) = self.persist() {
                    self.runtime_instances = previous.clone();
                    return Err(error);
                }
            }
            "list" => {}
            _ => return Err(RuntimeError::invalid("runtime operation")),
        }
        if changed && self.active_runtime_id.is_some() {
            self.shutdown_active()?;
        }
        Ok(
            json!({"runtimeInstances":self.runtime_instances,"runtimeTypes":runtime_types(&self.options.resources_directory),"requiresReconnect":changed}),
        )
    }

    pub(super) fn persist(&self) -> Result<(), RuntimeError> {
        self.repository.store(&crate::repository::PersistedConfig {
            schema_version: 2,
            gateways: self.gateways.clone(),
            runtime_instances: self.runtime_instances.clone(),
        })
    }
}
