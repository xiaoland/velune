//! Aggregated provider reports remain separate from local gateway request analytics.
use crate::{
    Application, Error, ProviderAnalyticsArea, ProviderAnalyticsChatgptProfileTotal,
    ProviderAnalyticsDailyTotal, ProviderAnalyticsFailure, ProviderAnalyticsOrganizationTotal,
    ProviderAnalyticsReport, ProviderAnalyticsScope, ProviderAnalyticsSource,
    ProviderAnalyticsSourceKind, ProviderAnalyticsSourceState, ProviderAnalyticsUsage,
    config::{ExecutionBackend, RequestProfile},
    local::{Focus, Login},
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use velune_application_api::{
    BillingScope, BillingService, ProviderAccountFailure, ProviderAccountSource,
    ProviderAccountState, ProviderAccountStatus, ProviderBillingStatus, ProviderSummary,
};

impl Application {
    /// Read cached provider-side reports and start bounded background refreshes when requested.
    /// This does not wait for provider network requests; callers poll while `refreshing` is true.
    pub fn provider_analytics(&mut self, refresh: bool) -> Result<ProviderAnalyticsReport, Error> {
        self.runtime.serve(Focus::None, Login::Allows, |core| {
            let gateways = core.cached_list().gateways;
            let mut sources = Vec::new();
            for gateway in gateways {
                for provider in gateway.providers {
                    let account = core.provider_account(&gateway.id, &provider.id, refresh)?;
                    if account.state != ProviderAccountState::Unsupported {
                        sources.push(account_source(
                            gateway.id.clone(),
                            provider.id.clone(),
                            provider.name.clone(),
                            account,
                        ));
                    }

                    let Some(service) = report_service(&provider) else {
                        continue;
                    };
                    let billing =
                        core.provider_billing(gateway.id.clone(), provider.id.clone(), refresh)?;
                    sources.push(billing_source(
                        gateway.id.clone(),
                        provider,
                        service,
                        billing,
                    ));
                }
            }
            aggregate_report(sources, now_ms())
        })
    }
}

fn report_service(provider: &ProviderSummary) -> Option<BillingService> {
    if provider.execution_backend != ExecutionBackend::Http
        || provider.request_profile != RequestProfile::Native
        || provider.authentication_source != crate::ProviderAuthenticationSource::ApiKey
    {
        return None;
    }
    velune_application_api::billing_service(&provider.endpoint)
}

fn account_source(
    gateway_id: String,
    provider_id: String,
    provider_name: String,
    status: ProviderAccountStatus,
) -> ProviderAnalyticsSource {
    let identity_key = status.identity_key.clone();
    let id = identity_key
        .as_ref()
        .map(|key| format!("chatgpt:{key}"))
        .unwrap_or_else(|| format!("chatgpt:provider:{gateway_id}:{provider_id}"));
    ProviderAnalyticsSource {
        id,
        gateway_ids: vec![gateway_id],
        provider_ids: vec![provider_id],
        provider_names: vec![provider_name.clone()],
        kind: ProviderAnalyticsSourceKind::ChatgptSubscription,
        identity_key,
        profile_identity_key: status.profile_identity_key,
        organization_id: None,
        label: format!("ChatGPT 订阅账户：{provider_name}"),
        state: account_state(status.state),
        scope: ProviderAnalyticsScope::SubscriptionAccount,
        scope_id: None,
        range_start_at_ms: None,
        range_end_at_ms: None,
        observed_at_ms: status.fetched_at.as_deref().and_then(parse_rfc3339_ms),
        data_refreshed_at_ms: None,
        credits_observed_at_ms: status
            .credits_observed_at
            .as_deref()
            .and_then(parse_rfc3339_ms),
        reset_count_observed_at_ms: status
            .reset_count_observed_at
            .as_deref()
            .and_then(parse_rfc3339_ms),
        reset_grants_observed_at_ms: status
            .reset_grants_observed_at
            .as_deref()
            .and_then(parse_rfc3339_ms),
        profile_observed_at_ms: status
            .profile_observed_at
            .as_deref()
            .and_then(parse_rfc3339_ms),
        stale: status.stale,
        refreshing: status.refreshing,
        stale_areas: status
            .stale_sources
            .into_iter()
            .map(|source| match source {
                ProviderAccountSource::Usage => ProviderAnalyticsArea::AccountUsage,
                ProviderAccountSource::Resets => ProviderAnalyticsArea::BankedResets,
                ProviderAccountSource::Profile => ProviderAnalyticsArea::Profile,
            })
            .collect(),
        usage: None,
        costs: None,
        profile: status.profile,
        credits: status.credits,
        banked_resets: status.banked_resets,
        failures: status.failures.into_iter().map(account_failure).collect(),
    }
}

fn account_state(state: ProviderAccountState) -> ProviderAnalyticsSourceState {
    match state {
        ProviderAccountState::Unsupported => ProviderAnalyticsSourceState::Unsupported,
        ProviderAccountState::Unauthenticated => ProviderAnalyticsSourceState::Unauthenticated,
        ProviderAccountState::Refreshing => ProviderAnalyticsSourceState::Refreshing,
        ProviderAccountState::Expired => ProviderAnalyticsSourceState::Expired,
        ProviderAccountState::Available => ProviderAnalyticsSourceState::Available,
        ProviderAccountState::PartialFailure => ProviderAnalyticsSourceState::PartialFailure,
        ProviderAccountState::Failed => ProviderAnalyticsSourceState::Failed,
    }
}

fn account_failure(failure: ProviderAccountFailure) -> ProviderAnalyticsFailure {
    let area = match failure.source {
        ProviderAccountSource::Usage => ProviderAnalyticsArea::AccountUsage,
        ProviderAccountSource::Resets => ProviderAnalyticsArea::BankedResets,
        ProviderAccountSource::Profile => ProviderAnalyticsArea::Profile,
    };
    ProviderAnalyticsFailure {
        area,
        message: failure.message,
    }
}

fn billing_source(
    gateway_id: String,
    provider: ProviderSummary,
    service: BillingService,
    status: ProviderBillingStatus,
) -> ProviderAnalyticsSource {
    let configuration = status.configuration.clone();
    let (organization_id, scope, scope_id) = match configuration.as_ref() {
        Some(configuration) => {
            let (scope, id) = match &configuration.scope {
                BillingScope::Organization => (ProviderAnalyticsScope::Organization, None),
                BillingScope::Project { id } => (ProviderAnalyticsScope::Project, Some(id.clone())),
                BillingScope::Workspace { id } => {
                    (ProviderAnalyticsScope::Workspace, Some(id.clone()))
                }
                BillingScope::ApiKey { id } => (ProviderAnalyticsScope::ApiKey, Some(id.clone())),
            };
            (Some(configuration.organization_id.clone()), scope, id)
        }
        None => (None, ProviderAnalyticsScope::Organization, None),
    };
    let kind = match service {
        BillingService::OpenAi => ProviderAnalyticsSourceKind::OpenAiOrganization,
        BillingService::Anthropic => ProviderAnalyticsSourceKind::AnthropicOrganization,
    };
    let identity_key = configuration.as_ref().map(|configuration| {
        digest(&format!(
            "provider-report-v1\0{:?}\0{}\0{:?}\0{}",
            configuration.service,
            configuration.organization_id,
            configuration.scope,
            match &configuration.scope {
                BillingScope::Organization => "".to_owned(),
                BillingScope::Project { id }
                | BillingScope::Workspace { id }
                | BillingScope::ApiKey { id } => id.clone(),
            }
        ))
    });
    let report = status.report;
    let mut failures = Vec::new();
    if let Some(report) = &report {
        failures.extend(
            report
                .problems
                .iter()
                .map(|problem| ProviderAnalyticsFailure {
                    area: billing_problem_area(problem),
                    message: problem.clone(),
                }),
        );
    }
    if let Some(problem) = status.problem {
        failures.push(ProviderAnalyticsFailure {
            area: ProviderAnalyticsArea::OrganizationUsage,
            message: problem,
        });
    }
    ProviderAnalyticsSource {
        // Keep every configured key visible as a source. Multiple keys for one
        // organization/scope are reconciled per metric below, so complementary
        // partial reports can contribute without adding duplicate metrics.
        id: format!(
            "billing:provider:{gateway_id}:{}:{}",
            provider.id,
            service_tag(service)
        ),
        gateway_ids: vec![gateway_id],
        provider_ids: vec![provider.id],
        provider_names: vec![provider.name.clone()],
        kind,
        identity_key,
        profile_identity_key: None,
        organization_id,
        label: configuration.as_ref().map_or_else(
            || format!("{} 组织报表（未配置）", service_name(service)),
            |configuration| {
                format!(
                    "{} 组织 {}",
                    service_name(service),
                    configuration.organization_id
                )
            },
        ),
        state: billing_state(
            configuration.is_some(),
            status.authentication.binding_confirmed,
            status.refreshing,
            status.stale,
            report.as_ref(),
            !failures.is_empty(),
        ),
        scope,
        scope_id,
        range_start_at_ms: report.as_ref().map(|value| value.starting_at_ms),
        range_end_at_ms: report.as_ref().map(|value| value.ending_at_ms),
        observed_at_ms: report.as_ref().map(|value| value.observed_at_ms),
        data_refreshed_at_ms: report.as_ref().and_then(|value| value.data_refreshed_at_ms),
        credits_observed_at_ms: None,
        reset_count_observed_at_ms: None,
        reset_grants_observed_at_ms: None,
        profile_observed_at_ms: None,
        stale: status.stale,
        refreshing: status.refreshing,
        stale_areas: if status.stale {
            report.as_ref().map_or_else(Vec::new, |report| {
                let mut areas = Vec::new();
                if report.usage.is_some() {
                    areas.push(ProviderAnalyticsArea::OrganizationUsage);
                }
                if report.costs.is_some() {
                    areas.push(ProviderAnalyticsArea::Costs);
                }
                areas
            })
        } else {
            Vec::new()
        },
        usage: report.as_ref().and_then(|report| {
            report.usage.as_ref().map(|usage| ProviderAnalyticsUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                requests: usage.requests,
                cached_input_tokens: usage.cached_input_tokens,
                cache_creation_input_tokens: usage.cache_creation_input_tokens,
            })
        }),
        costs: report.and_then(|value| value.costs),
        profile: None,
        credits: None,
        banked_resets: None,
        failures,
    }
}

