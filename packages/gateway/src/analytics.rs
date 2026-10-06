//! Request analytics emitted after the upstream response is observed.
//!
//! The gateway only defines the transport-neutral record. Persistence belongs
//! to the application composition root; the callback must be non-blocking.

use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AnalyticsProtocol {
    ChatCompletions,
    Responses,
    Messages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AnalyticsOutcome {
    Completed,
    Incomplete,
    Failed,
    Cancelled,
    Rejected,
}

pub use velune_ai_provider::metrics::TokenUsage;

#[derive(Debug, Clone)]
pub struct AnalyticsRecord {
    pub request_id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model_record_key: String,
    pub provider_model_id: String,
    pub protocol: AnalyticsProtocol,
    pub started_at_ms: i64,
    pub terminal_at_ms: i64,
    pub elapsed_ms: u64,
    pub first_output_ms: Option<u64>,
    pub terminal_elapsed_ms: Option<u64>,
    pub status: Option<u16>,
    pub outcome: AnalyticsOutcome,
    pub usage: TokenUsage,
    pub usage_reported: bool,
    pub usage_complete: bool,
}

pub trait AnalyticsSink: Send + Sync {
    fn record(&self, record: AnalyticsRecord);
}

pub type SharedAnalyticsSink = Arc<dyn AnalyticsSink>;

use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use velune_ai_provider::metrics::{ModelTerminal, ResponseMetrics};

pub(crate) struct Tracker {
    metrics: ResponseMetrics,
    started: Option<Instant>,
    first_output_ms: Option<u64>,
    terminal_elapsed_ms: Option<u64>,
    status: Option<u16>,
    transport_outcome: Option<AnalyticsOutcome>,
}
impl Tracker {
    pub(crate) fn begin(&mut self) {
        self.started = Some(Instant::now());
    }
    pub(crate) fn headers(&mut self, status: u16) {
        self.status = Some(status);
    }
    pub(crate) fn chunk(&mut self, bytes: &[u8]) {
        self.metrics.stream_chunk(bytes);
        self.update_times();
    }
    pub(crate) fn json(&mut self, status: u16, bytes: &[u8]) {
        self.headers(status);
        self.metrics.json(bytes);
        // A non-streaming response does not expose a first-output instant.
        self.update_times();
    }
    fn update_times(&mut self) {
        if self.metrics.first_output && self.first_output_ms.is_none() {
            self.first_output_ms = Some(self.elapsed());
        }
        if self.metrics.terminal.is_some() && self.terminal_elapsed_ms.is_none() {
            self.terminal_elapsed_ms = Some(self.elapsed());
        }
    }
    pub(crate) fn finish(&mut self, outcome: AnalyticsOutcome) {
        self.metrics.stream_end();
        self.update_times();
        self.transport_outcome = Some(outcome);
    }
    fn elapsed(&self) -> u64 {
        self.started.map_or(0, |s| {
            u64::try_from(s.elapsed().as_millis()).unwrap_or(u64::MAX)
        })
    }
}
pub(crate) type SharedTracker = Arc<Mutex<Tracker>>;
/// Kept in the dispatch future so aborting delivery records partial known usage.
pub(crate) struct RequestAnalytics {
    sink: SharedAnalyticsSink,
    record: AnalyticsRecord,
    pub(crate) tracker: SharedTracker,
    stopping: Arc<AtomicBool>,
}
impl RequestAnalytics {
    pub(crate) fn new(
        sink: SharedAnalyticsSink,
        record: AnalyticsRecord,
        stopping: Arc<AtomicBool>,
    ) -> Self {
        let protocol = match record.protocol {
            AnalyticsProtocol::ChatCompletions => {
                velune_ai_provider::translation::LlmProtocol::ChatCompletions
            }
            AnalyticsProtocol::Responses => velune_ai_provider::translation::LlmProtocol::Responses,
            AnalyticsProtocol::Messages => velune_ai_provider::translation::LlmProtocol::Messages,
        };
        Self {
            sink,
            record,
            stopping,
            tracker: Arc::new(Mutex::new(Tracker {
                metrics: ResponseMetrics::new(protocol),
                started: None,
                first_output_ms: None,
                terminal_elapsed_ms: None,
                status: None,
                transport_outcome: None,
            })),
        }
    }
}
impl Drop for RequestAnalytics {
    fn drop(&mut self) {
        let tracker = self.tracker.lock().expect("owned analytics tracker");
        self.record.terminal_at_ms = now_ms();
        self.record.elapsed_ms = tracker.elapsed();
        self.record.first_output_ms = tracker.first_output_ms;
        self.record.terminal_elapsed_ms = tracker.terminal_elapsed_ms;
        self.record.status = tracker.status;
        self.record.usage = tracker.metrics.usage();
        self.record.usage_reported = tracker.metrics.reported;
        self.record.usage_complete =
            self.record.usage.input.is_some() && self.record.usage.output.is_some();
        self.record.outcome = if tracker.status.is_some_and(|s| !(200..300).contains(&s)) {
            AnalyticsOutcome::Failed
        } else {
            match tracker.metrics.terminal {
                Some(ModelTerminal::Completed) => AnalyticsOutcome::Completed,
                Some(ModelTerminal::Incomplete) => AnalyticsOutcome::Incomplete,
                Some(ModelTerminal::Failed) => AnalyticsOutcome::Failed,
                None if self.stopping.load(Ordering::Acquire)
                    || tracker.transport_outcome.is_none() =>
                {
                    AnalyticsOutcome::Cancelled
                }
                None => match tracker.transport_outcome {
                    Some(AnalyticsOutcome::Completed) => AnalyticsOutcome::Incomplete,
                    Some(value) => value,
                    None => AnalyticsOutcome::Cancelled,
                },
            }
        };
        if tracker.metrics.degraded {
            tracing::warn!(
                event = "analytics_protocol_observation_degraded",
                request_id = self.record.request_id
            );
        }
        self.sink.record(self.record.clone());
    }
}
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|v| i64::try_from(v.as_millis()).ok())
        .unwrap_or(0)
}
