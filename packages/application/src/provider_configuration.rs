//! Pure provider, route and template edits of the gateway configuration.
use crate::{
    Error, GatewaySummary, ProviderDraft, ProviderSummary, config::*, provider_authentication::*,
};

/// One edit of a gateway's providers or routes.
pub(crate) enum ProviderEdit {
    Save {
        provider: ProviderDraft,
        authentication: AuthenticationEdit,
        quota_authentication: Option<crate::QuotaAuthenticationEdit>,
    },
    Delete {
        provider_id: String,
    },
    SaveRoute {
        route: ModelRoute,
    },
    DeleteRoute {
        route_record_key: String,
    },
}

/// `link` resolves a referenced Claude Code login to its `CLAUDE_CONFIG_DIR`
/// (`None` for the user's default).
pub(crate) fn edit_provider(
    gateways: &mut Vec<GatewayConfig>,
    gateway_id: &str,
    edit: ProviderEdit,
    link: &dyn Fn(&LinkedClaudeLogin) -> Result<Option<String>, Error>,
) -> Result<(), Error> {
    let index = match gateways.iter().position(|g| g.id == gateway_id) {
        Some(i) => i,
        None if gateway_id == "default" && matches!(edit, ProviderEdit::Save { .. }) => {
            gateways.push(GatewayConfig {
                id: gateway_id.into(),
                name: "默认网关".into(),
                providers: Vec::new(),
                routes: Vec::new(),
            });
            gateways.len() - 1
        }
        None => return Err(Error::invalid("gateway id")),
    };
    let gateway = &mut gateways[index];
    match edit {
        ProviderEdit::Save {
            provider: mut draft,
            authentication: edit,
            quota_authentication: quota_edit,
        } => {
            if cfg!(windows)
                && matches!(
                    &draft.execution_backend,
                    ExecutionBackend::AntigravityCli {
                        ssh_destination: Some(_),
                        ..
                    }
                )
            {
                return Err(Error::invalid(crate::antigravity_cli::SSH_UNSUPPORTED));
            }
            if draft.authentication_source == ProviderAuthenticationSource::AntigravityCli
                && matches!(edit, AuthenticationEdit::Clear)
            {
                return Err(Error::invalid(
                    "Antigravity 登录由官方 agy 管理，可删除此提供商来解除绑定",
                ));
            }
            if draft.id.is_empty() {
                draft.id = crate::new_record_key();
            }
            let previous = gateway.providers.iter().find(|p| p.id == draft.id);
            let target_changed = previous.is_some_and(|p| {
                p.endpoint != draft.endpoint
                    || p.protocol != draft.protocol
                    || p.request_profile != draft.request_profile
                    || p.quota_source != draft.quota_source
            });
            let mut authentication = previous
                .map(|p| p.authentication.clone())
                .unwrap_or_default();
            let source_changed = authentication.source() != draft.authentication_source;
            let reset = source_changed || matches!(edit, AuthenticationEdit::Clear);
            if reset {
                authentication.material = match draft.authentication_source {
                    ProviderAuthenticationSource::None => AuthenticationMaterial::None,
                    ProviderAuthenticationSource::ApiKey => AuthenticationMaterial::ApiKey {
                        value: String::new(),
                    },
                    ProviderAuthenticationSource::CodexOAuth => {
                        AuthenticationMaterial::CodexSubscription {
                            credential_id: crate::new_record_key(),
                            configured: false,
                        }
                    }
                    ProviderAuthenticationSource::ClaudeCli => AuthenticationMaterial::ClaudeCli {
                        credential_id: crate::new_record_key(),
                        configured: false,
                        account_label: None,
                        identity: None,
                    },
                    ProviderAuthenticationSource::AntigravityCli => {
                        AuthenticationMaterial::AntigravityCli
                    }
                    ProviderAuthenticationSource::RuntimeProvider => {
                        return Err(Error::invalid("运行时认证来源只能由提供商导入建立"));
                    }
                };
            }
            let changed = reset
                || (matches!(
                    authentication.material,
                    AuthenticationMaterial::AntigravityCli
                ) && previous.is_some_and(|p| p.execution_backend != draft.execution_backend))
                || (target_changed
                    && matches!(
                        authentication.material,
                        AuthenticationMaterial::ApiKey { .. }
                    ))
                || matches!(
                    edit,
                    AuthenticationEdit::SetApiKey { .. }
                        | AuthenticationEdit::UseClaudeCodeLogin { .. }
                );
            if let AuthenticationEdit::UseClaudeCodeLogin { login } = &edit {
                if draft.authentication_source != ProviderAuthenticationSource::ClaudeCli {
                    return Err(Error::invalid("只有 Claude Code 订阅可以沿用既有登录"));
                }
                // Signed in by the CLI later; the local composition checks the status.
                authentication.material = AuthenticationMaterial::ClaudeCliLinked {
                    config_dir: link(login)?,
                    login: login.clone(),
                    configured: false,
                    account_label: None,
                    identity: None,
                };
            }
            if let AuthenticationEdit::SetApiKey { value } = edit {
                if draft.authentication_source != ProviderAuthenticationSource::ApiKey
                    || value.trim().is_empty()
                    || value.chars().any(char::is_control)
                {
                    return Err(Error::invalid("API key 来源或内容无效"));
                }
                authentication.material = AuthenticationMaterial::ApiKey { value };
            }
            if changed {
                authentication.generation = authentication
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| Error::invalid("authentication revision overflow"))?;
            }
            let mut quota_authentication = previous
                .map(|p| p.quota_authentication.clone())
                .unwrap_or_default();
            if draft.quota_source == ProviderQuotaSource::ArkCodingUsage {
                quota_authentication.apply(quota_edit, authentication.generation, matches!(&authentication.material, AuthenticationMaterial::ApiKey { value } if !value.is_empty()), target_changed)?;
            } else if quota_edit
                .as_ref()
                .is_some_and(|edit| !matches!(edit, crate::QuotaAuthenticationEdit::Clear))
            {
                return Err(Error::invalid("此额度来源不接受新的独立额度查询认证"));
            } else {
                quota_authentication.apply(
                    quota_edit,
                    authentication.generation,
                    false,
                    target_changed,
                )?;
            }
            let models = draft
                .models
                .into_iter()
                .map(|model| {
                    let projection = previous
                        .filter(|p| p.protocol == draft.protocol)
                        .and_then(|p| {
                            p.models.iter().find(|old| {
                                old.record_key == model.record_key
                                    && old.provider_model_id == model.provider_model_id
                            })
                        })
                        .and_then(|old| old.pi_projection.clone());
                    let mut stored: ProviderModel = model.into();
                    stored.pi_projection = projection;
                    stored
                })
                .collect();
            let mut provider = ProviderDefinition {
                id: draft.id.clone(),
                name: draft.name,
                protocol: draft.protocol,
                endpoint: draft.endpoint,
                execution_backend: draft.execution_backend,
                request_profile: draft.request_profile,
                quota_source: draft.quota_source,
                authentication,
                quota_authentication,
                billing: previous.map(|p| p.billing.clone()).unwrap_or_default(),
                models,
                quota_limits: draft.quota_limits,
                codex_client_version: draft
                    .codex_client_version
                    .as_deref()
                    .map(str::trim)
                    .filter(|version| !version.is_empty())
                    .map(str::to_owned),
            };
            if provider.billing.settings.as_ref().is_some_and(|settings| {
                crate::billing::supported(&provider) != Some(settings.service)
            }) {
                provider.billing = Default::default();
            }
            gateway.providers.retain(|p| p.id != draft.id);
            gateway.providers.push(provider);
            gateway.assign_record_keys();
            gateway.prune_routes();
            gateway.validate().map_err(Error::invalid)?;
        }
        ProviderEdit::Delete { provider_id } => {
            if !gateway.providers.iter().any(|p| p.id == provider_id) {
                return Err(Error::invalid("provider id"));
            }
            gateway.providers.retain(|p| p.id != provider_id);
            gateway.prune_routes();
        }
        ProviderEdit::SaveRoute { mut route } => {
            if route.name.trim().is_empty() {
                return Err(Error::invalid("路由名称不能为空"));
            }
            if route.members.is_empty() {
                return Err(Error::invalid("路由至少需要一个模型"));
            }
            if route.record_key.is_empty() {
                route.record_key = crate::new_record_key();
            }
            match gateway
                .routes
                .iter_mut()
                .find(|r| r.record_key == route.record_key)
            {
                Some(existing) => *existing = route,
                None => gateway.routes.push(route),
            }
            gateway.validate().map_err(|error| {
                Error::invalid(match error {
                    "model route member is duplicated" => "路由中的模型不能重复",
                    "model route member is not a provider model" => "路由中的模型已不存在",
                    other => other,
                })
            })?;
        }
        ProviderEdit::DeleteRoute { route_record_key } => {
            if !gateway
                .routes
                .iter()
                .any(|r| r.record_key == route_record_key)
            {
                return Err(Error::invalid("route record key"));
            }
            gateway.routes.retain(|r| r.record_key != route_record_key);
        }
    }
    validate_ark_bindings(gateways)?;
    Ok(())
}
fn validate_ark_bindings(gateways: &[GatewayConfig]) -> Result<(), Error> {
    let mut keys = std::collections::BTreeSet::new();
    let mut query_keys = std::collections::BTreeSet::new();
    for provider in gateways
        .iter()
        .flat_map(|g| &g.providers)
        .filter(|p| p.request_profile == RequestProfile::ArkCoding)
    {
        if let AuthenticationMaterial::ApiKey { value } = &provider.authentication.material
            && !value.is_empty()
            && !keys.insert(value.as_str())
        {
            return Err(Error::invalid("相同的 ARK 模型 Key 已配置到另一个提供商"));
        }
        if let Some(key) = provider.quota_authentication.access_key()
            && !query_keys.insert(key.id.as_str())
        {
            return Err(Error::invalid(
                "相同的 ARK 查询凭据已关联另一个提供商；同一订阅只配置一个实例",
            ));
        }
    }
    Ok(())
}
pub(crate) fn save_template(
    templates: &mut Vec<ModelTemplate>,
    mut template: ModelTemplate,
) -> Result<(), Error> {
    if template.name.trim().is_empty() {
        return Err(Error::invalid("模板名称不能为空"));
    }
    if template.id.is_empty() {
        template.id = crate::new_record_key();
    }
    validate_template(&template)?;
    templates.retain(|t| t.id != template.id);
    templates.push(template);
    Ok(())
}
pub(crate) fn validate_template(template: &ModelTemplate) -> Result<(), Error> {
    if template.context_window == Some(0) || template.max_output_tokens == Some(0) {
        return Err(Error::invalid("模板能力参数无效"));
    }
    if let Some(levels) = &template.reasoning_levels {
        let mut seen = std::collections::BTreeSet::new();
        if levels
            .iter()
            .any(|l| l.is_empty() || l.chars().any(char::is_control) || !seen.insert(l))
        {
            return Err(Error::invalid("模板推理等级无效"));
        }
    }
    Ok(())
}
pub(crate) fn summaries(gateways: &[GatewayConfig]) -> Vec<GatewaySummary> {
    gateways.iter().map(GatewaySummary::from).collect()
}

