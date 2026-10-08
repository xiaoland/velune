//! Pure provider/template edits shared by local and configuration-only composition.
use crate::{Error, api::ProviderDraft, config::*, provider_authentication::*};
use serde_json::Value;
pub(crate) fn edit_provider(
    gateways: &mut Vec<GatewayConfig>,
    payload: &Value,
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
                failover: FailoverPolicy {
                    mode: FailoverMode::Disabled,
                },
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
            match serde_json::from_value::<AuthenticationEdit>(
                payload["authenticationEdit"].clone(),
            )? {
                AuthenticationEdit::Keep => {}
                AuthenticationEdit::SetApiKey { value } => {
                    if value.trim().is_empty() || value.chars().any(char::is_control) {
                        return Err(Error::invalid("API key 不能为空或含控制字符"));
                    }
                    authentication.material = AuthenticationMaterial::ApiKey { value };
                    authentication.generation = authentication
                        .generation
                        .checked_add(1)
                        .ok_or_else(|| Error::invalid("authentication revision overflow"))?;
                }
                AuthenticationEdit::Clear => {
                    authentication.material = AuthenticationMaterial::None;
                    authentication.generation = authentication
                        .generation
                        .checked_add(1)
                        .ok_or_else(|| Error::invalid("authentication revision overflow"))?;
                }
            }
            if previous.is_some_and(|p| p.protocol != draft.protocol) {
                for model in &mut draft.models {
                    model.pi_projection = None;
                }
            }
            let provider = ProviderDefinition {
                id: draft.id.clone(),
                name: draft.name,
                protocol: draft.protocol,
                endpoint: draft.endpoint,
                authentication,
                models: draft.models,
            };
            gateway.providers.retain(|p| p.id != draft.id);
            gateway.providers.push(provider);
            gateway.assign_record_keys();
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
