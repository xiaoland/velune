//! Configuration-only composition; shares provider edits without local execution.
use crate::{
    Error, Options,
    config::{FailoverMode, FailoverPolicy, GatewayConfig, RuntimeInstance},
    provider_authentication::AuthenticationMaterial,
    provider_configuration,
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
        let payload = &request["payload"];
        match request["action"].as_str() {
            Some("list") => Ok(json!({
                "conversations":[], "historyFailures":[],
                "gateways":provider_configuration::summaries(&self.config.gateways),
                "runtimeInstances":self.config.runtime_instances, "runtimeTypes":[],
                "modelTemplates":self.config.model_templates, "providerImportTypes":[],
                "conversationBrowserGroupLimit":self.config.conversation_browser_group_limit,
                "transcriptPresentation":self.config.transcript_presentation,
                "protocols":[{"id":"chatCompletionsV1","name":"OpenAI Chat Completions v1","supported":true},
                    {"id":"responsesV1","name":"OpenAI Responses v1","supported":true},
                    {"id":"messagesV1","name":"Anthropic Messages","supported":true}],
                "selectedRuntimeInstanceID":null
            })),
            Some("transcriptPresentation") => {
                let mut next = self.config.clone();
                next.transcript_presentation =
                    serde_json::from_value(payload["presentation"].clone())?;
                self.repository.store(&next)?;
                self.config = next;
                Ok(json!(self.config.transcript_presentation))
            }
            Some("conversationBrowserSettings") => {
                let limit = payload["limit"]
                    .as_u64()
                    .and_then(|value| u32::try_from(value).ok())
                    .filter(|value| *value > 0)
                    .ok_or_else(|| Error::invalid("每组会话加载数量必须大于零"))?;
                let mut next = self.config.clone();
                next.conversation_browser_group_limit = limit;
                self.repository.store(&next)?;
                self.config = next;
                Ok(json!(limit))
            }
            Some("providers") => {
                if payload["operation"] == "readApiKey" {
                    let provider = self
                        .config
                        .gateways
                        .iter()
                        .find(|gateway| Some(gateway.id.as_str()) == payload["gatewayID"].as_str())
                        .and_then(|gateway| {
                            gateway.providers.iter().find(|provider| {
                                Some(provider.id.as_str()) == payload["providerID"].as_str()
                            })
                        })
                        .ok_or_else(|| Error::invalid("provider id"))?;
                    return match &provider.authentication.material {
                        AuthenticationMaterial::ApiKey { value } => Ok(json!(value)),
                        _ => Err(Error::invalid("此提供商没有本机保存的 API key")),
                    };
                }
                let mut next = self.config.clone();
                provider_configuration::edit_provider(&mut next.gateways, payload)?;
                self.repository.store(&next)?;
                self.config = next;
                Ok(
                    json!({"gateways":provider_configuration::summaries(&self.config.gateways),
                    "executionInvalidated":false}),
                )
            }
            Some("modelTemplates") => {
                let mut next = self.config.clone();
                provider_configuration::edit_template(&mut next.model_templates, payload)?;
                self.repository.store(&next)?;
                self.config = next;
                Ok(json!(self.config.model_templates))
            }
            Some("runtimeInstances") => {
                let mut next = self.config.clone();
                match payload["operation"].as_str() {
                    Some("upsert") => {
                        let instance: RuntimeInstance = serde_json::from_str(
                            payload["runtimeInstance"]
                                .as_str()
                                .ok_or_else(|| Error::invalid("runtime instance"))?,
                        )?;
                        if instance.gateway_id == "default"
                            && !next.gateways.iter().any(|gateway| gateway.id == "default")
                        {
                            next.gateways.push(GatewayConfig {
                                id: "default".into(),
                                name: "默认网关".into(),
                                providers: Vec::new(),
                                failover: FailoverPolicy {
                                    mode: FailoverMode::Disabled,
                                },
                            });
                        }
                        if instance.id.is_empty()
                            || instance.name.is_empty()
                            || instance.type_id.is_empty()
                            || !next
                                .gateways
                                .iter()
                                .any(|gateway| gateway.id == instance.gateway_id)
                        {
                            return Err(Error::invalid("runtime instance"));
                        }
                        if instance.enabled {
                            instance
                                .validate_execution_paths()
                                .map_err(Error::invalid)?;
                        }
                        next.runtime_instances
                            .retain(|runtime| runtime.id != instance.id);
                        next.runtime_instances.push(instance);
                    }
                    Some("delete") => {
                        let id = payload["runtimeInstanceID"]
                            .as_str()
                            .ok_or_else(|| Error::invalid("runtime instance id"))?;
                        next.runtime_instances.retain(|runtime| runtime.id != id);
                    }
                    _ => return Err(Error::invalid("runtime operation")),
                }
                self.repository.store(&next)?;
                self.config = next;
                Ok(
                    json!({"runtimeInstances":self.config.runtime_instances, "runtimeTypes":[],
                    "executionInvalidated":false}),
                )
            }
            _ => Err(Error::Unsupported(
                "local-runtime capability is not linked".into(),
            )),
        }
    }
}
