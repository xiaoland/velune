import Foundation
import GRPC
import NIO
import NIOTransportServices
import OpenTelemetryApi
import OpenTelemetrySdk
import OpenTelemetryProtocolExporterGrpc
import OpenTelemetryProtocolExporterCommon
#if canImport(VeluneBindings)
import VeluneBindings
#endif

/// Shared source for macOS and iOS. One coordinator belongs to the current installation.
final class ClientTelemetry: @unchecked Sendable {
    let binding: VeluneTelemetry
    private let transition = NSRecursiveLock()
    private let lock = NSLock()
    private var generation: NativeTelemetryGeneration?
    private var started = false
    @TaskLocal static var parent: SpanContext?

    init(home: URL, platform: String, localLogs: Bool, developmentProfile: String? = nil, runID: String? = nil) throws {
        binding = try VeluneTelemetry.open(options: BindingTelemetryOptions(homeDirectory: home.path, platform: platform,
            version: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "development",
            developmentProfile: developmentProfile, runId: runID, localLogs: localLogs))
    }

    static func message(_ error: Error) -> String {
        guard let error = error as? BindingError else { return error.localizedDescription }
        switch error {
        case .Diagnostic(_, let detail, _, _, let id): return id.isEmpty ? detail : "\(detail)（诊断编号：\(id)）"
        case .RemoteRequest(_, let detail, _, _), .Invalid(let detail), .Io(let detail), .Contract(let detail), .Unsupported(let detail): return detail
        case .Closed: return "客户端已关闭"
        case .Unavailable: return "遥测服务不可用"
        }
    }

    /// Called on a worker executor. Default/unconfigured launch creates no native SDK or channel.
    func start() throws {
        try transition.withLock {
            guard !started else { return }
            let state = try binding.snapshot()
            if state.configurationAvailable && state.status != "development_configuration_unavailable" {
                if state.preference == .on || (state.preference == .unset && state.developmentOverride) {
                    do { try apply(try binding.prepareSaved()) }
                    catch { if let current = try? binding.snapshot() { try? binding.confirmNative(revision: current.revision, active: false) } }
                }
                else { try binding.confirmNative(revision: state.revision, active: true) }
            }
            started = true
        }
    }
    func save(revision: UInt64, preference: TelemetryPreference, profile: TelemetryProfile?) throws {
        try transition.withLock { try apply(try binding.prepare(expectedRevision: revision, preference: preference, profile: profile)); started = true }
    }
    private func apply(_ candidate: TelemetryPreparation) throws {
        let prepared: NativeTelemetryTransport?
        do { prepared = try candidate.profile.map { try NativeTelemetryTransport(profile: $0) } }
        catch { try? binding.cancelPreparation(preparationId: candidate.id); throw error }
        let revision: UInt64
        do { revision = try binding.commit(preparationId: candidate.id) }
        catch { prepared?.revoke(); try? binding.cancelPreparation(preparationId: candidate.id); throw error }
        // Commit has succeeded. Both retired transports are revoked before either new SDK can collect.
        let old = lock.withLock { let old = generation; generation = nil; return old }
        old?.revoke()
        if let old { DispatchQueue.global(qos: .utility).async { old.shutdown() } }
        do {
            let next = prepared.map { NativeTelemetryGeneration(transport: $0, candidate: candidate, binding: binding) }
            lock.withLock { generation = next }
            try binding.confirmNative(revision: revision, active: true)
            operation(.activation) { _ in }
        } catch {
            let failed = lock.withLock { let failed = generation; generation = nil; return failed }
            failed?.revoke(); failed?.shutdown()
            prepared?.revoke()
            try? binding.confirmNative(revision: revision, active: false)
            throw error
        }
    }

