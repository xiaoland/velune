//! Daily, model-attributed usage from the ChatGPT account analytics reports.
use super::{
    Capture, Collected, Observation, ProviderAccountSource, TTL, failure, fetch_json,
    observation_now,
};
use crate::subscription_auth::{AccountAnalyticsKind as Kind, AccountDataEndpoint as Endpoint};
use chrono::{Days, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};
use velune_application_api::ProviderAnalyticsModelUsage;

const SOURCE: ProviderAccountSource = ProviderAccountSource::AnalyticsUsage;
const DAYS: u64 = 30;
type Rows = BTreeMap<(NaiveDate, String), Amounts>;

#[derive(Clone, Default)]
pub(super) struct Snapshot {
    usage: Option<ObservedReport>,
    turns: Option<ObservedReport>,
}

#[derive(Clone)]
struct ObservedReport {
    rows: Rows,
    observed: Observation,
}

impl ObservedReport {
    fn new(rows: Rows) -> Self {
        Self {
            rows,
            observed: observation_now(),
        }
    }
}

impl Snapshot {
    pub(super) fn retain_missing(&mut self, previous: Self) {
        self.usage = self.usage.take().or(previous.usage);
        self.turns = self.turns.take().or(previous.turns);
    }

    pub(super) fn observed_at(&self) -> Option<&Observation> {
        self.reports()
            .map(|report| &report.observed)
            .min_by_key(|observation| observation.instant)
    }

    pub(super) fn is_stale(&self) -> bool {
        self.observed_at()
            .is_some_and(|observed| observed.instant.elapsed() >= TTL)
    }

    pub(super) fn into_rows(self) -> Option<Vec<ProviderAnalyticsModelUsage>> {
        if self.usage.is_none() && self.turns.is_none() {
            return None;
        }
        let mut rows = self.usage.map_or_else(Rows::new, |usage| usage.rows);
        if let Some(turns) = self.turns {
            for (key, amount) in turns.rows {
                let row = rows.entry(key).or_default();
                row.turns = amount.turns.or(row.turns);
            }
        }
        Some(
            rows.into_iter()
                .map(|((date, model), amount)| ProviderAnalyticsModelUsage {
                    model,
                    date: Some(date.to_string()),
                    from_ms: None,
                    to_ms: None,
                    input_tokens: amount
                        .uncached_input
                        .zip(amount.cached_input)
                        .and_then(|(uncached, cached)| uncached.checked_add(cached)),
                    uncached_input_tokens: amount.uncached_input,
                    cached_input_tokens: amount.cached_input,
                    output_tokens: amount.output,
                    requests: None,
                    turns: amount.turns,
                    credits: amount.credits,
                    relative_usage: amount.relative,
                })
                .collect(),
        )
    }

    fn reports(&self) -> impl Iterator<Item = &ObservedReport> {
        self.usage.iter().chain(self.turns.iter())
    }
}

pub(super) async fn collect(home: &Path, capture: &Capture, kind: Kind, collected: &mut Collected) {
    let end = Utc::now().date_naive();
    let start = end - Days::new(DAYS - 1);
    if let Some(payload) = fetch_json(
        home,
        capture,
        Endpoint::ModelUsage { kind, start, end },
        SOURCE,
        collected,
    )
    .await
    {
        match usage_rows(payload, kind, start, end) {
            Ok(Some(rows)) => {
                collected.snapshot.model_usage.usage = Some(ObservedReport::new(rows))
            }
            Ok(None) => collected
                .failures
                .push(failure(SOURCE, "订阅用量响应未报告模型明细")),
            Err(_) => collected
                .failures
                .push(failure(SOURCE, "订阅模型用量响应格式无效，请稍后刷新")),
        }
    }
    if !collected.expired
        && let Some(payload) = fetch_json(
            home,
            capture,
            Endpoint::ModelTurns { start, end },
            SOURCE,
            collected,
        )
        .await
    {
        match turn_rows(payload, start, end) {
            Ok(rows) => collected.snapshot.model_usage.turns = rows.map(ObservedReport::new),
            Err(_) => collected
                .failures
                .push(failure(SOURCE, "订阅模型轮次响应格式无效，请稍后刷新")),
        }
    }
    collected.succeeded = collected.snapshot.model_usage.observed_at().is_some();
}

