import XCTest
import AppKit
import ApplicationServices

final class MacTelemetryUI: XCTestCase {
    @objc func testSaveAndRestart() throws {
        continueAfterFailure = false
        try XCTSkipUnless(AXIsProcessTrusted(), "Native accessibility permission unavailable")
        let root = URL(fileURLWithPath: CommandLine.arguments[1])
        let home = root.appendingPathComponent("mac-save-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
        let bundle = URL(fileURLWithPath: CommandLine.arguments[2])
        let config = home.appendingPathComponent("telemetry/config.json")
        let status = home.appendingPathComponent("telemetry/status.json")
        func object(_ url: URL) -> [String: Any] {
            guard let d = try? Data(contentsOf: url), let value = try? JSONSerialization.jsonObject(with: d) as? [String: Any] else { return [:] }; return value
        }
        func wait(_ message: String, _ predicate: () -> Bool) {
            let end = Date().addingTimeInterval(8)
            while Date() < end && !predicate() { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
            XCTAssertTrue(predicate(), message)
        }
        var process: NSRunningApplication?
        func launch() throws -> AXUIElement {
            let child = Process()
            child.executableURL = bundle.appendingPathComponent("Contents/MacOS/Velune")
            child.environment = ProcessInfo.processInfo.environment.filter { ["PATH", "TMPDIR", "USER", "LOGNAME", "LANG"].contains($0.key) }
            child.environment?["VELUNE_HOME"] = home.path
            child.environment?["HOME"] = home.appendingPathComponent("os-home").path
            child.standardOutput = FileHandle.nullDevice; child.standardError = FileHandle.nullDevice
            try child.run()
            wait("isolated application did not register") { process = NSRunningApplication(processIdentifier: child.processIdentifier); return process != nil }
            let running = try XCTUnwrap(process)
            XCTAssertEqual(running.executableURL?.path, bundle.appendingPathComponent("Contents/MacOS/Velune").path)
            running.activate(options: [.activateAllWindows])
            let app = AXUIElementCreateApplication(running.processIdentifier)
            AXUIElementSetAttributeValue(app, "AXFrontmost" as CFString, kCFBooleanTrue)
            wait("isolated application has no visible window") {
                let windows = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]] ?? []
                return windows.contains { ($0[kCGWindowOwnerPID as String] as? Int) == Int(running.processIdentifier) }
            }
            return app
        }
        func attr(_ e: AXUIElement, _ key: String) -> CFTypeRef? { var v: CFTypeRef?; return AXUIElementCopyAttributeValue(e, key as CFString, &v) == .success ? v : nil }
        func elements(_ app: AXUIElement) -> [AXUIElement] {
            var all: [AXUIElement] = []
            func visit(_ e: AXUIElement, _ depth: Int) { guard depth < 24 && all.count < 1400 else { return }; all.append(e); for child in attr(e, "AXChildren") as? [AXUIElement] ?? [] { visit(child, depth + 1) } }
            for window in attr(app, "AXWindows") as? [AXUIElement] ?? [] { visit(window, 0) }
            return all
        }
        func named(_ app: AXUIElement, _ name: String) -> AXUIElement? { elements(app).first { e in
            if ["AXTitle", "AXDescription", "AXPlaceholderValue"].contains(where: { attr(e, $0) as? String == name }) { return true }
            if let title = attr(e, "AXTitleUIElement"), CFGetTypeID(title) == AXUIElementGetTypeID() { return attr(title as! AXUIElement, "AXValue") as? String == name }
            return false
        } }
        func press(_ app: AXUIElement, _ name: String) throws {
            wait("missing control: " + name) { named(app, name) != nil }
            let button = try XCTUnwrap(named(app, name)); XCTAssertEqual(AXUIElementPerformAction(button, "AXPress" as CFString), .success)
        }
        func replace(_ app: AXUIElement, _ name: String, _ text: String) throws {
            wait("missing actual field: " + name) { named(app, name) != nil }
            let field = try XCTUnwrap(named(app, name), "missing actual field: " + name)
            XCTAssertEqual(AXUIElementSetAttributeValue(field, "AXFocused" as CFString, kCFBooleanTrue), .success)
            let pid = try XCTUnwrap(process).processIdentifier
            func key(_ code: CGKeyCode, _ flags: CGEventFlags = []) {
                for down in [true, false] { let event = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down)!; event.flags = flags; event.postToPid(pid) }
            }
            key(0, .maskCommand); key(51)
            let chars = Array(text.utf16)
            for down in [true, false] { let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down)!; event.keyboardSetUnicodeString(stringLength: chars.count, unicodeString: chars); event.postToPid(pid) }
            RunLoop.current.run(until: Date().addingTimeInterval(0.15))
        }
        defer { if let process, !process.isTerminated { kill(process.processIdentifier, SIGTERM) } }
        var app = try launch()
        wait("ordinary launch missing status") { object(status)["preference"] as? String == "unset" }
        let initial = object(status)
        XCTAssertTrue(initial["rustActiveRevision"] is NSNull && initial["nativeActiveRevision"] is NSNull)
        func openSettings() throws {
            let pid = try XCTUnwrap(process).processIdentifier
            for down in [true, false] { let event = CGEvent(keyboardEventSource: nil, virtualKey: 43, keyDown: down)!; event.flags = .maskCommand; event.postToPid(pid) }
            wait("settings did not open") { named(app, "遥测") != nil }
        }
        try openSettings(); try press(app, "遥测")
        wait("telemetry page not loaded") { named(app, "保存") != nil }
        try replace(app, "OTLP/gRPC 地址", "http://127.0.0.1:34317")
        try press(app, "添加认证字段")
        try replace(app, "Metadata 名称", "x-velune-fixture")
        try replace(app, "值", "SYNTHETIC_MAC_NATIVE")
        try press(app, "保存")
        wait("real UI save did not persist expected profile") {
            let saved = object(config), profile = saved["profile"] as? [String: Any]
            return saved["preference"] as? String == "unset" && profile?["headers"] as? [String: String] == ["x-velune-fixture": "SYNTHETIC_MAC_NATIVE"]
        }
        let saved = object(config)
        process?.terminate(); wait("isolated process did not terminate") { process?.isTerminated == true }
        app = try launch(); try openSettings(); try press(app, "遥测")
        wait("saved endpoint missing after restart") { named(app, "OTLP/gRPC 地址").flatMap { attr($0, "AXValue") as? String } == "http://127.0.0.1:34317" }
        XCTAssertEqual(named(app, "Metadata 名称").flatMap { attr($0, "AXValue") as? String }, "x-velune-fixture")
        XCTAssertEqual(object(config)["revision"] as? Int, saved["revision"] as? Int)
        let restarted = object(status)
        XCTAssertTrue(restarted["rustActiveRevision"] is NSNull && restarted["nativeActiveRevision"] is NSNull)
        try JSONSerialization.data(withJSONObject: ["passed": true, "home": home.path, "defaultInactive": true, "nativeSavePersisted": true, "ordinaryRestartPreserved": true, "config": saved, "restarted": restarted], options: [.prettyPrinted, .sortedKeys]).write(to: root.appendingPathComponent("mac-save-evidence.json"), options: .atomic)
    }
}
let suite = MacTelemetryUI.defaultTestSuite
suite.run()
exit(suite.testRun?.hasSucceeded == true ? 0 : 1)