    /// Only fixed operation classes and outcomes are exported; errors and arguments stay local.
    private func begin(_ operation: NativeOperation) -> NativeObservation? {
        guard let current = lock.withLock({ generation }) else { return nil }
        let builder = current.traces.get(instrumentationName: "velune.client.native").spanBuilder(spanName: operation.rawValue)
        if let parent = Self.parent { builder.setParent(parent) } else { builder.setNoParent() }
        let id = UUID().uuidString.lowercased()
        let span = builder.startSpan()
        span.setAttribute(key: "velune.operation", value: operation.rawValue)
        span.setAttribute(key: "velune.operation.id", value: id)
        return NativeObservation(generation: current, operation: operation, span: span, id: id)
    }
    func operation<T>(_ operation: NativeOperation, _ body: (TraceCarrier?) throws -> T) rethrows -> T {
        guard let observation = begin(operation) else { return try body(nil) }
        do {
            let result = try Self.$parent.withValue(observation.span.context) { try body(observation.carrier) }
            observation.complete(outcome: "success"); return result
        } catch { observation.complete(outcome: Self.outcome(error)); throw error }
    }
    func operation<T>(_ operation: NativeOperation, _ body: (TraceCarrier?) async throws -> T) async rethrows -> T {
        guard let observation = begin(operation) else { return try await body(nil) }
        do {
            let result = try await Self.$parent.withValue(observation.span.context) { try await body(observation.carrier) }
            observation.complete(outcome: "success"); return result
        } catch { observation.complete(outcome: Self.outcome(error)); throw error }
    }
    private static func outcome(_ error: Error) -> String {
        if error is CancellationError { return "cancelled" }
        if let error = error as? BindingError {
            switch error {
            case .Diagnostic(let kind, _, _, _, _), .RemoteRequest(let kind, _, _, _):
                if kind == .outcomeUnknown { return "unknown" }
            default: break
            }
        }
        return "failure"
    }
    func recordQueueWait(_ elapsed: TimeInterval) {
        lock.withLock { generation }?.recordQueueWait(elapsed)
    }
    func shutdown() throws {
        try transition.withLock {
            let old = lock.withLock { let old = generation; generation = nil; return old }
            old?.flush(); try binding.shutdown(); old?.revoke(); old?.shutdown()
        }
    }
    deinit {
        if let old = generation {
            old.revoke()
            let binding = binding
            DispatchQueue.global(qos: .utility).async {
                if let state = try? binding.snapshot() { try? binding.confirmNative(revision: state.revision, active: false) }
                old.shutdown()
            }
        }
    }
}

private struct NativeObservation {
    let generation: NativeTelemetryGeneration
    let operation: NativeOperation
    let span: any Span
    let id: String
    let started = ProcessInfo.processInfo.systemUptime
    var carrier: TraceCarrier? {
        guard span.context.isValid else { return nil }
        var fields = [String: String]()
        W3CTraceContextPropagator().inject(spanContext: span.context, carrier: &fields, setter: CarrierSetter())
        return TraceCarrier(traceparent: fields["traceparent"] ?? "", tracestate: fields["tracestate"] ?? "")
    }
    func complete(outcome: String) { generation.complete(operation, id: id, span: span, elapsed: ProcessInfo.processInfo.systemUptime - started, outcome: outcome) }
}

private struct CarrierSetter: Setter {
    func set(carrier: inout [String: String], key: String, value: String) { carrier[key] = value }
}
enum NativeOperation: String {
    case activation = "telemetry.activated"
    case read = "ui.application.read", write = "ui.application.write"
    case configurationRead = "ui.configuration.read", configurationWrite = "ui.configuration.write"
    case conversationRead = "ui.conversation.read", conversationWrite = "ui.conversation.write"
    case send = "ui.turn.send", cancel = "ui.turn.cancel", compact = "ui.turn.compact", reply = "ui.interaction.reply"
    var quiet: Bool { self == .read || self == .conversationRead }
}

