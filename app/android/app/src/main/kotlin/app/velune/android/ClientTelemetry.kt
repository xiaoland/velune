package app.velune.android

import app.velune.bindings.*
import io.opentelemetry.api.common.Attributes
import io.opentelemetry.api.common.AttributeKey
import io.opentelemetry.sdk.common.InternalTelemetryVersion
import io.opentelemetry.sdk.metrics.InstrumentType
import io.opentelemetry.sdk.metrics.data.AggregationTemporality
import io.opentelemetry.sdk.metrics.export.MetricReader
import io.opentelemetry.sdk.metrics.export.CollectionRegistration
import io.opentelemetry.api.logs.Severity
import io.opentelemetry.api.trace.StatusCode
import io.opentelemetry.api.trace.propagation.W3CTraceContextPropagator
import io.opentelemetry.context.Context
import io.opentelemetry.exporter.otlp.trace.OtlpGrpcSpanExporter
import io.opentelemetry.exporter.otlp.logs.OtlpGrpcLogRecordExporter
import io.opentelemetry.exporter.otlp.metrics.OtlpGrpcMetricExporter
import io.opentelemetry.extension.kotlin.asContextElement
import io.opentelemetry.sdk.common.CompletableResultCode
import io.opentelemetry.sdk.resources.Resource
import io.opentelemetry.sdk.trace.SdkTracerProvider
import io.opentelemetry.sdk.trace.data.SpanData
import io.opentelemetry.sdk.trace.export.BatchSpanProcessor
import io.opentelemetry.sdk.trace.export.SpanExporter
import io.opentelemetry.sdk.trace.samplers.Sampler
import io.opentelemetry.sdk.logs.SdkLoggerProvider
import io.opentelemetry.sdk.logs.data.LogRecordData
import io.opentelemetry.sdk.logs.export.BatchLogRecordProcessor
import io.opentelemetry.sdk.logs.export.LogRecordExporter
import io.opentelemetry.sdk.metrics.SdkMeterProvider
import io.opentelemetry.sdk.metrics.data.MetricData
import io.opentelemetry.sdk.metrics.export.MetricExporter
import io.opentelemetry.sdk.metrics.export.PeriodicMetricReader
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.withContext
import java.io.File
import java.time.Duration
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

enum class NativeOperation(val wireName: String) {
    READ("ui.application.read"), WRITE("ui.application.write"),
    CONFIGURATION_READ("ui.configuration.read"), CONFIGURATION_WRITE("ui.configuration.write"),
    CONVERSATION_READ("ui.conversation.read"), CONVERSATION_WRITE("ui.conversation.write"),
    SEND("ui.turn.send"), CANCEL("ui.turn.cancel"), COMPACT("ui.turn.compact"), REPLY("ui.interaction.reply");
    val quiet: Boolean get() = this == READ || this == CONVERSATION_READ
}

/** One current-client owner. No global OTel SDK or automatic resource detector is installed. */
class ClientTelemetry(home: File, developmentProfile: String? = null, runId: String? = null) {
    val binding: VeluneTelemetry = VeluneTelemetry.open(BindingTelemetryOptions(
        homeDirectory = home.absolutePath, platform = "android", version = "0.1 beta.1",
        developmentProfile = developmentProfile, runId = runId, localLogs = false,
    ))
    private val transition = Any()
    private val current = AtomicReference<Generation?>(null)