#[cfg(test)]
mod antigravity_tests {
    use super::*;
    use crate::authentication_resolver::ProviderResolver;
    use serde_json::json;
    use velune_gateway::{CredentialMaterial, CredentialResolutionError, CredentialTarget};

    fn draft(id: &str, binary: &str) -> ProviderDraft {
        let template = crate::provider_templates()
            .into_iter()
            .find(|template| template.id == "antigravityCliSubscription")
            .unwrap();
        ProviderDraft {
            id: id.into(),
            name: id.into(),
            protocol: template.protocol,
            endpoint: template.endpoint,
            execution_backend: ExecutionBackend::AntigravityCli {
                binary: std::env::temp_dir().join(binary).to_str().unwrap().into(),
                ssh_destination: None,
            },
            request_profile: template.request_profile,
            authentication_source: template.authentication_source,
            quota_source: template.quota_source,
            models: Vec::new(),
            quota_limits: Vec::new(),
            codex_client_version: None,
        }
    }

    fn save(gateways: &mut Vec<GatewayConfig>, provider: ProviderDraft) -> Result<(), Error> {
        edit_provider(
            gateways,
            "default",
            ProviderEdit::Save {
                provider,
                authentication: AuthenticationEdit::Keep,
                quota_authentication: None,
            },
            &|_| unreachable!(),
        )
    }

