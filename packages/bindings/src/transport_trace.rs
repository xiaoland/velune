//! Bounded, explicitly private native transport evidence for the synthetic S1 probe.
use crate::{BindingError, VeluneTransportProbe};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::SubscriberExt};

const CAPACITY: usize = 512;
const TARGETS: &[&str] = &[
    "iroh::_events::qnt::init",
    "iroh::_events::path::open",
    "iroh::_events::path::selected",
    "iroh::_events::path::abandoned",
];
const MDNS_METADATA_TARGETS: &[&str] = &[
    "iroh_mdns_address_lookup",
    "swarm_discovery::socket",
    "swarm_discovery::guardian",
    "swarm_discovery::receiver",
    "swarm_discovery::sender",
];

fn enabled(metadata: &tracing::Metadata<'_>) -> bool {
    TARGETS.contains(&metadata.target())
        || (MDNS_METADATA_TARGETS.contains(&metadata.target())
            && *metadata.level() <= tracing::Level::WARN)
}

#[derive(Clone, uniffi::Record)]
pub struct BindingNativeTransportField {
    pub name: String,
    pub value: String,
}
#[derive(Clone, uniffi::Record)]
pub struct BindingNativeTransportEvent {
    pub sequence: u64,
    pub sampled_at_micros: u64,
    pub target: String,
    pub fields: Vec<BindingNativeTransportField>,
    pub dropped_before: u64,
}
#[derive(Default)]
struct Events {
    records: VecDeque<BindingNativeTransportEvent>,
    next: u64,
    dropped: u64,
}
static EVENTS: OnceLock<Arc<Mutex<Events>>> = OnceLock::new();
static INSTALLED: OnceLock<bool> = OnceLock::new();
struct Collector(Arc<Mutex<Events>>);
struct Fields(Vec<BindingNativeTransportField>);
impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        // White-listed events contain addresses/path IDs only. Bound each value even
        // if the upstream dependency changes its formatting or candidate capacity.
        let mut value = format!("{value:?}");
        if value.len() > 4096 {
            let mut end = 4096;
            while !value.is_char_boundary(end) {
                end -= 1;
            }
            value.truncate(end);
            value.push_str("[truncated]");
        }
        self.0.push(BindingNativeTransportField {
            name: field.name().into(),
            value,
        });
    }
}
impl<S: Subscriber> Layer<S> for Collector {
    fn on_event(&self, event: &Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let metadata = event.metadata();
        if !enabled(metadata) {
            return;
        }
        let fields = if MDNS_METADATA_TARGETS.contains(&metadata.target()) {
            // Dependency warnings can format arbitrary peer TXT data. Inspect only
            // callsite metadata, never their message, fields, errors or span values.
            vec![
                BindingNativeTransportField {
                    name: "sourceFile".into(),
                    value: metadata
                        .file()
                        .and_then(|file| file.rsplit('/').next())
                        .unwrap_or("unknown")
                        .into(),
                },
                BindingNativeTransportField {
                    name: "sourceLine".into(),
                    value: metadata.line().unwrap_or(0).to_string(),
                },
                BindingNativeTransportField {
                    name: "level".into(),
                    value: metadata.level().to_string(),
                },
            ]
        } else {
            let mut fields = Fields(Vec::new());
            event.record(&mut fields);
            fields.0
        };
        if let Ok(mut events) = self.0.lock() {
            events.next += 1;
            if events.records.len() == CAPACITY {
                events.records.pop_front();
                events.dropped += 1;
            }
            let record = BindingNativeTransportEvent {
                sequence: events.next,
                sampled_at_micros: velune_remote::monotonic_microseconds(),
                target: event.metadata().target().into(),
                fields,
                dropped_before: events.dropped,
            };
            events.records.push_back(record);
        }
    }
}

pub(crate) fn install() -> Result<(), BindingError> {
    let installed = *INSTALLED.get_or_init(|| {
        let events = EVENTS.get_or_init(|| Arc::new(Mutex::new(Events::default())));
        let collector =
            Collector(events.clone()).with_filter(tracing_subscriber::filter::filter_fn(enabled));
        tracing::subscriber::set_global_default(tracing_subscriber::registry().with(collector))
            .is_ok()
    });
    if installed {
        Ok(())
    } else {
        Err(BindingError::Invalid {
            detail: "传输探针无法独占原生日志观察，请使用独立诊断进程".into(),
        })
    }
}
#[uniffi::export]
impl VeluneTransportProbe {
    /// Private address-bearing evidence. Draining does not cause discovery or path probing.
    pub fn native_events(&self) -> Vec<BindingNativeTransportEvent> {
        EVENTS
            .get()
            .and_then(|events| events.lock().ok())
            .map(|mut events| events.records.drain(..).collect())
            .unwrap_or_default()
    }
}
