//! The problem panel is a projection of the installation's existing local log,
//! across runs. Recovery and explicit dismissal are journal events in that log.
use crate::{ClientTelemetry, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, Write},
};

#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Record)]
pub struct DiagnosticProblem {
    pub id: String,
    pub occurred_at_unix_ms: i64,
    pub source: String,
    pub detail: String,
    pub kind: String,
    pub code: Option<String>,
    pub phase: Option<String>,
    pub operation_id: Option<String>,
    pub activity_key: Option<String>,
    pub resolved_at_unix_ms: Option<i64>,
}

impl ClientTelemetry {
    pub fn record_problem(&self, scope: &str, problem: DiagnosticProblem) -> Result<()> {
        self.problem_event(scope, json!({"event":"problem_recorded","problem":problem}))
    }

    pub fn resolve_problems(&self, scope: &str, ids: Vec<String>) -> Result<()> {
        self.problem_event(scope, json!({"event":"problems_resolved","ids":ids}))
    }

    pub fn dismiss_problems(&self, scope: &str, ids: Vec<String>) -> Result<()> {
        self.problem_event(scope, json!({"event":"problems_dismissed","ids":ids}))
    }

    fn problem_event(&self, scope: &str, mut fields: Value) -> Result<()> {
        let writer = self.writer.as_ref().ok_or(Error::Unavailable)?;
        fields["scope"] = json!(scope);
        let entry = json!({"timestamp_unix_ms":now(),"level":"INFO","target":"velune_problems","fields":fields});
        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        // Share the actual rolling writer, including its lock and retention.
        // No second error file, OSLog-only path, or export of local details.
        let mut output = writer.0.lock().map_err(|_| Error::Unavailable)?;
        output.write_all(&line)?;
        output.flush()?;
        Ok(())
    }

