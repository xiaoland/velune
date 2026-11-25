//! Parse the structured output of Antigravity's read-only /usage command.
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use velune_ai_provider::antigravity::Model;
use velune_gateway::QuotaWindow;

pub(super) fn supported_version(output: &str) -> bool {
    let mut parts = output.trim().split('.');
    let mut parse_component = || {
        parts
            .next()
            .filter(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|part| part.parse::<u32>().ok())
    };
    let (Some(major), Some(minor), Some(patch)) =
        (parse_component(), parse_component(), parse_component())
    else {
        return false;
    };
    parts.next().is_none() && (major, minor, patch) >= (1, 5, 0)
}

pub(super) fn parse(
    output: &str,
    models: &[Model],
    observed: i64,
    ttl_ms: i64,
) -> Result<Vec<QuotaWindow>, ()> {
    if observed <= 0 || ttl_ms <= 0 {
        return Err(());
    }
    let expires = observed.checked_add(ttl_ms).ok_or(())?;
    let report: Value = serde_json::from_str(output).map_err(|_| ())?;
    let command = &report["command"];
    if report["status"] != "SUCCESS"
        || command["name"] != "usage"
        || report
            .get("num_turns")
            .is_some_and(|turns| turns.as_u64() != Some(0))
        || report["usage"]
            .get("total_tokens")
            .is_some_and(|tokens| tokens.as_u64() != Some(0))
    {
        return Err(());
    }

    let mut windows = Vec::new();
    let mut seen = HashSet::new();
    let catalog = Catalog::new(models);
    for group in command["data"]["groups"].as_array().ok_or(())? {
        let Some(buckets) = group["buckets"].as_array() else {
            continue;
        };
        let model_ids = catalog.group_models(group);
        for bucket in buckets {
            if !bucket["disabled"].is_null() && bucket["disabled"] != false {
                continue;
            }
            let Some(id) = bucket["id"].as_str().filter(|id| {
                !id.is_empty() && id.trim() == *id && !id.chars().any(char::is_control)
            }) else {
                continue;
            };
            let Some(remaining) = bucket["remaining_fraction"]
                .as_f64()
                .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
            else {
                continue;
            };
            if !seen.insert(id) {
                return Err(());
            }
            let reset_at_ms = bucket["reset_time"]
                .as_str()
                .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
                .map(|date| date.timestamp_millis())
                .filter(|time| *time > 0);
            let used = (1.0 - remaining) * 100.0;
            let whole = used.round();
            // Absorb floating-point noise at whole-percent cap boundaries.
            let used_percent = if (used - whole).abs() <= 1e-12 { whole } else { used };
            windows.push(QuotaWindow {
                id: id.to_owned(),
                model_ids: Some(model_ids.clone()),
                used_percent,
                window_ms: match bucket["window"].as_str() {
                    Some("5h") => Some(18_000_000),
                    Some("weekly") => Some(604_800_000),
                    _ => None,
                },
                reset_at_ms,
                observed_at_ms: observed,
                expires_at_ms: reset_at_ms.map_or(expires, |reset| reset.min(expires)),
            });
        }
    }
    if windows.is_empty() {
        return Err(());
    }
    Ok(windows)
}

struct Catalog<'a>(Vec<Entry<'a>>);

struct Entry<'a> {
    id: &'a str,
    name: String,
    family_id: &'a str,
    family_name: String,
}

impl<'a> Catalog<'a> {
    fn new(models: &'a [Model]) -> Self {
        let mut names = HashMap::new();
        for model in models {
            if model.id.is_empty()
                || model
                    .id
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control())
            {
                continue;
            }
            let name = normalize(&model.name);
            if names
                .insert(model.id.as_str(), name.clone())
                .is_some_and(|earlier| earlier != name)
            {
                // A catalog assigning multiple names to one ID cannot prove
                // uniqueness for names that appear to match either entry.
                return Self(Vec::new());
            }
        }

