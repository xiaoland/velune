//! Pi catalog, selection and session materialization at the application composition boundary.
use super::*;

pub(super) fn setting_path(runtime: &RuntimeInstance, key: &str) -> Result<PathBuf, RuntimeError> {
    setting_path_optional(runtime, key).ok_or_else(|| RuntimeError::invalid("runtime setting path"))
}

pub(super) fn setting_path_optional(runtime: &RuntimeInstance, key: &str) -> Option<PathBuf> {
    runtime
        .settings
        .get(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub(super) fn runnable_pi_model<'a>(
    gateway: &'a GatewayConfig,
    id: &str,
) -> Result<&'a ProviderModel, RuntimeError> {
    Ok(gateway
        .resolve_dispatch(id)
        .map_err(RuntimeError::invalid)?
        .model())
}

pub(super) struct PiManagedModel {
    pub(super) physical_id: String,
    pub(super) aliases: BTreeMap<String, String>,
    pub(super) protocol: &'static str,
}

impl PiManagedModel {
    pub(super) fn resolve(gateway: &GatewayConfig, key: &str) -> Result<Self, RuntimeError> {
        let protocol = pi_gateway_api(
            gateway
                .resolve_dispatch(key)
                .map_err(RuntimeError::invalid)?
                .provider()
                .protocol,
        );
        let binding = |key: &str| {
            gateway
                .pi_binding_id(key, gateway.authentication_revision(key))
                .map_err(RuntimeError::invalid)
        };
        Ok(Self {
            physical_id: binding(key)?,
            aliases: gateway
                .selectable_keys()
                .map(|key| binding(key).map(|alias| (alias, key.to_owned())))
                .collect::<Result<_, _>>()?,
            protocol,
        })
    }
}

pub(super) fn pi_gateway_api(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::ChatCompletions => "openai-completions",
        Protocol::Responses => "openai-responses",
        Protocol::Messages => "anthropic-messages",
        Protocol::Gemini => "google-generative-ai",
    }
}

pub(super) fn pi_compaction_reserves(
    runtime: &RuntimeInstance,
    managed_key: Option<&str>,
) -> Result<BTreeMap<String, u32>, RuntimeError> {
    use crate::ModelSelection;
    use velune_conversation::AutoCompactionSetting;

    if let Some(model_record_key) = managed_key {
        let selection = ModelSelection::Managed {
            model_record_key: model_record_key.to_owned(),
        };
        // Pi checks the physical context window, with settings keyed by the virtual selection.
        return Ok(match runtime.model_auto_compaction(&selection)? {
            Some(AutoCompactionSetting::Pi { reserve_tokens }) => {
                BTreeMap::from([("velune/auto".into(), reserve_tokens)])
            }
            _ => BTreeMap::new(),
        });
    }

    Ok(runtime
        .model_auto_compactions()?
        .into_iter()
        .filter_map(|entry| match (entry.model, entry.setting) {
            (
                ModelSelection::Native {
                    provider_id: Some(provider),
                    model_id,
                },
                AutoCompactionSetting::Pi { reserve_tokens },
            ) => Some((format!("{provider}/{model_id}"), reserve_tokens)),
            _ => None,
        })
        .collect())
}

