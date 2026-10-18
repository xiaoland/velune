//! Configuration-only composition; shares provider edits without local execution.
use crate::{
    Error, Options,
    config::{GatewayConfig, RuntimeInstance},
    provider_authentication::AuthenticationMaterial,
    provider_configuration,
    repository::{PersistedConfig, Repository},
};
use serde_json::{Value, json};

pub(crate) struct CoreRuntime {
    repository: Repository,
    config: PersistedConfig,
    client_baseline: Option<ClientState>,
}
impl CoreRuntime {
    pub(crate) fn open(
        options: Options,
        _analytics: std::sync::Arc<crate::analytics::AnalyticsStore>,
    ) -> Result<Self, Error> {
        options.validate()?;
        let (repository, config) = Repository::open(&options.home_directory)?;
        Ok(Self {
            repository,
            config,
            client_baseline: None,
        })
    }
    pub(crate) fn close_if_idle(&mut self, _stop_work: bool) -> Result<(), Error> {
        self.repository.unlock()
    }
    fn snapshot(&self) -> Value {
        json!({
            "conversations":[], "conversationGroups":[], "conversationProjects":[],
            "showsArchived":false, "historyFailures":[],
            "gateways":provider_configuration::summaries(&self.config.gateways),
            "runtimeInstances":self.config.runtime_instances, "runtimeTypes":[],
            "modelTemplates":self.config.model_templates, "providerImportTypes":[],
            "conversationBrowserLimits":{"page":self.config.conversation_browser_group_limit,
                "total":self.config.conversation_browser_total_limit},
            "outlinePresentation":self.config.outline_presentation,
            "statusLine":self.config.status_line,
            "conversationBrowserPreferences":self.config.conversation_browser_preferences,
            "protocols":[{"id":"chatCompletionsV1","name":"OpenAI Chat Completions v1","supported":true},
                {"id":"responsesV1","name":"OpenAI Responses v1","supported":true},
                {"id":"messagesV1","name":"Anthropic Messages","supported":true}],
            "selectedRuntimeInstanceID":null
        })
    }
    pub(crate) fn request_inner(&mut self, request: &Value) -> Result<Value, Error> {
        let payload = &request["payload"];
        match request["action"].as_str() {
            Some("list") => Ok(self.snapshot()),
            Some("outlinePresentation") => {
                let mut next = self.config.clone();
                next.outline_presentation =
                    serde_json::from_value(payload["presentation"].clone())?;
                self.persist_config(&next)?;
                self.config = next;
                Ok(json!(self.config.outline_presentation))
            }
            Some("statusLine") => {
                let mut next = self.config.clone();
                next.status_line = crate::config::canonical_status_line(serde_json::from_value(
                    payload["items"].clone(),
                )?);
                self.persist_config(&next)?;
                self.config = next;
                Ok(json!(self.config.status_line))
            }
            Some("conversationBrowserPreferences") => {
                let mut next = self.config.clone();
                next.conversation_browser_preferences =
                    serde_json::from_value(payload["preferences"].clone())?;
                self.persist_config(&next)?;
                self.config = next;
                Ok(self.snapshot())
            }
            Some("conversationBrowserSettings") => {
                let limits: crate::api::ConversationBrowserLimits =
                    serde_json::from_value(payload["limits"].clone())?;
                if limits.page == 0 || limits.total == 0 {
                    return Err(Error::invalid("会话加载数量必须大于零"));
                }
                let mut next = self.config.clone();
                next.conversation_browser_group_limit = limits.page;
                next.conversation_browser_total_limit = limits.total;
                self.persist_config(&next)?;
                self.config = next;
                Ok(json!(limits))
            }
            Some("providers") => {
                if matches!(
                    payload["operation"].as_str(),
                    Some("quota" | "refreshQuota")
                ) {
                    return Err(Error::Unsupported(
                        "subscription quota collection requires local-runtime".into(),
                    ));
                }

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
                provider_configuration::edit_provider(&mut next.gateways, payload, &|_| {
                    Err(Error::invalid("沿用 Claude Code 登录需要本机运行环境"))
                })?;
                self.persist_config(&next)?;
                self.config = next;
                Ok(
                    json!({"gateways":provider_configuration::summaries(&self.config.gateways),
                    "executionInvalidated":false}),
                )
            }
            Some("modelTemplates") => {
                let mut next = self.config.clone();
                provider_configuration::edit_template(&mut next.model_templates, payload)?;
                self.persist_config(&next)?;
                self.config = next;
                Ok(json!(self.config.model_templates))
            }
            Some("acpLaunch") => {
                let id = payload["runtimeInstanceID"]
                    .as_str()
                    .ok_or_else(|| Error::invalid("runtime instance id"))?;
                let runtime = self
                    .config
                    .runtime_instances
                    .iter()
                    .find(|runtime| runtime.id == id)
                    .ok_or_else(|| Error::invalid("runtime instance id"))?;
                Ok(serde_json::to_value(runtime.acp_launch()?)?)
            }
            Some("runtimeExecution" | "configureRuntimeExecution") => {
                let id = payload["runtimeInstanceID"]
                    .as_str()
                    .ok_or_else(|| Error::invalid("runtime instance id"))?;
                let mut runtime = self
                    .config
                    .runtime_instances
                    .iter()
                    .find(|runtime| runtime.id == id)
                    .cloned()
                    .ok_or_else(|| Error::invalid("runtime instance id"))?;
                if request["action"] == "runtimeExecution" {
                    return Ok(serde_json::to_value(runtime.execution()?)?);
                }
                runtime.set_execution(serde_json::from_value(payload["execution"].clone())?)?;
                self.request_inner(&json!({"action":"runtimeInstances","payload":{"operation":"upsert","runtimeInstance":serde_json::to_string(&runtime)?}}))
            }
            Some("nextTurnModelSelection") => Ok(Value::Null),
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
                                routes: Vec::new(),
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
                self.persist_config(&next)?;
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

#[derive(Clone)]
pub(crate) struct ClientState {
    preferences: crate::config::ConversationBrowserPreferences,
    limits: crate::ConversationBrowserLimits,
    outline: crate::config::OutlinePresentation,
    status: Vec<crate::config::StatusLineItem>,
}
impl CoreRuntime {
    pub(crate) fn retain_private_runtime_settings(&self, runtime: &mut RuntimeInstance) {
        if !runtime.settings.contains_key("acpLaunch")
            && let Some(launch) = self
                .config
                .runtime_instances
                .iter()
                .find(|stored| stored.id == runtime.id && stored.type_id == runtime.type_id)
                .and_then(|stored| stored.settings.get("acpLaunch"))
        {
            runtime.settings.insert("acpLaunch".into(), launch.clone());
        }
    }
    pub(crate) fn client_state(&self) -> ClientState {
        ClientState {
            preferences: self.config.conversation_browser_preferences.clone(),
            limits: crate::ConversationBrowserLimits {
                page: self.config.conversation_browser_group_limit,
                total: self.config.conversation_browser_total_limit,
            },
            outline: self.config.outline_presentation,
            status: self.config.status_line.clone(),
        }
    }
    pub(crate) fn enter_client_state(&mut self, state: &mut ClientState, isolated: bool) {
        self.client_baseline = isolated.then(|| self.client_state());
        self.swap_client_state(state);
    }
    pub(crate) fn leave_client_state(&mut self, state: &mut ClientState) {
        self.swap_client_state(state);
        self.client_baseline = None;
    }
    fn persist_config(&self, config: &PersistedConfig) -> Result<(), Error> {
        let mut stored = config.clone();
        if let Some(baseline) = &self.client_baseline {
            stored.conversation_browser_preferences = baseline.preferences.clone();
            stored.conversation_browser_group_limit = baseline.limits.page;
            stored.conversation_browser_total_limit = baseline.limits.total;
            stored.outline_presentation = baseline.outline;
            stored.status_line = baseline.status.clone();
        }
        self.repository.store(&stored)
    }
    fn swap_client_state(&mut self, state: &mut ClientState) {
        std::mem::swap(
            &mut self.config.conversation_browser_preferences,
            &mut state.preferences,
        );
        std::mem::swap(
            &mut self.config.conversation_browser_group_limit,
            &mut state.limits.page,
        );
        std::mem::swap(
            &mut self.config.conversation_browser_total_limit,
            &mut state.limits.total,
        );
        std::mem::swap(&mut self.config.outline_presentation, &mut state.outline);
        std::mem::swap(&mut self.config.status_line, &mut state.status);
    }
    pub(crate) fn advance(&mut self) -> Result<(), Error> {
        Ok(())
    }
}
