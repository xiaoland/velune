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
            if draft.id.is_empty() {
                draft.id = crate::new_record_key();
            }
            let previous = gateway.providers.iter().find(|p| p.id == draft.id);
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
                    ProviderAuthenticationSource::RuntimeProvider => {
                        return Err(Error::invalid("运行时认证来源只能由提供商导入建立"));
                    }
                };
            }
            let changed = reset
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
            let target_changed = previous.is_some_and(|p| {
                p.endpoint != draft.endpoint
                    || p.protocol != draft.protocol
                    || p.request_profile != draft.request_profile
                    || p.quota_source != draft.quota_source
            });
            if draft.quota_source == ProviderQuotaSource::ArkCodingUsage {
                quota_authentication.apply(quota_edit, authentication.generation, matches!(&authentication.material, AuthenticationMaterial::ApiKey { value } if !value.is_empty()), target_changed)?;
            } else if quota_edit.is_some() {
                return Err(Error::invalid("此提供商不使用独立额度查询认证"));
            } else {
                quota_authentication = Default::default();
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
            let provider = ProviderDefinition {
                id: draft.id.clone(),
                name: draft.name,
                protocol: draft.protocol,
                endpoint: draft.endpoint,
                execution_backend: draft.execution_backend,
                request_profile: draft.request_profile,
                quota_source: draft.quota_source,
                authentication,
                quota_authentication,
                models,
                quota_limits: draft.quota_limits,
            };
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
                    "model route members must share a protocol" => "路由中的模型必须使用同一协议",
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
        if let Some(key) = &provider.quota_authentication.access_key
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
                    protocol: p.protocol.clone(),
                    endpoint: p.endpoint.clone(),
                    execution_backend: p.execution_backend.clone(),
                    request_profile: p.request_profile,
                    authentication_source: p.authentication.source(),
                    quota_source: p.quota_source,
                    quota_authentication: p
                        .quota_authentication
                        .description(p.authentication.generation),
                    authentication: {
                        let mut description = p.authentication.description();
                        if p.request_profile == crate::config::RequestProfile::KimiCoding {
                            description.disclosure =
                                Some(crate::SubscriptionDisclosure::kimi_coding_plan());
                        }
                        if p.request_profile == crate::config::RequestProfile::ArkCoding {
                            description.disclosure =
                                Some(crate::SubscriptionDisclosure::ark_coding_plan());
                        }
                        description
                    },
                    quota_limits: p.quota_limits.clone(),
                    models: p.models.iter().map(Into::into).collect(),
                })
                .collect(),
            routes: g.routes.clone(),
        }
    }
}