    #[test]
    fn antigravity_snapshot_binds_to_the_saved_target_without_credentials() {
        let mut gateways = Vec::new();
        let original = draft("agy", "fixture-agy");
        save(&mut gateways, original.clone()).unwrap();
        let saved = serde_json::to_value(&gateways[0]).unwrap();
        assert_eq!(
            saved["providers"][0]["authentication"],
            json!({"material": {"kind": "antigravityCli"}, "generation": 1}),
        );
        assert_eq!(
            saved["providers"][0]["executionBackend"],
            json!(original.execution_backend)
        );
        let loaded: GatewayConfig = serde_json::from_value(saved).unwrap();
        assert_eq!(loaded, gateways[0]);
        let summary = GatewaySummary::from(&loaded).providers.remove(0);
        assert_eq!(
            summary.authentication_source,
            ProviderAuthenticationSource::AntigravityCli
        );
        assert!(summary.authentication.configured);
        assert!(summary.authentication.account_label.is_none());
        assert!(summary.authentication.actions.is_empty());

        let options = crate::Options {
            home_directory: std::env::temp_dir(),
            resources_directory: std::env::temp_dir(),
        };
        let old_reference = loaded.to_gateway_config().unwrap().providers[0]
            .credential_ref
            .clone()
            .unwrap();
        let old_resolver = ProviderResolver::capture(&loaded, &options);
        save(&mut gateways, draft("agy", "fixture-agy-updated")).unwrap();
        let new_reference = gateways[0].to_gateway_config().unwrap().providers[0]
            .credential_ref
            .clone()
            .unwrap();
        assert_ne!(old_reference, new_reference);
        let new_resolver = ProviderResolver::capture(&gateways[0], &options);
        let target = CredentialTarget {
            protocol: Protocol::Messages,
            endpoint: String::new(),
            execution_backend: velune_gateway::ExecutionBackend::AntigravityCli,
            request_profile: velune_gateway::RequestProfile::Native,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            assert!(matches!(
                old_resolver
                    .resolve(new_reference.clone(), target.clone())
                    .await,
                Err(CredentialResolutionError::Unavailable)
            ));
            assert!(matches!(
                new_resolver.resolve(old_reference, target.clone()).await,
                Err(CredentialResolutionError::Unavailable)
            ));
            let resolved = new_resolver.resolve(new_reference, target).await.unwrap();
            assert!(matches!(
                resolved.material,
                CredentialMaterial::AntigravityCli { .. }
            ));
        });
    }

    #[test]
    fn antigravity_gateway_rejects_duplicate_local_targets_and_accepts_distinct_ssh_targets() {
        let mut gateways = Vec::new();
        save(&mut gateways, draft("agy-local", "fixture-agy")).unwrap();
        assert!(save(&mut gateways.clone(), draft("duplicate", "other-agy")).is_err());
        let remote = |id: &str, destination: &str| {
            let mut provider = draft(id, "fixture-agy");
            provider.execution_backend = ExecutionBackend::AntigravityCli {
                binary: "/tools/agy".into(),
                ssh_destination: Some(destination.into()),
            };
            provider
        };
        if cfg!(windows) {
            assert!(save(&mut gateways, remote("agy-remote", "alice@host")).is_err());
            return;
        }
        save(&mut gateways, remote("agy-alice", "alice@host")).unwrap();
        save(&mut gateways, remote("agy-bob", "bob@host")).unwrap();
        assert!(save(&mut gateways.clone(), remote("duplicate", "bob@host")).is_err());
        for invalid in [
            "--flag",
            "@host",
            "user@host@extra",
            "ssh://host",
            "host\nextra",
        ] {
            assert!(save(&mut gateways.clone(), remote("invalid", invalid)).is_err());
        }
    }

    #[test]
    fn antigravity_quota_source_is_fixed_and_legacy_none_remains_readable() {
        let mut gateways = Vec::new();
        let mut provider = draft("agy", "fixture-agy");
        assert_eq!(
            provider.quota_source,
            ProviderQuotaSource::AntigravityCliUsage
        );
        provider.quota_source = ProviderQuotaSource::None;
        save(&mut gateways, provider.clone()).unwrap();
        let saved = serde_json::to_value(&gateways[0]).unwrap();
        assert_eq!(saved["providers"][0]["quotaSource"], "none");
        let loaded: GatewayConfig = serde_json::from_value(saved).unwrap();
        loaded.validate().unwrap();
        assert_eq!(
            GatewaySummary::from(&loaded).providers[0].quota_source,
            ProviderQuotaSource::AntigravityCliUsage,
        );

        let mut limited = provider.clone();
        limited.quota_source = ProviderQuotaSource::AntigravityCliUsage;
        limited.quota_limits.push(QuotaLimit {
            window_id: "fixture".into(),
            max_used_percent: 95,
        });
        assert!(save(&mut gateways.clone(), limited).is_err());

        provider.execution_backend = ExecutionBackend::Http;
        provider.endpoint = "https://quota-fixture.invalid".into();
        provider.authentication_source = ProviderAuthenticationSource::ApiKey;
        provider.quota_source = ProviderQuotaSource::AntigravityCliUsage;
        assert!(save(&mut gateways, provider).is_err());
    }
}