#[derive(Clone, Default)]
struct Amounts {
    uncached_input: Option<u64>,
    cached_input: Option<u64>,
    output: Option<u64>,
    turns: Option<u64>,
    credits: Option<f64>,
    relative: Option<f64>,
}

impl Amounts {
    fn add_parts(&mut self, part: Self) {
        // Groups of a model can be partitioned by speed or product surface. A missing
        // component in any part leaves that metric unknown for the model and day.
        let counts = |a: Option<u64>, b: Option<u64>| a.zip(b).and_then(|(a, b)| a.checked_add(b));
        let amounts = |a: Option<f64>, b: Option<f64>| {
            a.zip(b).map(|(a, b)| a + b).filter(|sum| sum.is_finite())
        };
        self.uncached_input = counts(self.uncached_input, part.uncached_input);
        self.cached_input = counts(self.cached_input, part.cached_input);
        self.output = counts(self.output, part.output);
        self.turns = counts(self.turns, part.turns);
        self.credits = amounts(self.credits, part.credits);
        self.relative = amounts(self.relative, part.relative);
    }
}

#[derive(Deserialize)]
struct Report {
    data: Vec<Day>,
    units: Option<String>,
}

#[derive(Deserialize)]
struct Day {
    date: String,
    models: Option<Vec<Model>>,
    groups: Option<Vec<Group>>,
    attribution: Option<Vec<Attribution>>,
}

#[derive(Default, Deserialize)]
struct ReportedAmounts {
    uncached_text_input_tokens: Option<u64>,
    cached_text_input_tokens: Option<u64>,
    text_output_tokens: Option<u64>,
    turns: Option<u64>,
    credits: Option<f64>,
    on_demand_credits: Option<f64>,
}

impl ReportedAmounts {
    fn token_counts(&self) -> Amounts {
        Amounts {
            uncached_input: self.uncached_text_input_tokens,
            cached_input: self.cached_text_input_tokens,
            output: self.text_output_tokens,
            ..Amounts::default()
        }
    }
}

#[derive(Deserialize)]
struct Model {
    model: String,
    #[serde(flatten)]
    amounts: ReportedAmounts,
}

#[derive(Deserialize)]
struct Group {
    dimensions: BTreeMap<String, String>,
    is_other: Option<bool>,
    #[serde(flatten)]
    amounts: ReportedAmounts,
}

#[derive(Deserialize)]
struct Attribution {
    model: String,
    value: Option<f64>,
}

fn nonnegative(amount: Option<f64>) -> Option<f64> {
    amount.filter(|amount| amount.is_finite() && *amount >= 0.0)
}

fn days(
    report: Vec<Day>,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<(NaiveDate, Day)>, &'static str> {
    let mut included = Vec::new();
    for day in report {
        let date = day
            .date
            .parse::<NaiveDate>()
            .or_else(|_| {
                chrono::DateTime::parse_from_rfc3339(&day.date).map(|date| date.date_naive())
            })
            .map_err(|_| "invalid daily report date")?;
        if (start..=end).contains(&date) {
            included.push((date, day));
        }
    }
    Ok(included)
}

