import AppKit
import VeluneBindings
import ObjectiveC

@MainActor private final class BackgroundNotificationCheckWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    private func refusePresentation() -> Never {
        FileHandle.standardError.write(Data("FAIL background notifications: presentation requested\n".utf8)); exit(1)
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

private struct BackgroundNotificationFixture {
    let id: String
    let messages: [DeliveredMessage]
    let turns: [TranscriptTurn]
    let items: [TranscriptItem]
    let expandedIDs: [String]
    let collapsedIDs: [String]
    let notices: [String: String]
    let groupedNoticeIDs: Set<String>
}

/// Exercises the production projection model, virtualized native list and local
/// AppKit disclosure actions. Fixtures are delivered-contract data, not a new classifier.
@MainActor func backgroundNotificationChecks() -> Never {
    func fail(_ message: String) -> Never {
        FileHandle.standardError.write(Data("FAIL background notifications: \(message)\n".utf8)); exit(1)
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
    var windows: [BackgroundNotificationCheckWindow] = []
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
    func shown(_ view: NSView) -> Bool {
        var current: NSView? = view
        while let item = current { if item.isHidden { return false }; current = item.superview }
        return true
    }
    func visibleTexts(in root: NSView) -> [TranscriptTextView] {
        var result: [TranscriptTextView] = []
        for view in descendants(root) {
            if let text = view as? TranscriptTextView, shown(text) { result.append(text) }
        }
        return result
    }
    func tool(_ id: String, _ title: String) -> DeliveredMessage {
        DeliveredMessage(id: id, role: .assistant, blocks: [.tool(toolId: id, title: title, state: .completed, output: .inline(text: "合成工具输出"), linkedConversationId: nil)])
    }
    func notice(_ id: String, _ marker: String, long: Bool = false) -> DeliveredMessage {
        let body = "后台任务通知 \(marker)：合成任务已完成。" + (long ? "\n" + String(repeating: "这是用于检查窄窗口换行的长通知，保留任务完成顺序和可读内容。", count: 7) : "")
        return DeliveredMessage(id: id, role: .client, blocks: [.notice(text: body)])
    }
    let user = DeliveredMessage(id: "user", role: .user, blocks: [.text(text: "检查后台任务通知如何进入工作过程（合成消息）")])
    let before = tool("before", "工具前：启动后台任务")
    let after = tool("after", "工具后：继续处理结果")
    let reply = DeliveredMessage(id: "reply", role: .assistant, blocks: [.text(text: "FINAL_REPLY：任务已完成，这是最终回复。")])
    func turn(_ work: [String], closing: String?, split: UInt32 = 0, running: Bool = false) -> TranscriptTurn {
        TranscriptTurn(id: "turn:message:user", userMessageId: "user", workMessageIds: work, lastMessageId: closing, replyStartBlock: split,
                       durationMs: running ? nil : 1200, isRunning: running, canFork: !running, outcome: running ? nil : .completed, settings: nil, usage: nil)
    }
    func presentationID(_ nativeID: String) -> String {
        if nativeID == "work:turn" { return "work:turn:message:user" }
        if nativeID.hasPrefix("reply:") { return "reply:message:" + String(nativeID.dropFirst("reply:".count)) }
        return "message:" + nativeID
    }
    func messageItem(_ id: String, source: String? = nil) -> TranscriptItem { .message(id: presentationID(id), messageId: source ?? id) }
    let startItems: [TranscriptItem] = [messageItem("user"), .work(turnId: "turn:message:user")]
    let mixed = DeliveredMessage(id: "closing", role: .assistant, blocks: [
        .reasoning(summary: "收尾工作：核对后台结果", text: .inline(text: "合成收尾推理")),
        .text(text: "FINAL_MIXED：混合 closing 的最终回复保持独立。")
    ])
    let fixtures: [BackgroundNotificationFixture] = [
        .init(id: "running", messages: [user, before, notice("notification", "N_RUNNING", long: true), after],
              turns: [turn(["before", "notification", "after"], closing: nil, running: true)], items: startItems,
              expandedIDs: ["user", "work:turn", "before", "notification", "after"], collapsedIDs: ["user", "work:turn"],
              notices: ["notification": "N_RUNNING"], groupedNoticeIDs: ["notification"]),
        .init(id: "before-final", messages: [user, before, notice("notification", "N_BEFORE_FINAL"), after, reply],
              turns: [turn(["before", "notification", "after"], closing: "reply")], items: startItems + [messageItem("reply")],
              expandedIDs: ["user", "work:turn", "before", "notification", "after", "reply"], collapsedIDs: ["user", "work:turn", "reply"],
              notices: ["notification": "N_BEFORE_FINAL"], groupedNoticeIDs: ["notification"]),
        .init(id: "mixed-closing", messages: [user, before, notice("notification", "N_MIXED"), mixed],
              turns: [turn(["before", "notification", "closing"], closing: "closing", split: 1)], items: startItems + [messageItem("reply:closing", source: "closing")],
              expandedIDs: ["user", "work:turn", "before", "notification", "closing", "reply:closing"], collapsedIDs: ["user", "work:turn", "reply:closing"],
              notices: ["notification": "N_MIXED"], groupedNoticeIDs: ["notification"]),
        .init(id: "late", messages: [user, before, reply, notice("late", "N_LATE")],
              turns: [turn(["before"], closing: "reply")], items: startItems + [messageItem("reply"), messageItem("late")],
              expandedIDs: ["user", "work:turn", "before", "reply", "late"], collapsedIDs: ["user", "work:turn", "reply", "late"],
              notices: ["late": "N_LATE"], groupedNoticeIDs: []),
        .init(id: "orphan", messages: [notice("orphan-first", "N_ORPHAN_FIRST"), DeliveredMessage(id: "orphan-text", role: .assistant, blocks: [.text(text: "无所属轮次的中间记录")]), notice("orphan-last", "N_ORPHAN_LAST")],
              turns: [], items: [messageItem("orphan-first"), messageItem("orphan-text"), messageItem("orphan-last")],
              expandedIDs: ["orphan-first", "orphan-text", "orphan-last"], collapsedIDs: ["orphan-first", "orphan-text", "orphan-last"],
              notices: ["orphan-first": "N_ORPHAN_FIRST", "orphan-last": "N_ORPHAN_LAST"], groupedNoticeIDs: []),
        .init(id: "unknown-custom", messages: [user, before, notice("notification", "N_KNOWN"), after, DeliveredMessage(id: "unknown", role: .client, blocks: [.text(text: "未知 custom 原样保留 N_UNKNOWN_CUSTOM，不属于后台通知归组。")]), reply],
              turns: [turn(["before", "notification", "after"], closing: "reply")], items: startItems + [messageItem("unknown"), messageItem("reply")],
              expandedIDs: ["user", "work:turn", "before", "notification", "after", "unknown", "reply"], collapsedIDs: ["user", "work:turn", "unknown", "reply"],
              notices: ["notification": "N_KNOWN", "unknown": "N_UNKNOWN_CUSTOM"], groupedNoticeIDs: ["notification"])
    ]
    var measurements: [[String: Any]] = []
    var captures: [String] = []
    var clicks = 0
    safe()
    for dark in [false, true] {
        let appearance = NSAppearance(named: dark ? .darkAqua : .aqua)!
        NSApp.appearance = appearance
        for width: CGFloat in [420, 900] {
            for fixture in fixtures {
                let model = TranscriptModel()
                let controller = TranscriptTableController()
                let scroll = controller.view()
                let window = BackgroundNotificationCheckWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 1100), styleMask: [.borderless], backing: .buffered, defer: true)
                window.isReleasedWhenClosed = false; window.isRestorable = false; window.appearance = appearance
                windows.append(window); window.contentView = scroll
                scroll.layoutSubtreeIfNeeded(); settle()
                guard let document = scroll.documentView else { fail("production transcript document missing") }
                func cells() -> [TranscriptCellView] { document.subviews.compactMap { $0 as? TranscriptCellView }.filter { !$0.isHidden && $0.rowID != nil } }
                func apply(_ messages: [DeliveredMessage], turns: [TranscriptTurn]) {
                    model.apply(messages, turns: turns, items: fixture.items, outline: [], identities: messages.map { .init(id: presentationID($0.id), messageId: $0.id) }, editable: [])
                    controller.update(TranscriptTable(controller: controller, conversationID: fixture.id, items: model.items(for: model.rows, turnHeaders: true), revision: model.contentRevision,
                        activity: nil, outlineCollapsed: false, outlineAnchorID: nil, cwd: nil, bottomRequest: 0, jump: nil,
                        followingChanged: { _ in }, outlineToggle: { _ in }, loadPayloads: { _ in fail("inline fixture must not request deferred data") }, openConversation: { _ in }, openAgentURI: { _ in }, openArtifact: { _ in },
                        editMessage: { _ in nil }, forkConversation: { _ in }, describeSettings: { _ in nil }))
                    scroll.layoutSubtreeIfNeeded(); settle(); controller.materializeVisibleRows(extra: document.bounds)
                }
                if fixture.id == "running" {
                    apply(fixture.messages.filter { $0.id != "notification" }, turns: [turn(["before", "after"], closing: nil, running: true)])
                }
                let retained = Dictionary(uniqueKeysWithValues: cells().filter { [presentationID("before"), presentationID("after")].contains($0.rowID!) }.map { ($0.rowID!, ObjectIdentifier($0)) })
                apply(fixture.messages, turns: fixture.turns)
                if fixture.id == "running" {
                    require(retained.count == 2, "running insertion must first materialize both neighboring native cells")
                    require(cells().filter { retained[$0.rowID!] != nil }.allSatisfy { retained[$0.rowID!] == ObjectIdentifier($0) }, "middle notification insertion must retain neighboring native cells")
                }
                func inspect(expanded: Bool, capture: Bool) {
                    scroll.layoutSubtreeIfNeeded(); settle(); controller.materializeVisibleRows(extra: document.bounds)
                    let expected = (expanded ? fixture.expandedIDs : fixture.collapsedIDs).map(presentationID)
                    let actualCells = cells().filter {
                        let id = $0.rowID!
                        return id != "transcript-end" && id != "transcript-activity" && !id.hasPrefix("work-actions:")
                    }
                    require(actualCells.map { $0.rowID! } == expected, "\(fixture.id): actual native row order \(actualCells.map { $0.rowID! }) must equal \(expected)")
                    require(zip(actualCells, actualCells.dropFirst()).allSatisfy { $0.0.frame.maxY <= $0.1.frame.minY + 0.5 }, "\(fixture.id): rows overlap or geometry disagrees with order")
                    var texts: [TranscriptTextView] = []
                    for cell in actualCells { texts.append(contentsOf: visibleTexts(in: cell)) }
                    for (id, marker) in fixture.notices {
                        let count = texts.filter { $0.string.contains(marker) }.count
                        let expectedCount = !expanded && fixture.groupedNoticeIDs.contains(id) ? 0 : 1
                        require(count == expectedCount, "\(fixture.id): notification \(id) must be visible exactly \(expectedCount) times, got \(count)")
                        if expectedCount == 1 {
                            let cell = actualCells.first { $0.rowID == presentationID(id) }!
                            let text = texts.first { $0.string.contains(marker) }!
                            let frame = cell.convert(text.bounds, from: text)
                            require(frame.minX >= -0.5 && frame.maxX <= cell.bounds.width + 0.5 && frame.minY >= -0.5 && frame.maxY <= cell.bounds.height + 0.5,
                                    "\(fixture.id): notification text exceeds its measured native row")
                        }
                    }
                    if fixture.id == "mixed-closing" {
                        let final = actualCells.first { $0.rowID == presentationID("reply:closing") }!
                        require(visibleTexts(in: final).contains { $0.string.contains("FINAL_MIXED") }, "mixed closing must retain an independent visible reply")
                        var workText: [TranscriptTextView] = []
                        for cell in actualCells where cell.rowID == presentationID("closing") { workText.append(contentsOf: visibleTexts(in: cell)) }
                        require(!workText.contains { $0.string.contains("FINAL_MIXED") }, "mixed closing reply must not duplicate in work")
                    }
                    measurements.append(["case": fixture.id, "dark": dark, "width": width, "expanded": expanded, "rowIDs": actualCells.map { $0.rowID! },
                        "rows": actualCells.map { ["id": $0.rowID!, "frame": NSStringFromRect($0.frame), "texts": visibleTexts(in: $0).map { $0.string }] as [String: Any] }])
                    if capture {
                        let height = max(220, min(1400, ceil(document.frame.height + 12)))
                        window.setContentSize(NSSize(width: width, height: height)); scroll.layoutSubtreeIfNeeded(); settle()
                        controller.materializeVisibleRows(extra: document.bounds)
                        scroll.contentView.scroll(to: .zero); scroll.reflectScrolledClipView(scroll.contentView)
                        scroll.displayIfNeeded(); safe()
                        guard let bitmap = scroll.bitmapImageRepForCachingDisplay(in: scroll.bounds) else { fail("cannot capture native transcript") }
                        scroll.cacheDisplay(in: scroll.bounds, to: bitmap)
                        guard let png = bitmap.representation(using: .png, properties: [:]) else { fail("cannot encode PNG") }
                        let filename = "background-notifications-\(fixture.id)-\(dark ? "dark" : "light")-\(Int(width))-\(expanded ? "expanded" : "collapsed").png"
                        do { try png.write(to: directory.appendingPathComponent(filename)) } catch { fail(error.localizedDescription) }
                        captures.append(filename)
                    }
                }
                func clickWork() {
                    guard let header = cells().first(where: { $0.rowID == presentationID("work:turn") }),
                          let button = descendants(header).compactMap({ $0 as? NSButton }).first(where: { !$0.isHidden && !$0.attributedTitle.string.isEmpty }) else { fail("native work disclosure missing") }
                    button.performClick(nil); clicks += 1
                    scroll.layoutSubtreeIfNeeded(); settle(); controller.materializeVisibleRows(extra: document.bounds)
                }
                let capture = (!dark && width == 420) || (dark && width == 900)
                let running = fixture.turns.first?.isRunning == true
                inspect(expanded: running || fixture.turns.isEmpty, capture: capture)
                if !fixture.turns.isEmpty {
                    clickWork(); inspect(expanded: !running, capture: capture)
                    clickWork(); inspect(expanded: running, capture: false)
                }
                controller.viewRemoved(); window.contentView = nil; safe()
            }
        }
    }
    let evidence: [String: Any] = ["synthetic": true, "networkCalls": 0, "modelCalls": 0, "visibleWindows": 0,
        "activationPolicy": NSApp.activationPolicy().rawValue, "activationEvents": activations, "keyEvents": keys, "mainEvents": mains,
        "systemActivationEvents": systemActivations, "nativeWorkClicks": clicks, "captures": captures, "measurements": measurements]
    do { try JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys]).write(to: directory.appendingPathComponent("background-notifications-check.json")) } catch { fail(error.localizedDescription) }
    print("PASS background notifications: 6 contract cases, native row/content/geometry/uniqueness, \(clicks) work clicks, \(captures.count) offscreen PNGs, no activation or network")
    exit(0)
}