pub(super) fn materialize_models(
    config: &PiConfig,
    gateway: &GatewayConfig,
    endpoint: &str,
    protocol: &str,
) -> Result<(), RuntimeError> {
    let target = config
        .models_path
        .as_ref()
        .ok_or_else(|| RuntimeError::invalid("runtime catalog path"))?;
    let catalog_dir = target
        .parent()
        .ok_or_else(|| RuntimeError::invalid("runtime catalog directory"))?;
    let provider = "velune-gateway";
    config
        .execution
        .create_dir_all(catalog_dir)
        .map_err(|error| RuntimeError::context("运行时模型目录", error))?;
    let marker = catalog_dir.join(".velune-managed");
    if config
        .execution
        .path_exists(target)
        .map_err(|error| RuntimeError::context("运行时模型目录", error))?
        && !config
            .execution
            .path_exists(&marker)
            .map_err(|error| RuntimeError::context("运行时模型目录", error))?
    {
        return Err(RuntimeError::invalid("refusing unmanaged runtime catalog"));
    }
    // Materialize each runnable provider model so Pi can switch using the same gateway revision.
    // A route takes its first member's parameters under the route's name.
    let entries = gateway
        .selectable_keys()
        .map(|key| -> Result<Value, RuntimeError> {
            let physical_id = gateway
                .pi_binding_id(key, gateway.authentication_revision(key))
                .map_err(RuntimeError::invalid)?;
            let routed = gateway
                .resolve_dispatch(key)
                .map_err(RuntimeError::invalid)?;
            let model = routed.model();
            let name = match gateway.route(key) {
                Some(route) => &route.name,
                None if model.nickname.trim().is_empty() => &model.provider_model_id,
                None => &model.nickname,
            };
            let protocol = routed.provider().protocol;
            let api = pi_gateway_api(protocol);
            let binding = routed.model();
            let binding_projection = binding.pi_projection.as_ref();
            let declared_levels = pi_declared_levels(binding);
            let mut entry = json!({
                "id": physical_id,
                "modelRecordKey": key,
                "name": name,
                // Velune does not know a gateway model's modalities; images
                // reach the upstream, which accepts or refuses them.
                "input": ["text", "image"],
                // `registerProvider` uses Pi's extension-provider path. Unlike
                // models.json composition, that path keeps the model object as
                // provided, while Pi's response accounting reads `cost.tiers`.
                // Keep the neutral zero-cost shape explicit so the injected
                // gateway model has the same complete model contract.
                "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
            });
            if let Some(value) = binding.max_output_tokens {
                entry["maxTokens"] = json!(value);
            }
            if let Some(value) = binding.context_window {
                entry["contextWindow"] = json!(value);
            }
            entry["api"] = Value::String(api.into());
            let origin = endpoint.strip_suffix("/v1").expect("gateway base path");
            match protocol {
                // Anthropic appends its version path. Pi sends Gemini's versioned URL verbatim.
                Protocol::Messages => entry["baseUrl"] = json!(origin),
                Protocol::Gemini => entry["baseUrl"] = json!(format!("{origin}/v1beta")),
                _ => {}
            }
            let supported_levels = binding_projection
                .map(|projection| projection.supported_levels(&declared_levels))
                .unwrap_or_else(|| declared_levels.to_vec());
            if !supported_levels.is_empty() {
                entry["reasoning"] = Value::Bool(true);
                entry["compat"] = json!({"supportsReasoningEffort": true});
                let levels = binding_projection
                    .map(|projection| {
                        projection.catalog_thinking_level_map(&declared_levels)
                    })
                    .unwrap_or_else(|| {
                        // Pi treats absent entries as implicit defaults. Null
                        // prevents it from emitting `none` for models without off.
                        pi::PI_THINKING_LEVELS.into_iter()
                            .map(|level| {
                                let wire = binding
                                    .reasoning_levels
                                    .iter()
                                    .flatten()
                                    .rev()
                                    .find(|wire| wire.as_str() == level || (level == "off" && wire.as_str() == "none"));
                                (level.to_owned(), wire.cloned())
                            })
                            .collect()
                    });
                let levels = levels
                    .into_iter()
                    .map(|(level, value)| (level, value.map(Value::String).unwrap_or(Value::Null)))
                    .collect::<serde_json::Map<_, _>>();
                entry["thinkingLevelMap"] = Value::Object(levels);
            }
            entry["reasoning"] = Value::Bool(!supported_levels.is_empty());
            if let Some(projection) = binding_projection {
                if let Some(compat) = &projection.completions_compat {
                    entry["compat"] = compat.clone();
                }
                if let Some(compat) = &projection.messages_compat {
                    entry["compat"] = compat.clone();
                }
                if let Some(input) = &projection.input {
                    entry["input"] = serde_json::to_value(input)?;
                }
                if let Some(params) = &projection.sampling_params {
                    entry["samplingParams"] = params.clone();
                }
                if let Some(params) = &projection.sampling_params_by_thinking_level {
                    entry["samplingParamsByThinkingLevel"] = params.clone();
                }
            }
            if let Some(field) = binding_projection.and_then(|projection| projection.completions_max_tokens_field) {
                let name = match field {
                    velune_agent_runtime::model_projection::CompletionsMaxTokensField::MaxTokens => "max_tokens",
                    velune_agent_runtime::model_projection::CompletionsMaxTokensField::MaxCompletionTokens => "max_completion_tokens",
                };
                entry["compat"]["maxTokensField"] = Value::String(name.into());
            }
            if let Some(responses_compat) =
                binding_projection.and_then(|projection| projection.responses_compat.as_ref())
            {
                entry["compat"] = serde_json::to_value(responses_compat)?;
            }
            Ok(entry)
        })
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    let mut value = json!({"providers":{}});
    value["providers"][provider] = json!({
        "baseUrl": endpoint,
        "api": protocol,
        "models": entries,
        "apiKey": if config.gateway_token.is_some() {
            Value::String("$VELUNE_GATEWAY_TOKEN".into())
        } else {
            Value::Null
        },
    });
    let bytes = serde_json::to_vec_pretty(&value)?;
    config
        .execution
        .write_file_atomic(target, &bytes)
        .and_then(|()| {
            config
                .execution
                .write_file_atomic(&marker, b"velune managed models config v1\n")
        })
        .map_err(|error| RuntimeError::context("运行时模型配置保存", error))?;
    Ok(())
}