impl From<&GatewayConfig> for GatewaySummary {
    fn from(g: &GatewayConfig) -> Self {
        Self {
            id: g.id.clone(),
            name: g.name.clone(),
            providers: g
                .providers
                .iter()
                .map(|p| ProviderSummary {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    protocol: p.protocol,
                    endpoint: p.endpoint.clone(),
                    execution_backend: p.execution_backend.clone(),
                    request_profile: p.request_profile,
                    authentication_source: p.authentication.source(),
                    quota_source: p.effective_quota_source(),
                    quota_authentication: p
                        .quota_authentication
                        .description(p.authentication.generation),
                    authentication: {
                        let mut description = p.authentication.description();
                        if p.request_profile == crate::config::RequestProfile::KimiCoding
                            || p.quota_source == ProviderQuotaSource::KimiUsage
                        {
                            description.disclosure =
                                Some(crate::SubscriptionDisclosure::kimi_coding_plan());
                        }
                        if p.request_profile == crate::config::RequestProfile::ArkCoding
                            || p.quota_source == ProviderQuotaSource::ArkCodingUsage
                        {
                            description.disclosure =
                                Some(crate::SubscriptionDisclosure::ark_coding_plan());
                        }
                        description
                    },
                    quota_limits: p.quota_limits.clone(),
                    models: p.models.iter().map(Into::into).collect(),
                    codex_client_version: p.codex_client_version.clone(),
                })
                .collect(),
            routes: g.routes.clone(),
        }
    }
}

