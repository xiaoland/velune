//! Usage and performance reports over the gateway's request metadata.
use crate::{Application, Error};
use std::collections::BTreeMap;
use velune_application_api::records::*;

impl Application {
    pub fn analytics_query(&self, query: AnalyticsQuery) -> Result<AnalyticsReport, Error> {
        if query.from_ms >= query.to_ms
            || query.bucket_boundaries_ms.len() < 2
            || query.bucket_boundaries_ms.len() > 370
            || query.bucket_boundaries_ms.first() != Some(&query.from_ms)
            || query.bucket_boundaries_ms.last() != Some(&query.to_ms)
            || query
                .bucket_boundaries_ms
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || query.request_limit == 0
        {
            return Err(Error::invalid("分析查询范围无效"));
        }
        let mut overview = AnalyticsAccumulator::default();
        let mut trend = query
            .bucket_boundaries_ms
            .windows(2)
            .map(|bounds| (bounds[0], bounds[1], AnalyticsAccumulator::default()))
            .collect::<Vec<_>>();
        let mut providers = BTreeMap::<String, AnalyticsGroup>::new();
        let mut models = BTreeMap::<(String, String), AnalyticsGroup>::new();
        let mut requests = Vec::new();
        self.analytics.visit(
            query.from_ms,
            query.to_ms,
            query.provider_id.as_deref(),
            query.model_record_key.as_deref(),
            |row| {
                overview.add(&row)?;
                let index = query
                    .bucket_boundaries_ms
                    .partition_point(|boundary| *boundary <= row.terminal_at_ms)
                    .saturating_sub(1);
                trend[index].2.add(&row)?;
                providers
                    .entry(row.provider_id.clone())
                    .or_insert_with(|| AnalyticsGroup::provider(&row))
                    .totals
                    .add(&row)?;
                models
                    .entry((row.provider_id.clone(), row.model_record_key.clone()))
                    .or_insert_with(|| AnalyticsGroup::model(&row))
                    .totals
                    .add(&row)?;
                if requests.len() < query.request_limit.min(500) as usize {
                    requests.push(analytics_request(row)?);
                }
                Ok(())
            },
        )?;
        Ok(AnalyticsReport {
            overview: overview.finish(),
            trend: trend
                .into_iter()
                .map(|(from_ms, to_ms, total)| AnalyticsBucket {
                    from_ms,
                    to_ms,
                    totals: total.finish(),
                })
                .collect(),
            providers: providers
                .into_values()
                .map(AnalyticsGroup::finish)
                .collect(),
            models: models.into_values().map(AnalyticsGroup::finish).collect(),
            requests,
            coverage: "gateway_upstream_observation".into(),
            storage_warning: self.analytics.warning(),
            dropped_count: self.analytics.dropped(),
        })
    }
}