fn billing_state(
    configured: bool,
    binding_confirmed: bool,
    refreshing: bool,
    stale: bool,
    report: Option<&velune_application_api::BillingReport>,
    has_failures: bool,
) -> ProviderAnalyticsSourceState {
    if !configured {
        return ProviderAnalyticsSourceState::Unconfigured;
    }
    if !binding_confirmed {
        return ProviderAnalyticsSourceState::Unauthenticated;
    }
    if refreshing {
        return ProviderAnalyticsSourceState::Refreshing;
    }
    if stale && report.is_some() {
        return ProviderAnalyticsSourceState::Expired;
    }
    if has_failures {
        return ProviderAnalyticsSourceState::PartialFailure;
    }
    if report.is_some() {
        ProviderAnalyticsSourceState::Available
    } else {
        ProviderAnalyticsSourceState::Failed
    }
}

fn billing_problem_area(problem: &str) -> ProviderAnalyticsArea {
    if problem.starts_with("费用：") {
        ProviderAnalyticsArea::Costs
    } else if problem.starts_with("支出限额：") {
        ProviderAnalyticsArea::SpendLimit
    } else if problem.starts_with("配置速率上限：") {
        ProviderAnalyticsArea::RateLimits
    } else {
        ProviderAnalyticsArea::OrganizationUsage
    }
}