    pub fn problems(&self, scope: &str) -> Result<Vec<DiagnosticProblem>> {
        let directory = self
            .directory
            .parent()
            .ok_or(Error::StorageFormat)?
            .join("logs");
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut files = std::fs::read_dir(directory)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.starts_with("velune.") && s.ends_with(".jsonl"))
            })
            .collect::<Vec<_>>();
        files.sort();
        let mut problems = BTreeMap::<String, DiagnosticProblem>::new();
        for file in files {
            for (index, line) in std::io::BufReader::new(std::fs::File::open(&file)?)
                .split(b'\n')
                .enumerate()
            {
                let line = line?;
                // An interrupted final write must not hide earlier diagnostics.
                let Ok(entry) = serde_json::from_slice::<LogEntry>(&line) else {
                    continue;
                };
                if entry.target == "velune_problems" {
                    if entry.fields.get("scope").and_then(Value::as_str) != Some(scope) {
                        continue;
                    }
                    match entry.fields.get("event").and_then(Value::as_str) {
                        Some("problem_recorded") => {
                            let Some(value) = entry.fields.get("problem") else {
                                continue;
                            };
                            let Ok(mut problem) =
                                serde_json::from_value::<DiagnosticProblem>(value.clone())
                            else {
                                continue;
                            };
                            problem.id = problem.id.to_ascii_uppercase();
                            if let Some(operation) = &problem.operation_id {
                                problems.retain(|_, p| p.operation_id.as_ref() != Some(operation));
                            }
                            problems.insert(problem.id.clone(), problem);
                        }
                        Some(event @ ("problems_resolved" | "problems_dismissed")) => {
                            let Some(ids) = entry.fields.get("ids") else {
                                continue;
                            };
                            let Ok(ids) = serde_json::from_value::<Vec<String>>(ids.clone()) else {
                                continue;
                            };
                            for id in ids {
                                let id = id.to_ascii_uppercase();
                                if event == "problems_dismissed" {
                                    problems.remove(&id);
                                } else if let Some(problem) = problems.get_mut(&id) {
                                    problem.resolved_at_unix_ms = Some(entry.timestamp_unix_ms);
                                }
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                // Recover pre-existing binding failures and detailed native errors
                // from the same logs, even when no UI record was written.
                if entry.level != "ERROR"
                    && !(entry.level == "WARN" && entry.fields.get("detail").is_some())
                {
                    continue;
                }
                if !entry.target.starts_with("velune_") {
                    continue;
                }
                let span = entry.span.as_ref().or_else(|| entry.spans.last());
                let operation_id = span
                    .and_then(|s| s.get("velune.operation.id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if operation_id.as_ref().is_some_and(|id| {
                    problems
                        .values()
                        .any(|p| p.operation_id.as_ref() == Some(id))
                }) {
                    continue;
                }
                let field = |name: &str| {
                    entry
                        .fields
                        .get(name)
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                };
                let source = span
                    .and_then(|s| s.get("velune.operation"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| field("event"))
                    .unwrap_or(entry.target);
                let occurred = entry
                    .timestamp
                    .as_deref()
                    .and_then(|s| {
                        time::OffsetDateTime::parse(
                            s,
                            &time::format_description::well_known::Rfc3339,
                        )
                        .ok()
                    })
                    .map(|t| (t.unix_timestamp_nanos() / 1_000_000) as i64)
                    .unwrap_or(entry.timestamp_unix_ms);
                let key = format!(
                    "{}:{index}",
                    file.file_name().unwrap_or_default().to_string_lossy()
                );
                let first = key.bytes().fold(0xcbf29ce484222325u64, |h, b| {
                    (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
                });
                let second = key.bytes().rev().fold(0xcbf29ce484222325u64, |h, b| {
                    (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
                });
                let hex = format!("{first:016X}{second:016X}");
                let id = format!(
                    "{}-{}-{}-{}-{}",
                    &hex[..8],
                    &hex[8..12],
                    &hex[12..16],
                    &hex[16..20],
                    &hex[20..]
                );
                let problem = DiagnosticProblem {
                    id: id.clone(),
                    occurred_at_unix_ms: occurred,
                    source,
                    detail: field("detail")
                        .or_else(|| field("message"))
                        .unwrap_or_else(|| "操作失败；当时的日志未记录原始详情".into()),
                    kind: field("failure_kind").unwrap_or_else(|| "local".into()),
                    code: field("code"),
                    phase: field("phase"),
                    operation_id,
                    activity_key: if field("event").as_deref()
                        == Some("runtime_history_read_failed")
                    {
                        field("runtime_id").map(|id| format!("history:{id}"))
                    } else {
                        None
                    },
                    resolved_at_unix_ms: None,
                };
                problems.insert(id, problem);
            }
        }
        let mut values = problems.into_values().collect::<Vec<_>>();
        values.sort_by_key(|p| p.occurred_at_unix_ms);
        Ok(values)
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
#[derive(Deserialize)]
struct LogEntry {
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    timestamp_unix_ms: i64,
    #[serde(default)]
    level: String,
    #[serde(default)]
    target: String,
    #[serde(default)]
    fields: serde_json::Map<String, Value>,
    #[serde(default)]
    span: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    spans: Vec<serde_json::Map<String, Value>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TelemetryIdentity;

    #[test]
    fn historical_problems_survive_recovery_and_explicit_dismissal() {
        let home = tempfile::tempdir().unwrap();
        let logs = home.path().join("logs");
        std::fs::create_dir(&logs).unwrap();
        let legacy = json!({"timestamp":"2026-01-01T00:00:00Z","level":"ERROR","target":"velune_bindings",
            "span":{"velune.operation":"bindings.organize","velune.operation.id":"synthetic-operation"},
            "fields":{"event":"operation_failed","code":"synthetic","phase":"application","failure_kind":"invalid"}});
        let mut bytes = format!("{legacy}\n{{\"target\":\"velune_problems\",\"fields\":{{\"scope\":\"test\",\"event\":\"problem_recorded\",\"problem\":{{}}}}}}\ninterrupted write\n").into_bytes();
        bytes.push(0xff); // A crash may truncate a UTF-8 code point too.
        std::fs::write(logs.join("velune.2026-01-01.jsonl"), bytes).unwrap();
        let open = || {
            ClientTelemetry::open(
                home.path(),
                TelemetryIdentity::new("test", "0.1").unwrap(),
                true,
                None,
            )
            .unwrap()
        };
        let first = open();
        let incident = first.problems("test").unwrap().pop().unwrap();
        assert_eq!(
            incident.operation_id.as_deref(),
            Some("synthetic-operation")
        );
        assert!(incident.detail.contains("未记录原始详情"));
        first
            .resolve_problems("test", vec![incident.id.to_lowercase()])
            .unwrap();
        first.shutdown().unwrap();
        drop(first);

        let second = open();
        let history = second.problems("test").unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].resolved_at_unix_ms.is_some());
        second
            .dismiss_problems("test", vec![incident.id.to_lowercase()])
            .unwrap();
        second.shutdown().unwrap();
        drop(second);

        let third = open();
        assert!(third.problems("test").unwrap().is_empty());
        third.shutdown().unwrap();
    }
}
