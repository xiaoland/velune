import Foundation
import OpenTelemetrySdk
#if canImport(VeluneBindings)
import VeluneBindings
#endif

/// SDK feedback stays local; backend messages and exception bodies never enter this state.
final class NativeExporterFeedback: @unchecked Sendable {
    private let binding: VeluneTelemetry
    private let revision: UInt64
    private let lock = NSLock()
    private var completed: UInt64 = 0
    private var failed: UInt64 = 0
    init(binding: VeluneTelemetry, revision: UInt64) { self.binding = binding; self.revision = revision }
    func record(success: Bool) {
        let health = lock.withLock {
            if success { completed += 1 } else { failed += 1 }
            return BindingTelemetryExportHealth(completedBatches: completed, failedBatches: failed,
                lastExportUnixMs: UInt64(Date().timeIntervalSince1970 * 1000), lastTransportResult: success ? "rpc_completed" : "transport_error", droppedRecords: nil)
        }
        try? binding.reportNativeExport(revision: revision, health: health)
    }
}

final class ObservedSpanExporter: SpanExporter, @unchecked Sendable {
    let base: any SpanExporter
    let feedback: NativeExporterFeedback
    init(_ base: any SpanExporter, _ feedback: NativeExporterFeedback) { self.base = base; self.feedback = feedback }
    func export(spans: [SpanData], explicitTimeout: TimeInterval?) -> SpanExporterResultCode {
        let result = base.export(spans: spans, explicitTimeout: explicitTimeout); feedback.record(success: result == .success); return result
    }
    func flush(explicitTimeout: TimeInterval?) -> SpanExporterResultCode { base.flush(explicitTimeout: explicitTimeout) }
    func shutdown(explicitTimeout: TimeInterval?) { base.shutdown(explicitTimeout: explicitTimeout) }
    func export(spans: [SpanData], explicitTimeout: TimeInterval?) async -> SpanExporterResultCode {
        let result = await base.export(spans: spans, explicitTimeout: explicitTimeout); feedback.record(success: result == .success); return result
    }
    func flush(explicitTimeout: TimeInterval?) async -> SpanExporterResultCode { await base.flush(explicitTimeout: explicitTimeout) }
    func shutdown(explicitTimeout: TimeInterval?) async { await base.shutdown(explicitTimeout: explicitTimeout) }
}
final class ObservedLogExporter: LogRecordExporter, @unchecked Sendable {
    let base: any LogRecordExporter
    let feedback: NativeExporterFeedback
    init(_ base: any LogRecordExporter, _ feedback: NativeExporterFeedback) { self.base = base; self.feedback = feedback }
    func export(logRecords: [ReadableLogRecord], explicitTimeout: TimeInterval?) -> ExportResult {
        let result = base.export(logRecords: logRecords, explicitTimeout: explicitTimeout); feedback.record(success: result == .success); return result
    }
    func forceFlush(explicitTimeout: TimeInterval?) -> ExportResult { base.forceFlush(explicitTimeout: explicitTimeout) }
    func shutdown(explicitTimeout: TimeInterval?) { base.shutdown(explicitTimeout: explicitTimeout) }
    func export(logRecords: [ReadableLogRecord], explicitTimeout: TimeInterval?) async -> ExportResult {
        let result = await base.export(logRecords: logRecords, explicitTimeout: explicitTimeout); feedback.record(success: result == .success); return result
    }
    func forceFlush(explicitTimeout: TimeInterval?) async -> ExportResult { await base.forceFlush(explicitTimeout: explicitTimeout) }
    func shutdown(explicitTimeout: TimeInterval?) async { await base.shutdown(explicitTimeout: explicitTimeout) }
}
final class ObservedMetricExporter: MetricExporter, @unchecked Sendable {
    let base: any MetricExporter
    let feedback: NativeExporterFeedback
    init(_ base: any MetricExporter, _ feedback: NativeExporterFeedback) { self.base = base; self.feedback = feedback }
    func export(metrics: [MetricData]) -> ExportResult {
        let result = base.export(metrics: metrics); feedback.record(success: result == .success); return result
    }
    func flush() -> ExportResult { base.flush() }
    func shutdown() -> ExportResult { base.shutdown() }
    func getAggregationTemporality(for instrument: InstrumentType) -> AggregationTemporality { base.getAggregationTemporality(for: instrument) }
    func getDefaultAggregation(for instrument: InstrumentType) -> Aggregation { base.getDefaultAggregation(for: instrument) }
    func export(metrics: [MetricData]) async -> ExportResult {
        let result = await base.export(metrics: metrics); feedback.record(success: result == .success); return result
    }
    func flush() async -> ExportResult { await base.flush() }
    func shutdown() async -> ExportResult { await base.shutdown() }
}