fn service_name(service: BillingService) -> &'static str {
    match service {
        BillingService::OpenAi => "OpenAI",
        BillingService::Anthropic => "Anthropic",
    }
}
fn service_tag(service: BillingService) -> &'static str {
    match service {
        BillingService::OpenAi => "openai",
        BillingService::Anthropic => "anthropic",
    }
}
fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn parse_rfc3339_ms(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.timestamp_millis())
}
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

fn aggregate_report(
    sources: Vec<ProviderAnalyticsSource>,
    observed_at_ms: i64,
) -> Result<ProviderAnalyticsReport, Error> {
    let sources = deduplicate_sources(sources);
    let chatgpt_profile_totals = aggregate_chatgpt_profiles(&sources)?;
    let organization_totals = aggregate_organizations(&sources)?;
    Ok(ProviderAnalyticsReport {
        observed_at_ms,
        sources,
        chatgpt_profile_totals,
        organization_totals,
    })
}

fn deduplicate_sources(sources: Vec<ProviderAnalyticsSource>) -> Vec<ProviderAnalyticsSource> {
    let mut unique = BTreeMap::<String, ProviderAnalyticsSource>::new();
    for mut source in sources {
        // Account endpoints can be exposed by several gateway configurations
        // for the same authenticated ChatGPT account. Billing rows must remain
        // distinct: their usage and cost requests may succeed independently.
        if source.kind != ProviderAnalyticsSourceKind::ChatgptSubscription {
            unique.insert(source.id.clone(), source);
            continue;
        }
        let key = source.identity_key.as_ref().map_or_else(
            || source.id.clone(),
            |identity| format!("chatgpt:{identity}"),
        );
        match unique.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(source);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let existing = entry.get_mut();
                let candidate_is_better = source_quality(&source) > source_quality(existing);
                if candidate_is_better {
                    source
                        .provider_ids
                        .extend(existing.provider_ids.iter().cloned());
                    source
                        .gateway_ids
                        .extend(existing.gateway_ids.iter().cloned());
                    source
                        .provider_names
                        .extend(existing.provider_names.iter().cloned());
                    source.failures.extend(existing.failures.iter().cloned());
                    deduplicate_strings(&mut source.provider_ids);
                    deduplicate_strings(&mut source.gateway_ids);
                    deduplicate_strings(&mut source.provider_names);
                    deduplicate_failures(&mut source.failures);
                    *existing = source;
                } else {
                    existing.gateway_ids.extend(source.gateway_ids);
                    existing.provider_ids.extend(source.provider_ids);
                    existing.provider_names.extend(source.provider_names);
                    existing.failures.extend(source.failures);
                    deduplicate_strings(&mut existing.gateway_ids);
                    deduplicate_strings(&mut existing.provider_ids);
                    deduplicate_strings(&mut existing.provider_names);
                    deduplicate_failures(&mut existing.failures);
                }
            }
        }
    }
    unique.into_values().collect()
}

