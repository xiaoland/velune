//! Lossless schedule editing. Simple controls are a projection, never a second
//! persisted schedule; unrecognised rules stay custom.
use super::{Error, occurrences};
use crate::{SchedulePattern as Pattern, SchedulePreview};
use chrono::{TimeZone, Utc};
use std::collections::BTreeMap;

pub(super) fn normalize(raw: &str, time_zone: &str, at: i64) -> Result<String, Error> {
    let zone = time_zone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| Error::invalid("请选择有效的 IANA 时区"))?;
    if raw.len() > 4096 {
        return Err(Error::invalid("定时规则过长"));
    }
    let raw = raw.trim().replace("\r\n", "\n");
    if let Some(start) = raw.lines().find(|line| line.starts_with("DTSTART")) {
        let (property, value) = start
            .split_once(':')
            .ok_or_else(|| Error::invalid("DTSTART 无效"))?;
        if let Some(old_zone) = property.strip_prefix("DTSTART;TZID=") {
            // Keep wall-clock time, including exception dates in that same zone.
            return Ok(raw
                .lines()
                .map(|line| {
                    if let Some((property, value)) = line.split_once(':')
                        && ["DTSTART", "RDATE", "EXDATE"]
                            .iter()
                            .any(|name| property == format!("{name};TZID={old_zone}"))
                    {
                        return format!(
                            "{};TZID={time_zone}:{value}",
                            property.split(';').next().unwrap()
                        );
                    }
                    line.to_owned()
                })
                .collect::<Vec<_>>()
                .join("\n"));
        }
        if property == "DTSTART" && value.ends_with('Z') && time_zone == "UTC" {
            return Ok(raw);
        }
        return Err(Error::invalid(
            "高级规则的 DTSTART 需要显式 TZID；请核对规则与时区",
        ));
    }
    let date = Utc
        .timestamp_millis_opt(at)
        .single()
        .ok_or_else(|| Error::invalid("调度时间超出范围"))?
        .with_timezone(&zone);
    Ok(format!(
        "DTSTART;TZID={time_zone}:{}\nRRULE:{}",
        date.format("%Y%m%dT%H%M%S"),
        raw.strip_prefix("RRULE:").unwrap_or(&raw)
    ))
}

pub(crate) fn preview(
    raw: &str,
    time_zone: &str,
    pattern: Option<Pattern>,
    at: i64,
) -> Result<SchedulePreview, Error> {
    let normalized = normalize(raw, time_zone, at)?;
    let rule = match pattern {
        None | Some(Pattern::Custom) => normalized,
        Some(pattern) => {
            let time = |hour: u8, minute: u8| {
                if hour > 23 || minute > 59 {
                    Err(Error::invalid("请选择有效的小时和分钟"))
                } else {
                    Ok(format!("BYHOUR={hour};BYMINUTE={minute};BYSECOND=0"))
                }
            };
            let rule = match pattern {
                Pattern::Daily { hour, minute } => format!("FREQ=DAILY;{}", time(hour, minute)?),
                Pattern::Weekly {
                    mut weekdays,
                    hour,
                    minute,
                } => {
                    if weekdays.is_empty() || weekdays.iter().any(|day| !(1..=7).contains(day)) {
                        return Err(Error::invalid("每周至少选择一天"));
                    }
                    weekdays.sort();
                    weekdays.dedup();
                    let days = weekdays
                        .iter()
                        .map(|day| ["MO", "TU", "WE", "TH", "FR", "SA", "SU"][*day as usize - 1])
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("FREQ=WEEKLY;BYDAY={days};{}", time(hour, minute)?)
                }
                Pattern::Hourly { interval } if (1..=168).contains(&interval) => {
                    format!("FREQ=HOURLY;INTERVAL={interval};BYMINUTE=0;BYSECOND=0")
                }
                _ => return Err(Error::invalid("小时间隔应在 1 到 168 之间")),
            };
            let start = normalized
                .lines()
                .find(|line| line.starts_with("DTSTART"))
                .expect("normalized start");
            format!("{start}\nRRULE:{rule}")
        }
    };
    let next_at_unix_ms = occurrences(&rule, time_zone, at, 3)?;
    Ok(SchedulePreview {
        pattern: recognize(&rule),
        rrule: rule,
        time_zone: time_zone.into(),
        next_at_unix_ms,
    })
}

