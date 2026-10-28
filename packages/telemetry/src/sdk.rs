use crate::{
    Error, Profile, ResourceIdentity, Result,
    transport::{self, Permission},
};
use opentelemetry::{
    KeyValue,
    metrics::{Counter, Histogram, MeterProvider},
    trace::TracerProvider,
};
use opentelemetry_otlp::WithTonicConfig;
use opentelemetry_sdk::{
    Resource,
    logs::{BatchLogProcessor, SdkLoggerProvider},
    metrics::{PeriodicReader, SdkMeterProvider},
    trace::{BatchConfigBuilder, BatchSpanProcessor, Sampler, SdkTracerProvider},
};
use std::{sync::Arc, time::Duration};
use tracing_subscriber::{Layer, layer::SubscriberExt};

#[derive(Clone)]
pub(crate) struct LocalWriter(
    pub Arc<std::sync::Mutex<tracing_appender::rolling::RollingFileAppender>>,
);
impl std::io::Write for LocalWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("diagnostics writer unavailable"))?
            .write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("diagnostics writer unavailable"))?
            .flush()
    }
}
pub(crate) struct Generation {
    pub revision: u64,
    pub dispatch: tracing::Dispatch,
    permission: Permission,
    traces: SdkTracerProvider,
    logs: SdkLoggerProvider,
    metrics: SdkMeterProvider,
    pub operations: Counter<u64>,
    pub duration: Histogram<f64>,
    drop_diagnostics: tracing::Dispatch,
    _execution_gauges: (
        opentelemetry::metrics::ObservableGauge<u64>,
        opentelemetry::metrics::ObservableGauge<u64>,
        opentelemetry::metrics::ObservableGauge<f64>,
    ),
    pub health: Arc<std::sync::Mutex<crate::ExportHealth>>,
}

fn approved(metadata: &tracing::Metadata<'_>) -> bool {
    metadata.target() == "velune_telemetry"
        && metadata.fields().iter().all(|field| {
            matches!(
                field.name(),
                "velune.operation"
                    | "velune.operation.id"
                    | "velune.outcome"
                    | "velune.request.id"
                    | "velune.operation.duration"
                    | "velune.correlation.gap"
                    | "error.type"
                    | "http.response.status_code"
                    | "velune.gateway.transport"
                    | "otel.name"
                    | "otel.kind"
            )
        })
        && if metadata.is_span() {
            metadata.name() == "client.operation"
        } else {
            matches!(
                metadata.name(),
                "velune.operation.completed"
                    | "velune.execution.turn.started"
                    | "velune.execution.turn.finished"
                    | "velune.execution.turn.not_started"
            )
        }
}