pub(super) fn write_selection_file(
    config: &PiConfig,
    model_record_key: &str,
    physical_model_id: &str,
) -> Result<(), RuntimeError> {
    let path = config
        .selection_file
        .as_ref()
        .ok_or_else(|| RuntimeError::invalid("runtime selection file"))?;
    let bytes = serde_json::to_vec(&json!({
        "provider": "velune-gateway",
        "modelRecordKey": model_record_key,
        "physicalModelId": physical_model_id
    }))?;
    config
        .execution
        .write_file_atomic(path, &bytes)
        .map_err(|error| RuntimeError::context("运行时模型选择保存", error))?;
    Ok(())
}

pub(super) fn pi_session_helper(config: &PiConfig, session: &Path) -> Result<Value, RuntimeError> {
    history::read_pi(config, session).map_err(|error| {
        RuntimeError::context("Pi 历史无法读取；请检查实例路径与 SDK 配置后重试", error)
    })
}

/// Translate the protocol declaration at the Pi adapter boundary, not in the AI model domain.
pub(super) fn pi_declared_levels(binding: &crate::config::ProviderModel) -> Vec<String> {
    let Some(reasoning) = &binding.reasoning_levels else {
        return Vec::new();
    };
    if let Some(projection) = &binding.pi_projection {
        projection
            .thinking_level_map
            .iter()
            .filter(|(level, _)| pi::PI_THINKING_LEVELS.contains(&level.as_str()))
            .filter_map(|(level, wire)| {
                wire.as_ref()
                    .filter(|wire| reasoning.contains(wire))
                    .map(|_| level.clone())
            })
            .collect()
    } else {
        reasoning
            .iter()
            .map(|level| {
                if level == "none" {
                    "off".into()
                } else {
                    level.clone()
                }
            })
            .filter(|level| pi::PI_THINKING_LEVELS.contains(&level.as_str()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ModelSelection;
    use velune_application_api::ModelAutoCompaction;
    use velune_conversation::AutoCompactionSetting;

    #[test]
    fn pi_compaction_reserves_follow_native_identity_and_selected_managed_model() {
        let mut runtime = RuntimeInstance {
            id: "synthetic-pi".into(),
            name: "Synthetic Pi".into(),
            type_id: RuntimeKind::Pi,
            gateway_id: "synthetic-gateway".into(),
            enabled: true,
            settings: BTreeMap::new(),
        };
        let entry = |model, reserve_tokens| ModelAutoCompaction {
            model,
            setting: AutoCompactionSetting::Pi { reserve_tokens },
        };
        let managed = |key: &str| ModelSelection::Managed {
            model_record_key: key.into(),
        };
        let native = |provider: &str| ModelSelection::Native {
            provider_id: Some(provider.into()),
            model_id: "family/model".into(),
        };
        runtime
            .set_model_auto_compactions(vec![
                entry(managed("a"), 10_000),
                entry(managed("b"), 20_000),
                entry(native("first"), 30_000),
                entry(native("second"), 40_000),
            ])
            .unwrap();
        for (key, reserve) in [("a", 10_000), ("b", 20_000)] {
            assert_eq!(
                pi_compaction_reserves(&runtime, Some(key)).unwrap(),
                BTreeMap::from([("velune/auto".into(), reserve)]),
            );
        }
        assert!(
            pi_compaction_reserves(&runtime, Some("other"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            pi_compaction_reserves(&runtime, None).unwrap(),
            BTreeMap::from([
                ("first/family/model".into(), 30_000),
                ("second/family/model".into(), 40_000),
            ]),
        );
        runtime.set_model_auto_compactions(Vec::new()).unwrap();
        assert!(
            pi_compaction_reserves(&runtime, Some("b"))
                .unwrap()
                .is_empty()
        );
        assert!(pi_compaction_reserves(&runtime, None).unwrap().is_empty());
    }
}
