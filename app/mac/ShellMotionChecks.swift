import AppKit
import SwiftUI

/// Measures how long the main thread is held while the window's sidebar
/// collapses and expands, and what one layout pass of the shell costs at
/// a new sidebar width, over preview data in a window that is never shown.
@MainActor func shellMotionChecks() -> Never {
    func note(_ text: String) { FileHandle.standardError.write(Data((text + "\n").utf8)) }
    _ = NSApplication.shared
    if NSApp.activationPolicy() != .prohibited { _ = NSApp.setActivationPolicy(.prohibited) }
    let device = DeviceStore(preview: true)
    let model = ConversationWindowModel(application: device, kind: .workspace)
    model.start(restoring: nil, seed: WindowSeed(conversationID: "sample-0"))
    let root = VeluneRootView(window: model, store: model.list, tab: model.focused,
                              focused: model.focused.store, previewEmpty: false, previewSettings: false)
    let host = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1400, height: 900),
                        styleMask: [.titled, .closable, .resizable, .fullSizeContentView], backing: .buffered, defer: true)
    host.isReleasedWhenClosed = false; host.isRestorable = false
    let shell = WorkspaceShell(window: host, model: model, content: root, previewEmpty: false)
    let split = shell.split
    func run(_ seconds: TimeInterval) { RunLoop.main.run(until: Date().addingTimeInterval(seconds)) }
    for _ in 0..<10 { split.view.layoutSubtreeIfNeeded(); run(0.05) }

    /// The longest stretch of work one turn of the main run loop did,
    /// from waking to going back to wait (Core Animation's commit included),
    /// while `work` ran and the run loop settled after it. Any turn longer
    /// than a frame drops one while the sidebar moves.
    func held(_ label: String, settle: TimeInterval = 0.6, _ work: () -> Void) {
        var turns: [Double] = []
        var woke = CACurrentMediaTime()
        let wake = CFRunLoopObserverCreateWithHandler(nil, CFRunLoopActivity.afterWaiting.rawValue, true, CFIndex.min) { _, _ in woke = CACurrentMediaTime() }
        let rest = CFRunLoopObserverCreateWithHandler(nil, CFRunLoopActivity.beforeWaiting.rawValue, true, CFIndex.max) { _, _ in
            turns.append((CACurrentMediaTime() - woke) * 1000)
        }
        run(0.05)
        CFRunLoopAddObserver(CFRunLoopGetMain(), wake, .commonModes)
        CFRunLoopAddObserver(CFRunLoopGetMain(), rest, .commonModes)
        let start = CACurrentMediaTime()
        woke = start
        work()
        let worked = (CACurrentMediaTime() - start) * 1000
        run(settle)
        CFRunLoopRemoveObserver(CFRunLoopGetMain(), wake, .commonModes)
        CFRunLoopRemoveObserver(CFRunLoopGetMain(), rest, .commonModes)
        let long = turns.filter { $0 > 16.7 }.count
        note(String(format: "MOTION %@: call %.1fms, first turn %.1fms, longest turn %.1fms, turns over a frame %d",
                    label, worked, turns.first ?? 0, turns.max() ?? 0, long))
    }

    let sidebar = split.sidebarItem.viewController.view
    let editor = split.contentItem.viewController.view
    func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    func listed() -> Bool { descendants(sidebar).contains { $0.identifier?.rawValue == "conversation-list-new" } }
    func fail(_ message: String) -> Never { note("FAIL shell motion: \(message)"); exit(1) }
    precondition(listed(), "the sidebar starts with the list")
    for round in 1...3 {
        held("Hide Sidebar #\(round)") { split.toggleSidebar(nil) }
        if !split.sidebarItem.isCollapsed || model.tools.isOpen(.left) { fail("Hide Sidebar must collapse the area") }
        // A collapsed area keeps its views, so opening it again builds nothing.
        if !listed() { fail("a collapsed sidebar keeps the list") }
        held("Show Sidebar #\(round)") { split.toggleSidebar(nil) }
        if split.sidebarItem.isCollapsed || model.tools.tools(in: .left) != [.list] { fail("Show Sidebar must bring the list back") }
    }
    for round in 1...3 { held("same tools republished #\(round)") { model.tools = model.tools } }
    // The split moves once SwiftUI has followed the tools: a second tool's
    // split is in the sidebar's views by the time the split's turn comes.
    let before = descendants(sidebar).count
    var after = before
    model.tools.move(.outline, to: .docked(.left))
    RunLoop.main.afterViewsUpdate { after = descendants(sidebar).count }
    run(0.3)
    if after == before { fail("the area's views follow the tools before the split moves (\(before) views either way)") }
    model.tools = ToolWindowLayout()
    run(0.3)

    // One layout pass per new width, as each animation or drag frame asks.
    func pass(_ label: String, _ view: NSView, widths: [CGFloat]) {
        var costs: [Double] = []
        for width in widths {
            view.setFrameSize(NSSize(width: width, height: view.frame.height))
            let start = CACurrentMediaTime()
            view.layoutSubtreeIfNeeded()
            costs.append((CACurrentMediaTime() - start) * 1000)
        }
        let sorted = costs.sorted()
        note(String(format: "LAYOUT %@: median %.2fms, worst %.2fms over %d widths", label, sorted[sorted.count / 2], sorted.last ?? 0, costs.count))
    }
    let steps = (0...30).map { CGFloat($0) / 30 }
    pass("sidebar", sidebar, widths: steps.map { 280 - 90 * $0 })
    pass("editor", editor, widths: steps.map { 900 + 280 * $0 })
    var positions: [Double] = []
    for step in steps {
        let start = CACurrentMediaTime()
        split.splitView.setPosition(280 - 90 * step, ofDividerAt: 0)
        split.splitView.layoutSubtreeIfNeeded()
        positions.append((CACurrentMediaTime() - start) * 1000)
    }
    let sorted = positions.sorted()
    note(String(format: "LAYOUT whole split: median %.2fms, worst %.2fms", sorted[sorted.count / 2], sorted.last ?? 0))

    host.toolbar = nil
    host.contentViewController = nil
    host.close()
    withExtendedLifetime(shell) {}
    note("PASS shell motion: measured")
    exit(0)
}
