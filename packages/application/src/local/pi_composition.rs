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
    let provider = config
        .provider
        .as_deref()
        .ok_or_else(|| RuntimeError::invalid("gateway provider"))?;
    fs::create_dir_all(catalog_dir)?;
    let marker = catalog_dir.join(".velune-managed");
    if target.exists() && !marker.exists() {
        return Err(RuntimeError::invalid("refusing unmanaged runtime catalog"));
    }
    // Materialize each runnable provider model so Pi can switch using the same gateway revision.
    let entries = gateway
        .providers
        .iter().flat_map(|p| &p.models)
        .map(|model| -> Result<Value, RuntimeError> {
            let physical_id = gateway
                .pi_binding_id(&model.record_key, gateway.authentication_revision(&model.record_key))
                .map_err(RuntimeError::invalid)?;
            let routed = gateway
                .resolve_dispatch(&model.record_key)
                .map_err(RuntimeError::invalid)?;
            let api = match routed.provider().protocol {
                GatewayProtocol::ChatCompletionsV1 => "openai-completions",
                GatewayProtocol::ResponsesV1 => "openai-responses",
                GatewayProtocol::MessagesV1 => "anthropic-messages",
            };
            let binding = routed.model();
            let binding_projection = binding.pi_projection.as_ref();
            let declared_levels = pi_declared_levels(binding);
            let mut entry = json!({
                "id": physical_id,
                "modelRecordKey": model.record_key,
                "name": if model.nickname.trim().is_empty() { &model.provider_model_id } else { &model.nickname },
                "input": ["text"],
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
            if api == "anthropic-messages" {
                // The Anthropic SDK appends /v1/messages to its base URL.
                entry["baseUrl"] = json!(endpoint.strip_suffix("/v1").expect("gateway base path"));
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
                        declared_levels.iter()
                            .map(|level| (level.clone(), Some(level.clone())))
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
    let temporary = target.with_extension("json.velune.tmp");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, target)?;
    fs::write(marker, b"velune managed models config v1\n")?;
    Ok(())
}

pub(super) fn write_selection_file(
    config: &PiConfig,
    model_record_key: &str,
    physical_model_id: &str,
    subscription_capability: bool,
) -> Result<(), RuntimeError> {
    let path = config
        .selection_file
        .as_ref()
        .ok_or_else(|| RuntimeError::invalid("runtime selection file"))?;
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| RuntimeError::invalid("runtime selection directory"))?,
    )?;
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(&json!({
        "provider": "velune-gateway",
        "modelRecordKey": model_record_key,
        "physicalModelId": physical_model_id,
        "thinkingLevel": "off",
        "subscriptionCapability": subscription_capability
    }))?;
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
}

pub(super) fn validate_session_cwd(cwd: &Path) -> Result<(), RuntimeError> {
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err(RuntimeError::invalid("conversation working directory"));
    }
    Ok(())
}

pub(super) fn pi_session_helper(
    config: &PiConfig,
    session: Option<&Path>,
) -> Result<Value, RuntimeError> {
    history::read_pi(config, session).map_err(|error| {
        RuntimeError::context("Pi 历史无法读取；请检查实例路径与 SDK 配置后重试", error)
    })
}

/// Translate the protocol declaration at the Pi adapter boundary, not in the AI model domain.
fn pi_declared_levels(binding: &crate::config::ProviderModel) -> Vec<String> {
    const PI_LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
    let Some(reasoning) = &binding.reasoning_levels else {
        return Vec::new();
    };
    if let Some(projection) = &binding.pi_projection {
        projection
            .thinking_level_map
            .iter()
            .filter(|(level, _)| PI_LEVELS.contains(&level.as_str()))
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
            .collect()
    }
}
