//! Provider import from a local Codex or Claude Code runtime instance. The
//! agent runtime reads what the harness configures; this maps it into the
//! gateway: an API becomes an HTTP provider (a literal key is copied, as Pi
//! import does for keys), and Claude Code's login is referenced, not copied.
use crate::Error;
use crate::config::RuntimeKind;
use crate::config::{
    ExecutionBackend, GatewayConfig, GatewayProtocol, ProviderDefinition, ProviderModel,
    ProviderQuotaSource, RequestProfile, RuntimeInstance,
};
use crate::provider_authentication::{
    AuthenticationMaterial, LinkedClaudeLogin, ProviderAuthentication,
};

use sha2::{Digest, Sha256};
use velune_agent_runtime::provider_inventory::{self, Access, Inventory, KeySource, Protocol};

/// Runtime types whose configuration the agent runtime can read for import.
pub(crate) const TYPES: [RuntimeKind; 2] = [RuntimeKind::Codex, RuntimeKind::ClaudeCode];

fn inventory(runtime: &RuntimeInstance) -> Result<Inventory, Error> {
    if runtime.execution()? != crate::RuntimeExecution::default() {
        return Err(Error::invalid(
            "提供商导入目前只支持使用既有本机 HOME 的运行时；远端与 managed HOME 可直接使用原生提供商",
        ));
    }
    let directory = runtime
        .settings
        .get("agentDir")
        .map(std::path::Path::new)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| Error::invalid("请先为所选运行时配置绝对路径的运行时目录"))?;
    match runtime.type_id {
        RuntimeKind::Codex => provider_inventory::codex(directory),
        RuntimeKind::ClaudeCode => provider_inventory::claude_code(directory),
        RuntimeKind::Pi | RuntimeKind::DeepSeek | RuntimeKind::Acp => {
            return Err(Error::invalid("所选 Agent 运行时不支持提供商导入"));
        }
    }
    .map_err(|error| Error::Invalid(error.0))
}
fn digest(prefix: &str, parts: &[&str]) -> String {
    let digest = Sha256::digest(parts.join("\0").as_bytes());
    let suffix = digest[..10]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{prefix}_{suffix}")
}
fn provider_id(runtime: &RuntimeInstance, source_provider_id: &str) -> String {
    let directory = runtime.settings.get("agentDir").map_or("", String::as_str);
    digest(
        "native_provider",
        &[&runtime.id, directory, source_provider_id],
    )
}
fn candidate_key(provider_id: &str, model: &str) -> String {
    digest("native_candidate", &[provider_id, model])
}
fn target_fingerprint(gateway: &GatewayConfig) -> Result<String, Error> {
    Ok(format!(
        "gateway_{:x}",
        Sha256::digest(serde_json::to_vec(gateway)?)
    ))
}
fn token(source: &str, target: &str) -> String {
    format!(
        "native_preview_{:x}",
        Sha256::digest(format!("{source}|{target}").as_bytes())
    )
}
fn protocol(value: Protocol) -> GatewayProtocol {
    match value {
        Protocol::Responses => GatewayProtocol::ResponsesV1,
        Protocol::Messages => GatewayProtocol::MessagesV1,
    }
}
pub(crate) fn family(runtime: &RuntimeInstance) -> &'static str {
    match runtime.type_id {
        RuntimeKind::Codex => "codex",
        RuntimeKind::ClaudeCode | RuntimeKind::Pi | RuntimeKind::DeepSeek | RuntimeKind::Acp => {
            "claude-code"
        }
    }
}
fn cli_binary(runtime: &RuntimeInstance) -> Option<&str> {
    runtime
        .settings
        .get("binary")
        .map(String::as_str)
        .filter(|path| std::path::Path::new(path).is_absolute())
}