private final class NativeTelemetryTransport {
    let group: NIOTSEventLoopGroup
    let channel: ClientConnection
    let config: OtlpConfiguration
    private let lock = NSLock()
    private var revoked = false
    init(profile: TelemetryProfile) throws {
        guard let url = URL(string: profile.endpoint), let host = url.host else { throw BindingError.Invalid(detail: "遥测地址无效") }
        // Network.framework applies the OS trust policy, including device-managed roots.
        group = NIOTSEventLoopGroup(loopCount: 1)
        channel = (url.scheme == "https" ? ClientConnection.usingPlatformAppropriateTLS(for: group) : ClientConnection.insecure(group: group))
            .connect(host: host.trimmingCharacters(in: CharacterSet(charactersIn: "[]")), port: url.port ?? (url.scheme == "https" ? 443 : 80))
        config = OtlpConfiguration(timeout: 3, compression: .none, headers: profile.headers.map { ($0.key, $0.value) })
    }
    func revoke() {
        lock.withLock {
            guard !revoked else { return }; revoked = true
            // Close is the barrier; SDK shutdown below must not flush a retired queue.
            _ = try? channel.close().wait()
        }
    }
    deinit { revoke(); group.shutdownGracefully { _ in } }
}

private final class NativeTelemetryGeneration {
    let traces: TracerProviderSdk
    private let logs: LoggerProviderSdk
    private let logProcessor: BatchLogRecordProcessor
    private let metrics: MeterProviderSdk
    private let transport: NativeTelemetryTransport
    private let sdkDiagnostics: NativeDropDiagnostics
    private let lock = NSLock()
    private var counter: any LongCounter
    private var duration: any DoubleHistogram
    private var queueWait: any DoubleHistogram
    private var stopped = false
    init(transport: NativeTelemetryTransport, candidate: TelemetryPreparation, binding: VeluneTelemetry) {
        self.transport = transport
        sdkDiagnostics = NativeDropDiagnostics(binding: binding, revision: candidate.revision)
        let identity = candidate.identity
        var attributes: [String: AttributeValue] = ["velune.telemetry.schema.version": .string("2"), "service.name": .string(identity.serviceName), "service.version": .string(identity.version),
            "service.instance.id": .string(identity.instanceId), "velune.platform": .string(identity.platform), "deployment.environment.name": .string(identity.environment)]
        if let runID = identity.runId { attributes["velune.run.id"] = .string(runID) }
        let resource = Resource(attributes: attributes)
        let feedback = NativeExporterFeedback(binding: binding, revision: candidate.revision)
        traces = TracerProviderBuilder().with(resource: resource)
            .with(sampler: Samplers.parentBased(root: Samplers.traceIdRatio(ratio: candidate.profile?.sampleRatio ?? 1)))
            .add(spanProcessor: BatchSpanProcessor(spanExporter: ObservedSpanExporter(OtlpTraceExporter(channel: transport.channel, config: transport.config, envVarHeaders: nil), feedback), meterProvider: sdkDiagnostics.provider, scheduleDelay: 2, exportTimeout: 3, maxQueueSize: 512, maxExportBatchSize: 64)).build()
        logProcessor = BatchLogRecordProcessor(logRecordExporter: ObservedLogExporter(OtlpLogExporter(channel: transport.channel, config: transport.config, envVarHeaders: nil), feedback), scheduleDelay: 2, exportTimeout: 3, maxQueueSize: 512, maxExportBatchSize: 64)
        logs = LoggerProviderBuilder().with(resource: resource).with(processors: [logProcessor]).build()
        metrics = MeterProviderSdk.builder().setResource(resource: resource)
            .registerMetricReader(reader: PeriodicMetricReaderBuilder(exporter: ObservedMetricExporter(OtlpMetricExporter(channel: transport.channel, config: transport.config, envVarHeaders: nil), feedback)).setInterval(timeInterval: 10).build())
            .registerView(selector: InstrumentSelector.builder().setInstrument(name: "^velune\\.ui\\.operations$").build(), view: View.builder().build())
            .registerView(selector: InstrumentSelector.builder().setInstrument(name: "^velune\\.ui\\.(operation\\.duration|queue\\.wait)$").build(), view: View.builder().withAggregation(aggregation: ExplicitBucketHistogramAggregation(bucketBoundaries: [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0])).build()).build()
        let meter = metrics.get(name: "velune.client.native")
        counter = meter.counterBuilder(name: "velune.ui.operations").build()
        duration = meter.histogramBuilder(name: "velune.ui.operation.duration").setUnit("s").build()
        queueWait = meter.histogramBuilder(name: "velune.ui.queue.wait").setUnit("s").build()
    }
    func recordQueueWait(_ elapsed: TimeInterval) {
        lock.withLock { if !stopped { queueWait.record(value: elapsed, attributes: [:]) } }
    }
    func complete(_ operation: NativeOperation, id: String, span: any Span, elapsed: Double, outcome: String) {
        defer { span.end() }
        span.setAttribute(key: "velune.outcome", value: outcome)
        if outcome == "failure" { span.status = .error(description: ""); span.setAttribute(key: "error.type", value: "other") }
        lock.withLock {
            guard !stopped else { return }
            var attributes: [String: AttributeValue] = ["velune.operation": .string(operation.rawValue), "velune.outcome": .string(outcome)]
            if outcome == "failure" { attributes["error.type"] = .string("other") }
            counter.add(value: 1, attributes: attributes); duration.record(value: elapsed, attributes: attributes)
            if outcome != "success" || !operation.quiet {
                attributes["velune.operation.id"] = .string(id)
                attributes["velune.operation.duration"] = .double(elapsed)
                logs.get(instrumentationScopeName: "velune.client.native").logRecordBuilder().setEventName("velune.operation.completed").setBody(.string("Operation completed"))
                    .setSpanContext(span.context).setAttributes(attributes).setSeverity(outcome == "failure" ? .error : .info).emit()
            }
        }
    }
    func revoke() { lock.withLock { stopped = true }; transport.revoke() }
    func flush() { traces.forceFlush(timeout: 3); _ = logProcessor.forceFlush(explicitTimeout: 3); _ = metrics.forceFlush() }
    func shutdown() { revoke(); traces.shutdown(); _ = logProcessor.shutdown(explicitTimeout: 2); sdkDiagnostics.capture(); _ = sdkDiagnostics.provider.shutdown(); _ = metrics.shutdown() }
}

