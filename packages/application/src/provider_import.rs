//! Read-only Pi provider/model import preview and transactional application.
//! The JavaScript adapter owns SDK 1.0.2 parsing; Rust only validates the
//! non-secret snapshot and projects supported models into gateway config.
use crate::config::RuntimeKind;
use crate::config::{GatewayConfig, Protocol, ProviderDefinition, ProviderModel, RuntimeInstance};
use crate::{
    Error as RuntimeError, ImportModelCandidate, ImportProviderCandidate, ImportSelection,
    Options as RuntimeOptions, ProviderImportPreview, ProviderImportSource,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use velune_agent_runtime::provider_source::{Snapshot, Source};

/// What an applied import changed in the target gateway.
pub(crate) struct Applied {
    pub(crate) imported: Vec<String>,
    pub(crate) skipped: Vec<String>,
}

pub(crate) fn descriptor() -> crate::config::RuntimeTypeDescriptor {
    crate::config::RuntimeTypeDescriptor {
        id: RuntimeKind::Pi,
        family_id: "pi".into(),
        compatible_versions: velune_agent_runtime::version::PI_VERSIONS.describe(),
        can_rename_conversations: true,
        can_delete_conversations: true,
        launch_options: false,
        agent_info: false,
        session_options: false,
        native_account_quota: false,
        permission_modes: Vec::new(),
        remote_resources: true,
        supported_protocols: crate::config::runtime_protocols(RuntimeKind::Pi),
        supported_provider_protocols: crate::config::runtime_provider_protocols(RuntimeKind::Pi),
        name: "Pi Agent 提供商目录".into(),
        fields: Vec::new(),
    }
}

fn source(selection: &ProviderImportSource) -> Source {
    Source {
        kind: selection.kind.clone(),
        harness_type_id: selection.harness_type_id.clone(),
        source_instance_id: selection.source_instance_id.clone(),
        provider_id: selection.provider_id.clone(),
        settings: selection.settings.clone(),
    }
}
/// The selected Codex or Claude Code instance, whose files the agent runtime
/// reads directly; Pi goes through its SDK adapter instead.
fn native_runtime<'a>(
    selection: &ProviderImportSource,
    runtimes: &'a [RuntimeInstance],
) -> Result<Option<&'a RuntimeInstance>, RuntimeError> {
    let Some(runtime) = runtimes.iter().find(|runtime| {
        Some(runtime.id.as_str()) == selection.source_instance_id.as_deref()
            && crate::native_provider_import::TYPES.contains(&runtime.type_id)
    }) else {
        return Ok(None);
    };
    if selection.kind != "harness"
        || !selection.settings.is_empty()
        || selection.provider_id.is_some()
        || selection.harness_type_id != crate::native_provider_import::family(runtime)
    {
        return Err(RuntimeError::invalid(
            "提供商导入只接受已配置运行时的选择，不能覆盖其来源配置",
        ));
    }
    Ok(Some(runtime))
}
/// A selection references persisted application configuration; callers cannot supply source paths.
fn configured_source(
    selection: &ProviderImportSource,
    runtimes: &[RuntimeInstance],
) -> Result<(Source, String), RuntimeError> {
    let selection = source(selection);
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
    if runtime.execution()? != crate::RuntimeExecution::default() {
        return Err(RuntimeError::invalid(
            "提供商导入目前只支持使用既有本机 HOME 的运行时；远端与 managed HOME 可直接使用原生提供商",
        ));
    }
    if runtime.type_id != RuntimeKind::Pi || selection.harness_type_id != "pi" {
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
    let binary = runtime
        .settings
        .get("binary")
        .ok_or_else(|| RuntimeError::invalid("请先配置所选运行时的入口"))?;
    let binary = velune_agent_runtime::version::resolve_pi_binary(
        std::path::Path::new(binary),
        std::path::Path::new(node),
    )
    .map_err(|error| RuntimeError::Invalid(error.to_string()))?;
    let sdk_version = velune_agent_runtime::version::check_version(
        runtime.type_id,
        &binary,
        Some(std::path::Path::new(node)),
    )
    .map_err(|_| RuntimeError::invalid("所选 Pi 安装版本与适配器不匹配"))?;
    let mut settings = BTreeMap::from([
        ("sourceDir".into(), directory.clone()),
        ("nodeBinary".into(), node.clone()),
        ("binary".into(), binary.to_string_lossy().into_owned()),
        ("runtimeTypeId".into(), runtime.type_id.to_string()),
        ("sdkVersion".into(), sdk_version),
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
fn protocol(value: &str) -> Option<Protocol> {
    match value {
        "chatCompletionsV1" => Some(Protocol::ChatCompletions),
        "responsesV1" => Some(Protocol::Responses),
        "messagesV1" => Some(Protocol::Messages),
        _ => None,
    }
}
pub(crate) fn protocol_name(value: &Protocol) -> &'static str {
    match value {
        Protocol::ChatCompletions => "chatCompletionsV1",
        Protocol::Responses => "responsesV1",
        Protocol::Messages => "messagesV1",
    }
}

pub(crate) fn preview(
    selection: &ProviderImportSource,
    gateway: &GatewayConfig,
    options: &RuntimeOptions,
    runtimes: &[RuntimeInstance],
) -> Result<ProviderImportPreview, RuntimeError> {
    if let Some(runtime) = native_runtime(selection, runtimes)? {
        return crate::native_provider_import::preview(runtime, gateway);
    }
    let (source, runtime_fingerprint) = configured_source(selection, runtimes)?;
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
        let identity = source_identity(
            &source,
            &snapshot.models_path,
            snapshot.auth_path.as_deref(),
        );
        let models = provider
            .models
            .iter()
            .map(|m| ImportModelCandidate {
                candidate_key: stable_id(
                    "pi_candidate",
                    &format!(
                        "{}|{}|{}|{}",
                        provider.source_provider_id, m.id, protocol_name, identity
                    ),
                ),
                provider_model_id: m.id.clone(),
                name: m.name.clone(),
                context_window: m.context_window,
                max_output_tokens: m.max_tokens,
                reasoning_levels: if m.reasoning_levels.is_empty() {
                    vec!["off".to_string()]
                } else {
                    m.reasoning_levels.clone()
                },
                can_import: m.supported,
                issues: if m.unsupported_features.is_empty() {
                    m.unsupported_reason.clone().into_iter().collect()
                } else {
                    m.unsupported_features.clone()
                },
            })
            .collect();
        providers.push(ImportProviderCandidate {
            already_imported: gateway.providers.iter().any(|item| item.id == provider_id),
            id: provider_id,
            source_provider_id: provider.source_provider_id.clone(),
            name: provider.name.clone(),
            protocol: protocol_name,
            endpoint: provider.endpoint.clone(),
            can_import: !supported.is_empty(),
            credential_status: provider
                .auth
                .status_label
                .clone()
                .unwrap_or_else(|| provider.auth.kind.clone()),
            issues: if provider.auth.ready {
                Vec::new()
            } else {
                vec![
                    provider
                        .auth
                        .reason
                        .clone()
                        .unwrap_or_else(|| "需要完成认证".into()),
                ]
            },
            models,
        });
    }
    Ok(ProviderImportPreview {
        contract_version: 1,
        token: preview_token(&source_fingerprint, &target_fingerprint),
        source_fingerprint,
        target_fingerprint,
        sdk_version: snapshot.sdk_version,
        source: ProviderImportSource {
            kind: source.kind,
            harness_type_id: source.harness_type_id,
            source_instance_id: source.source_instance_id,
            provider_id: source.provider_id,
            settings: source.settings,
        },
        providers,
        warnings,
        source_label: "Pi models.json".into(),
    })
}

pub(crate) fn apply(
    selection: &ProviderImportSource,
    preview_token_value: &str,
    selections: &[ImportSelection],
    replace: bool,
    gateway: &mut GatewayConfig,
    options: &RuntimeOptions,
    runtimes: &[RuntimeInstance],
) -> Result<Applied, RuntimeError> {
    if let Some(runtime) = native_runtime(selection, runtimes)? {
        return crate::native_provider_import::apply(
            preview_token_value,
            selections,
            replace,
            runtime,
            gateway,
            runtimes,
        );
    }
    let (source, runtime_fingerprint) = configured_source(selection, runtimes)?;
    let snapshot = run_helper(&source, options)?;
    let source_fingerprint = combined_source_fingerprint(&snapshot, &runtime_fingerprint);
    let target_fingerprint = fingerprint(gateway)?;
    if preview_token_value != preview_token(&source_fingerprint, &target_fingerprint) {
        return Err(RuntimeError::invalid(
            "provider import source or target changed",
        ));
    }
    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    if selections.is_empty() {
        return Err(RuntimeError::invalid("provider import selections"));
    }
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
    for selection in selections {
        let provider_id = selection.provider_id.as_str();
        let candidate_keys = &selection.candidate_keys;
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
            if selections.iter().any(|selection| {
                selection.provider_id == provider_id
                    || selection.provider_id == provider.source_provider_id
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
            let selected = selections.iter().any(|item| {
                (item.provider_id == provider.source_provider_id || item.provider_id == provider_id)
                    && item.candidate_keys.contains(&candidate_id)
            });
            if !selected {
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
                protocol: protocol_kind,
                endpoint: endpoint.clone(),
            },
        };
        let mut provider_def = ProviderDefinition {
            quota_authentication: Default::default(),
            id: provider_id.clone(),
            name: provider.name,
            protocol: protocol_kind,
            endpoint,
            execution_backend: crate::config::ExecutionBackend::Http,
            request_profile: crate::config::RequestProfile::Native,
            quota_source: crate::config::ProviderQuotaSource::None,
            authentication,
            models: bindings,
            quota_limits: gateway
                .providers
                .iter()
                .find(|p| p.id == provider_id)
                .map(|p| p.quota_limits.clone())
                .unwrap_or_default(),
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
    gateway.prune_routes();
    gateway.validate().map_err(RuntimeError::invalid)?;
    Ok(Applied { imported, skipped })
}
