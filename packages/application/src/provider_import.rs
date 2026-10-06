//! Read-only Pi provider/model import preview and transactional application.
//! The JavaScript adapter owns SDK 1.0.2 parsing; Rust only validates the
//! non-secret snapshot and projects supported models into gateway config.
use crate::config::{
    GatewayConfig, GatewayProtocol, ProviderDefinition, ProviderModel, RuntimeInstance,
};
use crate::{Error as RuntimeError, Options as RuntimeOptions};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use velune_agent_runtime::provider_source::{Snapshot, Source};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Preview {
    contract_version: u64,
    source_fingerprint: String,
    target_fingerprint: String,
    sdk_version: String,
    source: Value,
    providers: Vec<Value>,
    warnings: Vec<String>,
}

pub(crate) fn descriptor() -> crate::config::RuntimeTypeDescriptor {
    crate::config::RuntimeTypeDescriptor {
        id: "pi-1.0.2".into(),
        family_id: "pi".into(),
        version_regex: r"^1\.0\.2$".into(),
        supported_protocols: GatewayProtocol::runtime_protocols("pi-1.0.2")
            .expect("registered Pi adapter"),
        name: "Pi Agent 提供商目录".into(),
        fields: Vec::new(),
        actions: vec![
            crate::conversation::SettingAction {
                id: "preview".into(),
                label: "预览提供商与模型".into(),
            },
            crate::conversation::SettingAction {
                id: "apply".into(),
                label: "导入".into(),
            },
        ],
    }
}

fn source(payload: &Value) -> Result<Source, RuntimeError> {
    let value = if payload["source"].is_string() {
        serde_json::from_str(payload["source"].as_str().unwrap_or("{}"))
            .map_err(|_| RuntimeError::invalid("provider import source"))?
    } else {
        payload["source"].clone()
    };
    serde_json::from_value(value).map_err(|_| RuntimeError::invalid("provider import source"))
}
/// A selection references persisted application configuration; callers cannot supply source paths.
fn configured_source(
    payload: &Value,
    runtimes: &[RuntimeInstance],
) -> Result<(Source, String), RuntimeError> {
    let selection = source(payload)?;
    let id = selection
        .source_instance_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| RuntimeError::invalid("请选择已配置的 Agent 运行时"))?;
    if selection.kind != "harness"
        || !selection.settings.is_empty()
        || selection.provider_id.is_some()
    {
        return Err(RuntimeError::invalid(
            "提供商导入只接受已配置运行时的选择，不能覆盖其来源配置",
        ));
    }
    let runtime = runtimes
        .iter()
        .find(|runtime| runtime.id == id)
        .ok_or_else(|| RuntimeError::invalid("所选 Agent 运行时已不存在，请重新选择"))?;
    if runtime.type_id != "pi-1.0.2" || selection.harness_type_id != "pi" {
        return Err(RuntimeError::invalid("所选 Agent 运行时不支持提供商导入"));
    }
    let directory = runtime
        .settings
        .get("agentDir")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| RuntimeError::invalid("请先配置所选运行时的运行时目录"))?;
    let node = runtime
        .settings
        .get("nodeBinary")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| RuntimeError::invalid("请先配置所选运行时的 Node 可执行文件"))?;
    let mut settings = BTreeMap::from([
        ("sourceDir".into(), directory.clone()),
        ("nodeBinary".into(), node.clone()),
    ]);
    for key in ["modelsPath", "authPath"] {
        if let Some(value) = runtime.settings.get(key).filter(|value| !value.is_empty()) {
            settings.insert(key.into(), value.clone());
        }
    }
    let fingerprint = format!("runtime_{:x}", Sha256::digest(serde_json::to_vec(runtime)?));
    Ok((
        Source {
            kind: "harness".into(),
            harness_type_id: "pi".into(),
            source_instance_id: Some(runtime.id.clone()),
            provider_id: None,
            settings,
        },
        fingerprint,
    ))
}
fn combined_source_fingerprint(snapshot: &Snapshot, runtime_fingerprint: &str) -> String {
    format!(
        "pi_source_{:x}",
        Sha256::digest(format!(
            "{}|{}",
            snapshot.source_fingerprint, runtime_fingerprint
        ))
    )
}