/// SDK-owned counters are read locally; the Swift SDK has no equivalent log-drop counter.
private final class NativeDropDiagnostics {
    let provider: MeterProviderSdk
    let reader = SdkMetricSnapshot()
    let binding: VeluneTelemetry
    let revision: UInt64
    init(binding: VeluneTelemetry, revision: UInt64) {
        self.binding = binding; self.revision = revision
        provider = MeterProviderSdk.builder().registerMetricReader(reader: reader)
            .registerView(selector: InstrumentSelector.builder().setInstrument(name: "^processedSpans$").build(), view: View.builder().build()).build()
    }
    func capture() {
        let count = reader.collect().filter { $0.instrumentationScopeInfo.name == "io.opentelemetry.sdk.trace" && $0.name == "processedSpans" }
            .flatMap { $0.data.points }.compactMap { $0 as? LongPointData }.filter { $0.attributes["dropped"] == .bool(true) }.reduce(0) { $0 + $1.value }
        if count > 0 { try? binding.reportNativeDrops(revision: revision, signal: "traces", count: UInt64(count)) }
    }
}
private final class SdkMetricSnapshot: MetricReader, @unchecked Sendable {
    private let lock = NSLock()
    private var producer: (any MetricProducer)?
    func register(registration: CollectionRegistration) { lock.withLock { producer = registration as? any MetricProducer } }
    func collect() -> [MetricData] { lock.withLock { producer }?.collectAllMetrics() ?? [] }
    func forceFlush() -> ExportResult { .success }
    func shutdown() -> ExportResult { lock.withLock { producer = nil }; return .success }
    func getAggregationTemporality(for instrument: InstrumentType) -> AggregationTemporality { .cumulative }
    func getDefaultAggregation(for instrument: InstrumentType) -> Aggregation { Aggregations.defaultAggregation() }
}
