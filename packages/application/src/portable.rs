//! Configuration-only application composition without local runtime dependencies.
use crate::{
    Error, Options,
    config::{GatewayConfig, RuntimeInstance},
    repository::{PersistedConfig, Repository},
};
use serde_json::{Value, json};
pub(crate) struct CoreRuntime {
    repository: Repository,
    config: PersistedConfig,
}
impl CoreRuntime {
    pub(crate) fn open(options: Options) -> Result<Self, Error> {
        options.validate()?;
        let (repository, config) = Repository::open(&options.home_directory)?;
        Ok(Self { repository, config })
    }
    pub(crate) fn close_if_idle(&mut self) -> Result<(), Error> {
        self.repository.unlock()
    }
    pub(crate) fn request_inner(&mut self, request: &Value) -> Result<Value, Error> {
        match request["action"].as_str() {
            Some("list") => Ok(
                json!({"conversations":[],"connections":[],"models":self.config.gateways.iter().flat_map(|g|&g.models).collect::<Vec<_>>(),"gateways":self.config.gateways,"runtimeInstances":self.config.runtime_instances,"runtimeTypes":[],"authenticationBindings":crate::authentication_resources::AuthenticationManager {resources:self.config.authentication_bindings.clone()}.descriptors(),"providerImportTypes":[],"protocols":[{"id":"chatCompletionsV1","name":"OpenAI Chat Completions v1","supported":true},{"id":"responsesV1","name":"OpenAI Responses v1","supported":true}],"activeRuntimeInstanceID":null}),
            ),
            Some("authenticationResources") => {
                let manager = crate::authentication_resources::AuthenticationManager {
                    resources: self.config.authentication_bindings.clone(),
                };
                let pending =
                    manager.prepare_mutation(&request["payload"], &self.config.gateways)?;
                let result = serde_json::to_value(pending.result)?;
                let previous = self.config.clone();
                self.config.authentication_bindings = pending.manager.resources;
                if let Err(error) = self.repository.store(&self.config) {
                    self.config = previous;
                    return Err(error);
                }
                Ok(result)
            }
            Some("gateways") => {
                let previous = self.config.clone();
                match request["payload"]["operation"].as_str() {
                    Some("upsert") => {
                        let mut gateway: GatewayConfig = serde_json::from_str(
                            request["payload"]["gateway"]
                                .as_str()
                                .ok_or_else(|| Error::invalid("gateway"))?,
                        )?;
                        gateway.assign_record_keys();
                        gateway.validate().map_err(Error::invalid)?;
                        self.config.gateways.retain(|g| g.id != gateway.id);
                        self.config.gateways.push(gateway);
                    }
                    Some("delete") => {
                        let id = request["payload"]["gatewayID"]
                            .as_str()
                            .ok_or_else(|| Error::invalid("gateway id"))?;
                        if self
                            .config
                            .runtime_instances
                            .iter()
                            .any(|r| r.gateway_id == id)
                        {
                            return Err(Error::invalid("gateway is used by a runtime"));
                        }
                        self.config.gateways.retain(|g| g.id != id);
                    }
                    _ => return Err(Error::invalid("gateway operation")),
                }
                if let Err(error) = self.repository.store(&self.config) {
                    self.config = previous;
                    return Err(error);
                }
                Ok(
                    json!({"gateways":self.config.gateways,"models":self.config.gateways.iter().flat_map(|g|&g.models).collect::<Vec<_>>(),"requiresReconnect":false}),
                )
            }
            Some("runtimeInstances") => {
                let previous = self.config.clone();
                match request["payload"]["operation"].as_str() {
                    Some("upsert") => {
                        let instance: RuntimeInstance = serde_json::from_str(
                            request["payload"]["runtimeInstance"]
                                .as_str()
                                .ok_or_else(|| Error::invalid("runtime instance"))?,
                        )?;
                        if instance.id.is_empty()
                            || instance.name.is_empty()
                            || instance.type_id.is_empty()
                            || !self
                                .config
                                .gateways
                                .iter()
                                .any(|g| g.id == instance.gateway_id)
                        {
                            return Err(Error::invalid("runtime instance"));
                        }
                        instance
                            .validate_execution_paths()
                            .map_err(Error::invalid)?;
                        self.config
                            .runtime_instances
                            .retain(|r| r.id != instance.id);
                        self.config.runtime_instances.push(instance);
                    }
                    Some("delete") => {
                        let id = request["payload"]["runtimeInstanceID"]
                            .as_str()
                            .ok_or_else(|| Error::invalid("runtime instance id"))?;
                        self.config.runtime_instances.retain(|r| r.id != id);
                    }
                    _ => return Err(Error::invalid("runtime operation")),
                }
                if let Err(error) = self.repository.store(&self.config) {
                    self.config = previous;
                    return Err(error);
                }
                Ok(
                    json!({"runtimeInstances":self.config.runtime_instances,"runtimeTypes":[],"requiresReconnect":false}),
                )
            }
            _ => Err(Error::Unsupported(
                "local-runtime capability is not linked".into(),
            )),
        }
    }
}
