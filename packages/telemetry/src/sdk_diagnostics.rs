//! Only fixed SDK drop summaries are retained locally, including summaries from retired generations.
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tracing_subscriber::{
    Layer,
    layer::{Context, SubscriberExt},
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    instance_id: String,
    revision: u64,
    scope: &'static str,
    signal: &'static str,
    dropped_records: u64,
    observed_unix_ms: u64,
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Records {
    records: Vec<Record>,
    evicted_records: u64,
}
pub(crate) struct Diagnostics {
    path: PathBuf,
    instance: String,
    records: Mutex<Records>,
}
impl Diagnostics {
    pub fn new(path: PathBuf, instance: String) -> Arc<Self> {
        Arc::new(Self {
            path,
            instance,
            records: Mutex::new(Records::default()),
        })
    }
    pub fn record(
        &self,
        revision: u64,
        scope: &'static str,
        signal: &'static str,
        count: u64,
    ) -> crate::Result<()> {
        if count == 0 {
            return Ok(());
        }
        let mut records = self.records.lock().map_err(|_| crate::Error::Unavailable)?;
        if let Some(record) = records
            .records
            .iter_mut()
            .find(|r| r.revision == revision && r.scope == scope && r.signal == signal)
        {
            record.dropped_records = record.dropped_records.max(count);
            record.observed_unix_ms = crate::now_ms();
        } else {
            if records.records.len() == 64 {
                records.records.remove(0);
                records.evicted_records += 1;
            }
            records.records.push(Record {
                instance_id: self.instance.clone(),
                revision,
                scope,
                signal,
                dropped_records: count,
                observed_unix_ms: crate::now_ms(),
            });
        }
        crate::config::write(&self.path, &*records)
    }
    pub fn dispatch(self: &Arc<Self>, revision: u64) -> tracing::Dispatch {
        tracing::Dispatch::new(tracing_subscriber::registry().with(DropSummary {
            owner: self.clone(),
            revision,
        }))
    }
}
struct DropSummary {
    owner: Arc<Diagnostics>,
    revision: u64,
}
struct Count(u64);
impl tracing::field::Visit for Count {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        if matches!(field.name(), "dropped_span_count" | "dropped_logs_count") {
            self.0 = value;
        }
    }
}
impl<S: tracing::Subscriber> Layer<S> for DropSummary {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if event.metadata().target() != "opentelemetry_sdk" {
            return;
        }
        let signal = match event.metadata().name() {
            "BatchSpanProcessor.SpansDropped" => "traces",
            "BatchLogProcessor.LogsDropped" => "logs",
            _ => return,
        };
        let mut count = Count(0);
        event.record(&mut count);
        let _ = self.owner.record(self.revision, "rust", signal, count.0);
    }
}
