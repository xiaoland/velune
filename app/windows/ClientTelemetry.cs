using System.Diagnostics;
using System.Diagnostics.Tracing;
using System.Diagnostics.Metrics;
using Microsoft.Extensions.Logging;
using OpenTelemetry;
using OpenTelemetry.Context.Propagation;
using OpenTelemetry.Exporter;
using OpenTelemetry.Logs;
using OpenTelemetry.Metrics;
using OpenTelemetry.Resources;
using OpenTelemetry.Trace;
using Velune.Bindings;

namespace Velune;

sealed class ClientTelemetry
{
    public VeluneTelemetry Binding { get; }
    readonly object transition = new();
    Generation? current;

    public ClientTelemetry(string home)
    {
        Binding = VeluneTelemetry.Open(new BindingTelemetryOptions(home, "windows", "0.1 beta.1",
            Environment.GetEnvironmentVariable("VELUNE_TELEMETRY_PROFILE"), Environment.GetEnvironmentVariable("VELUNE_RUN_ID"), true));
    }
    public void Start()
    {
        lock (transition)
        {
            var state = Binding.Snapshot();
            if (!state.ConfigurationAvailable || state.Status == "development_configuration_unavailable") return;
            try
            {
                if (state.Preference == BindingTelemetryPreference.On || state.Preference == BindingTelemetryPreference.Unset && state.DevelopmentOverride) Apply(Binding.PrepareSaved());
                else Binding.ConfirmNative(state.Revision, true);
            }
            catch { Binding.ConfirmNative(Binding.Snapshot().Revision, false); }
        }
    }
    public void Save(ulong revision, BindingTelemetryPreference preference, BindingTelemetryProfile? profile)
    {
        lock (transition) Apply(Binding.Prepare(revision, preference, profile));
    }
    void Apply(BindingTelemetryPreparation candidate)
    {
        Transport? prepared;
        try { prepared = candidate.Profile is { } profile ? new Transport(profile) : null; }
        catch { Binding.CancelPreparation(candidate.Id); throw; }
        ulong revision;
        try { revision = Binding.Commit(candidate.Id); }
        catch { prepared?.Revoke(); Binding.CancelPreparation(candidate.Id); throw; }
        // Disposing the retired listeners before creating replacements prevents overlapping Activity sampling.
        try
        {
            Interlocked.Exchange(ref current, null)?.Dispose();
            var next = prepared == null ? null : new Generation(prepared, candidate, Binding);
            Volatile.Write(ref current, next);
            Binding.ConfirmNative(revision, true);
            Operation("telemetryActivated", _ => Task.FromResult(true)).GetAwaiter().GetResult();
        }
        catch
        {
            Interlocked.Exchange(ref current, null)?.Dispose(); prepared?.Revoke();
            Binding.ConfirmNative(revision, false);
            throw;
        }
    }
    public async Task<T> Operation<T>(string name, Func<BindingTraceCarrier?, Task<T>> body)
    {
        var generation = Volatile.Read(ref current);
        if (generation == null || generation.Transport.Closed) return await body(null);
        var operation = name switch
        {
            "telemetryActivated" => "telemetry.activated",
            "Send" or "SendTurn" => "ui.turn.send",
            "Stop" or "Cancel" => "ui.turn.cancel",
            "List" => "ui.configuration.read",
            _ => "ui.application.read",
        };
        using var span = generation.Source.StartActivity(operation);
        BindingTraceCarrier? carrier = null;
        // The .NET SDK skips local children of unsampled activities; propagate their existing context.
        var context = span?.Context ?? Activity.Current?.Context;
        if (context is { } propagationContext)
        {
            var fields = new Dictionary<string, string>();
            new TraceContextPropagator().Inject(new PropagationContext(propagationContext, default), fields, static (map, key, value) => map[key] = value);
            if (fields.TryGetValue("traceparent", out var parent)) carrier = new BindingTraceCarrier(parent, fields.GetValueOrDefault("tracestate", ""));
        }
        var started = Stopwatch.GetTimestamp();
        var outcome = "success";
        try { return await body(carrier); }
        catch (BindingException.RemoteRequest error) when (error.kind == BindingFailureKind.OutcomeUnknown) { outcome = "unknown"; throw; }
        catch (BindingException.Diagnostic error) when (error.kind == BindingFailureKind.OutcomeUnknown) { outcome = "unknown"; throw; }
        catch (OperationCanceledException) { outcome = "cancelled"; throw; }
        catch { outcome = "failure"; span?.SetStatus(ActivityStatusCode.Error); throw; }
        finally
        {
            span?.SetTag("outcome", outcome);
            if (!generation.Transport.Closed)
            {
                var tags = new TagList { { "operation", operation }, { "outcome", outcome } };
                generation.Counter.Add(1, tags); generation.Duration.Record(Stopwatch.GetElapsedTime(started).TotalSeconds, tags);
                if (outcome != "success" || operation is "ui.turn.send" or "ui.turn.cancel" or "telemetry.activated")
                    generation.Logger.Log(outcome == "failure" ? LogLevel.Error : LogLevel.Information, "operation_completed {operation} {outcome}", operation, outcome);
            }
        }
    }
    public void Shutdown()
    {
        lock (transition)
        {
            var previous = Interlocked.Exchange(ref current, null);
            previous?.Flush(); Binding.Shutdown(); previous?.Dispose();
        }
    }

