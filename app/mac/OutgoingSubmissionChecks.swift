import AppKit
import SwiftUI
import ObjectiveC
import VeluneBindings

@MainActor private final class OutgoingSubmissionCheckWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    private func refusePresentation() -> Never {
        FileHandle.standardError.write(Data("FAIL outgoing submissions: presentation requested\n".utf8)); exit(1)
    }
    override func makeKey() { refusePresentation() }
    override func makeMain() { refusePresentation() }
    override func makeKeyAndOrderFront(_ sender: Any?) { refusePresentation() }
    override func orderFront(_ sender: Any?) { refusePresentation() }
    override func orderBack(_ sender: Any?) { refusePresentation() }
    override func orderFrontRegardless() { refusePresentation() }
    override func order(_ place: NSWindow.OrderingMode, relativeTo otherWin: Int) {
        if place != .out { refusePresentation() }
        super.order(place, relativeTo: otherWin)
    }
}

/// Hosts the production split view and native titlebar without presenting a window.
@MainActor func outgoingSubmissionChecks() -> Never {
    func fail(_ message: String) -> Never {
        FileHandle.standardError.write(Data("FAIL outgoing submissions: \(message)\n".utf8)); exit(1)
    }
    func require(_ value: @autoclosure () -> Bool, _ message: String) {
        if !value() { fail(message) }
    }
    guard let path = ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"] else { fail("missing private output") }
    let directory = URL(fileURLWithPath: path).standardizedFileURL.resolvingSymlinksInPath()
    require(directory.path.hasPrefix("/private/tmp/") || directory.path.hasPrefix("/tmp/"), "output must be private temporary data")
    _ = NSApplication.shared
    if NSApp.activationPolicy() != .prohibited { _ = NSApp.setActivationPolicy(.prohibited) }
    require(NSApp.activationPolicy() == .prohibited, "activation policy must actually prohibit activation")
    var activations = 0, keys = 0, mains = 0, systemActivations = 0
    let frontmost = NSWorkspace.shared.frontmostApplication?.processIdentifier
    let active = NotificationCenter.default.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in activations += 1 }
    let key = NotificationCenter.default.addObserver(forName: NSWindow.didBecomeKeyNotification, object: nil, queue: .main) { _ in keys += 1 }
    let main = NotificationCenter.default.addObserver(forName: NSWindow.didBecomeMainNotification, object: nil, queue: .main) { _ in mains += 1 }
    let system = NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main) { notification in
        if (notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication)?.processIdentifier == ProcessInfo.processInfo.processIdentifier { systemActivations += 1 }
    }
    defer {
        NotificationCenter.default.removeObserver(active); NotificationCenter.default.removeObserver(key); NotificationCenter.default.removeObserver(main)
        NSWorkspace.shared.notificationCenter.removeObserver(system)
    }
    var windows: [OutgoingSubmissionCheckWindow] = []
    func safe() {
        require(NSApp.activationPolicy() == .prohibited && !NSApp.isActive && !NSRunningApplication.current.isActive,
                "activation is forbidden throughout the native check")
        require(activations == 0 && keys == 0 && mains == 0 && systemActivations == 0, "unexpected application/key/main activation")
        require(NSWorkspace.shared.frontmostApplication?.processIdentifier == frontmost, "frontmost application changed")
        for window in NSApp.windows {
            require(!window.isVisible && !window.isKeyWindow && !window.isMainWindow, "a window became visible, key or main")
            if !windows.contains(where: { $0 === window }) {
                let origin = class_getImageName(type(of: window)).map { String(cString: $0) } ?? ""
                require(origin.hasPrefix("/System/Library/"), "unexpected non-system window")
            }
        }
    }
    func settle() { RunLoop.main.run(until: Date().addingTimeInterval(0.08)); safe() }
    func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    let preferences = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
    var synthetic = preferences
    synthetic["toolbarButtonsHidden"] = [String]()
    synthetic["toolbarButtonOrder"] = [String]()
    UserDefaults.standard.setVolatileDomain(synthetic, forName: UserDefaults.argumentDomain)
    var evidence: [[String: Any]] = []
    for dark in [false, true] {
        NSApp.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        ConversationStore.checkOutgoingAdmissionLifecycle(home: directory.appendingPathComponent(dark ? "dark" : "light")) { model, state in
            let store = model.focused.store
            let root = VeluneRootView(window: model, store: model.list, tab: model.focused,
                                      focused: store, previewEmpty: false, previewSettings: false)
            let window = OutgoingSubmissionCheckWindow(contentRect: NSRect(x: 0, y: 0, width: 720, height: 760),
                                                       styleMask: [.titled, .closable, .resizable, .fullSizeContentView], backing: .buffered, defer: true)
            windows.append(window)
            window.isReleasedWhenClosed = false; window.isRestorable = false
            let shell = WorkspaceShell(window: window, model: model, content: root, previewEmpty: false)
            let host = shell.split
            window.setContentSize(NSSize(width: 720, height: 760))
            for _ in 0..<5 { host.view.layoutSubtreeIfNeeded(); settle() }
            let frame = window.contentView!.superview!
            let name = "outgoing-\(state)-\(dark ? "dark" : "light")"
            guard let bitmap = frame.bitmapImageRepForCachingDisplay(in: frame.bounds) else { fail("native window capture unavailable") }
            frame.cacheDisplay(in: frame.bounds, to: bitmap)
            try! bitmap.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent(name + ".png"))
            let expected = store.transcript.rows.map(\.id)
            let expectedSet = Set(expected)
            let cells = descendants(host.view).compactMap { $0 as? TranscriptCellView }.filter { !$0.isHidden && $0.rowID != nil }
            let sending = store.transcript.rows.filter(\.sending).map(\.id)
            let nativeSending = cells.filter { cell in
                descendants(cell).compactMap { $0 as? NSTextView }.contains { !$0.isHidden && $0.string == "发送中" }
            }.compactMap(\.rowID)
            require(nativeSending == sending, "sending caption must match the staged message: \(name), \(nativeSending), \(sending)")
            let nativeIDs = cells.compactMap(\.rowID).filter(expectedSet.contains)
            evidence.append(["case": name, "transcriptIDs": expected, "nativeIDs": nativeIDs, "sendingIDs": nativeSending,
                             "canonicalQueueIDs": store.followUp.queue.map(\.id),
                             "pendingQueueIDs": store.pendingQueuedMessages.map(\.id),
                             "nativeTableRowCounts": descendants(host.view).compactMap { ($0 as? NSTableView)?.numberOfRows },
                             "visible": window.isVisible, "key": window.isKeyWindow, "main": window.isMainWindow])
            try! JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys]).write(to: directory.appendingPathComponent("outgoing-submissions-check.json"))
            require(Set(nativeIDs) == expectedSet && nativeIDs.count == expected.count,
                    "production native transcript must show each expected row once: \(name), \(nativeIDs), \(expected)")
            for cell in cells { require(cell.frame.width > 0 && cell.frame.height > 0, "native transcript row must have actual geometry") }
            safe()
            window.contentViewController = nil
        }
    }
    UserDefaults.standard.setVolatileDomain(preferences, forName: UserDefaults.argumentDomain)
    safe()
    print("PASS outgoing submissions: real receipt-before-projection lane, poll-before-ACK, exact token ownership, late ACK, failure/retry/cancel/input revisions; \(evidence.count) native production-root PNGs; no activation or model calls")
    exit(0)
}