pub(crate) fn preview(
    runtime: &RuntimeInstance,
    gateway: &GatewayConfig,
) -> Result<crate::ProviderImportPreview, Error> {
    let inventory = inventory(runtime)?;
    let target = target_fingerprint(gateway)?;
    let providers = inventory
        .providers
        .iter()
        .map(|provider| {
            let id = provider_id(runtime, &provider.source_provider_id);
            let mut issues = provider.issues.clone();
            let (protocol_name, endpoint, credential) = match &provider.access {
                Access::Http {
                    protocol: kind,
                    endpoint,
                    key,
                } => (
                    crate::provider_import::protocol_name(&protocol(*kind)),
                    Some(endpoint.clone()),
                    match key {
                        KeySource::Literal { .. } => "复制来源中的 API key",
                        KeySource::Environment { .. } | KeySource::Helper | KeySource::Missing => {
                            "导入后填写 API key"
                        }
                    },
                ),
                Access::ClaudeLogin => {
                    if cli_binary(runtime).is_none() {
                        issues.push("请先为该运行时配置 Claude Code 可执行文件的绝对路径。".into());
                    }
                    (
                        crate::provider_import::protocol_name(&GatewayProtocol::MessagesV1),
                        None,
                        "沿用该运行时的 Claude Code 登录",
                    )
                }
            };
            let can_import = !provider.models.is_empty()
                && !matches!(provider.access, Access::ClaudeLogin if cli_binary(runtime).is_none());
            crate::ImportProviderCandidate {
                models: provider
                    .models
                    .iter()
                    .map(|model| crate::ImportModelCandidate {
                        candidate_key: candidate_key(&id, model),
                        provider_model_id: model.clone(),
                        name: model.clone(),
                        context_window: None,
                        max_output_tokens: None,
                        reasoning_levels: Vec::new(),
                        can_import: true,
                        issues: Vec::new(),
                    })
                    .collect(),
                already_imported: gateway.providers.iter().any(|p| p.id == id),
                id,
                source_provider_id: provider.source_provider_id.clone(),
                name: provider.name.clone(),
                protocol: protocol_name.into(),
                endpoint,
                can_import,
                credential_status: credential.into(),
                issues,
            }
        })
        .collect();
    Ok(crate::ProviderImportPreview {
        contract_version: 1,
        token: token(&inventory.fingerprint, &target),
        source_fingerprint: inventory.fingerprint,
        target_fingerprint: target,
        sdk_version: runtime.type_id.to_string(),
        source: crate::ProviderImportSource {
            kind: "harness".into(),
            harness_type_id: family(runtime).into(),
            source_instance_id: Some(runtime.id.clone()),
            provider_id: None,
            settings: Default::default(),
        },
        providers,
        warnings: inventory.warnings,
        source_label: inventory.source_label,
    })
}

