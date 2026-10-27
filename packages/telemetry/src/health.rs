use opentelemetry_sdk::{Resource, error::OTelSdkResult};
use std::{
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};

/// Transport feedback is independent of application telemetry and never recursively exported.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportHealth {
    pub completed_batches: u64,
    pub failed_batches: u64,
    pub last_export_unix_ms: Option<u64>,
    pub last_transport_result: Option<String>,
    /// SDK queue-drop totals are unavailable from the official batch processors.
    pub dropped_records: Option<u64>,
}
pub(crate) struct Observed<T> {
    pub exporter: T,
    pub health: Arc<Mutex<ExportHealth>>,
}
impl<T> fmt::Debug for Observed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ObservedOtlpExporter")
    }
}
impl<T> Observed<T> {
    fn record(&self, result: &OTelSdkResult) {
        let mut health = self.health.lock().unwrap_or_else(|e| e.into_inner());
        health.last_export_unix_ms = Some(crate::now_ms());
        if result.is_ok() {
            health.completed_batches += 1;
        } else {
            health.failed_batches += 1;
        }
        // The SDK returns Ok for partial rejection; the explicit test RPC examines that acknowledgment.
        health.last_transport_result = Some(
            if result.is_ok() {
                "rpc_completed"
            } else {
                "transport_error"
            }
            .into(),
        );
    }
}
impl<T: opentelemetry_sdk::trace::SpanExporter> opentelemetry_sdk::trace::SpanExporter
    for Observed<T>
{
    async fn export(&self, batch: Vec<opentelemetry_sdk::trace::SpanData>) -> OTelSdkResult {
        let result = self.exporter.export(batch).await;
        self.record(&result);
        result
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.exporter.set_resource(resource);
    }
    fn force_flush(&self) -> OTelSdkResult {
        self.exporter.force_flush()
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.exporter.shutdown_with_timeout(timeout)
    }
}
impl<T: opentelemetry_sdk::logs::LogExporter> opentelemetry_sdk::logs::LogExporter for Observed<T> {
    async fn export(&self, batch: opentelemetry_sdk::logs::LogBatch<'_>) -> OTelSdkResult {
        let result = self.exporter.export(batch).await;
        self.record(&result);
        result
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.exporter.set_resource(resource);
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.exporter.shutdown_with_timeout(timeout)
    }
}
impl<T: opentelemetry_sdk::metrics::exporter::PushMetricExporter>
    opentelemetry_sdk::metrics::exporter::PushMetricExporter for Observed<T>
{
    async fn export(
        &self,
        metrics: &opentelemetry_sdk::metrics::data::ResourceMetrics,
    ) -> OTelSdkResult {
        let result = self.exporter.export(metrics).await;
        self.record(&result);
        result
    }
    fn force_flush(&self) -> OTelSdkResult {
        self.exporter.force_flush()
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.exporter.shutdown_with_timeout(timeout)
    }
    fn temporality(&self) -> opentelemetry_sdk::metrics::Temporality {
        self.exporter.temporality()
    }
}