fn source_quality(source: &ProviderAnalyticsSource) -> (bool, bool, bool, Option<i64>) {
    let has_payload = source.usage.is_some()
        || source.costs.is_some()
        || source.profile.is_some()
        || source.credits.is_some()
        || source.banked_resets.is_some();
    (
        has_payload,
        !source.stale,
        !source.refreshing,
        source.observed_at_ms,
    )
}

fn metric_quality(source: &ProviderAnalyticsSource) -> (bool, bool, Option<i64>) {
    (!source.stale, !source.refreshing, source.observed_at_ms)
}

fn deduplicate_strings(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}
fn deduplicate_failures(values: &mut Vec<ProviderAnalyticsFailure>) {
    values.sort_by_key(|failure| (format!("{:?}", failure.area), failure.message.clone()));
    values.dedup_by(|a, b| a.area == b.area && a.message == b.message);
}

fn source_tag(kind: ProviderAnalyticsSourceKind) -> &'static str {
    match kind {
        ProviderAnalyticsSourceKind::ChatgptSubscription => "chatgpt",
        ProviderAnalyticsSourceKind::OpenAiOrganization => "openai",
        ProviderAnalyticsSourceKind::AnthropicOrganization => "anthropic",
    }
}

fn aggregate_chatgpt_profiles(
    sources: &[ProviderAnalyticsSource],
) -> Result<ProviderAnalyticsChatgptProfileTotal, Error> {
    let mut profiles = BTreeMap::<
        String,
        (
            Option<(bool, bool, bool, Option<i64>)>,
            Option<crate::ProviderAccountProfile>,
        ),
    >::new();
    for source in sources
        .iter()
        .filter(|source| source.kind == ProviderAnalyticsSourceKind::ChatgptSubscription)
    {
        let Some(identity) = source
            .profile_identity_key
            .as_ref()
            .or(source.identity_key.as_ref())
        else {
            continue;
        };
        let entry = profiles.entry(identity.clone()).or_insert((None, None));
        let Some(profile) = source.profile.clone() else {
            continue;
        };
        let profile_is_stale = source.stale_areas.contains(&ProviderAnalyticsArea::Profile);
        let quality = (
            !profile_is_stale,
            !source.stale,
            !source.refreshing,
            source.profile_observed_at_ms.or(source.observed_at_ms),
        );
        if entry.0.is_none_or(|previous| quality > previous) {
            entry.0 = Some(quality);
            entry.1 = Some(profile);
        }
    }

    let mut lifetime_tokens = None;
    let mut lifetime_source_count = 0u32;
    let mut daily_reported_profile_count = 0u32;
    let mut daily = BTreeMap::<String, (u64, u32)>::new();
    for (_, profile) in profiles.values() {
        let Some(profile) = profile else { continue };
        if let Some(tokens) = profile.lifetime_tokens.filter(|value| *value >= 0) {
            lifetime_tokens = Some(checked_add(lifetime_tokens.unwrap_or(0), tokens as u64)?);
            lifetime_source_count += 1;
        }
        if let Some(buckets) = &profile.daily {
            daily_reported_profile_count += 1;
            for bucket in buckets {
                if bucket.tokens < 0 || bucket.date.is_empty() {
                    continue;
                }
                let value = daily.entry(bucket.date.clone()).or_default();
                value.0 = checked_add(value.0, bucket.tokens as u64)?;
                value.1 += 1;
            }
        }
    }
    Ok(ProviderAnalyticsChatgptProfileTotal {
        profile_count: u32::try_from(profiles.len()).unwrap_or(u32::MAX),
        profile_reported_count: u32::try_from(
            profiles
                .values()
                .filter(|(_, profile)| profile.is_some())
                .count(),
        )
        .unwrap_or(u32::MAX),
        lifetime_tokens,
        lifetime_source_count,
        daily_reported_profile_count,
        daily: daily
            .into_iter()
            .map(
                |(date, (tokens, source_count))| ProviderAnalyticsDailyTotal {
                    date,
                    tokens,
                    source_count,
                },
            )
            .collect(),
    })
}

