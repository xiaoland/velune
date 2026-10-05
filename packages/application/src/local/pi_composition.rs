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
) -> Result<&'a ModelDefinition, RuntimeError> {
    let provider = gateway
        .validate_dispatch(id)
        .map_err(RuntimeError::invalid)?;
    let model = gateway.model(id).expect("validated model");
    if model.context_window.is_none() {
        return Err(RuntimeError::invalid(
            "请先在模型设置中填写上下文窗口（tokens）",
        ));
    }
    if model.reasoning_levels.iter().any(|level| {
        !["off", "minimal", "low", "medium", "high", "xhigh", "max"].contains(&level.as_str())
    }) {
        return Err(RuntimeError::invalid(
            "当前 Pi 适配器不支持该模型的推理等级",
        ));
    }
    if let Some(projection) = provider
        .models
        .iter()
        .find(|binding| binding.model_id == id)
        .and_then(|binding| binding.pi_projection.as_ref())
        && projection.reasoning_enabled
        && projection
            .supported_levels(&model.reasoning_levels)
            .is_empty()
    {
        return Err(RuntimeError::invalid(
            "Pi 适配器与模型当前推理等级没有共同能力",
        ));
    }
    Ok(model)
}

pub(super) fn materialize_models(
    config: &PiConfig,
    gateway: &GatewayConfig,
    endpoint: &str,
    protocol: &str,
    authentication: &crate::authentication_resources::AuthenticationManager,
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
    // Pi caches its available model catalog at startup. Include every explicit
    // route so subsequent set_model calls use the same immutable gateway revision.
    let entries = gateway
        .models
        .iter()
        .filter(|model| runnable_pi_model(gateway, &model.id).is_ok())
        .map(|model| -> Result<Value, RuntimeError> {
            let physical_id = gateway
                .pi_binding_id(&model.id, authentication.revision(gateway,&model.id))
                .map_err(RuntimeError::invalid)?;
            let api =
                gateway
                    .validate_dispatch(&model.id)
                    .ok()
                    .map(|provider| match provider.protocol {
                        GatewayProtocol::ChatCompletionsV1 => "openai-completions",
                        GatewayProtocol::ResponsesV1 => "openai-responses",
                        GatewayProtocol::MessagesV1 => "unsupported",
                    });
            let binding_projection = gateway
                .routes
                .iter()
                .find(|route| route.model_id == model.id)
                .and_then(|route| {
                    gateway
                        .providers
                        .iter()
                        .find(|provider| provider.id == route.provider_id)
                })
                .and_then(|provider| {
                    provider
                        .models
                        .iter()
                        .find(|binding| binding.model_id == model.id)
                })
                .and_then(|binding| binding.pi_projection.as_ref());
            let mut entry = json!({
                "id": physical_id,
                "logicalModelId": model.id,
                "name": model.nickname,
                "input": ["text"],
                "maxTokens": model.max_output_tokens,
                "contextWindow": model.context_window.expect("runnable model context window"),
            });
            if let Some(api) = api {
                entry["api"] = Value::String(api.into());
            }
            let supported_levels = binding_projection
                .map(|projection| projection.supported_levels(&model.reasoning_levels))
                .unwrap_or_else(|| model.reasoning_levels.clone());
            if !supported_levels.is_empty() {
                entry["reasoning"] = Value::Bool(true);
                entry["compat"] = json!({"supportsReasoningEffort": true});
                let levels = binding_projection
                    .map(|projection| {
                        projection.catalog_thinking_level_map(&model.reasoning_levels)
                    })
                    .unwrap_or_else(|| {
                        model
                            .reasoning_levels
                            .iter()
                            .map(|level| (level.clone(), Some(level.clone())))
                            .collect()
                    });
                let levels = levels
                    .into_iter()
                    .map(|(level, value)| (level, value.map(Value::String).unwrap_or(Value::Null)))
                    .collect::<serde_json::Map<_, _>>();
                entry["thinkingLevelMap"] = Value::Object(levels);
            }
            if let Some(projection) = binding_projection {
                if let Some(compat) = &projection.completions_compat {
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
    logical_model_id: &str,
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
        "logicalModelId": logical_model_id,
        "physicalModelId": physical_model_id,
        "modelId": physical_model_id,
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
    let helper = config
        .sdk_helper
        .as_ref()
        .ok_or_else(|| RuntimeError::invalid("Pi session helper is not configured"))?;
    let mut command = Command::new(
        config
            .node_binary
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("node")),
    );
    command.arg(helper);
    if let Some(path) = session {
        command.arg("--inspect-session").arg(path);
    } else {
        command.arg("--all");
    }
    if let Some(cwd) = &config.working_dir {
        // Pi records the physical process cwd; use the same path for SDK filtering.
        command.arg("--cwd").arg(fs::canonicalize(cwd)?);
    }
    if let Some(session_dir) = &config.session_dir {
        command.arg("--session-dir").arg(session_dir);
    }
    if let Some(agent_dir) = &config.agent_dir {
        command.env("PI_CODING_AGENT_DIR", agent_dir);
    }
    let output = command.output().map_err(RuntimeError::Io)?;
    if !output.status.success() {
        return Err(RuntimeError::invalid("Pi session helper failed"));
    }
    serde_json::from_slice(&output.stdout).map_err(RuntimeError::Json)
}
