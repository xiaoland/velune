//! Parse the structured output of Antigravity's read-only /usage command.
use serde_json::Value;
use std::collections::HashSet;
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
    parts.next().is_none() && (major, minor, patch) >= (1, 1, 11)
}

pub(super) fn parse(output: &str, observed: i64, ttl_ms: i64) -> Result<Vec<QuotaWindow>, ()> {
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
    for bucket in command["data"]["groups"]
        .as_array()
        .ok_or(())?
        .iter()
        .filter_map(|group| group["buckets"].as_array())
        .flatten()
    {
        if !bucket["disabled"].is_null() && bucket["disabled"] != false {
            continue;
        }
        let Some(id) = bucket["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.trim() == *id && !id.chars().any(char::is_control))
        else {
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
        windows.push(QuotaWindow {
            id: id.to_owned(),
            model_ids: Some(Vec::new()),
            used_percent: (1.0 - remaining) * 100.0,
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
    if windows.is_empty() {
        return Err(());
    }
    Ok(windows)
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
        let windows = parse(REPORT, observed, 600_000).unwrap();
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
        let windows = parse(&report.to_string(), 1000, 2000).unwrap();
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
            assert!(parse(&bad.to_string(), 1, 100).is_err(), "{field}");
        }
    }
}
