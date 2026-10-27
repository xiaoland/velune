// Appended to the temporary copy of Store.swift so the actual private scheduling boundary is exercised.
extension AppStore {
    @MainActor func telemetryQueueProbe(_ transport: Transport, entered: DispatchSemaphore, release: DispatchSemaphore, enqueued: DispatchSemaphore) -> Task<Double, Error> {
        performOnQueue { entered.signal(); release.wait() }
        return Task {
            try await withCheckedThrowingContinuation { continuation in
                performOnQueue {
                    let started = ProcessInfo.processInfo.systemUptime
                    do {
                        _ = try transport.list()
                        continuation.resume(returning: ProcessInfo.processInfo.systemUptime - started)
                    } catch { continuation.resume(throwing: error) }
                }
                enqueued.signal()
            }
        }
    }
}

extension Acceptance {
    @MainActor static func queueAcceptance(_ root: URL, _ resources: String, _ first: BindingTelemetryProfile, _ second: BindingTelemetryProfile) async throws -> [String: Any] {
        let home = root.appendingPathComponent("queue")
        let owner = try ClientTelemetry(home: home, platform: "macos", localLogs: false)
        try owner.start(); try owner.save(revision: 0, preference: .on, profile: first)
        let app = try VeluneApplication.openWithTelemetry(options: BindingOptions(homeDirectory: home.path, resourcesDirectory: resources), telemetry: owner.binding)
        let transport = Transport(stateDirectory: home, telemetry: .success(owner), remote: { app })
        let store = AppStore(transport: transport)
        let entered = DispatchSemaphore(value: 0), release = DispatchSemaphore(value: 0), enqueued = DispatchSemaphore(value: 0)
        let finished = store.telemetryQueueProbe(transport, entered: entered, release: release, enqueued: enqueued)
        await withCheckedContinuation { continuation in
            DispatchQueue.global().async { entered.wait(); enqueued.wait(); continuation.resume() }
        }
        try await Task.sleep(nanoseconds: 500_000_000)
        try owner.save(revision: try owner.binding.snapshot().revision, preference: .on, profile: second)
        release.signal()
        let duration = try await finished.value
        _ = owner.binding.flush()
        let instance = try owner.binding.snapshot().instanceId
        try await Task.sleep(nanoseconds: 11_000_000_000)
        try app.shutdown(); try owner.shutdown()
        return ["instanceId": instance, "fastTransportSeconds": duration, "sameOwnerReconfigured": true]
    }
}
