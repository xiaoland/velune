//! Retained quota observations, one file per provider. Only values, times and
//! an opaque binding epoch are stored; never credentials, accounts or payloads.
//! A binding whose epoch changed starts a separate history, and the previous
//! one is discarded with its next write.
use crate::QuotaHistoryPoint;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::PathBuf,
    sync::Mutex,
};

pub(crate) const RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// Unchanged values are kept at most this often, bounding a busy provider.
const UNCHANGED_INTERVAL_MS: i64 = 10 * 60 * 1000;
const PRUNE_INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Line {
    epoch: String,
    #[serde(flatten)]
    point: QuotaHistoryPoint,
}
struct Last {
    epoch: String,
    point: QuotaHistoryPoint,
    pruned_at_ms: i64,
}
pub(crate) struct QuotaHistory {
    directory: PathBuf,
    last: Mutex<BTreeMap<String, Last>>,
}
fn same_values(a: &QuotaHistoryPoint, b: &QuotaHistoryPoint) -> bool {
    a.available == b.available && a.windows == b.windows && a.balances == b.balances
}
impl QuotaHistory {
    pub(crate) fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            last: Mutex::new(BTreeMap::new()),
        }
    }
    pub(crate) fn name(gateway: &str, provider: &str) -> String {
        format!(
            "{:x}",
            Sha256::digest(format!("{gateway}\0{provider}").as_bytes())
        )
    }
    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(format!("{name}.jsonl"))
    }
    fn read_lines(&self, name: &str) -> Vec<Line> {
        std::fs::read_to_string(self.path(name))
            .map(|text| {
                text.lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect()
            })
            .unwrap_or_default()
    }
    fn rewrite(&self, name: &str, lines: &[Line]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.directory)?;
        let mut text = String::new();
        for line in lines {
            text.push_str(&serde_json::to_string(line).map_err(std::io::Error::other)?);
            text.push('\n');
        }
        let temporary = self.directory.join(format!("{name}.tmp"));
        std::fs::write(&temporary, text)?;
        std::fs::rename(temporary, self.path(name))
    }
    /// Append one successful observation. Failures leave earlier history readable.
    pub(crate) fn record(
        &self,
        gateway: &str,
        provider: &str,
        epoch: &str,
        point: QuotaHistoryPoint,
    ) {
        let name = Self::name(gateway, provider);
        let mut last = self.last.lock().expect("quota history");
        let now = point.observed_at_ms;
        let previous = match last.remove(&name) {
            Some(previous) => Some(previous),
            None => self.read_lines(&name).pop().map(|line| Last {
                epoch: line.epoch,
                point: line.point,
                pruned_at_ms: 0,
            }),
        };
        let mut pruned_at_ms = previous.as_ref().map_or(now, |p| p.pruned_at_ms);
        let result = match &previous {
            Some(previous)
                if previous.epoch == epoch
                    && previous.point.observed_at_ms >= now - UNCHANGED_INTERVAL_MS
                    && same_values(&previous.point, &point) =>
            {
                last.insert(
                    name,
                    Last {
                        epoch: epoch.into(),
                        point: previous.point.clone(),
                        pruned_at_ms,
                    },
                );
                return;
            }
            Some(previous) if previous.epoch == epoch && now - pruned_at_ms < PRUNE_INTERVAL_MS => {
                std::fs::create_dir_all(&self.directory).and_then(|()| {
                    let mut file = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(self.path(&name))?;
                    let line = Line {
                        epoch: epoch.into(),
                        point: point.clone(),
                    };
                    let mut text = serde_json::to_string(&line).map_err(std::io::Error::other)?;
                    text.push('\n');
                    file.write_all(text.as_bytes())
                })
            }
            // A new epoch replaces the file; otherwise retention is applied daily.
            _ => {
                let mut lines: Vec<Line> = self
                    .read_lines(&name)
                    .into_iter()
                    .filter(|line| {
                        line.epoch == epoch && line.point.observed_at_ms >= now - RETENTION_MS
                    })
                    .collect();
                lines.push(Line {
                    epoch: epoch.into(),
                    point: point.clone(),
                });
                pruned_at_ms = now;
                self.rewrite(&name, &lines)
            }
        };
        if result.is_ok() {
            last.insert(
                name,
                Last {
                    epoch: epoch.into(),
                    point,
                    pruned_at_ms,
                },
            );
        }
    }
    /// The epoch's retained points at or after `since_ms`, oldest first.
    pub(crate) fn read(
        &self,
        gateway: &str,
        provider: &str,
        epoch: &str,
        since_ms: i64,
        now_ms: i64,
    ) -> std::io::Result<Vec<QuotaHistoryPoint>> {
        let _guard = self.last.lock().expect("quota history");
        let text = match std::fs::read_to_string(self.path(&Self::name(gateway, provider))) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let from = since_ms.max(now_ms - RETENTION_MS);
        let mut points: Vec<_> = text
            .lines()
            .filter_map(|line| serde_json::from_str::<Line>(line).ok())
            .filter(|line| line.epoch == epoch && line.point.observed_at_ms >= from)
            .map(|line| line.point)
            .collect();
        points.sort_by_key(|point| point.observed_at_ms);
        Ok(points)
    }
    pub(crate) fn clear(&self, gateway: &str, provider: &str) -> std::io::Result<()> {
        let name = Self::name(gateway, provider);
        let mut last = self.last.lock().expect("quota history");
        last.remove(&name);
        match std::fs::remove_file(self.path(&name)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
    /// Delete history of providers no longer configured, including ones
    /// removed while the application was not running.
    pub(crate) fn retain(&self, names: &BTreeSet<String>) {
        let mut last = self.last.lock().expect("quota history");
        last.retain(|name, _| names.contains(name));
        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            return;
        };
        for entry in entries.flatten() {
            let file = entry.file_name();
            let Some(stem) = file
                .to_str()
                .and_then(|f| f.strip_suffix(".jsonl").or_else(|| f.strip_suffix(".tmp")))
            else {
                continue;
            };
            if !names.contains(stem) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}