pub(crate) fn local_dispatch(writer: Option<LocalWriter>) -> tracing::Dispatch {
    tracing::Dispatch::new(tracing_subscriber::registry().with(writer.map(local_layer)))
}
fn local_layer(writer: LocalWriter) -> impl Layer<tracing_subscriber::Registry> + Send + Sync {
    tracing_subscriber::fmt::layer()
        .json()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
            metadata.target().starts_with("velune_") && *metadata.level() <= tracing::Level::INFO
        }))
}
impl Generation {
    /// Providers remain unattached and the transport is denied until commit succeeds.
    pub fn prepare(
        profile: &Profile,
        identity: &ResourceIdentity,
        revision: u64,
        writer: Option<LocalWriter>,
        diagnostics: Arc<crate::sdk_diagnostics::Diagnostics>,
        execution: crate::ExecutionState,
    ) -> Result<Self> {
        let permission = Permission::default();
        let channel = transport::channel(profile, permission.clone())?;
        let headers = transport::metadata(profile)?;
        let intercept = move |mut request: tonic::Request<()>| {
            // The exporter merges OTEL_* headers even with an explicit channel.
            *request.metadata_mut() = headers.clone();
            Ok::<_, tonic::Status>(request)
        };
        let trace_exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_channel(channel.clone())
            .with_interceptor(intercept.clone())
            .build()
            .map_err(|_| Error::Transport)?;
        let log_exporter = opentelemetry_otlp::LogExporter::builder()
            .with_tonic()
            .with_channel(channel.clone())
            .with_interceptor(intercept.clone())
            .build()
            .map_err(|_| Error::Transport)?;
        let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_channel(channel)
            .with_interceptor(intercept)
            .build()
            .map_err(|_| Error::Transport)?;
        let resource = Resource::builder_empty()
            .with_attributes(identity.attributes())
            .build();
        let health = Arc::new(std::sync::Mutex::new(crate::ExportHealth::default()));
        let trace_exporter = crate::health::Observed {
            exporter: trace_exporter,
            health: health.clone(),
        };
        let log_exporter = crate::health::Observed {
            exporter: log_exporter,
            health: health.clone(),
        };
        let metric_exporter = crate::health::Observed {
            exporter: metric_exporter,
            health: health.clone(),
        };
        let traces = SdkTracerProvider::builder()
            .with_resource(resource.clone())
            .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
                profile.sample_ratio,
            ))))
            .with_max_attributes_per_span(24)
            .with_max_events_per_span(16)
            .with_span_processor(
                BatchSpanProcessor::builder(trace_exporter)
                    .with_batch_config(
                        BatchConfigBuilder::default()
                            .with_max_queue_size(512)
                            .with_max_export_batch_size(64)
                            .with_scheduled_delay(Duration::from_secs(2))
                            .build(),
                    )
                    .build(),
            )
            .build();
        let logs = SdkLoggerProvider::builder()
            .with_resource(resource.clone())
            .with_log_processor(
                BatchLogProcessor::builder(log_exporter)
                    .with_batch_config(
                        opentelemetry_sdk::logs::BatchConfigBuilder::default()
                            .with_max_queue_size(512)
                            .with_max_export_batch_size(64)
                            .with_scheduled_delay(Duration::from_secs(2))
                            .build(),
                    )
                    .build(),
            )
            .build();
        let metrics = SdkMeterProvider::builder()
            .with_resource(resource)
            .with_reader(
                PeriodicReader::builder(metric_exporter)
                    .with_interval(Duration::from_secs(10))
                    .build(),
            )
            .build();
        let meter = metrics.meter("velune.client.rust");
        let active = execution.clone();
        let active_permission = permission.clone();
        let active_turns = meter
            .u64_observable_gauge("velune.execution.active_turns")
            .with_callback(move |observer| {
                if !active_permission.is_enabled() {
                    return;
                }
                let snapshot = active.lock().unwrap_or_else(|e| e.into_inner()).clone();
                if let Some(snapshot) = snapshot.filter(|snapshot| snapshot.owner_available) {
                    observer.observe(snapshot.active_turns, &[]);
                }
            })
            .build();
        let availability = execution.clone();
        let availability_permission = permission.clone();
        let owner_available = meter
            .u64_observable_gauge("velune.execution.owner_available")
            .with_callback(move |observer| {
                if !availability_permission.is_enabled() {
                    return;
                }
                let snapshot = availability
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                if let Some(snapshot) = snapshot {
                    observer.observe(u64::from(snapshot.owner_available), &[]);
                }
            })
            .build();
        let age_permission = permission.clone();
        let observation_age = meter
            .f64_observable_gauge("velune.execution.observation.age")
            .with_unit("s")
            .with_callback(move |observer| {
                if !age_permission.is_enabled() {
                    return;
                }
                let snapshot = execution.lock().unwrap_or_else(|e| e.into_inner()).clone();
                if let Some(snapshot) = snapshot {
                    observer.observe(
                        crate::now_ms().saturating_sub(snapshot.observed_unix_ms) as f64 / 1000.0,
                        &[],
                    );
                }
            })
            .build();
        let operations = meter.u64_counter("velune.client.operations").build();
        let duration = meter
            .f64_histogram("velune.client.operation.duration")
            .with_unit("s")
            .with_boundaries(vec![
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
            ])
            .build();
        let layers: Vec<Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync>> = vec![
            Box::new(
                tracing_opentelemetry::layer()
                    .with_tracer(traces.tracer("velune.client.rust"))
                    .with_filter(tracing_subscriber::filter::filter_fn(|m| {
                        m.is_span() && approved(m)
                    })),
            ),
            Box::new(
                opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(&logs)
                    .with_filter(tracing_subscriber::filter::filter_fn(|m| {
                        m.is_event() && approved(m)
                    })),
            ),
        ];
        let mut layers = layers;
        if let Some(writer) = writer {
            layers.push(Box::new(local_layer(writer)));
        }
        let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(layers));
        Ok(Self {
            revision,
            dispatch,
            permission,
            traces,
            logs,
            metrics,
            operations,
            duration,
            health,
            drop_diagnostics: diagnostics.dispatch(revision),
            _execution_gauges: (active_turns, owner_available, observation_age),
        })
    }
    pub fn activate(&self) {
        self.permission.enable();
    }
    pub fn revoke(&self) {
        self.permission.revoke();
    }
    pub fn flush(&self) -> bool {
        let traces = self.traces.force_flush().is_ok();
        let logs = self.logs.force_flush().is_ok();
        let metrics = self.metrics.force_flush().is_ok();
        traces && logs && metrics
    }
    pub fn shutdown(&self) {
        self.revoke();
        tracing::dispatcher::with_default(&self.drop_diagnostics, || {
            let _ = self.traces.shutdown_with_timeout(Duration::from_secs(2));
            let _ = self.logs.shutdown_with_timeout(Duration::from_secs(2));
            let _ = self.metrics.shutdown_with_timeout(Duration::from_secs(2));
        });
    }
}

impl ResourceIdentity {
    fn attributes(&self) -> Vec<KeyValue> {
        let mut attributes = vec![
            KeyValue::new("velune.telemetry.schema.version", "2"),
            KeyValue::new("service.name", self.service_name.clone()),
            KeyValue::new("service.version", self.version.clone()),
            KeyValue::new("service.instance.id", self.instance_id.clone()),
            KeyValue::new("velune.platform", self.platform.clone()),
            KeyValue::new("deployment.environment.name", self.environment.clone()),
        ];
        if let Some(run_id) = &self.run_id {
            attributes.push(KeyValue::new("velune.run.id", run_id.clone()));
        }
        attributes
    }
}
