//! Local application configuration use cases.
use super::*;
impl CoreRuntime {
    pub(super) fn set_conversation_browser_group_limit(
        &mut self,
        request: &Value,
    ) -> Result<Value, RuntimeError> {
        let limit = request["payload"]["limit"]
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| RuntimeError::invalid("每组会话加载数量必须大于零"))?;
        let previous = self.conversation_browser_group_limit;
        self.conversation_browser_group_limit = limit;
        if let Err(error) = self.persist() {
            self.conversation_browser_group_limit = previous;
            return Err(error);
        }
        Ok(json!(limit))
    }
    pub(super) fn list(&self) -> Result<Value, RuntimeError> {
        let mut conversations = Vec::new();
        let mut history_failures = Vec::new();
        for (slot, runtime) in self.runtime_instances.iter().enumerate() {
            if !runtime.enabled {
                continue;
            }
            match self.summaries_for(runtime) {
                Ok(mut summaries) => conversations.append(&mut summaries),
                Err(_) => {
                    tracing::warn!(target:"velune_application",event="runtime_history_read_failed",phase="history_list",failure_kind="history_unavailable",runtime_slot=slot+1);
                    history_failures.push(crate::api::HistoryFailure {
                        runtime_id: runtime.id.clone(),
                        detail:
                            "此实例的会话历史无法读取，请检查运行时目录、Node 与 SDK 配置后重试。"
                                .into(),
                    });
                }
            }
        }
        // Active native drafts are legitimate current sessions even before their
        // runtime writes history. They disappear when that native session closes.
        if matches!(self.active_state, ActiveState::Pi | ActiveState::Native(_))
            && let Some(snapshot) = self.native_snapshot()
            && !conversations
                .iter()
                .any(|item| item.id == snapshot.conversation.id)
        {
            conversations.push(snapshot.conversation);
        }
        let mut conversations = self.linked_summaries(conversations);
        conversations
            .sort_by_key(|conversation| std::cmp::Reverse(conversation.updated_at_unix_ms));
        Ok(json!({
            "conversations": conversations,
            "historyFailures": history_failures,
            "gateways": self.public_gateways(),
            "runtimeInstances": self.runtime_instances,
            "runtimeTypes": runtime_types(),
            "modelTemplates":self.model_templates,
            "conversationBrowserGroupLimit":self.conversation_browser_group_limit,
            "providerImportTypes": [provider_import::descriptor()],
            "protocols": [
                {"id":"chatCompletionsV1","name":"OpenAI Chat Completions v1","supported":true},
                {"id":"responsesV1","name":"OpenAI Responses v1","supported":true},
                {"id":"messagesV1","name":"Anthropic Messages","supported":true}
            ],
            "selectedRuntimeInstanceID": self.next_turn_runtime_id,
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
                let execution_invalidated = (imported_gateway != previous)
                    && self
                        .next_turn_runtime_id
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
                if let Err(error) = self.persist() {
                    self.runtime_instances = previous_runtimes;
                    if let Some(index) = existing {
                        self.gateways[index] = previous;
                    } else {
                        self.gateways.pop();
                    }
                    return Err(error);
                }
                if execution_invalidated {
                    self.invalidate_execution()?;
                }
                Ok(
                    json!({"importedProviderIds":result["importedProviderIds"],"skippedProviderIds":result["skippedProviderIds"],"gateways":self.public_gateways(),"executionInvalidated":execution_invalidated}),
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
                if runtime.enabled {
                    runtime
                        .validate_execution_paths()
                        .map_err(RuntimeError::invalid)?;
                }
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
        let source_ids: Vec<String> = if let Some(projection) = &self.logical_projection {
            projection
                .link
                .segments
                .iter()
                .map(|s| s.runtime_instance_id.clone())
                .collect()
        } else {
            self.native_snapshot()
                .map(|view| vec![view.conversation.runtime_id])
                .unwrap_or_default()
        };
        let changed = |id: &String| {
            previous.iter().find(|runtime| &runtime.id == id)
                != self
                    .runtime_instances
                    .iter()
                    .find(|runtime| &runtime.id == id)
        };
        let source_changed = source_ids.iter().any(|id| {
            let origin = |runtime: &RuntimeInstance| {
                (
                    runtime.type_id.clone(),
                    runtime.enabled,
                    runtime.settings.clone(),
                )
            };
            previous
                .iter()
                .find(|runtime| &runtime.id == id)
                .map(origin)
                != self
                    .runtime_instances
                    .iter()
                    .find(|runtime| &runtime.id == id)
                    .map(origin)
        });
        let execution_invalidated =
            source_changed || self.execution_runtime_id.as_ref().is_some_and(changed);
        if execution_invalidated {
            self.invalidate_execution()?;
            if source_changed {
                // A native ID is scoped to the source configuration used to
                // load it. Editing that source requires an explicit reopen.
                self.active_state = ActiveState::Empty;
                self.logical_projection = None;
                self.model_record_key = None;
            }
        }
        if self.next_turn_runtime_id.as_ref().is_some_and(|id| {
            !self
                .runtime_instances
                .iter()
                .any(|runtime| &runtime.id == id && runtime.enabled)
        }) {
            self.next_turn_runtime_id = None;
        }
        Ok(
            json!({"runtimeInstances":self.runtime_instances,"runtimeTypes":runtime_types(),"executionInvalidated":execution_invalidated}),
        )
    }

    pub(super) fn persist(&self) -> Result<(), RuntimeError> {
        self.repository.store(&crate::repository::PersistedConfig {
            schema_version: 7,
            model_templates: self.model_templates.clone(),
            gateways: self.gateways.clone(),
            runtime_instances: self.runtime_instances.clone(),
            conversation_browser_group_limit: self.conversation_browser_group_limit,
        })
    }
}