        let mut entries = Vec::new();
        for (id, name) in names {
            let (family_id, family_name) = effort_family(id, &name);
            let family_name = family_name.to_owned();
            entries.push(Entry {
                id,
                name,
                family_id,
                family_name,
            });
        }
        Self(entries)
    }

    fn named(&self, name: &str) -> Option<Vec<&'a str>> {
        if name.is_empty() {
            return None;
        }
        let mut matches = self
            .0
            .iter()
            .filter(|entry| entry.name == name || entry.family_name == name);
        let first = matches.next()?;
        let mut output = vec![first.id];
        for entry in matches {
            if entry.family_id != first.family_id || entry.family_name != first.family_name {
                return Some(Vec::new());
            }
            output.push(entry.id);
        }
        Some(output)
    }

    fn group_models(&self, group: &Value) -> Vec<String> {
        let mut ids = BTreeSet::new();
        let description = normalize(group["description"].as_str().unwrap_or_default());
        if let Some(list) = description.strip_prefix("models within this group:") {
            for piece in list.trim_end_matches('.').split(',').map(str::trim) {
                let name = piece.strip_prefix("and ").unwrap_or(piece);
                if let Some(exact) = self.named(name) {
                    ids.extend(exact);
                } else {
                    for part in name.split(" and ") {
                        ids.extend(self.named(part.trim()).into_iter().flatten());
                    }
                }
            }
        } else if let Some(name) = group["name"].as_str() {
            // A whole group name can name exactly one model (for example,
            // the separate Argon pool). A vendor label is not a model list.
            ids.extend(self.named(&normalize(name)).into_iter().flatten());
        }
        ids.into_iter().map(str::to_owned).collect()
    }
}