fn usage_rows(
    payload: Value,
    kind: Kind,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Option<Rows>, &'static str> {
    let report: Report = serde_json::from_value(payload).map_err(|_| "invalid model usage")?;
    let mut available = report.data.is_empty();
    let daily = days(report.data, start, end)?;
    // Codex selects the richer attribution source when every reported day has it.
    // Otherwise it uses the legacy model breakdown for the whole period.
    let attributed =
        kind == Kind::Personal && daily.iter().all(|(_, day)| day.attribution.is_some());
    let legacy_credits = report.units.as_deref() == Some("credits");
    let mut rows = Rows::new();
    for (date, day) in daily {
        available |= day.models.is_some()
            || (attributed && day.attribution.is_some())
            || (kind != Kind::Personal && day.groups.is_some());
        let use_groups = kind != Kind::Personal && day.groups.is_some();
        let mut models = Rows::new();
        for model in day.models.into_iter().flatten() {
            let amounts = model.amounts;
            let mut record = if use_groups {
                Amounts::default()
            } else {
                amounts.token_counts()
            };
            record.turns = amounts.turns;
            if kind != Kind::Personal {
                record.credits = nonnegative(amounts.on_demand_credits);
            } else if !attributed {
                if legacy_credits {
                    record.credits = nonnegative(amounts.credits);
                } else {
                    record.relative = nonnegative(amounts.credits);
                }
            }
            add_row(&mut models, date, model.model, record);
        }
        if use_groups {
            let mut groups = Rows::new();
            for group in day.groups.into_iter().flatten() {
                let model = if group.is_other == Some(true) {
                    "other".into()
                } else {
                    let Some(model) = group.dimensions.get("model") else {
                        continue;
                    };
                    model.clone()
                };
                let mut amounts = group.amounts.token_counts();
                amounts.credits = nonnegative(group.amounts.on_demand_credits);
                add_row(&mut groups, date, model, amounts);
            }
            for (key, amounts) in groups {
                let model = models.entry(key).or_default();
                model.uncached_input = amounts.uncached_input;
                model.cached_input = amounts.cached_input;
                model.output = amounts.output;
                model.credits = model.credits.or(amounts.credits);
            }
        }
        if attributed {
            let mut attribution = Rows::new();
            for entry in day.attribution.into_iter().flatten() {
                add_row(
                    &mut attribution,
                    date,
                    entry.model,
                    Amounts {
                        relative: nonnegative(entry.value),
                        ..Amounts::default()
                    },
                );
            }
            for (key, amounts) in attribution {
                models.entry(key).or_default().relative = amounts.relative;
            }
        }
        for ((date, model), amounts) in models {
            add_row(&mut rows, date, model, amounts);
        }
    }
    Ok(available.then_some(rows))
}

fn turn_rows(
    payload: Value,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Option<Rows>, &'static str> {
    let report: Report = serde_json::from_value(payload).map_err(|_| "invalid model turns")?;
    let mut available = report.data.is_empty();
    let mut rows = Rows::new();
    for (date, day) in days(report.data, start, end)? {
        available |= day.models.is_some();
        for model in day.models.into_iter().flatten() {
            add_row(
                &mut rows,
                date,
                model.model,
                Amounts {
                    turns: model.amounts.turns,
                    ..Amounts::default()
                },
            );
        }
    }
    Ok(available.then_some(rows))
}

