import AppKit
import SwiftUI
import VeluneBindings

/// Synthetic, hidden checks: no scenes, hotkeys, orderFront, activation or input injection.
@MainActor func messageExperienceChecks() -> Never {
    NSApplication.shared.setActivationPolicy(.prohibited)
    guard let path = ProcessInfo.processInfo.environment["VELUNE_EXPERIENCE_CHECK_HOME"], path.hasPrefix("/tmp/") else { fatalError("需要 /tmp 下的合成检查目录") }
    let home = URL(fileURLWithPath: path, isDirectory: true)
    try! FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
    func note(_ text: String) { FileHandle.standardError.write(Data((text + "\n").utf8)) }
    func wait(_ ready: () -> Bool) {
        let end = Date().addingTimeInterval(15)
        while !ready(), Date() < end { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
        precondition(ready(), "合成检查超时")
    }
    AppearancePreferences.shared.load(home: home) { fatalError($0) }
    func stamp(_ date: Date) -> Int64 { Int64(date.timeIntervalSince1970 * 1000) }
    if CommandLine.arguments.contains("--experience-typography-only") {
        let model = ComposerModel(followsConversation: false)
        let editor = ComposerTextView()
        let coordinator = ComposerEditor.Coordinator(model: model)
        coordinator.view = editor; editor.delegate = coordinator
        model.restore("保留当前输入", attachments: [])
        editor.replace(model.submission)
        let undo = coordinator.undoManager(for: editor)!
        undo.beginUndoGrouping()
        editor.insertText("及撤销记录", replacementRange: NSRange(location: (editor.string as NSString).length, length: 0))
        undo.endUndoGrouping()
        let before = editor.string
        editor.setSelectedRange(NSRange(location: 2, length: 3))
        let size = editor.font!.pointSize
        AppearancePreferences.shared.values.messages = .extraLarge
        precondition(editor.string == before && editor.selectedRange() == NSRange(location: 2, length: 3))
        precondition(editor.font!.pointSize > size && undo.canUndo)
        undo.undo()
        precondition(editor.string == "保留当前输入" && model.submission == editor.string)
        precondition(!NSApp.isActive)
        note("PASS live typography: native composer text, selection and undo survive message font changes without recreating the editor")
        exit(0)
    }
    if !CommandLine.arguments.contains("--experience-visual-only") {
    let transport = Transport(stateDirectory: home.appendingPathComponent("application"), resourcesDirectory: Bundle.main.resourceURL!)
    let device = DeviceStore(transport: transport)
    let savedFile = home.appendingPathComponent("expected-window.json")
    if CommandLine.arguments.contains("--experience-reopen") {
        let expected = try! JSONDecoder().decode(SavedWindowLayout.self, from: Data(contentsOf: savedFile))
        let restored = ConversationWindowModel(application: device, kind: .workspace)
        restored.start(restoring: nil, seed: nil, persistenceID: UUID().uuidString)
        precondition(restored.savedLayout.layout == expected.layout && restored.savedLayout.panes == expected.panes)
        precondition(restored.savedLayout.creationTargets == expected.creationTargets)
        precondition(AppearancePreferences.shared.values.ui == .large && AppearancePreferences.shared.values.code == .extraLarge)
        note("PASS new process: editor groups, ordered tabs, selections, lazy creation targets and appearance restored")
        exit(0)
    }

    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(secondsFromGMT: 0)!
    let now = calendar.date(from: DateComponents(year: 2026, month: 11, day: 4, hour: 12))!
    precondition(MessageMetadata.time(stamp(now), now: now, calendar: calendar) == "12:00")
    precondition(MessageMetadata.time(stamp(calendar.date(byAdding: .day, value: -1, to: now)!), now: now, calendar: calendar) == "11-03 12:00")
    precondition(MessageMetadata.time(stamp(calendar.date(byAdding: .year, value: -1, to: now)!), now: now, calendar: calendar) == "2025-11-04 12:00")
    precondition(MessageMetadata.time(nil) == nil)
    note("PASS message action row: today, same year, previous year and missing native time")

    let conversation = Conversation(id: "synthetic", title: "Synthetic", updatedAtUnixMs: nil, runtimeID: "fixture", cwd: nil, canRename: false, canDelete: false)
    var snapshot = ConversationSnapshot(revision: 1, conversation: conversation, contextRuntimeID: "fixture", modelRecordKey: nil, runState: .running, messages: [], actions: ConversationActions(inputAccess: .editable, canSend: false, canCancel: true, canSwitch: false))
    let outgoing = OutgoingSubmissions()
    let first = outgoing.begin(.steer, text: "same input", attachments: [], snapshot: snapshot)
    precondition(outgoing.displayed.first?.status == "插入中")
    outgoing.accept(first, snapshot: snapshot)
    let second = outgoing.begin(.steer, text: "same input", attachments: [], snapshot: snapshot)
    outgoing.accept(second, snapshot: snapshot)
    snapshot.messages = [DeliveredMessage(id: "native-a", role: .user, blocks: [.text(text: "same input")])]
    snapshot.transcriptMessageIdentities = [.init(id: "stable-a", messageId: "native-a")]
    outgoing.reconcile(snapshot); outgoing.reconcile(snapshot)
    precondition(outgoing.displayed.count == 1, "同一个回显不能确认两个提交")
    snapshot.messages[0].id = "confirmed-native-a"
    snapshot.transcriptMessageIdentities[0].messageId = "confirmed-native-a"
    outgoing.reconcile(snapshot)
    precondition(outgoing.displayed.count == 1, "原生历史身份确认不能冒充新回显")
    snapshot.messages.append(DeliveredMessage(id: "native-b", role: .user, blocks: [.text(text: "same input")]))
    snapshot.transcriptMessageIdentities.append(.init(id: "stable-b", messageId: "native-b"))
    outgoing.reconcile(snapshot); precondition(outgoing.displayed.isEmpty)
    let queued = outgoing.begin(.queue, text: "queued", attachments: [], snapshot: snapshot)
    precondition(outgoing.displayed.first?.status == "排队中")
    snapshot.followUp.queue = [.init(id: "canonical-queue", text: "queued", attachments: [])]
    outgoing.accept(queued, snapshot: snapshot); precondition(outgoing.displayed.isEmpty)
    let sending = outgoing.begin(.send, text: "failed send", attachments: [], snapshot: snapshot)
    precondition(outgoing.displayed.first?.status == "发送中")
    outgoing.fail(sending); precondition(outgoing.displayed.isEmpty)
    note("PASS unified local submissions: immediate send/queue/steer; canonical queue; repeated and renamed echoes do not consume another submission; failure removes only its placeholder")

    _ = try! transport.upsertRuntime(RuntimeInstance(enabled: true, id: "fixture", name: "Synthetic ACP", typeId: .acp, gatewayId: "default", settings: ["binary": "/usr/bin/true"]))
    device.start()
    wait { device.enabledRuntimeInstances.contains { $0.id == "fixture" } }
    let window = ConversationWindowModel(application: device, kind: .workspace)
    let identity = UUID().uuidString
    window.start(restoring: nil, seed: nil, persistenceID: identity)
    wait { window.focused.store.isUnsentConversation }
    window.focused.store.setUnsentWorkingDirectory("/tmp/alpha")
    window.openInNewTab(); window.focused.store.setUnsentWorkingDirectory("/tmp/beta")
    window.split(.horizontal); window.focused.store.setUnsentWorkingDirectory("/tmp/gamma")
    window.openInNewTab(); window.focused.store.setUnsentWorkingDirectory("/tmp/delta")
    window.tools.move(.composer, to: .docked(.bottom)); window.tools.collapse(.bottom)
    wait { device.windowWorkspaceJournal.restore(id: identity, recoverLatest: false)?.panes.count == 2 }
    let expected = device.windowWorkspaceJournal.restore(id: identity, recoverLatest: false)!
    precondition(expected.panes.values.reduce(0, { $0 + $1.tabs.count }) == 4)
    try! JSONEncoder().encode(expected).write(to: savedFile, options: .atomic)
    let single = ConversationWindowModel(application: device, kind: .single)
    single.start(restoring: nil, seed: nil, persistenceID: UUID().uuidString)
    precondition(single.tools[.composer].place == .builtIn && single.tools[.composer].shown)
    single.persistWorkspace(); device.windowWorkspaceJournal.flush()
    let fallback = device.windowWorkspaceJournal.restore(id: UUID().uuidString, recoverLatest: true)!
    precondition(fallback.layout == expected.layout && fallback.panes == expected.panes)
    note("PASS actual window lifecycle: structural edits persist without a text edit; two groups/four tabs; independent composer default; latest single window cannot replace workspace recovery")
    }

    let preview = DeviceStore(preview: true)
    let previewWindow = ConversationWindowModel(application: preview, kind: .workspace)
    previewWindow.tools.move(.composer, to: .docked(.bottom))
    let model = previewWindow.composer(for: previewWindow.focused)
    model.restore("请继续核对这份方案，并保留我正在编辑的内容。", attachments: [])
    renderExperienceSamples(home: home, window: previewWindow, model: model, now: stamp(Date()))
    AppearancePreferences.shared.values.ui = .large
    AppearancePreferences.shared.values.code = .extraLarge
    precondition(!NSApp.isActive)
    note("PASS hidden native rendering: message actions, client notification, wide/narrow composer and appearance form; process never activated")
    exit(0)
}

@MainActor private func renderExperienceSamples(home: URL, window: ConversationWindowModel, model: ComposerModel, now: Int64) {
    func capture(_ view: NSView, name: String, size: NSSize) {
        view.frame = NSRect(origin: .zero, size: size)
        let host = NSWindow(contentRect: view.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        host.contentView = view
        view.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.15))
        view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
        guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { fatalError("无法绘制合成界面") }
        view.cacheDisplay(in: view.bounds, to: bitmap)
        try! bitmap.representation(using: .png, properties: [:])!.write(to: home.appendingPathComponent(name + ".png"))
        precondition(!host.isVisible && !host.isKeyWindow)
        host.contentView = nil
    }
    let rows = MessageSampleCanvas(frame: NSRect(x: 0, y: 0, width: 1000, height: 780))
    rows.wantsLayer = true; rows.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
    let delegate = TranscriptTableController()
    let messages: [DeliveredMessage] = [
        .init(id: "user", role: .user, timestampUnixMs: now, blocks: [.text(text: "这份修改需要保留现有行为，并验证关键路径。")]),
        .init(id: "assistant", role: .assistant, timestampUnixMs: now, blocks: [.text(text: "已保留输入与执行选择。下一步只检查本轮改变的路径。\n\n```swift\nlet input = workspace.currentInput\n```")]),
        .init(id: "notification", role: .client, timestampUnixMs: now, blocks: [.text(text: "后台任务完成：工作区检查已通过。")]),
    ]
    var y: CGFloat = 0
    func add(_ content: TranscriptRowContent, rendered: RenderedMessage?, id: String) {
        let layout = TranscriptRowLayout.make(content, rendered: rendered, width: 1000, measurer: TextMeasurer())
        let cell = TranscriptCellView()
        cell.frame = NSRect(x: 0, y: y, width: 1000, height: layout.height)
        cell.apply(layout, rowID: id, outlineToggle: nil, edit: nil, fork: nil, toggleHelp: nil, textDelegate: delegate, openArtifact: { _ in }, toggle: { _ in })
        for button in cell.subviews.compactMap({ $0 as? NSButton }) { button.alphaValue = 1 }
        rows.addSubview(cell); y += layout.height
    }
    for message in messages {
        if message.role == .assistant {
            add(.work(label: "工作过程", expanded: false, settings: nil), rendered: nil, id: "work")
            add(.messageActions(text: "核对运行时目录与提交状态。", timestamp: now, trailing: false), rendered: nil, id: "work-actions")
        }
        add(.message(message, revision: 0, expanded: [], status: nil, copies: true, edits: message.role == .user, forks: message.role == .assistant), rendered: RenderedMessage(message), id: message.id)
    }
    capture(rows, name: "message-actions", size: NSSize(width: 1000, height: max(y, 500)))
    for width: CGFloat in [1100, 560] {
        let composer = ComposerView(store: window.focused.store, model: model, layout: window, send: {}, steer: {}, submission: { window.focused.submission(using: model) }, resume: {}, fillsHeight: true, recipient: "架构评审会话")
        capture(NSHostingView(rootView: composer.applicationAppearance().background(Color(nsColor: .windowBackgroundColor))), name: "composer-\(Int(width))", size: NSSize(width: width, height: 230))
    }
    capture(NSHostingView(rootView: Form { AppearanceSettingsSection() }.formStyle(.grouped).applicationAppearance()), name: "appearance", size: NSSize(width: 690, height: 480))
}

@MainActor private final class MessageSampleCanvas: NSView {
    override var isFlipped: Bool { true }
}