fn normalize(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn effort_family<'a, 'b>(id: &'a str, name: &'b str) -> (&'a str, &'b str) {
    if let Some((family, effort)) = id.rsplit_once('-')
        && matches!(
            effort,
            "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        )
        && let Some((label, shown_effort)) = name
            .strip_suffix(')')
            .and_then(|name| name.rsplit_once(" ("))
        && shown_effort == effort
    {
        return (family, label);
    }
    (id, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const REPORT: &str = include_str!("../tests/fixtures/antigravity-quota/usage.json");

    #[test]
    fn parses_measured_buckets_with_their_original_identity_and_reset() {
        let observed = chrono::DateTime::parse_from_rfc3339("2031-04-03T11:19:33Z")
            .unwrap()
            .timestamp_millis();
        let windows = parse(REPORT, &[], observed, 600_000).unwrap();
        assert_eq!(windows.len(), 5);
        assert!(
            windows
                .iter()
                .all(|window| window.model_ids == Some(Vec::new()))
        );
        assert!(
            windows
                .iter()
                .all(|window| window.observed_at_ms == observed)
        );
        let third_party = windows
            .iter()
            .find(|window| window.id == "3p-weekly")
            .unwrap();
        assert_eq!(third_party.used_percent, (1.0 - 0.3360503911972046) * 100.0);
        assert_eq!(third_party.window_ms, Some(604_800_000));
        assert_eq!(
            third_party.reset_at_ms,
            Some(
                chrono::DateTime::parse_from_rfc3339("2031-04-07T09:01:10Z")
                    .unwrap()
                    .timestamp_millis()
            )
        );
        assert_eq!(third_party.expires_at_ms, observed + 600_000);
        assert_eq!(windows[3].used_percent, 100.0);
        assert_eq!(windows[3].window_ms, Some(18_000_000));
        assert_eq!(windows[3].expires_at_ms, windows[3].reset_at_ms.unwrap());
    }

    fn model(id: &str, name: &str) -> Model {
        Model {
            id: id.to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn model_scopes_follow_subscribed_catalog_and_separate_or_shared_argon() {
        let models = [
            model("gemini-4-argon-high", "Gemini 4 Argon (High)"),
            model("gemini-4-argon-medium", "Gemini 4 Argon (Medium)"),
            model("gemini-4-hydrogen-xhigh", "Gemini 4 Hydrogen (XHigh)"),
            model("gemini-3.8-flash-low", "Gemini 3.8 Flash (Low)"),
            model("gemini-pro-orchestrator-argon", "Orchestrator (Argon)"),
        ];
        let windows = parse(REPORT, &models, 1, 100).unwrap();
        assert_eq!(
            windows[0].model_ids.as_deref().unwrap(),
            ["gemini-3.8-flash-low", "gemini-4-hydrogen-xhigh"]
        );
        assert_eq!(
            windows[4].model_ids.as_deref().unwrap(),
            ["gemini-4-argon-high", "gemini-4-argon-medium"]
        );

        let mut report: Value = serde_json::from_str(REPORT).unwrap();
        let groups = report["command"]["data"]["groups"].as_array_mut().unwrap();
        groups[0]["description"] = json!(
            "Models within this group: Gemini 4 Argon, Gemini 4 Hydrogen and Gemini 3.8 Flash."
        );
        groups.pop();
        let shared_argon = parse(&report.to_string(), &models, 1, 100).unwrap();
        assert_eq!(
            shared_argon[0].model_ids.as_deref().unwrap(),
            [
                "gemini-3.8-flash-low",
                "gemini-4-argon-high",
                "gemini-4-argon-medium",
                "gemini-4-hydrogen-xhigh"
            ]
        );
        assert_eq!(shared_argon.len(), 4);
    }

    #[test]
    fn ambiguous_and_unknown_names_stay_out_of_model_scope() {
        let models = [
            model("twin-a-high", "Twin (High)"),
            model("twin-b-low", "Twin (Low)"),
            model("single", "Solo"),
            model("gemini-4-hydrogen-high", "Gemini 4 Hydrogen (High)"),
            model("engineering-and-research", "Engineering and Research"),
            model("engineering", "Engineering"),
            model("research", "Research"),
        ];
        let catalog = Catalog::new(&models);
        assert_eq!(
            catalog.group_models(&json!({
                "description": "Models within this group: Twin, Solo, Unknown",
            })),
            ["single"]
        );
        for group in [
            json!({"name": "Gemini Models"}),
            json!({"name": "Solo", "description": "Models within this group: Unrecognized label"}),
            json!({"name": "Unrecognized group", "description": "For example, messages can use Gemini 4 Hydrogen."}),
        ] {
            assert!(catalog.group_models(&group).is_empty(), "{group}");
        }
        assert_eq!(
            catalog.group_models(
                &json!({"description": "Models within this group: Engineering and Research"})
            ),
            ["engineering-and-research"]
        );
        assert_eq!(
            catalog.group_models(&json!({"description": "Models within this group: Twin (High)"})),
            ["twin-a-high"]
        );
    }

    #[test]
    fn skips_unmeasured_or_disabled_buckets_and_retains_new_ids() {
        let mut report: Value = serde_json::from_str(REPORT).unwrap();
        report["command"]["data"]["groups"] = json!([{"buckets": [
            {"id":"unknown-cadence","window":"custom","remaining_fraction":0.125,"reset_time":"2031-04-03T12:00:00+02:00"},
            {"id":"missing-fraction","window":"weekly"},
            {"id":"disabled","remaining_fraction":0,"disabled":true},
            {"id":"invalid-fraction","remaining_fraction":1.01},
            {"id":"string-fraction","remaining_fraction":"0.5"},
            {"id":"invalid-date","remaining_fraction":0.2,"reset_time":"tomorrow"}
        ]}]);
        let windows = parse(&report.to_string(), &[], 1000, 2000).unwrap();
        assert_eq!(
            windows
                .iter()
                .map(|window| window.id.as_str())
                .collect::<Vec<_>>(),
            ["unknown-cadence", "invalid-date"]
        );
        assert_eq!(windows[0].window_ms, None);
        assert_eq!(windows[0].used_percent, 87.5);
        assert_eq!(
            windows[0].reset_at_ms,
            Some(
                chrono::DateTime::parse_from_rfc3339("2031-04-03T10:00:00Z")
                    .unwrap()
                    .timestamp_millis()
            )
        );
        assert_eq!(windows[1].reset_at_ms, None);
    }

    #[test]
    fn rejects_model_answers_errors_and_reports_without_a_measured_bucket() {
        let report: Value = serde_json::from_str(REPORT).unwrap();
        assert!(
            parse(
                &json!({"status": "SUCCESS", "response": REPORT}).to_string(),
                &[],
                1,
                100
            )
            .is_err()
        );
        for (field, value) in [
            ("status", json!("ERROR")),
            (
                "command",
                json!({"name": "usage", "data": {"groups": [
                    {"buckets": [{"id":"x","remaining_fraction":0.0,"disabled":true}]}
                ]}}),
            ),
            ("num_turns", json!(1)),
            ("usage", json!({"total_tokens": 1})),
        ] {
            let mut bad = report.clone();
            bad[field] = value;
            assert!(parse(&bad.to_string(), &[], 1, 100).is_err(), "{field}");
        }
    }
}