#[cfg(test)]
mod codex_client_version_tests {
    use super::*;
    use serde_json::Value;

    fn codex_draft() -> ProviderDraft {
        let template = crate::provider_templates()
            .into_iter()
            .find(|template| template.id == "codexSubscription")
            .unwrap();
        ProviderDraft {
            id: "subscription".into(),
            name: "Subscription".into(),
            protocol: template.protocol,
            endpoint: template.endpoint,
            execution_backend: template.execution_backend,
            request_profile: template.request_profile,
            authentication_source: template.authentication_source,
            quota_source: template.quota_source,
            models: vec![],
            quota_limits: vec![],
            codex_client_version: None,
        }
    }

    fn save(gateways: &mut Vec<GatewayConfig>, draft: &ProviderDraft) -> Result<(), Error> {
        edit_provider(
            gateways,
            "default",
            ProviderEdit::Save {
                provider: draft.clone(),
                authentication: AuthenticationEdit::Keep,
                quota_authentication: None,
            },
            &|_| unreachable!(),
        )
        .map(|_| ())
    }

    #[test]
    fn codex_version_defaults_and_persists_without_changing_login() {
        let mut gateways = vec![];
        let mut draft = codex_draft();
        save(&mut gateways, &draft).unwrap();
        let login = gateways[0].providers[0].authentication.clone();
        let automatic = serde_json::to_value(&gateways[0]).unwrap();

        assert!(
            automatic["providers"][0]
                .get("codexClientVersion")
                .is_none()
        );
        let old_config: GatewayConfig = serde_json::from_value(automatic.clone()).unwrap();
        assert_eq!(serde_json::to_value(&old_config).unwrap(), automatic);
        assert_eq!(
            old_config.to_gateway_config().unwrap().providers[0]
                .codex_client_version
                .as_deref(),
            Some(crate::default_codex_client_version()),
        );

        draft.codex_client_version = Some(" 0.166.0 ".into());
        save(&mut gateways, &draft).unwrap();
        let stored = serde_json::to_string(&gateways[0]).unwrap();
        let reloaded: GatewayConfig = serde_json::from_str(&stored).unwrap();
        assert_eq!(
            GatewaySummary::from(&reloaded).providers[0]
                .codex_client_version
                .as_deref(),
            Some("0.166.0"),
        );
        assert_eq!(
            reloaded.to_gateway_config().unwrap().providers[0]
                .codex_client_version
                .as_deref(),
            Some("0.166.0"),
        );

        draft.codex_client_version = Some(" ".into());
        save(&mut gateways, &draft).unwrap();
        let current: Value = serde_json::to_value(&gateways[0]).unwrap();
        assert_eq!(current, automatic);
        assert_eq!(gateways[0].providers[0].authentication, login);
    }

