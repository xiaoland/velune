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
    if CommandLine.arguments.contains("--experience-drag-provider-only") { dragProviderChecks() }
    AppearancePreferences.shared.load(home: home) { fatalError($0) }
    if CommandLine.arguments.contains("--experience-runtime-editor-only") { runtimeEditorChecks(home: home) }
    func stamp(_ date: Date) -> Int64 { Int64(date.timeIntervalSince1970 * 1000) }
    if CommandLine.arguments.contains("--experience-navigation-only") {
        ConversationStore.checkOpeningCatalogAdmission()
        navigationTranscriptChecks(home: home)
    }
    if CommandLine.arguments.contains("--experience-typography-only") {
        let model = ComposerModel()
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
        for label in cell.subviews where label.accessibilityIdentifier() == "message-created-at" { label.alphaValue = 1 }
        rows.addSubview(cell); y += layout.height
    }
    for message in messages {
        if message.role == .assistant {
            add(.work(label: "工作过程", expanded: false, settings: nil), rendered: nil, id: "work")
            add(.workActions(text: "核对运行时目录与提交状态。"), rendered: nil, id: "work-actions")
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

/// Targeted navigation and transcript checks, using only synthetic state and
/// an unordered native window. Events below go directly to one cell, never
/// through the application event queue or the system pointer.
@MainActor private func navigationTranscriptChecks(home: URL) -> Never {
    func note(_ text: String) { FileHandle.standardError.write(Data((text + "\n").utf8)) }
    func descendants(_ root: NSView) -> [NSView] { [root] + root.subviews.flatMap(descendants) }
    workspaceContinuityChecks(home: home)
    let device = DeviceStore(preview: true)
    let window = ConversationWindowModel(application: device, kind: .workspace)
    let original = window.focused
    original.store.selectConversation(id: "sample-0")
    let draft = window.composer(for: original)
    draft.restore("PRESERVED SYNTHETIC DRAFT", attachments: [])
    original.store.noteFocused()
    let count = window.tabCount
    precondition(ConversationWindowModel.openNotificationConversation("sample-1", application: device) === window)
    let notified = window.focused
    precondition(window.tabCount == count + 1 && notified !== original)
    precondition(original.store.loadedConversationID == "sample-0" && draft.submission == "PRESERVED SYNTHETIC DRAFT")
    window.selectTab(original.id)
    precondition(ConversationWindowModel.openNotificationConversation("sample-1", application: device) === window)
    precondition(window.focused === notified && window.tabCount == count + 1)
    let second = ConversationWindowModel(application: device, kind: .workspace)
    second.focused.store.selectConversation(id: "sample-2")
    window.selectTab(original.id)
    precondition(ConversationWindowModel.openNotificationConversation("sample-2", application: device) === second)
    precondition(second.tabCount == 1 && window.focused === original)
    original.outlineCollapsed = true
    original.beginFind()
    precondition(!original.outlineCollapsed)
    note("PASS notification navigation: new tab preserves original conversation/draft; repeated and other-window targets reuse their tab; find reveals full transcript")

    let now: Int64 = 1_793_793_600_000
    let messages: [DeliveredMessage] = [
        .init(id: "user", role: .user, timestampUnixMs: now, blocks: [.text(text: "合成输入")]),
        .init(id: "work", role: .assistant, timestampUnixMs: now, blocks: [
            .text(text: "记录工作步骤"),
            .reasoning(summary: "隐藏推理", text: .inline(text: "A Café Needle inside work.")),
            .tool(toolId: "unrelated", title: "无关工具", state: .completed, output: .inline(text: "UNRELATED HIDDEN DETAIL"), linkedConversationId: nil)
        ]),
        .init(id: "deferred", role: .assistant, timestampUnixMs: now, blocks: [
            .tool(toolId: "deferred-tool", title: "延迟工具", state: .completed, output: .deferred(bytes: 4096), linkedConversationId: nil)
        ]),
        .init(id: "answer", role: .assistant, timestampUnixMs: now, blocks: [.text(text: "结果 Needle\n\n```swift\nlet needle = true\n```")])
    ]
    let turn = TranscriptTurn(id: "turn", userMessageId: "user", workMessageIds: ["work", "deferred"], lastMessageId: "answer", replyStartBlock: 0, durationMs: 1200, isRunning: false, canFork: true, outcome: .completed, settings: nil, usage: nil)
    let model = TranscriptModel()
    model.apply(messages, turns: [turn], items: [.message(id: "user", messageId: "user"), .work(turnId: "turn"), .message(id: "answer", messageId: "answer")], outline: [.init(id: "user", messageId: "user")], identities: messages.map { .init(id: $0.id, messageId: $0.id) }, editable: ["user"])
    let find = TranscriptFind()
    let controller = TranscriptTableController()
    controller.observeFind(find)
    let scroll = controller.view()
    let host = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 960, height: 1000), styleMask: [.borderless], backing: .buffered, defer: false)
    host.contentView = scroll
    var loaded: Set<String> = []
    func input(jump: TranscriptJump? = nil) -> TranscriptTable {
        TranscriptTable(controller: controller, conversationID: "synthetic", items: model.items(for: model.rows, turnHeaders: true), revision: model.contentRevision, activity: nil, outlineCollapsed: false, outlineAnchorID: nil, cwd: nil, bottomRequest: 0, jump: jump, followingChanged: { _ in }, outlineToggle: { _ in }, loadPayloads: { loaded.formUnion($0) }, openConversation: { _ in }, openAgentURI: { _ in }, openArtifact: { _ in }, editMessage: { _ in {} }, forkConversation: { _ in }, describeSettings: { _ in nil })
    }
    func cell(_ id: String) -> TranscriptCellView? { scroll.documentView?.subviews.compactMap { $0 as? TranscriptCellView }.first { $0.rowID == id } }
    controller.update(input())
    precondition(cell("work") == nil, "完成的工作默认保持折叠")
    find.isShown = true
    find.query = "needle"
    precondition(find.matches(in: model) == ["work", "answer"])
    precondition(loaded == ["deferred"], "仅搜索期间读取延迟文本")
    controller.update(input(jump: .init(id: "work", token: 1)))
    guard let work = cell("work"), let answer = cell("answer"), let user = cell("user") else { fatalError("命中工作行未展开或未物化") }
    let workText = descendants(work).compactMap { $0 as? TranscriptTextView }.filter { !$0.isHidden }
    precondition(workText.contains { $0.string == "A Café Needle inside work." })
    precondition(!workText.contains { $0.string == "UNRELATED HIDDEN DETAIL" }, "不展开未命中的工具详情")
    let highlighted = workText.first { $0.string.contains("Needle") }!
    let range = TranscriptSearch.ranges(in: highlighted.string, query: "needle").first!
    precondition(highlighted.layoutManager?.temporaryAttribute(.backgroundColor, atCharacterIndex: range.location, effectiveRange: nil) != nil)
    let code = descendants(answer).compactMap { $0 as? TranscriptTextView }.first { $0.string.contains("let needle") }!
    precondition(code.layoutManager?.temporaryAttribute(.backgroundColor, atCharacterIndex: 4, effectiveRange: nil) != nil)
    precondition(TranscriptSearch.ranges(in: "Café CAFE", query: "cafe").count == 2)
    find.isShown = false
    precondition(highlighted.layoutManager?.temporaryAttribute(.backgroundColor, atCharacterIndex: range.location, effectiveRange: nil) == nil)
    find.isShown = true
    note("PASS transcript search: folded reasoning and reply results; selected work/disclosure opens; unrelated detail stays folded; deferred text loads; prose/code highlights clear with find")

    func timeLabel(_ cell: TranscriptCellView) -> TranscriptTextView? { cell.subviews.compactMap { $0 as? TranscriptTextView }.first { $0.accessibilityIdentifier() == "message-created-at" } }
    let footer = cell("work-actions:turn")!
    precondition(timeLabel(footer) == nil && timeLabel(work) == nil)
    for row in [user, answer] {
        let label = timeLabel(row)!
        let buttons = row.subviews.compactMap { $0 as? NSButton }.filter { !$0.isHidden }
        precondition(!buttons.isEmpty && label.alphaValue == 0 && buttons.allSatisfy { $0.alphaValue == 0 })
        let entered = NSEvent.enterExitEvent(with: .mouseEntered, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, eventNumber: 0, trackingNumber: 0, userData: nil)!
        row.mouseEntered(with: entered)
        RunLoop.main.run(until: Date().addingTimeInterval(0.18))
        precondition(label.alphaValue == 1 && buttons.allSatisfy { $0.alphaValue == 1 })
        let exited = NSEvent.enterExitEvent(with: .mouseExited, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, eventNumber: 0, trackingNumber: 0, userData: nil)!
        row.mouseExited(with: exited)
        RunLoop.main.run(until: Date().addingTimeInterval(0.18))
        precondition(label.alphaValue == 0 && buttons.allSatisfy { $0.alphaValue == 0 })
    }
    let creationTimes = descendants(scroll).compactMap { $0 as? TranscriptTextView }.filter { $0.accessibilityIdentifier() == "message-created-at" && !$0.isHidden }
    precondition(creationTimes.count == 2, "仅真实用户消息和最终回复拥有创建时间")
    answer.mouseEntered(with: NSEvent.enterExitEvent(with: .mouseEntered, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, eventNumber: 0, trackingNumber: 0, userData: nil)!)
    RunLoop.main.run(until: Date().addingTimeInterval(0.18))
    scroll.layoutSubtreeIfNeeded(); scroll.displayIfNeeded()
    if let bitmap = scroll.bitmapImageRepForCachingDisplay(in: scroll.bounds) {
        scroll.cacheDisplay(in: scroll.bounds, to: bitmap)
        try! bitmap.representation(using: .png, properties: [:])!.write(to: home.appendingPathComponent("transcript-search.png"))
    }
    precondition(!host.isVisible && !host.isKeyWindow && !NSApp.isActive)
    note("PASS message creation time: only real messages; copy/edit/fork and time share hover visibility; no work-footer or assistant-top time; hidden native window never activated")
    exit(0)
}