fn run_helper(source: &Source, options: &RuntimeOptions) -> Result<Snapshot, RuntimeError> {
    velune_agent_runtime::provider_source::read(source, &options.resources_directory)
        .map_err(RuntimeError::ProviderImport)
}
fn fingerprint(gateway: &impl Serialize) -> Result<String, RuntimeError> {
    let bytes = serde_json::to_vec(gateway)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("gateway_{digest:x}"))
}
fn preview_token(source: &str, target: &str) -> String {
    let digest = Sha256::digest(format!("{source}|{target}").as_bytes());
    format!("pi_preview_{digest:x}")
}
fn stable_id(prefix: &str, source: &str) -> String {
    let digest = Sha256::digest(source.as_bytes());
    let suffix = digest[..10]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{prefix}_{suffix}")
}
fn source_identity(source: &Source, models_path: &str, auth_path: Option<&str>) -> String {
    format!(
        "{}|{}|{}",
        source
            .settings
            .get("sourceDir")
            .or_else(|| source.settings.get("agentDir"))
            .map(String::as_str)
            .unwrap_or(""),
        auth_path
            .or_else(|| source.settings.get("authPath").map(String::as_str))
            .unwrap_or(""),
        source
            .settings
            .get("modelsPath")
            .map(String::as_str)
            .unwrap_or(models_path)
    )
}
fn protocol(value: &str) -> Option<GatewayProtocol> {
    match value {
        "chatCompletionsV1" => Some(GatewayProtocol::ChatCompletionsV1),
        "responsesV1" => Some(GatewayProtocol::ResponsesV1),
        _ => None,
    }
}
fn protocol_name(value: &GatewayProtocol) -> &'static str {
    match value {
        GatewayProtocol::ChatCompletionsV1 => "chatCompletionsV1",
        GatewayProtocol::ResponsesV1 => "responsesV1",
        GatewayProtocol::MessagesV1 => "messagesV1",
    }
}

pub(crate) fn preview(
    source_value: &Value,
    gateway: &GatewayConfig,
    options: &RuntimeOptions,
    runtimes: &[RuntimeInstance],
) -> Result<Value, RuntimeError> {
    let (source, runtime_fingerprint) = configured_source(source_value, runtimes)?;
    let snapshot = run_helper(&source, options)?;
    let source_fingerprint = combined_source_fingerprint(&snapshot, &runtime_fingerprint);
    let target_fingerprint = fingerprint(gateway)?;
    let warnings = snapshot.warnings.clone();
    let mut providers = Vec::new();
    for provider in &snapshot.providers {
        let supported: Vec<_> = provider
            .models
            .iter()
            .filter(|model| {
                model.supported && model.protocol.as_deref().and_then(protocol).is_some()
            })
            .collect();
        let protocol_name = supported
            .first()
            .and_then(|m| m.protocol.clone())
            .or_else(|| provider.models.first().and_then(|m| m.protocol.clone()))
            .unwrap_or_else(|| "unknown".into());
        let provider_id = stable_id(
            "pi_provider",
            &format!(
                "{}|{}|{}|{}",
                provider.source_provider_id,
                provider.endpoint.as_deref().unwrap_or(""),
                protocol_name,
                source_identity(
                    &source,
                    &snapshot.models_path,
                    snapshot.auth_path.as_deref()
                )
            ),
        );
        providers.push(json!({"id":provider_id,"sourceProviderId":provider.source_provider_id,"name":provider.name,"protocol":protocol_name,"endpoint":provider.endpoint,"canImport":!supported.is_empty(),"alreadyImported":gateway.providers.iter().any(|item| item.id == provider_id),"credentialStatus":provider.auth.status_label.clone().unwrap_or_else(|| provider.auth.kind.clone()),"issues":if provider.auth.ready { Vec::<String>::new() } else { vec![provider.auth.reason.clone().unwrap_or_else(|| "需要完成认证".into())] },"models":provider.models.iter().map(|m| json!({"candidateKey":stable_id("pi_candidate", &format!("{}|{}|{}|{}", provider.source_provider_id, m.id, protocol_name, source_identity(&source, &snapshot.models_path, snapshot.auth_path.as_deref()))),"providerModelId":m.id,"name":m.name,"contextWindow":m.context_window,"maxOutputTokens":m.max_tokens,"reasoningLevels":if m.reasoning_levels.is_empty() { vec!["off".to_string()] } else { m.reasoning_levels.clone() },"canImport":m.supported,"issues":if m.unsupported_features.is_empty() { m.unsupported_reason.clone().into_iter().collect() } else { m.unsupported_features.clone() }})).collect::<Vec<_>>() }));
    }
    let preview = Preview {
        contract_version: 1,
        source_fingerprint,
        target_fingerprint,
        sdk_version: snapshot.sdk_version,
        source: json!({"kind":source.kind,"harnessTypeId":source.harness_type_id,"sourceInstanceId":source.source_instance_id,"providerId":source.provider_id,"settings":source.settings}),
        providers,
        warnings,
    };
    let mut value = serde_json::to_value(preview)?;
    value["sourceLabel"] = Value::String("Pi models.json".into());
    value["token"] = Value::String(preview_token(
        value["sourceFingerprint"].as_str().unwrap_or(""),
        value["targetFingerprint"].as_str().unwrap_or(""),
    ));
    Ok(value)
}

