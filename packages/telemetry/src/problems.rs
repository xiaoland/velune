//! The problem panel is a projection of the installation's existing local log,
//! across runs. Recovery and explicit dismissal are journal events in that log.
use crate::{ClientTelemetry, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::BufRead,
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
    pub peer_id: Option<String>,
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
        // Journal callers require a write-and-flush receipt; ordinary tracing
        // only enqueues. A timeout is indeterminate and is not replayed here.
        writer.persist(line)?;
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
        // Read one committed boundary. Keep the same directory lock as append
        // and prefix retention until replay ends: reading an old source while
        // missing its concurrently pruned cleanup would resurrect the problem.
        let _committed = crate::local_log::lock_directory(&directory)?;
        let files = crate::local_log::ordered_logs(&directory)?;
        let mut problems = BTreeMap::<String, DiagnosticProblem>::new();
        // Operations already represented, so a failure's twin lines and its UI
        // record never become separate problems.
        let mut operations = HashSet::<String>::new();
        // A background poll can fail the same way thousands of times; repeated
        // backfilled failures stay one open problem at their latest occurrence.
        let mut repeated =
            HashMap::<(String, String, Option<String>, Option<String>, String), String>::new();
        for file in files {
            let input = std::fs::File::open(&file)?;
            let mut input = std::io::BufReader::new(input);
            let mut index = 0;
            let mut line = Vec::new();
            loop {
                line.clear();
                if input.read_until(b'\n', &mut line)? == 0 || !line.ends_with(b"\n") {
                    break; // An unterminated tail is not a committed journal line.
                }
                let line_index = index;
                index += 1;
                // Interrupted or otherwise invalid lines do not hide earlier diagnostics.
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
                            problem.operation_id = problem.operation_id.filter(|id| !id.is_empty());
                            if let Some(operation) = &problem.operation_id {
                                problems.retain(|_, p| p.operation_id.as_ref() != Some(operation));
                                operations.insert(operation.clone());
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
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned);
                if let Some(id) = &operation_id
                    && !operations.insert(id.clone())
                {
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
                let detail = field("detail")
                    .or_else(|| field("message"))
                    .unwrap_or_else(|| "操作失败；当时的日志未记录原始详情".into());
                let kind = field("failure_kind").unwrap_or_else(|| "local".into());
                let repeat = (
                    source.clone(),
                    kind.clone(),
                    field("code"),
                    field("phase"),
                    detail.clone(),
                );
                if let Some(open) = repeated
                    .get(&repeat)
                    .and_then(|id| problems.get_mut(id))
                    .filter(|p| p.resolved_at_unix_ms.is_none())
                {
                    open.occurred_at_unix_ms = open.occurred_at_unix_ms.max(occurred);
                    continue;
                }
                let key = format!(
                    "{}:{line_index}",
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
                    detail,
                    kind,
                    code: field("code"),
                    phase: field("phase"),
                    operation_id,
                    peer_id: None,
                    activity_key: if field("event").as_deref()
                        == Some("runtime_history_read_failed")
                    {
                        field("runtime_id").map(|id| format!("history:{id}"))
                    } else {
                        None
                    },
                    resolved_at_unix_ms: None,
                };
                repeated.insert(repeat, id.clone());
                problems.insert(id, problem);
            }
        }
        if let Some(health) = self
            .local_log_health()
            .filter(|health| health.dropped_records > 0 || health.failed_writes > 0)
        {
            problems.insert(
                "local-log-health".into(),
                DiagnosticProblem {
                    id: "4F43414C-4C4F-4748-4541-4C5448000001".into(),
                    occurred_at_unix_ms: now(),
                    source: "本地诊断存储".into(),
                    detail: format!(
                        "本次进程拒绝或未确认 {} 条诊断记录，磁盘写入失败 {} 次；日志可能不完整。",
                        health.dropped_records, health.failed_writes
                    ),
                    kind: "io".into(),
                    code: Some("local_log_incomplete".into()),
                    phase: Some("local_log".into()),
                    operation_id: None,
                    peer_id: None,
                    activity_key: Some("local-log-health".into()),
                    resolved_at_unix_ms: None,
                },
            );
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

    #[test]
    fn repeated_backfilled_failures_stay_one_problem() {
        let home = tempfile::tempdir().unwrap();
        let logs = home.path().join("logs");
        std::fs::create_dir(&logs).unwrap();
        let failure = |second: u32, operation: &str| {
            let span = json!({"velune.operation":"bindings.synthetic_poll","velune.operation.id":operation});
            format!(
                "{}\n{}\n",
                json!({"timestamp":format!("2026-01-01T00:00:{second:02}Z"),"level":"ERROR","target":"velune_bindings","span":span,
                    "fields":{"event":"operation_failed","code":"closed","phase":"remote","failure_kind":"closed","detail":"synthetic closed"}}),
                json!({"timestamp":format!("2026-01-01T00:00:{second:02}Z"),"level":"ERROR","target":"velune_telemetry","span":span,
                    "fields":{"velune.outcome":"failure","error.type":"closed"}}),
            )
        };
        let early = (0..30)
            .map(|n| failure(n, &format!("early-{n}")))
            .collect::<String>();
        std::fs::write(logs.join("velune.2026-01-01.jsonl"), early).unwrap();
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
        let problems = first.problems("test").unwrap();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].detail, "synthetic closed");
        assert_eq!(problems[0].occurred_at_unix_ms, 1_767_225_629_000);
        first
            .dismiss_problems("test", vec![problems[0].id.clone()])
            .unwrap();
        first.shutdown().unwrap();
        drop(first);

        // A recurrence appended after the dismissal is a new problem; the
        // dismissed history does not come back.
        let second = open();
        second
            .writer
            .as_ref()
            .unwrap()
            .persist(failure(59, "late").into_bytes())
            .unwrap();
        let problems = second.problems("test").unwrap();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].operation_id.as_deref(), Some("late"));
        second.shutdown().unwrap();
    }

    fn synthetic_problem(id: &str) -> DiagnosticProblem {
        DiagnosticProblem {
            id: id.into(),
            occurred_at_unix_ms: 1,
            source: "synthetic".into(),
            detail: "synthetic shared journal failure".into(),
            kind: "local".into(),
            code: None,
            phase: None,
            operation_id: None,
            peer_id: None,
            activity_key: None,
            resolved_at_unix_ms: None,
        }
    }

    #[test]
    fn later_writer_records_are_resolved_and_dismissed_by_the_earlier_writer() {
        let home = tempfile::tempdir().unwrap();
        let open = || {
            ClientTelemetry::open(
                home.path(),
                TelemetryIdentity::new("test", "0.1").unwrap(),
                true,
                None,
            )
            .unwrap()
        };
        let a = open();
        let b = open();
        let p = synthetic_problem("00000000-0000-0000-0000-000000000001");
        b.record_problem("test", p.clone()).unwrap();
        // A complete JSON value without its commit newline is still a bad tail.
        // The next successful resolution must neither promote it nor stick to it.
        let logs = home.path().join("logs");
        {
            use std::io::Write;
            let _lock = crate::local_log::lock_directory(&logs).unwrap();
            let files = crate::local_log::ordered_logs(&logs).unwrap();
            let partial = json!({"target":"velune_problems","fields":{
                "scope":"test","event":"problem_recorded",
                "problem":synthetic_problem("00000000-0000-0000-0000-000000000003")}});
            std::fs::OpenOptions::new()
                .append(true)
                .open(files.last().unwrap())
                .unwrap()
                .write_all(&serde_json::to_vec(&partial).unwrap())
                .unwrap();
        }
        assert_eq!(a.problems("test").unwrap().len(), 1);
        a.resolve_problems("test", vec![p.id.clone()]).unwrap();
        let q = synthetic_problem("00000000-0000-0000-0000-000000000002");
        b.record_problem("test", q.clone()).unwrap();
        a.dismiss_problems("test", vec![q.id]).unwrap();
        for client in [&a, &b] {
            let problems = client.problems("test").unwrap();
            assert_eq!(problems.len(), 1);
            assert_eq!(problems[0].id, p.id);
            assert!(problems[0].resolved_at_unix_ms.is_some());
        }
        a.shutdown().unwrap();
        b.shutdown().unwrap();
    }

    #[test]
    fn prefix_retention_never_drops_a_cleanup_while_retaining_its_source() {
        let home = tempfile::tempdir().unwrap();
        let open = || {
            ClientTelemetry::open(
                home.path(),
                TelemetryIdentity::new("test", "0.1").unwrap(),
                true,
                None,
            )
            .unwrap()
        };
        let a = open();
        let p = synthetic_problem("00000000-0000-0000-0000-000000000001");
        a.record_problem("test", p.clone()).unwrap();
        let logs = home.path().join("logs");
        let source = {
            let _lock = crate::local_log::lock_directory(&logs).unwrap();
            crate::local_log::ordered_logs(&logs)
                .unwrap()
                .pop()
                .unwrap()
        };
        let b = open();
        let mut filler = vec![b'x'; 256 * 1024];
        *filler.last_mut().unwrap() = b'\n';
        // Real 8 MiB of writes moves B's cleanup to a later shard while A lives.
        for _ in 0..32 {
            b.writer.as_ref().unwrap().persist(filler.clone()).unwrap();
        }
        b.dismiss_problems("test", vec![p.id]).unwrap();
        b.shutdown().unwrap();
        let cleanup = {
            let _lock = crate::local_log::lock_directory(&logs).unwrap();
            crate::local_log::ordered_logs(&logs)
                .unwrap()
                .pop()
                .unwrap()
        };
        assert!(cleanup > source);
        assert!(source.exists());
        let c = crate::local_log::LocalWriter::new(logs.clone()).unwrap();
        for _ in 0..8 {
            for _ in 0..32 {
                c.persist(filler.clone()).unwrap();
            }
            // This deterministically failed with per-writer active pins: A's
            // source survived while B's closed cleanup was evicted around it.
            if source.exists() {
                assert!(cleanup.exists());
            }
            assert!(a.problems("test").unwrap().is_empty());
        }
        assert!(!source.exists());
        c.shutdown().unwrap();
        a.shutdown().unwrap();
    }
}
