//! The targets each gateway model name may reach.
use super::GatewayError;
use crate::config::{ExecutionBackend, GatewayConfig, Protocol, QuotaLimit, RoutingStrategy};
use std::{collections::BTreeMap, sync::Arc};
use velune_ai::ProviderModelId;
use velune_ai_provider::config::{HttpEndpoint, HttpTarget, RequestProfile};

#[derive(Clone)]
pub(super) struct RouteTarget {
    pub(super) aggregate: bool,
    pub(super) route_key: String,
    pub(super) strategy: RoutingStrategy,
    pub(super) sticky: bool,
    pub(super) weight: u32,
    pub(super) quota_limits: Vec<QuotaLimit>,
    pub(super) max_output_tokens: Option<u32>,
    pub(super) request_parameters: std::collections::BTreeMap<String, String>,
    pub(super) provider_slot: usize,
    pub(super) model_slot: usize,
    pub(super) model: ProviderModelId,
    pub(super) protocol: Protocol,
    pub(super) endpoint: String,
    pub(super) config: Option<HttpTarget>,
    pub(super) execution_backend: ExecutionBackend,
    pub(super) request_profile: RequestProfile,
    pub(super) continuation_required: bool,
    pub(super) reference: String,
    pub(super) provider_id: String,
    pub(super) provider_name: String,
    pub(super) model_record_key: String,
}
/// The targets one request may use, in priority order: a provider model's own
/// target, or a model route's members.
pub(super) type Route = Arc<[RouteTarget]>;

pub(super) fn build_routes(
    config: &GatewayConfig,
    aliases: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Route>, Box<dyn std::error::Error>> {
    let mut routes = BTreeMap::new();
    for (provider_slot, configured_provider) in config.providers.iter().enumerate() {
        for (model_slot, entry) in configured_provider.models.iter().enumerate() {
            let provider = configured_provider;
            let provider_config = match provider.execution_backend {
                ExecutionBackend::Http => Some(HttpTarget::new(
                    provider.protocol,
                    HttpEndpoint::parse(&provider.endpoint)?,
                    provider.request_profile,
                )?),
                ExecutionBackend::ClaudeCli { .. } => None,
            };
            let model = ProviderModelId::new(entry.provider_model_id.clone())?;
            routes.insert(
                entry.record_key.clone(),
                RouteTarget {
                    aggregate: false,
                    route_key: entry.record_key.clone(),
                    strategy: RoutingStrategy::Order,
                    sticky: false,
                    weight: 1,
                    quota_limits: provider.quota_limits.clone(),
                    max_output_tokens: entry.max_output_tokens,
                    request_parameters: entry.request_parameters.clone(),
                    provider_slot: provider_slot + 1,
                    model_slot: model_slot + 1,
                    model,
                    protocol: provider.protocol,
                    endpoint: provider.endpoint.clone(),
                    config: provider_config,
                    execution_backend: provider.execution_backend.clone(),
                    request_profile: provider.request_profile,
                    continuation_required: false,
                    reference: provider
                        .credential_ref
                        .clone()
                        .unwrap_or_else(|| provider.id.clone()),
                    provider_id: provider.id.clone(),
                    provider_name: provider.name.clone(),
                    model_record_key: entry.record_key.clone(),
                },
            );
        }
    }
    let targets = routes;
    let mut logical = targets
        .iter()
        .map(|(key, target)| (key.clone(), Route::from([target.clone()])))
        .collect::<BTreeMap<_, _>>();
    for route in &config.routes {
        let members = route
            .members
            .iter()
            .map(|key| {
                let mut target = targets.get(key).cloned().expect("validated route member");
                target.aggregate = true;
                target.route_key = route.record_key.clone();
                target.strategy = route.strategy;
                target.sticky = route.sticky;
                target.weight = route.weights.get(key).copied().unwrap_or(1);
                target
            })
            .collect::<Route>();
        logical.insert(route.record_key.clone(), members);
    }
    let mut routes = BTreeMap::new();
    for (key, route) in &logical {
        routes.insert(format!("velune/model/{key}"), route.clone());
    }
    for (alias, logical_key) in aliases {
        let target = logical
            .get(logical_key)
            .cloned()
            .ok_or(GatewayError("gateway route target"))?;
        if routes.insert(alias.clone(), target).is_some() {
            return Err(Box::new(GatewayError("gateway binding identity collision")));
        }
    }
    Ok(routes)
}