fn add_row(rows: &mut Rows, date: NaiveDate, model: String, amounts: Amounts) {
    use std::collections::btree_map::Entry;
    if model.trim().is_empty() {
        return;
    }
    match rows.entry((date, model)) {
        Entry::Occupied(mut row) => row.get_mut().add_parts(amounts),
        Entry::Vacant(row) => {
            row.insert(amounts);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_day(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, day).unwrap()
    }

    #[test]
    fn complete_personal_attribution_uses_relative_values_even_with_credit_units() {
        let usage = usage_rows(
            json!({"units": "credits", "data": [
                {"date": "2026-10-01"},
                {"date": "2026-11-02", "models": [{"model": "alpha", "credits": 90.0}],
                    "attribution": [
                        {"model": "alpha", "value": 1.5},
                        {"model": "alpha", "value": 2.5},
                        {"model": "beta", "value": 0.0}
                    ]}
            ]}),
            Kind::Personal,
            test_day(1),
            test_day(30),
        )
        .unwrap()
        .unwrap();
        assert_eq!(usage.len(), 2);
        let alpha = &usage[&(test_day(2), "alpha".into())];
        assert_eq!(alpha.relative, Some(4.0));
        assert_eq!(alpha.credits, None);
        assert_eq!(alpha.uncached_input, None);
        assert_eq!(usage[&(test_day(2), "beta".into())].relative, Some(0.0));
    }

    #[test]
    fn legacy_personal_breakdown_preserves_explicit_units_and_missing_metrics() {
        for (unit, expected_credits, expected_relative) in [
            (Some("credits"), Some(12.5), None),
            (Some("percent"), None, Some(12.5)),
            (None, None, Some(12.5)),
        ] {
            let usage = usage_rows(
                json!({"units": unit, "data": [
                    {"date": "2026-11-02",
                        "models": [{"model": "alpha", "credits": 12.5}, {"model": "beta"}]},
                    {"date": "2026-11-03",
                        "models": [{"model": "alpha"}],
                        "attribution": [{"model": "alpha", "value": 42.0}]}
                ]}),
                Kind::Personal,
                test_day(1),
                test_day(30),
            )
            .unwrap()
            .unwrap();
            let alpha = &usage[&(test_day(2), "alpha".into())];
            assert_eq!(alpha.credits, expected_credits);
            assert_eq!(alpha.relative, expected_relative);
            assert_eq!(usage[&(test_day(2), "beta".into())].relative, None);
            assert_eq!(usage[&(test_day(3), "alpha".into())].relative, None);
        }
    }

    #[test]
    fn workspace_tokens_and_turns_use_named_metrics_without_adding_overlap() {
        let usage = usage_rows(
            json!({"data": [{"date": "2026-11-02",
                "groups": [
                    {"dimensions": {"model": "alpha", "speed": "standard"},
                        "uncached_text_input_tokens": 10, "cached_text_input_tokens": 20,
                        "text_output_tokens": 30},
                    {"dimensions": {"model": "alpha", "speed": "fast"},
                        "uncached_text_input_tokens": 40, "cached_text_input_tokens": 50,
                        "text_output_tokens": 60},
                    {"dimensions": {"model": "beta"}, "credits": 0,
                        "uncached_text_input_tokens": 5, "text_output_tokens": 0}
                ],
                "models": [{"model": "alpha", "turns": 5,
                    "uncached_text_input_tokens": 1000, "on_demand_credits": 1.25}]
            }]}),
            Kind::Business,
            test_day(1),
            test_day(30),
        )
        .unwrap()
        .map(ObservedReport::new);
        let turns = turn_rows(
            json!({"data": [{"date": "2026-11-02", "totals": {"turns": 300},
                "models": [{"model": "alpha", "turns": 4}, {"model": "beta", "turns": 0}]
            }]}),
            test_day(1),
            test_day(30),
        )
        .unwrap()
        .map(ObservedReport::new);
        let rows = Snapshot { usage, turns }.into_rows().unwrap();
        let alpha = &rows[0];
        assert_eq!(alpha.model, "alpha");
        assert_eq!(alpha.date.as_deref(), Some("2026-11-02"));
        assert_eq!(alpha.input_tokens, Some(120));
        assert_eq!(alpha.uncached_input_tokens, Some(50));
        assert_eq!(alpha.cached_input_tokens, Some(70));
        assert_eq!(alpha.output_tokens, Some(90));
        assert_eq!(alpha.turns, Some(4));
        assert_eq!(alpha.credits, Some(1.25));
        assert_eq!(alpha.requests, None);
        assert_eq!(rows[1].input_tokens, None);
        assert_eq!(rows[1].uncached_input_tokens, Some(5));
        assert_eq!(rows[1].output_tokens, Some(0));
        assert_eq!(rows[1].turns, Some(0));
        assert_eq!(rows[1].credits, None);
    }

    #[test]
    fn absent_model_groups_and_explicit_empty_report_are_distinct() {
        let start = test_day(1);
        let end = test_day(30);
        let payload = json!({"data": [{"date": "2026-11-02", "product_surface_usage_values": {}}]});
        assert!(
            usage_rows(payload.clone(), Kind::Personal, start, end)
                .unwrap()
                .is_none()
        );
        assert!(turn_rows(payload, start, end).unwrap().is_none());
        assert!(
            usage_rows(json!({"data": []}), Kind::Personal, start, end)
                .unwrap()
                .unwrap()
                .is_empty()
        );
        assert!(usage_rows(json!({}), Kind::Personal, start, end).is_err());
    }
}
