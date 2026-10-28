#!/usr/bin/env python3
"""Isolated acceptance of the actual Apple coordinator, UniFFI application and OTLP wire."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import subprocess
import tempfile
import runpy
import sys
import threading
import time

from mac_telemetry_build import add_app_sources, add_telemetry

SWIFT = r'''
import Foundation
import VeluneBindings

enum Synthetic: Error { case expected }
@main struct Acceptance {
    static func require(_ condition: Bool, _ message: String) throws {
        if !condition { throw NSError(domain: "TelemetryAcceptance", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
    }
    static func invoke(_ owner: ClientTelemetry, _ app: VeluneApplication, _ name: NativeOperation, bad: Bool = false) throws -> String {
        var parent = ""
        do {
            try owner.operation(name) { carrier in
                parent = carrier?.traceparent ?? ""
                let view = try app.withTraceContext(carrier: carrier)
                if bad { _ = try view.snapshot(runtimeId: "CANARY-private-path", conversationId: "CANARY-private-body") }
                else { _ = try view.list() }
            }
            try require(!bad, "synthetic missing runtime unexpectedly succeeded")
        } catch {
            if !bad { throw error }
        }
        return parent
    }
    static func main() async {
        do { try await run() }
        catch {
            FileHandle.standardError.write(Data(("Telemetry acceptance failed: \(error)\n").utf8))
            exit(1)
        }
    }
    static func run() async throws {
        let args = CommandLine.arguments
        let home = URL(fileURLWithPath: args[1])
        let owner = try ClientTelemetry(home: home, platform: "macos", localLogs: false)
        try owner.start()
        let initial = try owner.binding.snapshot()
        try require(initial.rustActiveRevision == nil && initial.nativeActiveRevision == nil, "default must be inert")
        let app = try VeluneApplication.openWithTelemetry(options: BindingOptions(homeDirectory: home.path, resourcesDirectory: args[2]), telemetry: owner.binding)
        _ = try invoke(owner, app, .configurationRead)
        try await Task.sleep(nanoseconds: 300_000_000)
        let first = BindingTelemetryProfile(endpoint: args[3], headers: ["x-synthetic-authorization": "first"], sampleRatio: 1)
        let second = BindingTelemetryProfile(endpoint: args[4], headers: ["x-synthetic-authorization": "second"], sampleRatio: 1)
        try owner.save(revision: initial.revision, preference: .on, profile: first)
        let active = try owner.binding.snapshot()
        try require(active.applied && active.rustActiveRevision == active.nativeActiveRevision, "SDK revisions diverged")
        async let a = Task.detached { try invoke(owner, app, .send) }.value
        async let b = Task.detached { try invoke(owner, app, .send, bad: true) }.value
        let parents = try await [a, b]
        try require(parents[0] != parents[1] && !parents[0].isEmpty, "concurrent calls share their parent")
        _ = owner.binding.flush()
        try await Task.sleep(nanoseconds: 11_000_000_000)
        let discarded = try invoke(owner, app, .send)
        let revision = try owner.binding.snapshot().revision
        try owner.save(revision: revision, preference: .on, profile: second)
        let after = try invoke(owner, app, .send, bad: true)
        _ = owner.binding.flush()
        try await Task.sleep(nanoseconds: 11_000_000_000)
        let current = try owner.binding.snapshot()
        try require(current.nativeExport.completedBatches > 0 && current.rustExport.completedBatches > 0, "missing transport feedback")
        try owner.save(revision: current.revision, preference: .off, profile: second)
        try require(try invoke(owner, app, .send).isEmpty, "off SDK still creates a native context")
        try require(try owner.binding.snapshot().applied, "off is not acknowledged")
        // Dropping a view must not close the session; explicit shutdown must close every view.
        do { _ = try app.withTraceContext(carrier: nil).list() }
        let held = try app.withTraceContext(carrier: nil)
        _ = try app.list()
        try app.shutdown()
        do { _ = try held.list(); throw Synthetic.expected } catch BindingError.Diagnostic(let kind, _, _, _, _) { try require(kind == .closed, "shared close lost") }
        try owner.shutdown()
        let zeroHome = home.appendingPathComponent("zero")
        let zero = try ClientTelemetry(home: zeroHome, platform: "macos", localLogs: false)
        try zero.start()
        var unsampled = second; unsampled.sampleRatio = 0
        try zero.save(revision: 0, preference: .on, profile: unsampled)
        let zeroApp = try VeluneApplication.openWithTelemetry(options: BindingOptions(homeDirectory: zeroHome.path, resourcesDirectory: args[2]), telemetry: zero.binding)
        try? zero.operation(.send) { carrier in _ = try zeroApp.withTraceContext(carrier: carrier).snapshot(runtimeId: "CANARY-zero", conversationId: "CANARY-zero") }
        _ = zero.binding.flush()
        try await Task.sleep(nanoseconds: 11_000_000_000)
        try zeroApp.shutdown(); try zero.shutdown()
        let tls = try ClientTelemetry(home: home.appendingPathComponent("untrusted-tls"), platform: "macos", localLogs: false)
        try tls.start()
        let tlsProfile = BindingTelemetryProfile(endpoint: ProcessInfo.processInfo.environment["VELUNE_TEST_UNTRUSTED_TLS_ENDPOINT"]!, headers: [:], sampleRatio: 1)
        try tls.save(revision: 0, preference: .on, profile: tlsProfile)
        let tlsApp = try VeluneApplication.openWithTelemetry(options: BindingOptions(homeDirectory: home.appendingPathComponent("untrusted-tls").path, resourcesDirectory: args[2]), telemetry: tls.binding)
        _ = try invoke(tls, tlsApp, .send)
        _ = tls.binding.flush()
        try await Task.sleep(nanoseconds: 6_000_000_000)
        let tlsState = try tls.binding.snapshot()
        try require(tlsState.nativeExport.failedBatches > 0 && tlsState.rustExport.failedBatches > 0, "untrusted TLS failure was not observable on both SDKs")
        _ = try tlsApp.list()
        try tlsApp.shutdown(); try tls.shutdown()
        var result: [String: Any] = ["parents": parents, "discarded": discarded, "after": after, "instanceId": active.instanceId,
            "zeroInstanceId": try zero.binding.snapshot().instanceId, "sharedClose": true, "defaultOff": true, "offAcknowledged": true, "untrustedTlsRejectedBothSdks": true, "businessSurvivesTlsFailure": true]
        DISTRIBUTED_ACCEPTANCE
        QUEUE_ACCEPTANCE
        let data = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
        print(String(decoding: data, as: UTF8.self))
    }
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--swift-build", type=Path, required=True)
    parser.add_argument("--library", type=Path, required=True)
    parser.add_argument("--resources", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--distributed", action="store_true")
    parser.add_argument("--gateway", action="store_true")
    parser.add_argument("--queue", action="store_true")
    args = parser.parse_args()
    repository = Path(__file__).resolve().parent.parent
    for path in (args.swift_build, args.library, args.resources, args.output):
        if not path.is_absolute():
            parser.error("paths must be absolute")
    import grpc
    from google.protobuf.json_format import MessageToDict
    from opentelemetry.proto.collector.trace.v1 import trace_service_pb2 as traces, trace_service_pb2_grpc as trace_rpc
    from opentelemetry.proto.collector.logs.v1 import logs_service_pb2 as logs, logs_service_pb2_grpc as log_rpc
    from opentelemetry.proto.collector.metrics.v1 import metrics_service_pb2 as metrics, metrics_service_pb2_grpc as metric_rpc
    received = [[], [], []]
    lock = threading.Lock()
    servers = []
    ports = []

    class Receiver(trace_rpc.TraceServiceServicer, log_rpc.LogsServiceServicer, metric_rpc.MetricsServiceServicer):
        def __init__(self, index):
            self.index = index

        def Export(self, request, context):
            with lock:
                assert len(received[self.index]) < 1000, "unbounded export loop"
                received[self.index].append({"type": request.DESCRIPTOR.full_name, "metadata": dict(context.invocation_metadata()), "payload": MessageToDict(request)})
            if isinstance(request, traces.ExportTraceServiceRequest):
                return traces.ExportTraceServiceResponse()
            if isinstance(request, logs.ExportLogsServiceRequest):
                return logs.ExportLogsServiceResponse()
            return metrics.ExportMetricsServiceResponse()

    try:
        for index in range(2):
            server = grpc.server(ThreadPoolExecutor(max_workers=4))
            receiver = Receiver(index)
            trace_rpc.add_TraceServiceServicer_to_server(receiver, server)
            log_rpc.add_LogsServiceServicer_to_server(receiver, server)
            metric_rpc.add_MetricsServiceServicer_to_server(receiver, server)
            ports.append(server.add_insecure_port("127.0.0.1:0"))
            server.start(); servers.append(server)
        with tempfile.TemporaryDirectory(prefix="velune-telemetry-") as temporary:
            directory = Path(temporary)
            key, certificate = directory / "key.pem", directory / "cert.pem"
            subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", str(key), "-out", str(certificate), "-days", "1",
                            "-subj", "/CN=localhost", "-addext", "subjectAltName=IP:127.0.0.1"], check=True, capture_output=True)
            tls_server = grpc.server(ThreadPoolExecutor(max_workers=3))
            tls_receiver = Receiver(2)
            trace_rpc.add_TraceServiceServicer_to_server(tls_receiver, tls_server)
            log_rpc.add_LogsServiceServicer_to_server(tls_receiver, tls_server)
            metric_rpc.add_MetricsServiceServicer_to_server(tls_receiver, tls_server)
            tls_port = tls_server.add_secure_port("127.0.0.1:0", grpc.ssl_server_credentials([(key.read_bytes(), certificate.read_bytes())]))
            assert tls_port > 0
            tls_server.start(); servers.append(tls_server)
            source = directory / "Acceptance.swift"
            swift_source = SWIFT.replace("QUEUE_ACCEPTANCE", 'result["queue"] = try await queueAcceptance(home, args[2], first, second)' if args.queue else "")
            swift_source = swift_source.replace("DISTRIBUTED_ACCEPTANCE", 'result["distributed"] = try await distributed(home, args[2], first, second, args[5], args[6])' if args.distributed else "")
            if not (args.distributed or args.queue):
                swift_source = swift_source.replace("var result: [String: Any]", "let result: [String: Any]")
            extra_arguments = []
            if args.distributed:
                fixture_root = directory / "home"
                fixture_root.mkdir()
                fixtures = runpy.run_path(str(repository / "scripts/manual-remote-control.py"), run_name="fixture")
                fixtures["write_peers"](fixture_root)
                peer = fixture_root / "beta.py"
                text = peer.read_text().replace("if text == 'GATE':", "if text.startswith('GATE'):\n            pending=identity")
                text = text.replace("def release(identity=identity):", "def release(identity=identity):\n                global pending")
                text = text.replace("                answer(identity, 'DISCONNECTED_TURN_COMPLETED')", "                if pending == identity:\n                    answer(identity, 'DISCONNECTED_TURN_COMPLETED')\n                    pending=None")
                text = text.replace("        if text.startswith('GATE'):", "        if text.startswith('BREAK_CANCEL'):\n            os.close(0)\n            (root / 'cancel-ready').write_text('ready')\n            time.sleep(20)\n        elif text.startswith('GATE'):")
                peer.write_text(text)
                marker = "    if method=='initialize':"
                (fixture_root / "slow-peer.py").write_text(text.replace(marker, marker + "\n        Path(os.environ['SLOW_READY']).write_text('ready')\n        while Path(os.environ['SLOW_GATE']).exists(): time.sleep(.02)"))
                swift_source += (repository / "tasks/otlp-observability/acceptance/distributed.swift").read_text()
                extra_arguments = [str(peer), sys.executable]
            source.write_text(swift_source)
            command = ["xcrun", "swiftc", "-parse-as-library", "-swift-version", "5", "-warnings-as-errors", "-I", str(args.swift_build / "Modules"), "-I", str(repository / "target/swift-ffi"), str(source)]
            if args.queue:
                command.remove("-warnings-as-errors")  # Match the app build's existing UI warning policy.
                store = directory / "Store.swift"
                store.write_text((repository / "app/mac/Store.swift").read_text() + "\n" + (repository / "tasks/otlp-observability/acceptance/queue.swift").read_text())
                add_app_sources(command, repository, directory, store)
            add_telemetry(command, args.swift_build, repository)
            command.extend(["-L", str(args.library.parent), "-lvelune_bindings", "-Xlinker", "-rpath", "-Xlinker", str(args.library.parent), "-o", str(directory / "acceptance")])
            subprocess.run(command, cwd=repository, check=True)
            home = directory / "home"; home.mkdir(exist_ok=True)
            environment = {"HOME": str(home), "PATH": "/usr/bin:/bin", "RUST_BACKTRACE": "1", "OTEL_EXPORTER_OTLP_HEADERS": "x-env-canary=CANARY-env-secret", "OTEL_EXPORTER_OTLP_ENDPOINT": "http://127.0.0.1:1", "OTEL_RESOURCE_ATTRIBUTES": "private=CANARY-resource", "OTEL_TRACES_SAMPLER": "always_off"}
            environment["VELUNE_TEST_UNTRUSTED_TLS_ENDPOINT"] = f"https://127.0.0.1:{tls_port}"
            output = subprocess.check_output([str(directory / "acceptance"), str(home), str(args.resources), *[f"http://127.0.0.1:{port}" for port in ports], *extra_arguments], cwd=directory, env=environment, text=True, timeout=180)
            result = json.loads(output.strip().splitlines()[-1])
            if args.gateway:
                fixture = runpy.run_path(str(repository / "tasks/otlp-observability/acceptance/gateway.py"))
                result["gateway"] = fixture["run_gateway"](repository, directory, ports, args.library)
            args.output.with_suffix(".boundaries.json").write_text(json.dumps(result) + "\n")
        scopes = []
        spans = []
        wire = args.output.with_suffix(".wire.json")
        wire.parent.mkdir(parents=True, exist_ok=True)
        wire.write_text(json.dumps(received) + "\n")
        instances = {}
        zero_signals = set()
        assert not received[2], "a product SDK bypassed the OS certificate policy"
        for index, batches in enumerate(received[:2]):
            assert batches, "no actual SDK export"
            signals = set()
            instance_scopes = set()
            these_spans = []
            for batch in batches:
                assert batch["metadata"].get("x-synthetic-authorization") == ("first" if index == 0 else "second")
                assert "x-env-canary" not in batch["metadata"]
                payload = batch["payload"]
                assert "CANARY" not in json.dumps(payload), "private input reached an OTLP payload"
                signal = next(iter(payload)).removeprefix("resource")
                signals.add(signal)
                for resources in payload.values():
                    for resource in resources:
                        attributes = {entry["key"]: entry.get("value", {}).get("stringValue") for entry in resource.get("resource", {}).get("attributes", [])}
                        instance = attributes.get("service.instance.id")
                        if attributes.get("service.instance.id") == result["zeroInstanceId"]:
                            zero_signals.add(signal)
                        for key in ("scopeSpans", "scopeLogs", "scopeMetrics"):
                            for scope in resource.get(key, []):
                                instance_scopes.add((signal, scope["scope"]["name"]))
                                these_spans.extend(scope.get("spans", []))
                                for field in ("spans", "logRecords", "metrics"):
                                    instances.setdefault((index, instance, signal), []).extend(scope.get(field, []))
            assert signals == {"Spans", "Logs", "Metrics"}, signals
            for signal in signals:
                assert (signal, "velune.client.native") in instance_scopes and (signal, "velune_telemetry" if signal == "Logs" else "velune.client.rust") in instance_scopes, instance_scopes
            spans.append(these_spans); scopes.append(sorted(instance_scopes))
        assert zero_signals == {"Logs", "Metrics"}, "zero sampling lost errors/metrics or emitted traces"
        import base64
        def hexadecimal(value):
            return base64.b64decode(value).hex()
        for parent in result["parents"]:
            _, trace_id, span_id, _ = parent.split("-")
            children = [span for span in spans[0] if hexadecimal(span["traceId"]) == trace_id and hexadecimal(span.get("parentSpanId", "")) == span_id]
            assert len(children) == 1, "native -> immutable FFI parent was lost or duplicated"
        discarded_trace = result["discarded"].split("-")[1]
        assert not any(hexadecimal(span["traceId"]) == discarded_trace for group in spans for span in group), "a retired queue was exported"
        def attributes(record):
            return {entry["key"]: next(iter(entry.get("value", {}).values()), None) for entry in record.get("attributes", [])}
        def trace_records(index, instance, parent, signal="Spans"):
            trace = parent.split("-")[1]
            return [record for record in instances.get((index, instance, signal), []) if hexadecimal(record.get("traceId", "")) == trace]
        if "distributed" in result:
            distributed = result["distributed"]
            host, controller = distributed["hostInstance"], distributed["controllerInstance"]
            rejected = trace_records(0, host, distributed["preparationRejectedParent"], "Logs")
            assert not any(record.get("eventName") in ("velune.execution.turn.started", "velune.execution.turn.finished") for record in rejected)
            failed_preparation = trace_records(0, host, distributed["preparationFailedParent"])
            assert any(attributes(span).get("velune.operation") == "harness.start" and attributes(span).get("velune.outcome") == "failure" for span in failed_preparation), "real harness preparation failure was not observed"
            for parent in distributed["parents"]:
                combined = trace_records(0, host, parent) + trace_records(0, controller, parent)
                by_id = {hexadecimal(span["spanId"]): span for span in combined}
                def descends(child, ancestor):
                    seen = set()
                    while child and child not in seen:
                        if child == ancestor:
                            return True
                        seen.add(child)
                        child = hexadecimal(by_id.get(child, {}).get("parentSpanId", ""))
                    return False
                operations = {name: [span for span in combined if attributes(span).get("velune.operation") == name] for name in ("remote.request", "remote.execute", "execution.turn")}
                assert all(len(group) == 1 for group in operations.values()), {name: len(group) for name, group in operations.items()}
                request, execute, turn = [operations[name][0] for name in operations]
                assert descends(hexadecimal(request["spanId"]), parent.split("-")[2]), "native caller lost request ancestry"
                assert descends(hexadecimal(execute["spanId"]), hexadecimal(request["spanId"])), "QUIC execute lost request ancestry"
                assert descends(hexadecimal(turn["spanId"]), hexadecimal(execute["spanId"])), "actual turn lost accepted-command ancestry"
            for parent, outcome in zip(distributed["switchedParents"], ("success", "cancelled")):
                starts = [record for record in trace_records(0, host, parent, "Logs") if record.get("eventName") == "velune.execution.turn.started"]
                finishes = [record for record in trace_records(1, host, parent, "Logs") if record.get("eventName") == "velune.execution.turn.finished"]
                assert len(starts) == len(finishes) == 1, "switched turn lost or duplicated lifecycle"
                before, after = attributes(starts[0]), attributes(finishes[0])
                assert before["velune.operation.id"] == after["velune.operation.id"] and after["velune.outcome"] == outcome
                assert after.get("velune.correlation.gap") == "retired_generation", "retired result hid the missing old span"
            broken = trace_records(1, host, distributed["brokenCancelParent"], "Logs")
            terminal = [attributes(record) for record in broken if record.get("eventName") == "velune.execution.turn.finished"]
            assert len(terminal) == 1 and terminal[0]["velune.outcome"] == "failure", "broken cancel fabricated cancellation or lost its terminal event"
            gauges = [int(point["asInt"]) for metric in instances.get((1, host, "Metrics"), []) if metric["name"] == "velune.execution.active_turns" for point in metric.get("gauge", {}).get("dataPoints", [])]
            changes = [value for index, value in enumerate(gauges) if index == 0 or gauges[index - 1] != value]
            assert changes[:3] == [2, 1, 0], ("new generation lost live business state", changes)
            unknown = [point for metric in instances.get((0, controller, "Metrics"), []) if metric["name"] == "velune.ui.operations" for point in metric.get("sum", {}).get("dataPoints", []) if attributes(point).get("velune.outcome") == "unknown"]
            assert unknown and max(int(point["asInt"]) for point in unknown) >= 1, "native timeout was counted as business failure"
            sampled = trace_records(1, host, distributed["sampledIntoZero"])
            assert any(attributes(span).get("velune.operation") == "execution.turn" for span in sampled), "root ratio zero discarded a sampled remote parent"
            unsampled = distributed["unsampledIntoOne"].split("-")[1]
            assert not any(hexadecimal(span["traceId"]) == unsampled for group in spans for span in group), "root ratio one re-sampled an unsampled parent"
            inherited_logs = trace_records(1, host, distributed["unsampledIntoOne"], "Logs")
            assert any(record.get("eventName") == "velune.execution.turn.finished" for record in inherited_logs), "sampled=0 context was lost before the actual remote turn"
        if "gateway" in result:
            gateway = result["gateway"]
            instance = gateway["instanceId"]
            startups = [span for span in instances.get((0, instance, "Spans"), []) if attributes(span).get("velune.operation") == "harness.start"]
            assert len(startups) == 1, "a live runtime reader retained or lost its completed startup span"
            assert not any(attributes(span).get("velune.operation") == "harness.start" for span in instances.get((1, instance, "Spans"), [])), "closing an old reader exported or duplicated startup in the new generation"
            retry = trace_records(0, instance, gateway["parents"]["retry"])
            requests = [span for span in retry if attributes(span).get("velune.operation") == "gateway.request"]
            attempts = [span for span in retry if attributes(span).get("velune.operation") == "gateway.attempt"]
            assert len(requests) == 1 and len(attempts) == 2
            assert sorted(attributes(span)["velune.outcome"] for span in attempts) == ["failure", "success"]
            assert next(attributes(span)["velune.operation.id"] for span in attempts if attributes(span)["velune.outcome"] == "failure") == gateway["failedAttemptOperationId"], "OTLP failure could not locate the local upstream diagnostic"
            assert all(span["parentSpanId"] == requests[0]["spanId"] for span in attempts), "retry attempt escaped its immutable request parent"
            for name in ("one", "two"):
                records = trace_records(1, instance, gateway["parents"][name])
                results = [span for span in records if span.get("name") in ("gateway.request.result", "gateway.attempt.result")]
                assert len(results) == 2 and all(attributes(span).get("velune.correlation.gap") == "retired_generation" for span in results), "held request changed parent or lost its retired result"
                assert all(attributes(span).get("velune.outcome") == "success" for span in results)
            missing = [span for span in instances.get((1, instance, "Spans"), []) if attributes(span).get("velune.operation") == "gateway.request" and attributes(span).get("velune.correlation.gap") == "missing_parent"]
            assert len(missing) == 1, "unprovable gateway ownership was hidden"
        if "queue" in result:
            queue = result["queue"]
            current = instances.get((1, queue["instanceId"], "Metrics"), [])
            waits = [point for metric in current if metric["name"] == "velune.ui.queue.wait" for point in metric.get("histogram", {}).get("dataPoints", [])]
            assert waits and max(point["sum"] for point in waits) >= .5, "actual Store queue wait was invisible"
            assert queue["fastTransportSeconds"] < .3, "the synthetic quick call itself was slow"
            reads = [point for metric in current if metric["name"] == "velune.ui.operations" for point in metric.get("sum", {}).get("dataPoints", []) if attributes(point).get("velune.operation") == "ui.configuration.read"]
            assert reads and max(int(point["asInt"]) for point in reads) == 1, "queue timing duplicated the business operation count"
            old = instances.get((0, queue["instanceId"], "Metrics"), [])
            assert not any(metric["name"] == "velune.ui.queue.wait" and any(point["sum"] >= .5 for point in metric.get("histogram", {}).get("dataPoints", [])) for metric in old), "wait was exported to the enqueue generation"
        evidence = {"passed": True, "boundaries": result, "scopes": scopes, "batches": list(map(len, received)), "timeUnixMs": int(time.time() * 1000)}
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(evidence, indent=2) + "\n")
        print(json.dumps(evidence))
    finally:
        for server in servers:
            server.stop(0).wait()


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        raise SystemExit(f"acceptance subprocess failed (exit {error.returncode})")