fn recognize(raw: &str) -> Pattern {
    fn simple(raw: &str) -> Option<Pattern> {
        let lines: Vec<_> = raw
            .lines()
            .filter(|line| !line.starts_with("DTSTART"))
            .collect();
        if lines.len() != 1 {
            return None;
        }
        let mut fields = BTreeMap::new();
        for part in lines[0]
            .strip_prefix("RRULE:")
            .unwrap_or(lines[0])
            .split(';')
        {
            let (key, value) = part.split_once('=')?;
            if fields.insert(key, value).is_some() {
                return None;
            }
        }
        let frequency = fields.remove("FREQ")?;
        let interval = fields
            .remove("INTERVAL")
            .unwrap_or("1")
            .parse::<u16>()
            .ok()?;
        let seconds = fields.remove("BYSECOND")?;
        // Without explicit minute/hour, RFC rules inherit DTSTART; keep them custom.
        if seconds != "0" {
            return None;
        }
        let minute = fields.remove("BYMINUTE")?.parse::<u8>().ok()?;
        if minute > 59 {
            return None;
        }
        if frequency == "HOURLY"
            && minute == 0
            && fields.is_empty()
            && (1..=168).contains(&interval)
        {
            return Some(Pattern::Hourly { interval });
        }
        if interval != 1 {
            return None;
        }
        let hour = fields.remove("BYHOUR")?.parse::<u8>().ok()?;
        if hour > 23 {
            return None;
        }
        match frequency {
            "DAILY" if fields.is_empty() => Some(Pattern::Daily { hour, minute }),
            "WEEKLY" => {
                let days = fields.remove("BYDAY")?;
                if !fields.is_empty() {
                    return None;
                }
                let mut weekdays = days
                    .split(',')
                    .map(|day| {
                        ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]
                            .iter()
                            .position(|known| *known == day)
                            .map(|index| index as u8 + 1)
                    })
                    .collect::<Option<Vec<_>>>()?;
                weekdays.sort();
                weekdays.dedup();
                if weekdays.is_empty() {
                    return None;
                }
                Some(Pattern::Weekly {
                    weekdays,
                    hour,
                    minute,
                })
            }
            _ => None,
        }
    }
    simple(raw).unwrap_or(Pattern::Custom)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(value: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(value)
            .unwrap()
            .timestamp_millis()
    }
    #[test]
    fn simple_rules_round_trip_and_preview_is_actual_calendar() {
        let rule = "DTSTART;TZID=Asia/Shanghai:20260101T090000\nRRULE:FREQ=DAILY;BYHOUR=9;BYMINUTE=0;BYSECOND=0";
        let weekly = Pattern::Weekly {
            weekdays: vec![1, 5],
            hour: 9,
            minute: 30,
        };
        let result = preview(
            rule,
            "Asia/Shanghai",
            Some(weekly.clone()),
            at("2026-11-02T09:31:00+08:00"),
        )
        .unwrap();
        assert_eq!(result.pattern, weekly);
        assert_eq!(result.next_at_unix_ms[0], at("2026-11-06T09:30:00+08:00"));
        assert_eq!(
            occurrences(
                &result.rrule,
                "Asia/Shanghai",
                at("2026-11-02T09:31:00+08:00"),
                1
            )
            .unwrap()[0],
            result.next_at_unix_ms[0]
        );
    }
    #[test]
    fn time_zone_edit_preserves_wall_time_and_dst() {
        let rule = "DTSTART;TZID=Asia/Shanghai:20260101T090000\nRRULE:FREQ=DAILY;BYHOUR=9;BYMINUTE=0;BYSECOND=0";
        let result = preview(
            rule,
            "America/New_York",
            None,
            at("2026-10-31T08:00:00-04:00"),
        )
        .unwrap();
        assert_eq!(
            result.next_at_unix_ms,
            vec![
                at("2026-10-31T09:00:00-04:00"),
                at("2026-11-01T09:00:00-05:00"),
                at("2026-11-02T09:00:00-05:00")
            ]
        );
        assert!(
            result
                .rrule
                .starts_with("DTSTART;TZID=America/New_York:20260101T090000")
        );
    }
    #[test]
    fn complex_rules_are_not_approximated() {
        let raw = "DTSTART;TZID=Asia/Shanghai:20260101T090000\nRRULE:FREQ=MONTHLY;BYDAY=MO;BYSETPOS=1;COUNT=20";
        let result = preview(raw, "Asia/Shanghai", None, at("2026-11-02T08:00:00+08:00")).unwrap();
        assert_eq!(result.pattern, Pattern::Custom);
        assert_eq!(result.rrule, raw);
        assert!(
            preview(
                raw,
                "Asia/Shanghai",
                Some(Pattern::Weekly {
                    weekdays: vec![],
                    hour: 9,
                    minute: 0
                }),
                at("2026-11-02T08:00:00+08:00")
            )
            .is_err()
        );
    }
}