pub(crate) fn apply(
    preview_token: &str,
    selections: &[crate::ImportSelection],
    replace: bool,
    runtime: &RuntimeInstance,
    gateway: &mut GatewayConfig,
    runtimes: &[RuntimeInstance],
) -> Result<crate::provider_import::Applied, Error> {
    let inventory = inventory(runtime)?;
    if preview_token != token(&inventory.fingerprint, &target_fingerprint(gateway)?) {
        return Err(Error::invalid("provider import source or target changed"));
    }
    if selections.is_empty() {
        return Err(Error::invalid("provider import selections"));
    }
    let mut chosen = Vec::new();
    for selection in selections {
        let selected_id = selection.provider_id.as_str();
        let provider = inventory
            .providers
            .iter()
            .find(|p| provider_id(runtime, &p.source_provider_id) == selected_id)
            .ok_or_else(|| Error::invalid("provider import selection provider"))?;
        if chosen
            .iter()
            .any(|(id, _, _): &(String, _, _)| id == selected_id)
        {
            return Err(Error::invalid(
                "provider import duplicate provider selection",
            ));
        }
        let keys = &selection.candidate_keys;
        if keys.is_empty() {
            return Err(Error::invalid("provider import selection models"));
        }
        let mut models = Vec::new();
        for key in keys {
            let model = provider
                .models
                .iter()
                .find(|model| candidate_key(selected_id, model) == *key)
                .ok_or_else(|| Error::invalid("provider import selection model"))?;
            if models.contains(model) {
                return Err(Error::invalid("provider import selection model"));
            }
            models.push(model.clone());
        }
        chosen.push((selected_id.to_owned(), provider, models));
    }
    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    for (id, source, models) in chosen {
        let existing = gateway.providers.iter().find(|p| p.id == id);
        if existing.is_some() && !replace {
            skipped.push(id);
            continue;
        }
        let generation = existing.map_or(0, |p| p.authentication.generation + 1);
        let models = models
            .into_iter()
            .map(|model| ProviderModel {
                record_key: existing
                    .and_then(|p| p.models.iter().find(|m| m.provider_model_id == model))
                    .map_or_else(crate::new_record_key, |m| m.record_key.clone()),
                provider_model_id: model,
                nickname: String::new(),
                icon: None,
                context_window: None,
                max_output_tokens: None,
                reasoning_levels: None,
                pi_projection: None,
            })
            .collect();
        let quota_limits = existing.map(|p| p.quota_limits.clone()).unwrap_or_default();
        let mut provider = match &source.access {
            Access::Http {
                protocol: kind,
                endpoint,
                key,
            } => ProviderDefinition {
                quota_authentication: Default::default(),
                id: id.clone(),
                name: source.name.clone(),
                protocol: protocol(*kind),
                endpoint: endpoint.clone(),
                execution_backend: ExecutionBackend::Http,
                request_profile: RequestProfile::Native,
                quota_source: ProviderQuotaSource::None,
                authentication: ProviderAuthentication {
                    material: AuthenticationMaterial::ApiKey {
                        value: match key {
                            KeySource::Literal { value, .. } => value.expose().to_owned(),
                            KeySource::Environment { .. }
                            | KeySource::Helper
                            | KeySource::Missing => String::new(),
                        },
                    },
                    generation,
                },
                models,
                quota_limits,
            },
            Access::ClaudeLogin => {
                let binary = cli_binary(runtime).ok_or_else(|| {
                    Error::invalid("请先为该运行时配置 Claude Code 可执行文件的绝对路径")
                })?;
                let login = LinkedClaudeLogin::Runtime {
                    instance_id: runtime.id.clone(),
                };
                ProviderDefinition {
                    quota_authentication: Default::default(),
                    id: id.clone(),
                    name: format!("{} · {}", source.name, runtime.name),
                    protocol: GatewayProtocol::MessagesV1,
                    endpoint: String::new(),
                    execution_backend: ExecutionBackend::ClaudeCli {
                        binary: binary.to_owned(),
                    },
                    request_profile: RequestProfile::Native,
                    quota_source: ProviderQuotaSource::ClaudeCliUsage,
                    authentication: ProviderAuthentication {
                        material: AuthenticationMaterial::ClaudeCliLinked {
                            config_dir: crate::claude_cli::linked_directory(&login, runtimes)?,
                            login,
                            configured: false,
                            account_label: None,
                            identity: None,
                        },
                        generation,
                    },
                    models,
                    quota_limits,
                }
            }
        };
        if let ExecutionBackend::ClaudeCli { binary } = &provider.execution_backend {
            let binary = binary.clone();
            crate::claude_cli::refresh_linked(&mut provider.authentication.material, &binary)?;
        }
        gateway.providers.retain(|p| p.id != id);
        gateway.providers.push(provider);
        crate::claude_cli::reject_duplicate_account(std::slice::from_ref(gateway), &id)?;
        imported.push(id);
    }
    gateway.prune_routes();
    gateway.validate().map_err(Error::invalid)?;
    Ok(crate::provider_import::Applied { imported, skipped })
}