    fun start() = synchronized(transition) {
        val state = binding.snapshot()
        if (state.configurationAvailable && state.status != "development_configuration_unavailable") {
            if (state.preference == BindingTelemetryPreference.ON || (state.preference == BindingTelemetryPreference.UNSET && state.developmentOverride)) {
                try { apply(binding.prepareSaved()) }
                catch (_: Exception) { binding.confirmNative(binding.snapshot().revision, false) }
            }
            else binding.confirmNative(state.revision, true)
        }
    }
    fun save(revision: ULong, preference: BindingTelemetryPreference, profile: BindingTelemetryProfile?) = synchronized(transition) {
        apply(binding.prepare(revision, preference, profile))
    }
    private fun apply(candidate: BindingTelemetryPreparation) {
        val transport = try { candidate.profile?.let { Transport(it) } }
        catch (error: Exception) { binding.cancelPreparation(candidate.id); throw error }
        val revision = try { binding.commit(candidate.id) }
        catch (error: Exception) { transport?.revoke(); binding.cancelPreparation(candidate.id); throw error }
        current.getAndSet(null)?.close()
        try {
            val generation = transport?.let { Generation(it, candidate, binding) }
            current.set(generation)
            binding.confirmNative(revision, true)
            generation?.activated()
        } catch (error: Exception) {
            current.getAndSet(null)?.close(); transport?.revoke()
            binding.confirmNative(revision, false)
            throw error
        }
    }
    suspend fun <T> operation(category: NativeOperation, body: suspend (BindingTraceCarrier?) -> T): T {
        val generation = current.get() ?: return body(null)
        if (generation.transport.closed) return body(null)
        val operation = category.wireName
        val id = java.util.UUID.randomUUID().toString()
        val span = generation.traces.get("velune.client.native").spanBuilder(operation).startSpan()
        span.setAttribute("velune.operation", operation).setAttribute("velune.operation.id", id)
        val context = Context.current().with(span)
        val fields = mutableMapOf<String, String>()
        W3CTraceContextPropagator.getInstance().inject(context, fields) { carrier, key, value -> carrier?.put(key, value) }
        val carrier = fields["traceparent"]?.let { BindingTraceCarrier(it, fields["tracestate"].orEmpty()) }
        val started = System.nanoTime()
        var outcome = "success"
        try { return withContext(context.asContextElement()) { body(carrier) } }
        catch (error: CancellationException) { outcome = "cancelled"; throw error }
        catch (error: Exception) {
            outcome = when (error) {
                is BindingException.Diagnostic -> if (error.kind == BindingFailureKind.OUTCOME_UNKNOWN) "unknown" else "failure"
                is BindingException.RemoteRequest -> if (error.kind == BindingFailureKind.OUTCOME_UNKNOWN) "unknown" else "failure"
                else -> "failure"
            }
            if (outcome == "failure") span.setStatus(StatusCode.ERROR)
            throw error
        }
        finally {
            val elapsed = (System.nanoTime() - started) / 1e9
            val attributes = Attributes.builder().put("velune.operation", operation).put("velune.outcome", outcome)
                .also { if (outcome == "failure") it.put("error.type", "other") }.build()
            span.setAttribute("velune.outcome", outcome)
            if (outcome == "failure") span.setAttribute("error.type", "other")
            if (!generation.transport.closed) {
                generation.counter.add(1, attributes); generation.duration.record(elapsed, attributes)
                if (outcome != "success" || !category.quiet) generation.logs.get("velune.client.native").logRecordBuilder()
                    .setEventName("velune.operation.completed").setBody("Operation completed").setContext(context).setAllAttributes(attributes.toBuilder().put("velune.operation.id", id).put("velune.operation.duration", elapsed).build()).setSeverity(if (outcome == "failure") Severity.ERROR else Severity.INFO).emit()
            }
            span.end()
        }
    }
    fun shutdown() = synchronized(transition) {
        current.getAndSet(null)?.let { it.flush(); it.close() }; binding.shutdown()
    }

