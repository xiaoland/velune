//! Application-owned request metadata, asynchronous persistence and bounded reads.
use rusqlite::{Connection, params};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use velune_gateway::{AnalyticsOutcome, AnalyticsProtocol, AnalyticsRecord, AnalyticsSink};
const QUEUE: usize = 256;
const WAIT: Duration = Duration::from_secs(3);
enum QueueItem {
    Record(Box<AnalyticsRecord>),
    Flush(SyncSender<()>),
}
pub(crate) struct AnalyticsStore {
    path: PathBuf,
    sender: Option<SyncSender<QueueItem>>,
    worker: Option<JoinHandle<()>>,
    warning: Arc<Mutex<Option<String>>>,
    dropped: Arc<AtomicU64>,
    stopping: Arc<AtomicBool>,
}
#[derive(Debug, Clone)]
pub(crate) struct AnalyticsRow {
    pub request_id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model_record_key: String,
    pub provider_model_id: String,
    pub protocol: String,
    pub started_at_ms: i64,
    pub terminal_at_ms: i64,
    pub elapsed_ms: u64,
    pub first_output_ms: Option<u64>,
    pub terminal_elapsed_ms: Option<u64>,
    pub status: Option<u16>,
    pub outcome: String,
    pub input_tokens: Option<u64>,
    pub uncached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub usage_reported: bool,
    pub usage_complete: bool,
}
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS request_usage (
request_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, provider_name TEXT NOT NULL,
model_record_key TEXT NOT NULL, provider_model_id TEXT NOT NULL, protocol TEXT NOT NULL,
started_at_ms INTEGER NOT NULL, terminal_at_ms INTEGER NOT NULL, elapsed_ms INTEGER NOT NULL,
first_output_ms INTEGER, terminal_elapsed_ms INTEGER, status INTEGER, outcome TEXT NOT NULL,
input_tokens INTEGER, output_tokens INTEGER, reasoning_output_tokens INTEGER, cached_input_tokens INTEGER,
cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER, usage_reported INTEGER NOT NULL,
usage_complete INTEGER NOT NULL, uncached_input_tokens INTEGER);
CREATE INDEX IF NOT EXISTS usage_terminal ON request_usage(terminal_at_ms DESC, request_id);
PRAGMA user_version=1;";
impl AnalyticsStore {
    pub(crate) fn open(home: &Path) -> Result<Self, rusqlite::Error> {
        let path = home.join("analytics.sqlite");
        let db = Connection::open(&path)?;
        db.busy_timeout(WAIT)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != 1 {
            db.execute_batch("DROP TABLE IF EXISTS request_usage;")?;
        }
        db.execute_batch(SCHEMA)?;
        drop(db);
        let (sender, receiver) = mpsc::sync_channel(QUEUE);
        let warning = Arc::new(Mutex::new(None));
        let dropped = Arc::new(AtomicU64::new(0));
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = stopping.clone();
        let worker_warning = warning.clone();
        let worker_dropped = dropped.clone();
        let worker_path = path.clone();
        let dispatcher = tracing::dispatcher::get_default(Clone::clone);
        let worker = thread::Builder::new()
            .name("velune-analytics".into())
            .spawn(move || {
                tracing::dispatcher::with_default(&dispatcher, || {
                    let db = match Connection::open(worker_path).and_then(|db| {
                        db.busy_timeout(WAIT)?;
                        Ok(db)
                    }) {
                        Ok(db) => db,
                        Err(error) => {
                            warn(&worker_warning, format!("分析存储打开失败：{error}"));
                            return;
                        }
                    };
                    while let Ok(item) = receiver.recv() {
                        if worker_stopping.load(Ordering::Acquire) {
                            worker_dropped.fetch_add(
                                u64::from(matches!(item, QueueItem::Record(_))),
                                Ordering::Relaxed,
                            );
                            continue;
                        }
                        match item {
                            QueueItem::Record(record) => {
                                if let Err(error) = insert(&db, &record) {
                                    worker_dropped.fetch_add(1, Ordering::Relaxed);
                                    warn(&worker_warning, format!("分析记录保存失败：{error}"));
                                }
                            }
                            QueueItem::Flush(done) => {
                                let _ = done.try_send(());
                            }
                        }
                    }
                });
            })
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        Ok(Self {
            path,
            sender: Some(sender),
            worker: Some(worker),
            warning,
            dropped,
            stopping,
        })
    }
    /// Statistics failure must not prevent normal configuration or execution.
    pub(crate) fn unavailable(home: &Path, error: impl std::fmt::Display) -> Self {
        let warning = Arc::new(Mutex::new(None));
        warn(&warning, format!("分析存储不可用：{error}"));
        Self {
            path: home.join("analytics.sqlite"),
            sender: None,
            worker: None,
            warning,
            dropped: Arc::new(AtomicU64::new(0)),
            stopping: Arc::new(AtomicBool::new(false)),
        }
    }
    pub(crate) fn warning(&self) -> Option<String> {
        self.warning.lock().expect("analytics warning").clone()
    }
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
    pub(crate) fn barrier(&self) -> Result<(), crate::Error> {
        let Some(sender) = &self.sender else {
            return Ok(());
        };
        let (done, wait) = mpsc::sync_channel(1);
        let deadline = std::time::Instant::now() + WAIT;
        let mut item = QueueItem::Flush(done);
        loop {
            match sender.try_send(item) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(value)) if std::time::Instant::now() < deadline => {
                    item = value;
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => {
                    let detail = format!("分析存储同步失败：{error}");
                    warn(&self.warning, detail.clone());
                    return Err(crate::Error::Invalid(detail));
                }
            }
        }
        wait.recv_timeout(WAIT).map_err(|error| {
            let detail = format!("分析存储同步失败：{error}");
            warn(&self.warning, detail.clone());
            crate::Error::Invalid(detail)
        })
    }
    /// Stream the complete population; only the application read model limits its page.
    pub(crate) fn visit(
        &self,
        from_ms: i64,
        to_ms: i64,
        provider_id: Option<&str>,
        model_key: Option<&str>,
        mut visitor: impl FnMut(AnalyticsRow) -> Result<(), crate::Error>,
    ) -> Result<(), crate::Error> {
        if self.sender.is_none() {
            return Ok(());
        }
        self.barrier()?;
        let mut read = || -> Result<(), crate::Error> {
            let db = Connection::open(&self.path).map_err(database_error)?;
            db.busy_timeout(WAIT).map_err(database_error)?;
            let mut statement = db.prepare("SELECT request_id,provider_id,provider_name,model_record_key,provider_model_id,protocol,started_at_ms,terminal_at_ms,elapsed_ms,first_output_ms,terminal_elapsed_ms,status,outcome,input_tokens,output_tokens,reasoning_output_tokens,cached_input_tokens,cache_read_input_tokens,cache_creation_input_tokens,usage_reported,usage_complete,uncached_input_tokens FROM request_usage WHERE terminal_at_ms >= ?1 AND terminal_at_ms < ?2 AND (?3 IS NULL OR provider_id = ?3) AND (?4 IS NULL OR model_record_key = ?4) ORDER BY terminal_at_ms DESC,request_id DESC").map_err(database_error)?;
            let rows = statement
                .query_map(params![from_ms, to_ms, provider_id, model_key], |row| {
                    Ok(AnalyticsRow {
                        request_id: row.get(0)?,
                        provider_id: row.get(1)?,
                        provider_name: row.get(2)?,
                        model_record_key: row.get(3)?,
                        provider_model_id: row.get(4)?,
                        protocol: row.get(5)?,
                        started_at_ms: row.get(6)?,
                        terminal_at_ms: row.get(7)?,
                        elapsed_ms: row.get(8)?,
                        first_output_ms: row.get(9)?,
                        terminal_elapsed_ms: row.get(10)?,
                        status: row.get(11)?,
                        outcome: row.get(12)?,
                        input_tokens: row.get(13)?,
                        output_tokens: row.get(14)?,
                        reasoning_output_tokens: row.get(15)?,
                        cached_input_tokens: row.get(16)?,
                        cache_read_input_tokens: row.get(17)?,
                        cache_creation_input_tokens: row.get(18)?,
                        usage_reported: row.get(19)?,
                        usage_complete: row.get(20)?,
                        uncached_input_tokens: row.get(21)?,
                    })
                })
                .map_err(database_error)?;
            for row in rows {
                visitor(row.map_err(database_error)?)?;
            }
            Ok(())
        };
        read()
    }
}
impl AnalyticsSink for AnalyticsStore {
    fn record(&self, record: AnalyticsRecord) {
        if self.sender.as_ref().is_none_or(|sender| {
            sender
                .try_send(QueueItem::Record(Box::new(record)))
                .is_err()
        }) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            warn(&self.warning, "分析记录未保存：队列已满或存储不可用".into());
        }
    }
}
impl Drop for AnalyticsStore {
    fn drop(&mut self) {
        // Drain accepted writes first; after a bounded barrier failure do not
        // spend a separate busy timeout on every queued record during shutdown.
        if self.barrier().is_err() {
            self.stopping.store(true, Ordering::Release);
        }
        self.sender.take();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            warn(&self.warning, "分析存储线程异常结束".into());
        }
    }
}
fn warn(warning: &Mutex<Option<String>>, detail: String) {
    tracing::warn!(event = "analytics_storage_failed", detail);
    *warning.lock().expect("analytics warning") = Some(detail);
}
fn database_error(error: rusqlite::Error) -> crate::Error {
    crate::Error::Invalid(format!("分析数据库读取失败：{error}"))
}
fn integer(value: Option<u64>) -> Result<Option<i64>, rusqlite::Error> {
    value
        .map(i64::try_from)
        .transpose()
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}
fn insert(db: &Connection, r: &AnalyticsRecord) -> Result<(), rusqlite::Error> {
    let protocol = match r.protocol {
        AnalyticsProtocol::ChatCompletions => "chatCompletions",
        AnalyticsProtocol::Responses => "responses",
        AnalyticsProtocol::Messages => "messages",
    };
    let outcome = match r.outcome {
        AnalyticsOutcome::Completed => "completed",
        AnalyticsOutcome::Incomplete => "incomplete",
        AnalyticsOutcome::Failed => "failed",
        AnalyticsOutcome::Cancelled => "cancelled",
        AnalyticsOutcome::Rejected => "rejected",
    };
    db.execute("INSERT INTO request_usage VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)", params![r.request_id,r.provider_id,r.provider_name,r.model_record_key,r.provider_model_id,protocol,r.started_at_ms,r.terminal_at_ms,integer(Some(r.elapsed_ms))?,integer(r.first_output_ms)?,integer(r.terminal_elapsed_ms)?,r.status.map(i64::from),outcome,integer(r.usage.input)?,integer(r.usage.output)?,integer(r.usage.reasoning_output)?,integer(r.usage.cached_input)?,integer(r.usage.cache_read_input)?,integer(r.usage.cache_creation_input)?,r.usage_reported,r.usage_complete,integer(r.usage.uncached_input)?])?;
    Ok(())
}
