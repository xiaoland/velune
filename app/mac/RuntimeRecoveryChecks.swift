import AppKit
import SwiftUI
import ObjectiveC
import VeluneBindings

/// Synthetic lifecycle checks, shared by the native interaction check entry.
func runtimeRecoveryLifecycleChecks() {
    func message(_ id: String, _ retry: RuntimeRetry?) -> DeliveredMessage {
        DeliveredMessage(stamp: 0, completed: true, id: id, role: .system, timestampUnixMs: nil, blocks: [.runtimeFailure(failure: .other, detail: "合成断连", retry: retry)])
    }
    let pending = RuntimeRecovery.latest(in: [message("first", .native)])!
    precondition(pending.active && pending.isVisible(dismissed: nil))
    precondition(!pending.isVisible(dismissed: pending.dismissalID), "忽略提示不等于停止重试")
    let recovered = message("first", .recovered)
    precondition(RuntimeRecovery.latest(in: [recovered]) == nil, "恢复后自动撤下提示")
    precondition(RuntimeRecovery.latest(in: [message("older", nil), recovered]) == nil, "恢复后不能重新显示更早错误")
    let next = RuntimeRecovery.latest(in: [recovered, message("second", .native)])!
    precondition(next.isVisible(dismissed: pending.dismissalID), "同一轮再次断连必须重新显示")
    for retry: RuntimeRetry? in [nil, .cancelled, .exhausted(attempts: 5)] {
        let ended = RuntimeRecovery.latest(in: [message("first", retry)])!
        precondition(!ended.active && ended.isVisible(dismissed: pending.dismissalID), "终止结果不能被先前忽略隐藏")
        precondition(!ended.isVisible(dismissed: ended.dismissalID))
    }
    let user = DeliveredMessage(stamp: 0, completed: true, id: "new-turn", role: .user, timestampUnixMs: nil, blocks: [.text(text: "合成新轮次")])
    precondition(RuntimeRecovery.latest(in: [message("old", nil), user]) == nil)
}

@MainActor private final class RuntimeRecoveryCheckWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    private func refusePresentation() -> Never {
        FileHandle.standardError.write(Data("FAIL runtime recovery: presentation requested\n".utf8)); exit(1)
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

@MainActor private struct RuntimeRecoveryCheckHost: View {
    @ObservedObject var store: ConversationStore
    let reading: TranscriptReading
    let stop: () -> Void
    var body: some View {
        TranscriptView(model: store.transcript, reading: reading, conversationID: store.loadedConversationID,
            scrollRequest: 0, cwd: nil, outlineCollapsed: .constant(false), navigation: nil, activity: nil,
            loadPayloads: { _ in preconditionFailure("inline recovery fixtures must not load runtime payloads") },
            openConversation: { _ in }, openAgentURI: { _ in }, describeSettings: { _ in nil },
            recovery: store.runtimeRecovery, canStopRecovery: true, stopRecovery: stop)
    }
}

/// Exercises the existing Store selector, collapsed native Work and actual
/// recovery banner. No runtime, scene, global input or new retry policy.
@MainActor func runtimeRecoveryNativeChecks(home: URL) -> Never {
    func fail(_ message: String) -> Never {
        FileHandle.standardError.write(Data("FAIL runtime recovery: \(message)\n".utf8)); exit(1)
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
    var windows: [RuntimeRecoveryCheckWindow] = []
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
    runtimeRecoveryLifecycleChecks()
    let detail = "Login expired · Please run /login\nClaude Code 错误类型：authentication_failed"
    let plainNotice = DeliveredMessage(id: "ordinary-notice", role: .system, blocks: [.notice(text: detail)])
    require(RuntimeRecovery.latest(in: [plainNotice]) == nil, "ordinary Notice must never be classified by its text")
    var evidence: [[String: Any]] = []
    var captures: [String] = []
    var dismissClicks = 0, workCollapseClicks = 0
    for dark in [false, true] {
        NSApp.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        for width: CGFloat in [420, 900] {
            let device = DeviceStore(preview: true)
            let model = ConversationWindowModel(application: device, kind: .workspace)
            let store = model.focused.store
            let reading = TranscriptReading()
            var stopRequests = 0
            let user = DeliveredMessage(id: "auth-user", role: .user, blocks: [.text(text: "合成请求：核对异常结束原因")])
            let tool = DeliveredMessage(id: "auth-tool", role: .assistant,
                blocks: [.tool(toolId: "tool-check", title: "合成工具工作", state: .completed,
                               output: .inline(text: "仅用于折叠工作过程验收"), linkedConversationId: nil)])
            func deliver(_ sequence: UInt64, failureID: String = "auth-failure", completed: Bool = false,
                         newUser: Bool = false, noticeOnly: Bool = false, retry: RuntimeRetry? = nil) {
                let failure = DeliveredMessage(id: failureID, role: .system,
                    blocks: noticeOnly ? [.notice(text: detail)] : [.runtimeFailure(failure: .other, detail: detail, retry: retry)])
                let reply = DeliveredMessage(id: "auth-success", role: .assistant, blocks: [.text(text: "同轮后续真实成功回复（合成）")])
                let next = DeliveredMessage(id: "next-user", role: .user, blocks: [.text(text: "新的用户轮次（合成）")])
                let messages = [user, tool, failure] + (completed ? [reply] : []) + (newUser ? [next] : [])
                let turn = TranscriptTurn(id: "turn:auth", userMessageId: user.id, workMessageIds: [tool.id, failure.id],
                    lastMessageId: completed ? reply.id : nil, replyStartBlock: 0, durationMs: 500, isRunning: false,
                    canFork: false, outcome: completed ? .completed : .failed, settings: nil, usage: nil)
                var items: [TranscriptItem] = [.message(id: "message:" + user.id, messageId: user.id), .work(turnId: turn.id)]
                if completed { items.append(.message(id: "message:" + reply.id, messageId: reply.id)) }
                if newUser { items.append(.message(id: "message:" + next.id, messageId: next.id)) }
                let summary = ConversationSummary(id: "auth-recovery-fixture", title: .native("合成认证失败"),
                    updatedAtUnixMs: nil, createdAtUnixMs: nil, runtimeId: "sample-instance", cwd: nil,
                    canRename: false, canDelete: false, parentConversationId: nil, nativeReadState: nil)
                let snapshot = DeliveredSnapshot(cursor: .init(generation: 808, sequence: sequence), base: nil,
                    messageIds: messages.map(\.id), messages: messages, revision: sequence, conversation: summary,
                    contextRuntimeId: "sample-instance", resourceId: nil, modelRecordKey: nil,
                    runState: completed ? .idle : .failed, transcriptTurns: [turn], usage: .init(), transcriptItems: items,
                    transcriptOutline: [], transcriptMessageIdentities: messages.map { .init(id: "message:" + $0.id, messageId: $0.id) },
                    messageIdentityConfirmations: [], pendingInteractions: [],
                    actions: .init(inputAccess: .editable, canSend: true, canCancel: false, canSwitch: true),
                    followUp: .init(), subAgents: [], collaborators: [])
                store.adopt(SnapshotResult(snapshot: snapshot))
            }
            deliver(1)
            guard let original = store.runtimeRecovery else { fail("typed authentication failure must reach the current recovery selector") }
            require(original.failure == .other && original.detail == detail && original.retry == nil && !original.active,
                    "authentication failure keeps native detail without inventing retry")
            let host = NSHostingController(rootView: RuntimeRecoveryCheckHost(store: store, reading: reading, stop: { stopRequests += 1 }))
            let window = RuntimeRecoveryCheckWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 620),
                                                    styleMask: [.borderless], backing: .buffered, defer: true)
            windows.append(window)
            window.isReleasedWhenClosed = false; window.isRestorable = false
            window.contentViewController = host
            window.setContentSize(NSSize(width: width, height: 620))
            func layout() { for _ in 0..<5 { host.view.layoutSubtreeIfNeeded(); settle() } }
            func closeButtons() -> [NSButton] {
                descendants(host.view).compactMap { $0 as? NSButton }.filter { button in
                    let r = button.convert(button.bounds, to: host.view)
                    let top = host.view.isFlipped ? r.minY : host.view.bounds.height - r.maxY
                    return !button.isHiddenOrHasHiddenAncestor && r.width > 0 && r.width <= 32 && r.height <= 32
                        && r.midX > width / 2 && top > host.view.bounds.height / 2
                }
            }
            func capture(_ state: String) {
                let name = "recovery-\(state)-\(dark ? "dark" : "light")-\(Int(width))"
                guard let bitmap = host.view.bitmapImageRepForCachingDisplay(in: host.view.bounds) else { fail("native recovery capture unavailable") }
                host.view.cacheDisplay(in: host.view.bounds, to: bitmap)
                try! bitmap.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent(name + ".png"))
                captures.append(name)
            }
            layout()
            // Failed work is deliberately expanded by default. Exercise the
            // real local disclosure before claiming coverage of collapsed Work.
            let initialCells = descendants(host.view).compactMap { $0 as? TranscriptCellView }.filter { !$0.isHidden }
            if initialCells.contains(where: { $0.rowID == "message:auth-failure" }) {
                guard let header = initialCells.first(where: { $0.rowID == "work:turn:auth" }),
                      let disclosure = descendants(header).compactMap({ $0 as? NSButton }).first(where: {
                          !$0.isHidden && !$0.attributedTitle.string.isEmpty
                      }) else { capture("diagnostic-expanded"); fail("native Work disclosure must be available") }
                disclosure.performClick(nil)
                workCollapseClicks += 1
                layout()
            }
            let cells = descendants(host.view).compactMap { $0 as? TranscriptCellView }.filter { !$0.isHidden }
            require(cells.contains { $0.rowID == "work:turn:auth" }, "native collapsed Work header must be materialized")
            require(!cells.contains { $0.rowID == "message:auth-failure" || $0.rowID == "message:auth-tool" },
                    "failure details must stay inside the still-collapsed Work while banner remains outside")
            capture("authentication-collapsed")
            let candidates = closeButtons()
            let nativeButtons = descendants(host.view).compactMap { $0 as? NSButton }.map { button -> [String: Any] in
                ["title": button.attributedTitle.string, "label": button.accessibilityLabel() ?? "", "help": button.toolTip ?? "",
                 "frame": NSStringFromRect(button.convert(button.bounds, to: host.view))]
            }
            evidence.append(["tone": dark ? "dark" : "light", "width": width, "recoveryID": original.id,
                             "detail": original.detail, "active": original.active, "retry": "none", "workCollapsed": true,
                             "closeCandidates": candidates.count, "nativeWorkCollapse": workCollapseClicks, "nativeButtons": nativeButtons])
            try! JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys]).write(to: directory.appendingPathComponent("recovery-check.json"))
            require(candidates.count == 1, "terminal banner must expose one native dismiss button and no stop/retry action")

            // Match the producer: genuine same-execution success marks the
            // original failure Recovered in both cold and live projections.
            deliver(2, completed: true, retry: .recovered)
            layout()
            require(RuntimeRecovery.latest(in: store.transcript.rows.map(\.message)) == nil
                    && store.runtimeRecovery == nil && closeButtons().isEmpty,
                    "producer Recovered state must withdraw the old failure independently of a Store completion guard")
            if !dark && width == 900 { capture("completed-cleared") }
            deliver(3, newUser: true)
            layout()
            require(store.runtimeRecovery == nil && closeButtons().isEmpty, "new user turn must not inherit an earlier terminal error")
            deliver(4, noticeOnly: true)
            layout()
            require(store.runtimeRecovery == nil && closeButtons().isEmpty, "ordinary Notice must not create a failure banner")
            deliver(5, failureID: "auth-again")
            layout()
            guard let again = store.runtimeRecovery else { fail("new typed occurrence must become visible") }
            require(closeButtons().count == 1 && again.detail == detail && !again.active, "new authentication occurrence remains terminal")
            closeButtons()[0].performClick(nil)
            dismissClicks += 1
            layout()
            require(closeButtons().isEmpty && store.runtimeRecovery == again && stopRequests == 0,
                    "native Ignore only hides the prompt, preserving failure data without invoking stop")
            if !dark && width == 420 { capture("ignored") }
            evidence[evidence.count - 1]["clearedOnCompleted"] = true
            evidence[evidence.count - 1]["clearedOnNewUser"] = true
            evidence[evidence.count - 1]["ordinaryNoticeIgnored"] = true
            evidence[evidence.count - 1]["nativeDismissed"] = true
            evidence[evidence.count - 1]["stopRequests"] = stopRequests
            try! JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys]).write(to: directory.appendingPathComponent("recovery-check.json"))
            require(device.transport == nil && !MacNativePersistenceGate.isEnabled && model.window == nil,
                    "recovery fixtures remain preview-only and cannot access runtime state")
            safe()
            window.contentViewController = nil
            store.close(); model.list.close()
        }
    }
    print("PASS runtime recovery: typed authentication failure outside collapsed Work, \(workCollapseClicks) native Work collapses, terminal/no retry, ordinary Notice, same-turn success/new user clearing, \(dismissClicks) native Ignore clicks without stop, \(captures.count) offscreen PNGs; no presentation, backend or model")
    exit(0)
}