    private class Transport(profile: BindingTelemetryProfile) {
        private val gate = Any()
        @Volatile var closed = false
            private set
        val spans = OtlpGrpcSpanExporter.builder().setEndpoint(profile.endpoint).setTimeout(Duration.ofSeconds(3)).also { builder -> profile.headers.forEach { (k, v) -> builder.addHeader(k, v) } }.build()
        val logs = OtlpGrpcLogRecordExporter.builder().setEndpoint(profile.endpoint).setTimeout(Duration.ofSeconds(3)).also { builder -> profile.headers.forEach { (k, v) -> builder.addHeader(k, v) } }.build()
        val metrics = OtlpGrpcMetricExporter.builder().setEndpoint(profile.endpoint).setTimeout(Duration.ofSeconds(3)).also { builder -> profile.headers.forEach { (k, v) -> builder.addHeader(k, v) } }.build()
        fun export(body: () -> CompletableResultCode): CompletableResultCode = synchronized(gate) { if (closed) CompletableResultCode.ofFailure() else body() }
        fun revoke() {
            val pending = synchronized(gate) {
                if (closed) return
                closed = true
                // Admission and shutdown share this gate, so cancelAll cannot precede a late enqueue.
                listOf(spans.shutdown(), logs.shutdown(), metrics.shutdown())
            }
            CompletableResultCode.ofAll(pending).join(3, TimeUnit.SECONDS)
        }
    }
    private class Feedback(private val binding: VeluneTelemetry, private val revision: ULong) {
        private var completed = 0uL
        private var failed = 0uL
        fun observe(result: CompletableResultCode): CompletableResultCode {
            result.whenComplete {
                val state = synchronized(this) {
                    if (result.isSuccess) completed++ else failed++
                    BindingTelemetryExportHealth(completed, failed, System.currentTimeMillis().toULong(), if (result.isSuccess) "rpc_completed" else "transport_error", null)
                }
                runCatching { binding.reportNativeExport(revision, state) }
            }
            return result
        }
    }
    private class Generation(val transport: Transport, candidate: BindingTelemetryPreparation, binding: VeluneTelemetry) {
        private val feedback = Feedback(binding, candidate.revision)
        private val sdkReader = SdkMetricSnapshot()
        private val sdkMetrics = SdkMeterProvider.builder().registerMetricReader(sdkReader).build()
        private val owner = binding
        private val revision = candidate.revision
        private val spanExporter = object : SpanExporter by transport.spans {
            override fun export(spans: Collection<SpanData>): CompletableResultCode = feedback.observe(transport.export { transport.spans.export(spans) })
        }
        private val logExporter = object : LogRecordExporter by transport.logs {
            override fun export(logs: Collection<LogRecordData>): CompletableResultCode = feedback.observe(transport.export { transport.logs.export(logs) })
        }
        private val metricExporter = object : MetricExporter by transport.metrics {
            override fun export(metrics: Collection<MetricData>): CompletableResultCode = feedback.observe(transport.export { transport.metrics.export(metrics) })
        }
        private val attributes = Attributes.builder().put("velune.telemetry.schema.version", "2").put("service.name", candidate.identity.serviceName).put("service.version", candidate.identity.version)
            .put("service.instance.id", candidate.identity.instanceId).put("velune.platform", "android").put("deployment.environment.name", candidate.identity.environment)
            .also { builder -> candidate.identity.runId?.let { builder.put("velune.run.id", it) } }.build()
        private val resource = Resource.create(attributes)
        val traces: SdkTracerProvider = SdkTracerProvider.builder().setResource(resource).setSampler(Sampler.parentBased(Sampler.traceIdRatioBased(candidate.profile?.sampleRatio ?: 1.0)))
            .addSpanProcessor(BatchSpanProcessor.builder(spanExporter).setMeterProvider(sdkMetrics).setInternalTelemetryVersion(InternalTelemetryVersion.LEGACY).setScheduleDelay(Duration.ofSeconds(2)).setExporterTimeout(Duration.ofSeconds(3)).setMaxQueueSize(512).setMaxExportBatchSize(64).build()).build()
        val logs: SdkLoggerProvider = SdkLoggerProvider.builder().setResource(resource)
            .addLogRecordProcessor(BatchLogRecordProcessor.builder(logExporter).setMeterProvider(sdkMetrics).setInternalTelemetryVersion(InternalTelemetryVersion.LEGACY).setScheduleDelay(Duration.ofSeconds(2)).setExporterTimeout(Duration.ofSeconds(3)).setMaxQueueSize(512).setMaxExportBatchSize(64).build()).build()
        private val metrics = SdkMeterProvider.builder().setResource(resource).registerMetricReader(PeriodicMetricReader.builder(metricExporter).setInterval(Duration.ofSeconds(10)).build()).build()
        val counter = metrics.get("velune.client.native").counterBuilder("velune.ui.operations").build()
        val duration = metrics.get("velune.client.native").histogramBuilder("velune.ui.operation.duration").setUnit("s").setExplicitBucketBoundariesAdvice(listOf(0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0)).build()
        fun activated() {
            val started = System.nanoTime()
            val span = traces.get("velune.client.native").spanBuilder("telemetry.activated").startSpan()
            val id = java.util.UUID.randomUUID().toString()
            span.setAttribute("velune.operation", "telemetry.activated").setAttribute("velune.operation.id", id).setAttribute("velune.outcome", "success")
            val attributes = Attributes.builder().put("velune.operation", "telemetry.activated").put("velune.outcome", "success").build()
            counter.add(1, attributes); duration.record((System.nanoTime() - started) / 1e9, attributes)
            logs.get("velune.client.native").logRecordBuilder().setEventName("velune.operation.completed").setBody("Operation completed").setContext(Context.current().with(span)).setAllAttributes(attributes.toBuilder().put("velune.operation.id", id).put("velune.operation.duration", (System.nanoTime() - started) / 1e9).build()).setSeverity(Severity.INFO).emit()
            span.end()
        }
        fun flush() { CompletableResultCode.ofAll(listOf(traces.forceFlush(), logs.forceFlush(), metrics.forceFlush())).join(3, TimeUnit.SECONDS) }
        fun close() {
            transport.revoke()
            CompletableResultCode.ofAll(listOf(traces.shutdown(), logs.shutdown(), metrics.shutdown())).join(3, TimeUnit.SECONDS)
            sdkReader.registration.collectAllMetrics().forEach { metric ->
                val signal = when {
                    metric.instrumentationScopeInfo.name == "io.opentelemetry.sdk.trace" && metric.name == "processedSpans" -> "traces"
                    metric.instrumentationScopeInfo.name == "io.opentelemetry.sdk.logs" && metric.name == "processedLogs" -> "logs"
                    else -> return@forEach
                }
                val count = metric.longSumData.points.filter { it.attributes.get(AttributeKey.booleanKey("dropped")) == true }.sumOf { it.value }
                if (count > 0) runCatching { owner.reportNativeDrops(revision, signal, count.toULong()) }
            }
            sdkMetrics.shutdown()
        }
    }
    private class SdkMetricSnapshot : MetricReader {
        @Volatile var registration: CollectionRegistration = CollectionRegistration.noop()
        override fun register(registration: CollectionRegistration) { this.registration = registration }
        override fun getAggregationTemporality(instrumentType: InstrumentType): AggregationTemporality = AggregationTemporality.CUMULATIVE
        override fun forceFlush(): CompletableResultCode = CompletableResultCode.ofSuccess()
        override fun shutdown(): CompletableResultCode { registration = CollectionRegistration.noop(); return CompletableResultCode.ofSuccess() }
    }

}