#[derive(Default)]
struct AnalyticsAccumulator {
    totals: AnalyticsTotals,
    eligible_output: u64,
    eligible_elapsed_ms: u64,
}
impl AnalyticsAccumulator {
    fn add(&mut self, row: &crate::analytics::AnalyticsRow) -> Result<(), Error> {
        let total = &mut self.totals;
        total.request_count += 1;
        match row.outcome.as_str() {
            "completed" => total.completed_count += 1,
            "cancelled" => total.cancelled_count += 1,
            "failed" => total.failed_count += 1,
            "incomplete" => total.incomplete_count += 1,
            "rejected" => total.rejected_count += 1,
            _ => return Err(Error::invalid("分析记录包含未知结果")),
        }
        total.usage_reported_count += u64::from(row.usage_reported);
        total.usage_complete_count += u64::from(row.usage_complete);
        sum_optional(&mut total.input_tokens, row.input_tokens)?;
        sum_optional(&mut total.output_tokens, row.output_tokens)?;
        sum_optional(
            &mut total.reasoning_output_tokens,
            row.reasoning_output_tokens,
        )?;
        if let (Some(input), Some(output)) = (row.input_tokens, row.output_tokens) {
            sum_optional(&mut total.total_tokens, Some(sum(input, output)?))?;
            total.total_reported_count += 1;
        }
        total.input_reported_count += u64::from(row.input_tokens.is_some());
        total.output_reported_count += u64::from(row.output_tokens.is_some());
        total.reasoning_reported_count += u64::from(row.reasoning_output_tokens.is_some());
        total.cache_reported_count += u64::from(
            row.cached_input_tokens.is_some()
                || row.cache_read_input_tokens.is_some()
                || row.cache_creation_input_tokens.is_some(),
        );
        if matches!(row.outcome.as_str(), "completed" | "incomplete")
            && let (Some(output), Some(elapsed)) = (
                row.output_tokens,
                row.terminal_elapsed_ms.filter(|value| *value > 0),
            )
        {
            total.eligible_speed_count += 1;
            self.eligible_output = sum(self.eligible_output, output)?;
            self.eligible_elapsed_ms = sum(self.eligible_elapsed_ms, elapsed)?;
        }
        Ok(())
    }
    fn finish(mut self) -> AnalyticsTotals {
        if self.eligible_elapsed_ms > 0 {
            self.totals.output_tokens_per_second =
                Some(self.eligible_output as f64 * 1000.0 / self.eligible_elapsed_ms as f64);
        }
        self.totals
    }
}
fn sum(a: u64, b: u64) -> Result<u64, Error> {
    a.checked_add(b)
        .ok_or_else(|| Error::invalid("分析统计超出数量范围"))
}
fn sum_optional(target: &mut Option<u64>, value: Option<u64>) -> Result<(), Error> {
    if let Some(value) = value {
        *target = Some(sum(target.unwrap_or(0), value)?);
    }
    Ok(())
}
struct AnalyticsGroup {
    key: String,
    name: String,
    provider_id: String,
    model_record_key: Option<String>,
    totals: AnalyticsAccumulator,
}
impl AnalyticsGroup {
    fn provider(row: &crate::analytics::AnalyticsRow) -> Self {
        Self {
            key: row.provider_id.clone(),
            name: row.provider_name.clone(),
            provider_id: row.provider_id.clone(),
            model_record_key: None,
            totals: Default::default(),
        }
    }
    fn model(row: &crate::analytics::AnalyticsRow) -> Self {
        Self {
            key: serde_json::json!([row.provider_id, row.model_record_key]).to_string(),
            name: row.provider_model_id.clone(),
            provider_id: row.provider_id.clone(),
            model_record_key: Some(row.model_record_key.clone()),
            totals: Default::default(),
        }
    }
    fn finish(self) -> AnalyticsBreakdown {
        AnalyticsBreakdown {
            key: self.key,
            name: self.name,
            provider_id: Some(self.provider_id),
            model_record_key: self.model_record_key,
            totals: self.totals.finish(),
        }
    }
}
fn analytics_request(row: crate::analytics::AnalyticsRow) -> Result<AnalyticsRequest, Error> {
    let protocol = match row.protocol.as_str() {
        "chatCompletions" => AnalyticsProtocol::ChatCompletions,
        "responses" => AnalyticsProtocol::Responses,
        "messages" => AnalyticsProtocol::Messages,
        _ => return Err(Error::invalid("分析记录包含未知协议")),
    };
    let outcome = match row.outcome.as_str() {
        "completed" => AnalyticsOutcome::Completed,
        "incomplete" => AnalyticsOutcome::Incomplete,
        "cancelled" => AnalyticsOutcome::Cancelled,
        "rejected" => AnalyticsOutcome::Rejected,
        "failed" => AnalyticsOutcome::Failed,
        _ => return Err(Error::invalid("分析记录包含未知结果")),
    };
    let output_tokens_per_second = if matches!(
        outcome,
        AnalyticsOutcome::Completed | AnalyticsOutcome::Incomplete
    ) {
        row.terminal_elapsed_ms
            .filter(|v| *v > 0)
            .zip(row.output_tokens)
            .map(|(elapsed, tokens)| tokens as f64 * 1000.0 / elapsed as f64)
    } else {
        None
    };
    Ok(AnalyticsRequest {
        request_id: row.request_id,
        provider_id: row.provider_id,
        provider_name: row.provider_name,
        model_record_key: row.model_record_key,
        provider_model_id: row.provider_model_id,
        protocol,
        started_at_ms: row.started_at_ms,
        terminal_at_ms: row.terminal_at_ms,
        elapsed_ms: row.elapsed_ms,
        first_output_ms: row.first_output_ms,
        terminal_elapsed_ms: row.terminal_elapsed_ms,
        output_tokens_per_second,
        status: row.status,
        outcome,
        input_tokens: row.input_tokens,
        uncached_input_tokens: row.uncached_input_tokens,
        output_tokens: row.output_tokens,
        reasoning_output_tokens: row.reasoning_output_tokens,
        cached_input_tokens: row.cached_input_tokens,
        cache_read_input_tokens: row.cache_read_input_tokens,
        cache_creation_input_tokens: row.cache_creation_input_tokens,
        usage_reported: row.usage_reported,
        usage_complete: row.usage_complete,
    })
}
