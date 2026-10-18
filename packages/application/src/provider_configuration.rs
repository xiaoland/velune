//! Pure provider/template edits shared by local and configuration-only composition.
use crate::{Error, api::ProviderDraft, config::*, provider_authentication::*};
use serde_json::Value;
/// `link` resolves a referenced Claude Code login to its `CLAUDE_CONFIG_DIR`
/// (`None` for the user's default); only a local composition can.
pub(crate) fn edit_provider(
    gateways: &mut Vec<GatewayConfig>,
    payload: &Value,
    link: &dyn Fn(&LinkedClaudeLogin) -> Result<Option<String>, Error>,
) -> Result<(), Error> {
    let gateway_id = payload["gatewayID"]
        .as_str()
        .ok_or_else(|| Error::invalid("gateway id"))?;
    let operation = payload["operation"]
        .as_str()
        .ok_or_else(|| Error::invalid("provider operation"))?;
    let index = match gateways.iter().position(|g| g.id == gateway_id) {
        Some(i) => i,
        None if gateway_id == "default" && operation == "save" => {
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
    match operation {
        "save" => {
            let mut draft: ProviderDraft = serde_json::from_value(payload["provider"].clone())?;
            if draft.id.is_empty() {
                draft.id = crate::new_record_key();
            }
            let previous = gateway.providers.iter().find(|p| p.id == draft.id);
            let mut authentication = previous
                .map(|p| p.authentication.clone())
                .unwrap_or_default();
            let edit: AuthenticationEdit =
                serde_json::from_value(payload["authenticationEdit"].clone())?;
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
                models,
                quota_limits: draft.quota_limits,
            };
            gateway.providers.retain(|p| p.id != draft.id);
            gateway.providers.push(provider);
            gateway.assign_record_keys();
            gateway.prune_routes();
            gateway.validate().map_err(Error::invalid)?;
        }
        "delete" => {
            let id = payload["providerID"]
                .as_str()
                .ok_or_else(|| Error::invalid("provider id"))?;
            if !gateway.providers.iter().any(|p| p.id == id) {
                return Err(Error::invalid("provider id"));
            }
            gateway.providers.retain(|p| p.id != id);
            gateway.prune_routes();
        }
        "saveRoute" => {
            let mut route: ModelRoute = serde_json::from_value(payload["route"].clone())?;
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
        "deleteRoute" => {
            let key = payload["routeRecordKey"]
                .as_str()
                .ok_or_else(|| Error::invalid("route record key"))?;
            if !gateway.routes.iter().any(|r| r.record_key == key) {
                return Err(Error::invalid("route record key"));
            }
            gateway.routes.retain(|r| r.record_key != key);
        }
        _ => return Err(Error::invalid("provider operation")),
    }
    Ok(())
}
pub(crate) fn edit_template(
    templates: &mut Vec<ModelTemplate>,
    payload: &Value,
) -> Result<(), Error> {
    match payload["operation"].as_str() {
        Some("save") => {
            let mut template: ModelTemplate = serde_json::from_value(payload["template"].clone())?;
            if template.name.trim().is_empty() {
                return Err(Error::invalid("模板名称不能为空"));
            }
            if template.id.is_empty() {
                template.id = crate::new_record_key();
            }
            validate_template(&template)?;
            templates.retain(|t| t.id != template.id);
            templates.push(template);
        }
        Some("delete") => {
            let id = payload["templateID"]
                .as_str()
                .ok_or_else(|| Error::invalid("template id"))?;
            templates.retain(|t| t.id != id);
        }
        _ => return Err(Error::invalid("template operation")),
    }
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
pub(crate) fn summaries(gateways: &[GatewayConfig]) -> Vec<crate::api::GatewaySummary> {
    gateways
        .iter()
        .map(crate::api::GatewaySummary::from)
        .collect()
}
