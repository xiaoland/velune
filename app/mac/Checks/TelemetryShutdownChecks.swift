#if VELUNE_DEV_CHECKS
import Foundation
import VeluneBindings

@MainActor func telemetryShutdownChecks() -> Never {
    func fail(_ reason: String) -> Never {
        FileHandle.standardError.write(Data("FAIL telemetry shutdown: \(reason)\n".utf8))
        exit(1)
    }
    guard let output = ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"],
          let resources = Bundle.main.resourceURL else { fail("missing isolated check directory or resources") }
    do {
        let directory = URL(fileURLWithPath: output, isDirectory: true).appendingPathComponent("telemetry-shutdown")
        let transport = Transport(stateDirectory: directory, resourcesDirectory: resources)
        try transport.prepareForLaunch()
        MacNativePersistenceGate.open()
        let store = DeviceStore(transport: transport)
        let layout = ConversationWindowModel(application: store, kind: .workspace).savedLayout
        let windowID = UUID().uuidString
        store.windowWorkspaceJournal.save(layout, id: windowID)
        let telemetry = try transport.telemetryCoordinator()
        try telemetry.save(revision: telemetry.binding.snapshot().revision, preference: .on,
                           profile: TelemetryProfile(endpoint: "http://127.0.0.1:1", headers: [:], sampleRatio: 1))
        let release = try telemetry.holdNativeTelemetryEventLoopForCheck()
        let started = ProcessInfo.processInfo.systemUptime
        let deadline = started + 7
        var closed: Bool?
        store.shutdown { closed = $0 }
        while closed == nil && ProcessInfo.processInfo.systemUptime < deadline {
            RunLoop.main.run(until: Date().addingTimeInterval(0.01))
        }
        release.signal()
        guard closed == true else { fail("application close did not complete while the native event loop was held") }
        guard try telemetry.binding.snapshot().status == "stopped" else { fail("telemetry owner did not stop") }
        let snapshot = directory.appendingPathComponent("mac-window-workspaces/\(windowID).json")
        guard try SavedWindowLayout.decodeKnown(Data(contentsOf: snapshot)) == layout else { fail("workspace recovery point did not persist") }
        let elapsed = String(format: "%.2f", ProcessInfo.processInfo.systemUptime - started)
        print("PASS application closed and workspace persisted in \(elapsed)s while the native telemetry event loop was held")
        exit(0)
    } catch { fail(ClientTelemetry.message(error)) }
}
#endif