fn aggregate_organizations(
    sources: &[ProviderAnalyticsSource],
) -> Result<Vec<ProviderAnalyticsOrganizationTotal>, Error> {
    let mut organizations = BTreeMap::<String, Vec<&ProviderAnalyticsSource>>::new();
    for source in sources.iter().filter(|source| {
        matches!(
            source.kind,
            ProviderAnalyticsSourceKind::OpenAiOrganization
                | ProviderAnalyticsSourceKind::AnthropicOrganization
        ) && source.organization_id.is_some()
            && source.range_start_at_ms.is_some()
            && source.range_end_at_ms.is_some()
    }) {
        let service = source_tag(source.kind);
        let key = format!(
            "{service}:{}",
            source.organization_id.as_deref().unwrap_or_default()
        );
        organizations.entry(key).or_default().push(source);
    }

    // Resolve overlapping scopes independently for each service and organization. An
    // organization-wide result wins; otherwise one child scope family is selected.
    let mut selected = Vec::new();
    for organization_sources in organizations.into_values() {
        let has_organization_total = organization_sources.iter().any(|source| {
            source.scope == ProviderAnalyticsScope::Organization
                && (source.usage.is_some() || source.costs.is_some())
        });
        let scope = if has_organization_total {
            ProviderAnalyticsScope::Organization
        } else if organization_sources.iter().any(|source| {
            matches!(
                source.scope,
                ProviderAnalyticsScope::Project | ProviderAnalyticsScope::Workspace
            ) && (source.usage.is_some() || source.costs.is_some())
        }) {
            organization_sources
                .iter()
                .find(|source| {
                    matches!(
                        source.scope,
                        ProviderAnalyticsScope::Project | ProviderAnalyticsScope::Workspace
                    ) && (source.usage.is_some() || source.costs.is_some())
                })
                .expect("child scope exists")
                .scope
        } else {
            ProviderAnalyticsScope::ApiKey
        };
        selected.extend(
            organization_sources
                .into_iter()
                .filter(|source| source.scope == scope),
        );
    }

    let mut groups =
        BTreeMap::<(String, i64, ProviderAnalyticsScope), Vec<&ProviderAnalyticsSource>>::new();
    for source in selected {
        let (Some(start), Some(_end)) = (source.range_start_at_ms, source.range_end_at_ms) else {
            continue;
        };
        groups
            .entry((source_tag(source.kind).into(), start, source.scope))
            .or_default()
            .push(source);
    }
    let mut output = Vec::new();
    for ((service, range_start_at_ms, scope), rows) in groups {
        let mut input_tokens = None;
        let mut output_tokens = None;
        let mut usage_source_count = 0u32;
        let mut costs = BTreeMap::<String, f64>::new();
        let mut cost_source_count = 0u32;
        let mut organization_ids = BTreeSet::new();
        let mut provider_ids = BTreeSet::new();
        let mut scope_ids = BTreeSet::new();
        let mut end_range = Vec::new();

        // Reconcile duplicate configured keys within one logical source scope
        // per metric. A usage success from one key and a cost success from
        // another are both useful; two usage successes for the same scope are
        // still one observation and must not be added together.
        let mut logical_scopes = BTreeMap::<String, Vec<&ProviderAnalyticsSource>>::new();
        for source in &rows {
            let key = source
                .identity_key
                .clone()
                .unwrap_or_else(|| source.id.clone());
            logical_scopes.entry(key).or_default().push(source);
        }
        let mut source_count = 0u32;
        for scope_rows in logical_scopes.into_values() {
            let best_usage = scope_rows
                .iter()
                .copied()
                .filter(|source| source.usage.is_some())
                .max_by_key(|source| metric_quality(source));
            let best_costs = scope_rows
                .iter()
                .copied()
                .filter(|source| source.costs.is_some())
                .max_by_key(|source| metric_quality(source));
            if best_usage.is_none() && best_costs.is_none() {
                continue;
            }
            source_count = source_count.saturating_add(1);
            for source in &scope_rows {
                if let Some(id) = &source.organization_id {
                    organization_ids.insert(id.clone());
                }
                provider_ids.extend(source.provider_ids.iter().cloned());
                if let Some(id) = &source.scope_id {
                    scope_ids.insert(id.clone());
                }
            }
            if let Some(source) = best_usage {
                let usage = source.usage.as_ref().expect("usage candidate");
                input_tokens = Some(checked_add(input_tokens.unwrap_or(0), usage.input_tokens)?);
                output_tokens = Some(checked_add(
                    output_tokens.unwrap_or(0),
                    usage.output_tokens,
                )?);
                usage_source_count = usage_source_count.saturating_add(1);
                if let Some(end) = source.range_end_at_ms {
                    end_range.push(end);
                }
            }
            if let Some(source) = best_costs {
                let source_costs = source.costs.as_ref().expect("cost candidate");
                cost_source_count = cost_source_count.saturating_add(1);
                for cost in source_costs {
                    if !cost.amount.is_finite() {
                        return Err(Error::invalid("提供商费用聚合无效"));
                    }
                    let currency = cost.currency.to_ascii_uppercase();
                    let amount = costs.entry(currency).or_default();
                    *amount += cost.amount;
                    if !amount.is_finite() {
                        return Err(Error::invalid("提供商费用聚合超出数值范围"));
                    }
                }
                if let Some(end) = source.range_end_at_ms {
                    end_range.push(end);
                }
            }
        }
        if source_count == 0 {
            continue;
        }
        end_range.sort_unstable();
        let range_end_min_at_ms = *end_range.first().expect("reported metric end range");
        let range_end_max_at_ms = *end_range.last().expect("reported metric end range");
        output.push(ProviderAnalyticsOrganizationTotal {
            services: vec![match service.as_str() {
                "openai" => BillingService::OpenAi,
                "anthropic" => BillingService::Anthropic,
                _ => unreachable!("known provider report service"),
            }],
            organization_ids: organization_ids.into_iter().collect(),
            provider_ids: provider_ids.into_iter().collect(),
            scope,
            scope_ids: scope_ids.into_iter().collect(),
            range_start_at_ms,
            range_end_at_ms: (range_end_min_at_ms == range_end_max_at_ms)
                .then_some(range_end_min_at_ms),
            range_end_min_at_ms,
            range_end_max_at_ms,
            range_is_heterogeneous: range_end_min_at_ms != range_end_max_at_ms,
            input_tokens,
            output_tokens,
            usage_source_count,
            source_count,
            costs: costs
                .into_iter()
                .map(|(currency, amount)| crate::BilledCost { amount, currency })
                .collect(),
            cost_source_count,
        });
    }
    Ok(output)
}

