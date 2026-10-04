//! Read-only Pi provider/model import preview and transactional application.
//! The JavaScript adapter owns SDK 1.0.2 parsing; Rust only validates the
//! non-secret snapshot and projects supported models into gateway config.
use crate::config::{
    CredentialSource, CredentialSourceKind, GatewayConfig, GatewayProtocol, ModelDefinition,
    ProviderDefinition, ProviderModelBinding,
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

pub(crate) fn descriptor() -> Value {
    json!({"id":"pi","name":"Pi Agent 提供商目录","fields":[
    {"key":"sourceDir","label":"Pi 运行时目录","kind":"directoryPath","required":true,"value":"","options":[],"help":"只读取选定目录中的 models.json/auth.json 元数据。"},
    {"key":"modelsPath","label":"模型配置文件","kind":"filePath","required":false,"value":"","options":[],"help":"留空使用 agentDir/models.json。"},
    {"key":"authPath","label":"认证文件","kind":"filePath","required":false,"value":"","options":[],"help":"只读取凭据类型元数据，不复制凭据值。"},
    {"key":"nodeBinary","label":"Node 可执行文件","kind":"filePath","required":true,"value":"","options":[],"help":"Node 22.19+ 的绝对路径。"}],"actions":[{"id":"preview","label":"预览提供商与模型"},{"id":"apply","label":"导入"}],"capability":"Pi 1.0.2 models.json 合成目录；凭据值永不进入 Velune。"})
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
fn run_helper(source: &Source, options: &RuntimeOptions) -> Result<Snapshot, RuntimeError> {
    velune_agent_runtime::provider_source::read(source, &options.resources_directory)
        .map_err(RuntimeError::Invalid)
}
fn fingerprint(gateway: &GatewayConfig) -> Result<String, RuntimeError> {
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
) -> Result<Value, RuntimeError> {
    let source = source(source_value)?;
    let snapshot = run_helper(&source, options)?;
    let target_fingerprint = fingerprint(gateway)?;
    let mut warnings = Vec::new();
    let mut providers = Vec::new();
    for provider in &snapshot.providers {
        let supported: Vec<_> = provider
            .models
            .iter()
            .filter(|model| {
                model.supported && model.protocol.as_deref().and_then(protocol).is_some()
            })
            .collect();
        if supported.is_empty() {
            warnings.push(format!(
                "provider {} has no supported models",
                provider.source_provider_id
            ));
        }
        if !provider.auth.ready {
            warnings.push(format!(
                "provider {} is draft: {}",
                provider.source_provider_id,
                provider
                    .auth
                    .reason
                    .as_deref()
                    .unwrap_or("authentication required")
            ));
        }
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
        providers.push(json!({"id":provider_id,"sourceProviderId":provider.source_provider_id,"name":provider.name,"protocol":protocol_name,"endpoint":provider.endpoint,"canImport":!supported.is_empty(),"alreadyImported":gateway.providers.iter().any(|item| item.id == provider_id),"credentialStatus":provider.auth.status_label.clone().unwrap_or_else(|| provider.auth.kind.clone()),"issues":if provider.auth.ready { Vec::<String>::new() } else { vec![provider.auth.reason.clone().unwrap_or_else(|| "authentication required".into())] },"models":provider.models.iter().map(|m| json!({"id":stable_id("pi_model", &format!("{}|{}|{}|{}", provider.source_provider_id, m.id, protocol_name, source_identity(&source, &snapshot.models_path, snapshot.auth_path.as_deref()))),"externalModelId":m.id,"name":m.name,"contextWindow":m.context_window,"maxOutputTokens":m.max_tokens,"reasoningLevels":if m.reasoning_levels.is_empty() { vec!["off".to_string()] } else { m.reasoning_levels.clone() },"canImport":m.supported && m.context_window.unwrap_or(0) > 0 && m.max_tokens.unwrap_or(0) > 0,"issues":if m.unsupported_features.is_empty() { m.unsupported_reason.clone().into_iter().collect() } else { m.unsupported_features.clone() }})).collect::<Vec<_>>() }));
    }
    let preview = Preview {
        contract_version: 1,
        source_fingerprint: snapshot.source_fingerprint,
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
) -> Result<Value, RuntimeError> {
    let source = source(source_value)?;
    let snapshot = run_helper(&source, options)?;
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
        expected == Some(preview_token(&snapshot.source_fingerprint, &target_fingerprint).as_str());
    if !valid_token {
        return Err(RuntimeError::invalid(
            "provider import source or target changed",
        ));
    }
    let mut imported = Vec::new();
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
    let mut candidate_models = BTreeMap::<String, Vec<String>>::new();
    for provider in &snapshot.providers {
        let Some(endpoint) = provider.endpoint.as_deref() else {
            continue;
        };
        let Some(protocol_kind) = provider
            .models
            .iter()
            .find(|model| {
                model.supported
                    && model.context_window.unwrap_or(0) > 0
                    && model.max_tokens.unwrap_or(0) > 0
            })
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
                model.supported
                    && model.protocol.as_deref().and_then(protocol).is_some()
                    && model.context_window.unwrap_or(0) > 0
                    && model.max_tokens.unwrap_or(0) > 0
            })
            .map(|model| {
                stable_id(
                    "pi_model",
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
        candidate_models.insert(provider.source_provider_id.clone(), ids.clone());
        candidate_models.insert(provider_id, ids);
    }
    for selection in selection_items {
        let provider_id = selection["providerId"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("provider import selection provider"))?;
        let model_ids = selection["modelIds"]
            .as_array()
            .ok_or_else(|| RuntimeError::invalid("provider import selection models"))?;
        if model_ids.is_empty() {
            return Err(RuntimeError::invalid("provider import selection models"));
        }
        let candidates = candidate_models
            .get(provider_id)
            .ok_or_else(|| RuntimeError::invalid("provider import selection provider"))?;
        let mut selected = std::collections::BTreeSet::new();
        for model_id in model_ids {
            let model_id = model_id
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("provider import selection model"))?;
            if !selected.insert(model_id)
                || !candidates.iter().any(|candidate| candidate == model_id)
            {
                return Err(RuntimeError::invalid("provider import selection model"));
            }
        }
        if let Some(mappings) = selection["modelMappings"].as_object() {
            for (model_id, target_id) in mappings {
                if !selected.iter().any(|selected_id| *selected_id == model_id)
                    || !target_id
                        .as_str()
                        .is_some_and(|target| gateway.model(target).is_some())
                {
                    return Err(RuntimeError::invalid("provider import model mapping"));
                }
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
            .filter(|m| {
                m.supported
                    && m.protocol.as_deref().and_then(protocol).is_some()
                    && m.context_window.unwrap_or(0) > 0
                    && m.max_tokens.unwrap_or(0) > 0
            })
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
            continue;
        }
        let mut bindings = Vec::new();
        for model in models {
            let candidate_id = stable_id(
                "pi_model",
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
                        && item["modelIds"].as_array().is_some_and(|ids| {
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
            let model_id = selection
                .and_then(|item| item["modelMappings"][&candidate_id].as_str())
                .filter(|id| !id.is_empty())
                .unwrap_or(&candidate_id)
                .to_owned();
            if gateway.model(&model_id).is_none() {
                gateway.models.push(ModelDefinition {
                    id: model_id.clone(),
                    nickname: model.name.clone(),
                    icon: None,
                    max_output_tokens: model.max_tokens.unwrap(),
                    context_window: model.context_window,
                    reasoning_levels: if model.reasoning_levels.is_empty() {
                        vec!["off".into()]
                    } else {
                        model.reasoning_levels.clone()
                    },
                });
            }
            bindings.push(ProviderModelBinding {
                model_id: model_id.clone(),
                external_model_id: model.id,
                pi_projection: model.pi_projection.clone(),
            });
        }
        let mut settings = source.settings.clone();
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
                    .map(|binding| binding.external_model_id.clone())
                    .collect::<Vec<_>>(),
            )?,
        );
        let projection_snapshot = bindings
            .iter()
            .filter_map(|binding| {
                binding
                    .pi_projection
                    .as_ref()
                    .map(|execution| (binding.external_model_id.clone(), execution))
            })
            .collect::<BTreeMap<_, _>>();
        settings.insert(
            "bindingProjection".into(),
            serde_json::to_string(&projection_snapshot)?,
        );
        let credential_source = CredentialSource {
            kind: CredentialSourceKind::Harness,
            harness_type_id: "pi".into(),
            source_instance_id: source.source_instance_id.clone(),
            provider_id: provider.source_provider_id,
            settings,
        };
        let provider_def = ProviderDefinition {
            id: provider_id.clone(),
            name: provider.name,
            protocol: protocol_kind,
            endpoint,
            credential_ref: None,
            credential_source: Some(credential_source),
            credential_generation: 0,
            models: bindings,
        };
        gateway.providers.retain(|p| p.id != provider_id);
        gateway.providers.push(provider_def);
        imported.push(provider_id);
    }
    gateway.validate().map_err(RuntimeError::invalid)?;
    Ok(
        json!({"importedProviderIds":imported,"sourceFingerprint":snapshot.source_fingerprint,"targetFingerprint":fingerprint(gateway)?}),
    )
}