    #[test]
    fn invalid_codex_version_is_rejected() {
        let mut gateways = vec![];
        let mut draft = codex_draft();
        save(&mut gateways, &draft).unwrap();
        for value in ["0.167", "0.167.0-preview", "0.167.0\r\nVersion: 1.0.0"] {
            draft.codex_client_version = Some(value.into());
            assert!(save(&mut gateways.clone(), &draft).is_err());
        }
    }
}

#[cfg(test)]
mod quota_composition_tests {
    use super::*;

    #[test]
    fn switching_quota_source_preserves_private_credentials_but_revokes_confirmation() {
        let template = crate::provider_templates()
            .into_iter()
            .find(|t| t.id == "arkCodingPlan")
            .unwrap();
        let mut draft = ProviderDraft {
            id: "ark".into(),
            name: "Synthetic".into(),
            protocol: template.protocol,
            endpoint: template.endpoint,
            execution_backend: template.execution_backend,
            request_profile: template.request_profile,
            codex_client_version: None,
            authentication_source: template.authentication_source,
            quota_source: template.quota_source,
            models: vec![],
            quota_limits: vec![],
        };
        let mut gateways = Vec::new();
        edit_provider(
            &mut gateways,
            "default",
            ProviderEdit::Save {
                provider: draft.clone(),
                authentication: AuthenticationEdit::SetApiKey {
                    value: "synthetic-model-key".into(),
                },
                quota_authentication: Some(crate::QuotaAuthenticationEdit::SetAccessKey {
                    access_key_id: "synthetic-id".into(),
                    secret_access_key: "synthetic-secret".into(),
                    confirmed: true,
                }),
            },
            &|_| unreachable!(),
        )
        .unwrap();
        assert!(
            gateways[0].providers[0]
                .quota_authentication
                .description(gateways[0].providers[0].authentication.generation)
                .binding_confirmed
        );
        draft.quota_source = ProviderQuotaSource::None;
        edit_provider(
            &mut gateways,
            "default",
            ProviderEdit::Save {
                provider: draft,
                authentication: AuthenticationEdit::Keep,
                quota_authentication: None,
            },
            &|_| unreachable!(),
        )
        .unwrap();
        let saved = &gateways[0].providers[0];
        let status = saved
            .quota_authentication
            .description(saved.authentication.generation);
        assert!(status.configured);
        assert!(!status.binding_confirmed);
        saved.validate_composition().unwrap();
    }