pub(crate) fn apply(
    source_value: &Value,
    gateway: &mut GatewayConfig,
    options: &RuntimeOptions,
    runtimes: &[RuntimeInstance],
) -> Result<Value, RuntimeError> {
    let (source, runtime_fingerprint) = configured_source(source_value, runtimes)?;
    let snapshot = run_helper(&source, options)?;
    let source_fingerprint = combined_source_fingerprint(&snapshot, &runtime_fingerprint);
    let replace = source_value["replace"]
        .as_bool()
        .or_else(|| source_value["replaceExisting"].as_bool())
        .or_else(|| {
            source_value["replaceExisting"]
                .as_str()
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(false);
    let target_fingerprint = fingerprint(gateway)?;
    let expected = source_value["previewToken"]
        .as_str()
        .or_else(|| source_value["preview"]["token"].as_str());
    let valid_token =
        expected == Some(preview_token(&source_fingerprint, &target_fingerprint).as_str());
    if !valid_token {
        return Err(RuntimeError::invalid(
            "provider import source or target changed",
        ));
    }
    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    let selections = if source_value["selections"].is_string() {
        serde_json::from_str::<Value>(source_value["selections"].as_str().unwrap_or("[]"))
            .unwrap_or_else(|_| json!([]))
    } else {
        source_value["selections"].clone()
    };
    if !selections.is_array() || selections.as_array().is_some_and(|items| items.is_empty()) {
        return Err(RuntimeError::invalid("provider import selections"));
    }
    let selection_items = selections
        .as_array()
        .ok_or_else(|| RuntimeError::invalid("provider import selections"))?;
    let mut candidate_models = BTreeMap::<String, (String, Vec<String>)>::new();
    for provider in &snapshot.providers {
        let Some(endpoint) = provider.endpoint.as_deref() else {
            continue;
        };
        let Some(protocol_kind) = provider
            .models
            .iter()
            .find(|model| model.supported)
            .and_then(|model| model.protocol.as_deref().and_then(protocol))
        else {
            continue;
        };
        let provider_id = stable_id(
            "pi_provider",
            &format!(
                "{}|{}|{}|{}",
                provider.source_provider_id,
                endpoint,
                protocol_name(&protocol_kind),
                source_identity(
                    &source,
                    &snapshot.models_path,
                    snapshot.auth_path.as_deref()
                )
            ),
        );
        let ids = provider
            .models
            .iter()
            .filter(|model| {
                model.supported && model.protocol.as_deref().and_then(protocol).is_some()
            })
            .map(|model| {
                stable_id(
                    "pi_candidate",
                    &format!(
                        "{}|{}|{}|{}",
                        provider.source_provider_id,
                        model.id,
                        protocol_name(&protocol_kind),
                        source_identity(
                            &source,
                            &snapshot.models_path,
                            snapshot.auth_path.as_deref()
                        )
                    ),
                )
            })
            .collect::<Vec<_>>();
        candidate_models.insert(
            provider.source_provider_id.clone(),
            (provider_id.clone(), ids.clone()),
        );
        candidate_models.insert(provider_id.clone(), (provider_id, ids));
    }
    let mut selected_providers = std::collections::BTreeSet::new();
    for selection in selection_items {
        let provider_id = selection["providerId"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("provider import selection provider"))?;
        let candidate_keys = selection["candidateKeys"]
            .as_array()
            .ok_or_else(|| RuntimeError::invalid("provider import selection models"))?;
        if candidate_keys.is_empty() {
            return Err(RuntimeError::invalid("provider import selection models"));
        }
        let (canonical_id, candidates) = candidate_models
            .get(provider_id)
            .ok_or_else(|| RuntimeError::invalid("provider import selection provider"))?;
        // A source ID and its preview ID name the same provider. Reject repeated
        // selections before mutation rather than silently using only the first.
        if !selected_providers.insert(canonical_id) {
            return Err(RuntimeError::invalid(
                "provider import duplicate provider selection",
            ));
        }
        let mut selected = std::collections::BTreeSet::new();
        for model_record_key in candidate_keys {
            let model_record_key = model_record_key
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("provider import selection model"))?;
            if !selected.insert(model_record_key)
                || !candidates
                    .iter()
                    .any(|candidate| candidate == model_record_key)
            {
                return Err(RuntimeError::invalid("provider import selection model"));
            }
        }
    }
    for provider in snapshot.providers {
        let Some(endpoint) = provider.endpoint.clone() else {
            continue;
        };
        let models: Vec<_> = provider
            .models
            .into_iter()
            .filter(|m| m.supported && m.protocol.as_deref().and_then(protocol).is_some())
            .collect();
        if models.is_empty() {
            continue;
        }
        let Some(protocol_kind) = models[0].protocol.as_deref().and_then(protocol) else {
            continue;
        };
        let provider_id = stable_id(
            "pi_provider",
            &format!(
                "{}|{}|{}|{}",
                provider.source_provider_id,
                endpoint,
                protocol_name(&protocol_kind),
                source_identity(
                    &source,
                    &snapshot.models_path,
                    snapshot.auth_path.as_deref()
                )
            ),
        );
        if gateway.providers.iter().any(|p| p.id == provider_id) && !replace {
            if selection_items.iter().any(|selection| {
                selection["providerId"] == provider_id
                    || selection["providerId"] == provider.source_provider_id
            }) {
                skipped.push(provider_id);
            }
            continue;
        }
        let mut bindings = Vec::new();
        for model in models {
            let candidate_id = stable_id(
                "pi_candidate",
                &format!(
                    "{}|{}|{}|{}",
                    provider.source_provider_id,
                    model.id,
                    protocol_name(&protocol_kind),
                    source_identity(
                        &source,
                        &snapshot.models_path,
                        snapshot.auth_path.as_deref()
                    ),
                ),
            );
            let selection = selections.as_array().and_then(|items| {
                items.iter().find(|item| {
                    (item["providerId"] == provider.source_provider_id
                        || item["providerId"] == provider_id)
                        && item["candidateKeys"].as_array().is_some_and(|ids| {
                            ids.iter()
                                .any(|id| id.as_str() == Some(candidate_id.as_str()))
                        })
                })
            });
            if selections.is_array()
                && selections.as_array().is_some_and(|items| !items.is_empty())
                && selection.is_none()
            {
                continue;
            }
            let model_record_key = gateway
                .providers
                .iter()
                .find(|p| p.id == provider_id)
                .and_then(|p| p.models.iter().find(|m| m.provider_model_id == model.id))
                .map(|m| m.record_key.clone())
                .unwrap_or_else(crate::new_record_key);
            bindings.push(ProviderModel {
                record_key: model_record_key,
                nickname: model.name,
                icon: None,
                provider_model_id: model.id,
                context_window: model.context_window,
                max_output_tokens: model.max_tokens,
                reasoning_levels: Some(
                    model
                        .pi_projection
                        .as_ref()
                        .map(|projection| {
                            projection
                                .thinking_level_map
                                .values()
                                .flatten()
                                .cloned()
                                .collect::<std::collections::BTreeSet<_>>()
                                .into_iter()
                                .collect()
                        })
                        .unwrap_or(model.reasoning_levels),
                ),
                pi_projection: model.pi_projection.clone(),
            });
        }
        // An explicit model selection must not create other source providers.
        if bindings.is_empty() {
            continue;
        }
        let mut settings = source.settings.clone();
        settings.insert("credentialKind".into(), provider.auth.kind.clone());
        settings.insert("modelsPath".into(), snapshot.models_path.clone());
        if let Some(auth_path) = snapshot.auth_path.clone() {
            settings.insert("authPath".into(), auth_path);
        }
        settings.insert(
            "credentialLocation".into(),
            provider
                .auth
                .location
                .clone()
                .unwrap_or_else(|| "auth".into()),
        );
        settings.insert(
            "bindingProtocol".into(),
            protocol_name(&protocol_kind).into(),
        );
        settings.insert("bindingEndpoint".into(), endpoint.clone());
        settings.insert(
            "bindingModelIds".into(),
            serde_json::to_string(
                &bindings
                    .iter()
                    .map(|binding| binding.provider_model_id.clone())
                    .collect::<Vec<_>>(),
            )?,
        );
        let projection_snapshot = bindings
            .iter()
            .filter_map(|binding| {
                binding
                    .pi_projection
                    .as_ref()
                    .map(|execution| (binding.provider_model_id.clone(), execution))
            })
            .collect::<BTreeMap<_, _>>();
        settings.insert(
            "bindingProjection".into(),
            serde_json::to_string(&projection_snapshot)?,
        );
        use crate::provider_authentication::{
            AuthenticationMaterial, AuthenticationMethod, AuthenticationSource,
            ProviderAuthentication,
        };
        let credential_source = AuthenticationSource {
            kind: "harness".into(),
            harness_type_id: "pi".into(),
            source_instance_id: source.source_instance_id.clone(),
            provider_id: provider.source_provider_id,
            settings,
        };
        let method = match provider.auth.kind.as_str() {
            "oauth" => AuthenticationMethod::OAuth,
            "literal_api_key" | "stored_environment" => AuthenticationMethod::ApiKey,
            _ => AuthenticationMethod::Unconfigured,
        };
        let authentication = ProviderAuthentication {
            generation: gateway
                .providers
                .iter()
                .find(|p| p.id == provider_id)
                .map_or(0, |p| p.authentication.generation + 1),
            material: AuthenticationMaterial::RuntimeProvider {
                source: credential_source,
                method,
                configured: provider.auth.ready,
                protocol: protocol_kind.clone(),
                endpoint: endpoint.clone(),
            },
        };
        let mut provider_def = ProviderDefinition {
            id: provider_id.clone(),
            name: provider.name,
            protocol: protocol_kind,
            endpoint,
            authentication,
            models: bindings,
        };
        if matches!(
            &provider_def.authentication.material,
            AuthenticationMaterial::RuntimeProvider {
                method: AuthenticationMethod::ApiKey,
                configured: true,
                ..
            }
        ) {
            let value = crate::authentication_resolver::read_api_key(&provider_def, options)?;
            provider_def.authentication.material = AuthenticationMaterial::ApiKey { value };
        }
        if let AuthenticationMaterial::RuntimeProvider { source, method, .. } =
            &mut provider_def.authentication.material
            && method != &AuthenticationMethod::ApiKey
        {
            // OAuth consumes only its original auth store, not its source model catalog.
            source.settings.retain(|key, _| {
                !key.starts_with("binding") && key != "modelsPath" && key != "credentialLocation"
            });
        }
        gateway.providers.retain(|p| p.id != provider_id);
        gateway.providers.push(provider_def);
        imported.push(provider_id);
    }
    gateway.validate().map_err(RuntimeError::invalid)?;
    Ok(
        json!({"importedProviderIds":imported,"skippedProviderIds":skipped,"sourceFingerprint":snapshot.source_fingerprint,"targetFingerprint":fingerprint(gateway)?}),
    )
}