    sealed class Transport
    {
        readonly object gate = new();
        public HttpClient Client { get; }
        volatile bool closed;
        public bool Closed => closed;
        readonly string endpoint;
        public Action<bool>? Feedback { get; set; }
        public Transport(BindingTelemetryProfile profile)
        {
            endpoint = profile.Endpoint;
            Client = new HttpClient(new FeedbackHandler(this, new SocketsHttpHandler { AllowAutoRedirect = false, UseCookies = false })) { Timeout = TimeSpan.FromSeconds(3) };
            foreach (var (key, value) in profile.Headers)
                if (!Client.DefaultRequestHeaders.TryAddWithoutValidation(key, value)) throw new InvalidOperationException("遥测认证 metadata 无效");
        }
        public OtlpExporterOptions Options() => new() { Endpoint = new Uri(endpoint), Protocol = OtlpExportProtocol.Grpc, Headers = "", Compression = OtlpExportCompression.None, TimeoutMilliseconds = 3000, HttpClientFactory = () => Client };
        public void Revoke()
        {
            lock (gate)
            {
                if (Closed) return;
                closed = true; Client.CancelPendingRequests(); Client.Dispose();
            }
        }
        sealed class FeedbackHandler(Transport owner, HttpMessageHandler inner) : DelegatingHandler(inner)
        {
            protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
            {
                try
                {
                    var response = await base.SendAsync(request, cancellationToken).ConfigureAwait(false);
                    // A gRPC acknowledgment is small; cap untrusted receiver replies before inspecting trailers.
                    await response.Content.LoadIntoBufferAsync(32 * 1024, cancellationToken).ConfigureAwait(false);
                    var status = response.TrailingHeaders.TryGetValues("grpc-status", out var trailing) ? trailing.FirstOrDefault()
                        : response.Headers.TryGetValues("grpc-status", out var headers) ? headers.FirstOrDefault() : null;
                    owner.Feedback?.Invoke(response.IsSuccessStatusCode && status == "0");
                    return response;
                }
                catch { owner.Feedback?.Invoke(false); throw; }
            }
        }
    }
    sealed class Generation : IDisposable
    {
        public Transport Transport { get; }
        public ActivitySource Source { get; } = new("velune.client.native");
        readonly Meter meter = new("velune.client.native");
        readonly TracerProvider traces;
        readonly MeterProvider metrics;
        readonly BatchLogRecordExportProcessor logs;
        readonly ILoggerFactory loggers;
        public ILogger Logger { get; }
        public Counter<long> Counter { get; }
        public Histogram<double> Duration { get; }
        readonly VeluneTelemetry owner;
        readonly ulong revision;
        long completed;
        long failed;
        public Generation(Transport transport, BindingTelemetryPreparation candidate, VeluneTelemetry binding)
        {
            Transport = transport; owner = binding; revision = candidate.Revision;
            transport.Feedback = success =>
            {
                if (success) Interlocked.Increment(ref completed); else Interlocked.Increment(ref failed);
                try { binding.ReportNativeExport(candidate.Revision, new BindingTelemetryExportHealth((ulong)Interlocked.Read(ref completed), (ulong)Interlocked.Read(ref failed), (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(), success ? "rpc_completed" : "transport_error", null)); }
                catch (BindingException) { }
                catch (ObjectDisposedException) { }
            };
            var identity = candidate.Identity;
            var attributes = new Dictionary<string, object> { ["service.name"] = identity.ServiceName, ["service.version"] = identity.Version,
                ["service.instance.id"] = identity.InstanceId, ["velune.platform"] = identity.Platform, ["deployment.environment.name"] = identity.Environment };
            if (identity.RunId != null) attributes["velune.run.id"] = identity.RunId;
            var resource = ResourceBuilder.CreateEmpty().AddAttributes(attributes);
            try
            {
            traces = Sdk.CreateTracerProviderBuilder().SetResourceBuilder(resource).AddSource("velune.client.native")
                .SetSampler(new ParentBasedSampler(new TraceIdRatioBasedSampler(candidate.Profile?.SampleRatio ?? 1)))
                .AddProcessor(new BatchActivityExportProcessor(new OtlpTraceExporter(transport.Options()), maxQueueSize: 512, scheduledDelayMilliseconds: 2000, exporterTimeoutMilliseconds: 3000, maxExportBatchSize: 64)).Build();
            metrics = Sdk.CreateMeterProviderBuilder().SetResourceBuilder(resource).AddMeter("velune.client.native")
                .AddView("velune.ui.operation.duration", new ExplicitBucketHistogramConfiguration { Boundaries = [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0] })
                .AddReader(new PeriodicExportingMetricReader(new OtlpMetricExporter(transport.Options()), exportIntervalMilliseconds: 10000, exportTimeoutMilliseconds: 3000)).Build();
            logs = new BatchLogRecordExportProcessor(new OtlpLogExporter(transport.Options()), maxQueueSize: 512, scheduledDelayMilliseconds: 2000, exporterTimeoutMilliseconds: 3000, maxExportBatchSize: 64);
            loggers = LoggerFactory.Create(builder => builder.AddOpenTelemetry(options => options.SetResourceBuilder(resource).AddProcessor(logs)));
            Logger = loggers.CreateLogger("velune.client.native");
            Counter = meter.CreateCounter<long>("velune.ui.operations"); Duration = meter.CreateHistogram<double>("velune.ui.operation.duration", "s");
            }
            catch
            {
                // A partially built generation can already own process-wide Activity/Meter listeners.
                transport.Revoke(); loggers?.Dispose(); logs?.Dispose(); metrics?.Dispose(); traces?.Dispose(); meter.Dispose(); Source.Dispose();
                throw;
            }
        }
        public void Flush() { traces.ForceFlush(3000); logs.ForceFlush(3000); metrics.ForceFlush(3000); }
        public void Dispose()
        {
            Transport.Revoke(); Source.Dispose(); meter.Dispose();
            using (SdkDropListener.Observe(owner, revision)) { traces.Dispose(); loggers.Dispose(); }
            metrics.Dispose();
        }
    }
    // Event 32 is emitted synchronously by shutdown; outside this scope it has no provider identity.
    sealed class SdkDropListener : EventListener
    {
        static readonly SdkDropListener listener = new();
        [ThreadStatic] static Action<string, ulong>? report;
        public static IDisposable Observe(VeluneTelemetry owner, ulong revision)
        {
            GC.KeepAlive(listener);
            var previous = report;
            report = (signal, count) => { try { owner.ReportNativeDrops(revision, signal, count); } catch (Exception) { } };
            return new Scope(previous);
        }
        sealed class Scope(Action<string, ulong>? previous) : IDisposable { public void Dispose() => report = previous; }
        protected override void OnEventSourceCreated(EventSource source)
        {
            if (source.Name == "OpenTelemetry-Sdk") EnableEvents(source, EventLevel.Warning);
        }
        protected override void OnEventWritten(EventWrittenEventArgs data)
        {
            if (report == null || data.EventId != 32 || data.Payload?.Count != 3 || data.Payload[2] is not long count || count <= 0) return;
            var signal = (data.Payload[0] as string, data.Payload[1] as string) switch
            {
                ("BatchActivityExportProcessor", "OtlpTraceExporter") => "traces",
                ("BatchLogRecordExportProcessor", "OtlpLogExporter") => "logs",
                _ => null,
            };
            if (signal != null) report(signal, (ulong)count);
        }
    }

}