    #[test]
    fn quota_edit_is_independent_of_creation_template_and_survives_reload() {
        let templates = crate::provider_templates();
        let mut gateways = Vec::new();
        for (id, template_id) in [("from-api", "api"), ("from-kimi", "kimiCodingPlan")] {
            let template = templates
                .iter()
                .find(|item| item.id == template_id)
                .unwrap();
            let mut draft = ProviderDraft {
                id: id.into(),
                name: id.into(),
                protocol: template.protocol,
                endpoint: "https://api.kimi.com/coding/".into(),
                execution_backend: template.execution_backend.clone(),
                request_profile: template.request_profile,
                codex_client_version: None,
                authentication_source: template.authentication_source,
                quota_source: template.quota_source,
                models: vec![],
                quota_limits: vec![],
            };
            edit_provider(
                &mut gateways,
                "default",
                ProviderEdit::Save {
                    provider: draft.clone(),
                    authentication: AuthenticationEdit::SetApiKey {
                        value: "synthetic-only".into(),
                    },
                    quota_authentication: None,
                },
                &|_| unreachable!(),
            )
            .unwrap();
            draft.quota_source = ProviderQuotaSource::KimiUsage;
            if template_id == "api" {
                draft.endpoint = "https://api.kimi.com/coding/v1".into();
            }
            edit_provider(
                &mut gateways,
                "default",
                ProviderEdit::Save {
                    provider: draft.clone(),
                    authentication: AuthenticationEdit::Keep,
                    quota_authentication: None,
                },
                &|_| unreachable!(),
            )
            .unwrap();
            let saved = gateways[0].providers.iter().find(|p| p.id == id).unwrap();
            assert_eq!(saved.quota_source, ProviderQuotaSource::KimiUsage);
            assert_eq!(saved.request_profile, template.request_profile);
            // The selected model protocol is independent of quota collection.
            assert_eq!(saved.protocol, template.protocol);
            let home = std::env::temp_dir().join(format!(
                "velune-quota-composition-{}",
                crate::new_record_key()
            ));
            let quota = crate::subscription_quota::SubscriptionQuota::new(
                home.clone(),
                &gateways,
                Default::default(),
            );
            // Check the production query binding without starting a network
            // request. A configured Kimi /v1 API provider must await its first
            // sample, rather than silently becoming an unsupported source.
            quota.stop();
            let observation = quota.query("default", id, false).unwrap().observation;
            assert!(matches!(
                observation.freshness,
                crate::QuotaFreshness::Unknown
            ));
            assert!(!observation.refreshing);
            drop(quota);
            if home.exists() {
                std::fs::remove_dir_all(home).unwrap();
            }
            draft.quota_source = ProviderQuotaSource::None;
            edit_provider(
                &mut gateways,
                "default",
                ProviderEdit::Save {
                    provider: draft,
                    authentication: AuthenticationEdit::Keep,
                    quota_authentication: None,
                },
                &|_| unreachable!(),
            )
            .unwrap();
        }
        let reloaded: Vec<GatewayConfig> =
            serde_json::from_slice(&serde_json::to_vec(&gateways).unwrap()).unwrap();
        for provider in &reloaded[0].providers {
            assert_eq!(provider.quota_source, ProviderQuotaSource::None);
            provider.validate_composition().unwrap();
            let mut foreign = provider.clone();
            foreign.request_profile = RequestProfile::Native;
            foreign.quota_source = ProviderQuotaSource::KimiUsage;
            foreign.endpoint = "https://example.invalid/coding/".into();
            assert!(foreign.validate_composition().is_err());
        }
    }
}