fn checked_add(left: u64, right: u64) -> Result<u64, Error> {
    left.checked_add(right)
        .ok_or_else(|| Error::invalid("提供商 Token 聚合超出数值范围"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderAccountDailyUsage;

    const START: i64 = 1_800_000_000_000;
    const END: i64 = 1_800_086_400_000;

    fn source(
        id: &str,
        kind: ProviderAnalyticsSourceKind,
        identity: Option<&str>,
    ) -> ProviderAnalyticsSource {
        ProviderAnalyticsSource {
            id: id.into(),
            gateway_ids: vec!["gateway-a".into()],
            provider_ids: vec![id.into()],
            provider_names: vec![id.into()],
            kind,
            identity_key: identity.map(str::to_owned),
            profile_identity_key: None,
            organization_id: None,
            label: id.into(),
            state: ProviderAnalyticsSourceState::Available,
            scope: ProviderAnalyticsScope::SubscriptionAccount,
            scope_id: None,
            range_start_at_ms: None,
            range_end_at_ms: None,
            observed_at_ms: Some(START),
            data_refreshed_at_ms: None,
            credits_observed_at_ms: None,
            reset_count_observed_at_ms: None,
            reset_grants_observed_at_ms: None,
            profile_observed_at_ms: None,
            stale: false,
            refreshing: false,
            stale_areas: Vec::new(),
            usage: None,
            costs: None,
            profile: None,
            credits: None,
            banked_resets: None,
            failures: Vec::new(),
        }
    }

    fn profile(
        lifetime: Option<i64>,
        daily: Option<Vec<(&str, i64)>>,
    ) -> crate::ProviderAccountProfile {
        crate::ProviderAccountProfile {
            lifetime_tokens: lifetime,
            peak_daily_tokens: None,
            longest_task_seconds: None,
            current_streak_days: None,
            longest_streak_days: None,
            fast_mode_percent: None,
            reasoning_effort: None,
            reasoning_effort_percent: None,
            skills_explored: None,
            total_skills_used: None,
            total_threads: None,
            daily: daily.map(|rows| {
                rows.into_iter()
                    .map(|(date, tokens)| ProviderAccountDailyUsage {
                        date: date.into(),
                        tokens,
                    })
                    .collect()
            }),
            top_invocations: None,
            stats_as_of: None,
            generated_at: None,
        }
    }

    fn organization(
        id: &str,
        identity: &str,
        organization_id: &str,
        scope: ProviderAnalyticsScope,
        scope_id: Option<&str>,
        start: i64,
        end: i64,
        usage: Option<(u64, u64)>,
        costs: Option<Vec<(&str, f64)>>,
    ) -> ProviderAnalyticsSource {
        let mut row = source(
            id,
            ProviderAnalyticsSourceKind::OpenAiOrganization,
            Some(identity),
        );
        row.organization_id = Some(organization_id.into());
        row.scope = scope;
        row.scope_id = scope_id.map(str::to_owned);
        row.range_start_at_ms = Some(start);
        row.range_end_at_ms = Some(end);
        row.usage = usage.map(|(input, output)| ProviderAnalyticsUsage {
            input_tokens: input,
            output_tokens: output,
            requests: 1,
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
        });
        row.costs = costs.map(|items| {
            items
                .into_iter()
                .map(|(currency, amount)| crate::BilledCost {
                    amount,
                    currency: currency.into(),
                })
                .collect()
        });
        row
    }

    #[test]
    fn chatgpt_profiles_deduplicate_by_profile_identity_and_keep_unknown_coverage() {
        let mut first = source(
            "codex-personal",
            ProviderAnalyticsSourceKind::ChatgptSubscription,
            Some("account-a"),
        );
        first.profile_identity_key = Some("subject-a".into());
        first.profile = Some(profile(Some(0), Some(vec![("2026-11-15", 0)])));

        let mut duplicate_workspace = source(
            "codex-workspace",
            ProviderAnalyticsSourceKind::ChatgptSubscription,
            Some("account-b"),
        );
        duplicate_workspace.profile_identity_key = Some("subject-a".into());
        duplicate_workspace.observed_at_ms = Some(END);
        duplicate_workspace.profile = Some(profile(Some(4), Some(vec![("2026-11-15", 4)])));

        let mut profile_missing = source(
            "codex-workspace-refreshing",
            ProviderAnalyticsSourceKind::ChatgptSubscription,
            Some("account-cached-separately"),
        );
        profile_missing.profile_identity_key = Some("subject-a".into());
        profile_missing.observed_at_ms = Some(END + 1);
        profile_missing.refreshing = true;

        let mut other = source(
            "codex-other",
            ProviderAnalyticsSourceKind::ChatgptSubscription,
            Some("account-c"),
        );
        other.profile_identity_key = Some("subject-c".into());
        other.profile = Some(profile(Some(8), None));

        let report = aggregate_report(
            vec![first, duplicate_workspace, profile_missing, other],
            END,
        )
        .unwrap();
        let total = report.chatgpt_profile_totals;
        assert_eq!(total.profile_count, 2);
        assert_eq!(total.profile_reported_count, 2);
        assert_eq!(total.lifetime_tokens, Some(12));
        assert_eq!(total.lifetime_source_count, 2);
        assert_eq!(total.daily_reported_profile_count, 1);
        assert_eq!(total.daily.len(), 1);
        assert_eq!(total.daily[0].date, "2026-11-15");
        assert_eq!(total.daily[0].tokens, 4);
        assert_eq!(total.daily[0].source_count, 1);
    }

    #[test]
    fn organization_scope_wins_and_same_org_different_keys_do_not_double_count() {
        let root_one = organization(
            "key-one",
            "same-org-scope",
            "org-a",
            ProviderAnalyticsScope::Organization,
            None,
            START,
            END,
            Some((100, 20)),
            Some(vec![("usd", 2.5)]),
        );
        let root_duplicate = organization(
            "key-two",
            "same-org-scope",
            "org-a",
            ProviderAnalyticsScope::Organization,
            None,
            START,
            END,
            Some((100, 20)),
            Some(vec![("USD", 2.5)]),
        );
        let project = organization(
            "project",
            "project-a",
            "org-a",
            ProviderAnalyticsScope::Project,
            Some("proj-a"),
            START,
            END,
            Some((50, 10)),
            Some(vec![("USD", 1.25)]),
        );
        let report = aggregate_report(vec![root_one, root_duplicate, project], END).unwrap();
        assert_eq!(report.sources.len(), 3);
        assert_eq!(report.organization_totals.len(), 1);
        let total = &report.organization_totals[0];
        assert_eq!(total.scope, ProviderAnalyticsScope::Organization);
        assert!(total.scope_ids.is_empty());
        assert_eq!(total.organization_ids, vec!["org-a"]);
        assert_eq!(total.source_count, 1);
        assert_eq!(total.usage_source_count, 1);
        assert_eq!(total.input_tokens, Some(100));
        assert_eq!(total.output_tokens, Some(20));
        assert_eq!(total.cost_source_count, 1);
        assert_eq!(total.costs[0].currency, "USD");
        assert_eq!(total.costs[0].amount, 2.5);
    }

    #[test]
    fn fallback_child_totals_choose_one_scope_dimension_and_preserve_missing_values() {
        let project_a = organization(
            "project-a-key-1",
            "project-a",
            "org-a",
            ProviderAnalyticsScope::Project,
            Some("proj-a"),
            START,
            END,
            Some((10, 2)),
            None,
        );
        let project_a_duplicate = organization(
            "project-a-key-2",
            "project-a",
            "org-a",
            ProviderAnalyticsScope::Project,
            Some("proj-a"),
            START,
            END,
            Some((10, 2)),
            None,
        );
        let project_b = organization(
            "project-b",
            "project-b",
            "org-a",
            ProviderAnalyticsScope::Project,
            Some("proj-b"),
            START,
            END,
            Some((0, 0)),
            None,
        );
        let api_key = organization(
            "api-key",
            "api-key-a",
            "org-a",
            ProviderAnalyticsScope::ApiKey,
            Some("key-a"),
            START,
            END,
            Some((99, 99)),
            None,
        );
        let report = aggregate_report(
            vec![project_a, project_a_duplicate, project_b, api_key],
            END,
        )
        .unwrap();
        assert_eq!(report.organization_totals.len(), 1);
        let total = &report.organization_totals[0];
        assert_eq!(total.scope, ProviderAnalyticsScope::Project);
        assert_eq!(total.scope_ids, vec!["proj-a", "proj-b"]);
        assert_eq!(total.source_count, 2);
        assert_eq!(total.input_tokens, Some(10));
        assert_eq!(total.output_tokens, Some(2));
        assert_eq!(total.cost_source_count, 0);
        assert!(total.costs.is_empty());
    }

    #[test]
    fn organization_aggregates_preserve_heterogeneous_ends_and_partial_coverage() {
        let mut no_usage = organization(
            "org-a-costs",
            "org-a",
            "org-a",
            ProviderAnalyticsScope::Organization,
            None,
            START,
            END,
            None,
            Some(vec![("USD", 1.0), ("EUR", 2.0)]),
        );
        no_usage.failures.push(ProviderAnalyticsFailure {
            area: ProviderAnalyticsArea::OrganizationUsage,
            message: "synthetic usage unavailable".into(),
        });
        let exact_window = organization(
            "org-b-usage",
            "org-b",
            "org-b",
            ProviderAnalyticsScope::Organization,
            None,
            START,
            END,
            Some((0, 0)),
            None,
        );
        let adjacent_window = organization(
            "org-c-usage",
            "org-c",
            "org-c",
            ProviderAnalyticsScope::Organization,
            None,
            START,
            END - 3_600_000,
            Some((7, 1)),
            Some(vec![("USD", 4.0)]),
        );
        let report = aggregate_report(vec![no_usage, exact_window, adjacent_window], END).unwrap();
        assert_eq!(report.organization_totals.len(), 1);
        let total = &report.organization_totals[0];
        assert_eq!(total.source_count, 3);
        assert_eq!(total.usage_source_count, 2);
        assert_eq!(total.input_tokens, Some(7));
        assert_eq!(total.output_tokens, Some(1));
        assert_eq!(total.cost_source_count, 2);
        assert_eq!(total.costs.len(), 2);
        assert_eq!(total.range_end_at_ms, None);
        assert_eq!(total.range_end_min_at_ms, END - 3_600_000);
        assert_eq!(total.range_end_max_at_ms, END);
        assert!(total.range_is_heterogeneous);
    }

    #[test]
    fn same_scope_complementary_partial_reports_keep_each_known_metric_once() {
        let usage_only = organization(
            "provider-key-usage",
            "same-logical-scope",
            "org-a",
            ProviderAnalyticsScope::Organization,
            None,
            START,
            END,
            Some((10, 3)),
            None,
        );
        let costs_only = organization(
            "provider-key-costs",
            "same-logical-scope",
            "org-a",
            ProviderAnalyticsScope::Organization,
            None,
            START,
            END - 3_600_000,
            None,
            Some(vec![("USD", 1.75)]),
        );
        let report = aggregate_report(vec![usage_only, costs_only], END).unwrap();
        assert_eq!(report.organization_totals.len(), 1);
        let total = &report.organization_totals[0];
        assert_eq!(total.source_count, 1);
        assert_eq!(total.usage_source_count, 1);
        assert_eq!(total.cost_source_count, 1);
        assert_eq!(total.input_tokens, Some(10));
        assert_eq!(total.output_tokens, Some(3));
        assert_eq!(total.costs.len(), 1);
        assert_eq!(total.costs[0].amount, 1.75);
        assert_eq!(
            total.provider_ids,
            vec!["provider-key-costs", "provider-key-usage"]
        );
        assert_eq!(total.range_end_at_ms, None);
        assert_eq!(total.range_end_min_at_ms, END - 3_600_000);
        assert_eq!(total.range_end_max_at_ms, END);
        assert!(total.range_is_heterogeneous);
    }
}
