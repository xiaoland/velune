import AppKit
import SwiftUI
import VeluneBindings

/// Exercise native historical continuation through Bindings and Claude Code.
/// The runner supplies a private archive and a loopback model.
@MainActor private func claudeContinuationExperienceChecks(home: URL, editing: Bool) -> Never {
    let action = editing ? "compact-edit" : "fork-send"
    func fail(_ message: String) -> Never {
        FileHandle.standardError.write(Data("FAIL Claude \(action): \(message)\n".utf8))
        exit(1)
    }
    let foreground = NSWorkspace.shared.frontmostApplication?.processIdentifier
    func settle() {
        RunLoop.main.run(until: Date().addingTimeInterval(0.025))
        guard NSApp.activationPolicy() == .prohibited, !NSApp.isActive,
              NSWorkspace.shared.frontmostApplication?.processIdentifier == foreground,
              NSApp.windows.allSatisfy({ !$0.isVisible && !$0.isKeyWindow && !$0.isMainWindow }) else {
            fail("离屏操作触及桌面焦点")
        }
    }
    func until(_ ready: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(12)
        while !ready(), Date() < deadline { settle() }
        return ready()
    }
    do {
        let privateRoot = home.deletingLastPathComponent().standardizedFileURL.resolvingSymlinksInPath()
        let context = MacExecutionContext.admitted
        let stateDirectory = context.stateDirectory.resolvingSymlinksInPath()
        guard context.isDevelopment, stateDirectory.path == privateRoot.appendingPathComponent("state", isDirectory: true).path,
              let osHome = context.harnessUserHome?.resolvingSymlinksInPath().path,
              osHome == stateDirectory.appendingPathComponent("harness-user", isDirectory: true).path else {
            fail("需要独立合成状态目录和 HOME")
        }
        let data = try Data(contentsOf: home.appendingPathComponent("fixture.json"))
        guard let fixture = try JSONSerialization.jsonObject(with: data) as? [String: String],
              let binary = fixture["binary"], let directory = fixture["agentDirectory"],
              let sourceID = fixture["sourceID"], let userID = fixture["userID"],
              let endpoint = fixture["endpoint"], endpoint.hasPrefix("http://127.0.0.1:"),
              URL(fileURLWithPath: directory, isDirectory: true).resolvingSymlinksInPath().path == privateRoot.appendingPathComponent("claude", isDirectory: true).path else {
            fail("合成会话配置缺失")
        }
        let transport = Transport(stateDirectory: stateDirectory, resourcesDirectory: Bundle.main.resourceURL!)
        let runtimeID = "claude-check"
        let instance = RuntimeInstance(enabled: true, id: runtimeID, name: "Claude Code", typeId: .claudeCode,
            gatewayId: "default", settings: ["binary": binary, "agentDir": directory])
        _ = try transport.upsertRuntime(instance, execution: RuntimeExecution(transport: .local,
            home: .existing(path: osHome), remoteResourcesDirectory: nil))
        let gateway = try transport.saveProvider(gatewayID: "default", provider: ProviderDraft(
            id: "fork-loopback", name: "Synthetic loopback", protocol: .messages, endpoint: endpoint,
            executionBackend: .http, requestProfile: .native, authenticationSource: .apiKey, quotaSource: .none,
            models: [ProviderModel(recordKey: "", providerModelId: "claude-sonnet-4-6", nickname: "Synthetic model", contextWindow: 8192, maxOutputTokens: 4096)],
            quotaLimits: [], codexClientVersion: nil), authenticationEdit: .setApiKey(value: "synthetic"))
        guard let modelKey = gateway.gateways.flatMap(\.providers).first(where: { $0.id == "fork-loopback" })?.models.first?.recordKey else {
            fail("回环模型未保存")
        }
        let device = DeviceStore(transport: transport)
        device.start()
        let workspace = ConversationWindowModel(application: device, kind: .workspace)
        let source = workspace.focused
        source.store.start(loadFirstConversation: false, opening: sourceID)
        let choice = ModelSelection.managed(modelRecordKey: modelKey)
        if editing {
            func textVisible(_ text: String) -> Bool {
                source.store.transcript.rows.contains { $0.message.blocks.contains(.text(text: text)) }
            }
            guard until({ source.store.loadedConversationID == sourceID && source.store.editability(of: userID)?.inPlace == true }),
                  textVisible("old edit prompt"), textVisible("later prompt after compact") else {
                fail("压缩前的旧消息或压缩后的历史未加载")
            }
            source.edit(messageID: userID)
            guard let edit = source.editing, edit.placement == .inPlace else { fail("未打开原地替换编辑器") }
            edit.target.chooseModel(choice)
            let host = NSHostingView(rootView: MessageEditorView(session: edit, window: workspace, source: source)
                .background(Color(nsColor: .windowBackgroundColor)).applicationAppearance())
            let window = OffscreenMessageEditorWindow(contentRect: NSRect(x: 0, y: 0, width: 560, height: 300),
                styleMask: [.borderless], backing: .buffered, defer: true)
            defer { withExtendedLifetime(window) {} }
            window.isReleasedWhenClosed = false
            window.isRestorable = false
            window.contentView = host
            host.layoutSubtreeIfNeeded()
            guard until({ edit.editor.textView?.isEditable == true }), let editor = edit.editor.textView,
                  editor.string == "old edit prompt" else { fail("原生编辑器未载入原消息") }
            let replacement = "edited historical message"
            editor.insertText(replacement, replacementRange: NSRange(location: 0, length: (editor.string as NSString).length))
            guard edit.editor.submission == replacement else { fail("原生编辑器没有提交修改后的正文") }
            guard until({ edit.canSubmit }) else { fail(edit.noticeText ?? "编辑器禁止提交") }
            edit.send(in: workspace)
            guard until({ edit.failure != nil || (source.editing == nil && textVisible("edited reply received")) }),
                  edit.failure == nil else { fail(edit.failure ?? "编辑提交后未收到 Claude 回复") }
            guard source.store.loadedConversationID == sourceID, workspace.focused.id == source.id else {
                fail("原地提交切换了标签或会话 ID")
            }
            let missing = ["oldest fork prompt", "oldest fork reply", replacement].filter { !textVisible($0) }
            let retained = ["old edit prompt", "old edit reply", "retained later prompt", "later prompt after compact"].filter { textVisible($0) }
            guard missing.isEmpty, retained.isEmpty else { fail("替换后的历史不完整，缺少 \(missing)，仍保留 \(retained)") }
            settle()
            try transport.close(stoppingWork: true)
            print("PASS Claude compact-edit: original conversation retained, discarded history replaced, Claude reply received")
            exit(0)
        }
        guard until({ source.store.loadedConversationID == sourceID && source.store.canFork && source.store.message(id: userID) != nil }) else {
            fail("旧轮次未提供分叉入口")
        }
        let originalText = source.store.transcript.rows.map(\.id)
        source.store.forkConversation(through: userID) { workspace.openInNewTab(adopting: $0, device: device) }
        guard until({ workspace.focused.id != source.id && workspace.focused.store.nextTurn != nil }),
              let targetID = workspace.focused.store.loadedConversationID, targetID != sourceID else {
            fail("分叉未打开独立标签页")
        }
        let tab = workspace.focused
        let store = tab.store
        let input = workspace.composer(for: tab)
        workspace.composerContext(for: tab).selectModel(choice)
        guard until({ workspace.composerContext(for: tab).chosenModel == choice && store.draftEditable }) else {
            fail("分叉输入不可编辑 \(store.inputAccess)")
        }
        let composer = ComposerView(store: store, model: input, layout: workspace,
            send: { workspace.composerContext(for: tab).send() }, steer: {},
            submission: { workspace.composerContext(for: tab).submission() }, resume: {}, inputTab: tab)
        let bounds = NSRect(x: 0, y: 0, width: 620, height: 180)
        let host = NSHostingView(rootView: composer.background(Color(nsColor: .windowBackgroundColor)).applicationAppearance())
        host.appearance = NSAppearance(named: .aqua)
        let window = OffscreenMessageEditorWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false
        window.isRestorable = false
        window.contentView = host
        host.layoutSubtreeIfNeeded()
        guard until({ input.textView != nil }), let editor = input.textView, editor.isEditable else { fail("原生 Composer 输入框只读") }
        let prompt = "first message after fork"
        editor.insertText(prompt, replacementRange: NSRange(location: 0, length: 0))
        guard input.submission == prompt,
              until({ workspace.composerContext(for: tab).submission().canSubmitDraft }) else {
            fail("Composer 禁止发送 \(store.inputAccess) \(workspace.composerContext(for: tab).inputIssue ?? "")")
        }
        host.layoutSubtreeIfNeeded()
        if let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) {
            host.cacheDisplay(in: host.bounds, to: bitmap)
            try bitmap.representation(using: .png, properties: [:])?.write(to: home.appendingPathComponent("fork-ready.png"))
        }
        let sourceBefore = source.store.loadedConversationID
        workspace.composerContext(for: tab).send()
        guard until({ !store.inputSubmissionAllowed || store.transcript.rows.contains { row in
            row.message.blocks.contains(.text(text: "fork reply received"))
        } }), store.inputSubmissionAllowed else { fail("首次发送后未收到回复 \(store.inputAccess) \(store.device.problems.first?.detail ?? "")") }
        guard store.loadedConversationID == targetID, source.store.loadedConversationID == sourceBefore,
              source.store.transcript.rows.map(\.id) == originalText else { fail("分叉发送改变了原标签页") }
        settle()
        try transport.close(stoppingWork: true)
        print("PASS Claude fork-send: original tab preserved, new native editor editable, first submission received a Claude reply")
        exit(0)
    } catch { fail(error.localizedDescription) }
}

/// Receive a fork on a non-default device and immediately change its next
/// turn's settings. No runtime or daily state directory is opened.
@MainActor private func forkExperienceChecks(home: URL) {
    ConversationStore.checkConcurrentForkDispatch(home: home)
    let local = DeviceStore(preview: true)
    let source = DeviceStore(preview: true)
    let window = ConversationWindowModel(application: local, kind: .workspace)
    let summary = ConversationSummary(id: "synthetic-fork", title: .native("Synthetic (1)"), updatedAtUnixMs: nil,
        createdAtUnixMs: nil, runtimeId: "sample-instance", cwd: nil, canRename: true, canDelete: true,
        parentConversationId: nil, nativeReadState: nil)
    let messages = [DeliveredMessage(id: "u", role: .user, blocks: [.text(text: "Synthetic request")]),
                    DeliveredMessage(id: "a", role: .assistant, blocks: [.text(text: "Synthetic reply")])]
    let identities = messages.map { TranscriptMessageIdentity(id: $0.id, messageId: $0.id) }
    let items = messages.map { TranscriptItem.message(id: $0.id, messageId: $0.id) }
    let result = SnapshotResult(snapshot: DeliveredSnapshot(cursor: .init(generation: 1, sequence: 1), base: nil,
        messageIds: messages.map(\.id), messages: messages, revision: 1, conversation: summary,
        contextRuntimeId: "sample-instance", resourceId: nil, modelRecordKey: nil, runState: .idle,
        transcriptTurns: [], usage: .init(), transcriptItems: items, transcriptOutline: [],
        transcriptMessageIdentities: identities, messageIdentityConfirmations: [], pendingInteractions: [],
        actions: .init(inputAccess: .editable, canSend: true, canCancel: false, canSwitch: true),
        followUp: .init(), subAgents: [], collaborators: []))
    window.openInNewTab(adopting: result, device: source)
    let store = window.focused.store
    precondition(store.device === source && store.device !== local, "fork 必须留在发起设备")
    precondition(store.loadedConversationID == "synthetic-fork" && !store.isLoading)
    precondition(store.nextTurn != nil, "接收快照后立即提供执行配置，不等待列表扫描")
    store.selectComposerRuntime("sample-review")
    precondition(store.nextTurnRuntimeID == "sample-review", "新分支可立即更改执行运行时")
    store.selectComposerRuntime("sample-instance")
    store.changeComposerChoice(.effort(level: "high"))
    precondition(store.nextTurn?.choice.effort == "high", "新分支可立即更改推理配置")
    let transcript = TranscriptModel()
    transcript.apply(messages, turns: [], items: items, outline: [], identities: identities, editable: [])
    let view = NSHostingView(rootView: TranscriptView(model: transcript, reading: TranscriptReading(),
        conversationID: "synthetic-fork", scrollRequest: 0, cwd: nil, outlineCollapsed: .constant(false), navigation: nil,
        activity: nil, loadPayloads: { _ in }, openConversation: { _ in }, openAgentURI: { _ in },
        describeSettings: { _ in nil }, forking: true))
    view.frame = NSRect(x: 0, y: 0, width: 720, height: 420)
    let host = NSWindow(contentRect: view.frame, styleMask: [.borderless], backing: .buffered, defer: false)
    host.contentView = view
    view.layoutSubtreeIfNeeded()
    RunLoop.main.run(until: Date().addingTimeInterval(0.1))
    guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { fatalError("无法绘制 fork 加载态") }
    view.cacheDisplay(in: view.bounds, to: bitmap)
    try! bitmap.representation(using: .png, properties: [:])!.write(to: home.appendingPathComponent("fork-loading.png"))
    store.close()
    FileHandle.standardError.write(Data("PASS fork: original device, adopted snapshot, immediate runtime/effort edits and native loading presentation; the sidebar entry comes from the core list alone\n".utf8))
}

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
    if CommandLine.arguments.contains("--experience-fork-send-only") { claudeContinuationExperienceChecks(home: home, editing: false) }
    if CommandLine.arguments.contains("--experience-claude-compact-edit-only") { claudeContinuationExperienceChecks(home: home, editing: true) }
    if CommandLine.arguments.contains("--experience-sidebar-only") {
        sidebarActionsChecks(home: home)
        exit(0)
    }
    if CommandLine.arguments.contains("--experience-compaction-only") {
        compactionTranscriptChecks(home: home)
        exit(0)
    }
    if CommandLine.arguments.contains("--experience-runtime-recovery-only") {
        runtimeRecoveryNativeChecks(home: home)
    }
    if CommandLine.arguments.contains("--experience-file-links-only") {
        let path = home.appendingPathComponent("document with space.md").path
        try! Data("synthetic".utf8).write(to: URL(fileURLWithPath: path))
        precondition(TranscriptFileLink.resolve(path + ":20:3", cwd: nil, remote: false)?.path == path)
        precondition(TranscriptFileLink.resolve(URL(fileURLWithPath: path).absoluteString + ":20", cwd: nil, remote: false)?.path == path)
        precondition(TranscriptFileLink.resolve("document%20with%20space.md:20", cwd: home.path, remote: false)?.path == path)
        precondition(TranscriptFileLink.resolve(path + ":20", cwd: nil, remote: true) == nil)
        precondition(TranscriptFileLink.resolve("https://example.com:8080/a:20", cwd: nil, remote: false)?.absoluteString == "https://example.com:8080/a:20")
        let literal = path + ":20"
        try! Data().write(to: URL(fileURLWithPath: literal))
        precondition(TranscriptFileLink.resolve(literal, cwd: nil, remote: false)?.path == literal)
        let segments = MarkdownAttributedRenderer.render("见 `/Users/example/document.md:20`。")
        guard case .prose(let rendered) = segments.first else { fatalError("missing prose") }
        let range = (rendered.string as NSString).range(of: "/Users/example/document.md:20")
        precondition(rendered.attribute(.link, at: range.location, effectiveRange: nil) as? String == "/Users/example/document.md:20")
        // Asterisks beside CJK punctuation pair as written; others follow CommonMark; code is untouched.
        func strong(_ source: String) -> (text: String, bold: [String]) {
            guard case .prose(let rendered) = MarkdownAttributedRenderer.render(source).first else { fatalError("missing prose") }
            var bold: [String] = []
            rendered.enumerateAttribute(.font, in: NSRange(location: 0, length: rendered.length)) { value, range, _ in
                if let font = value as? NSFont, font.fontDescriptor.symbolicTraits.contains(.bold) { bold.append((rendered.string as NSString).substring(with: range)) }
            }
            return (rendered.string, bold)
        }
        for (source, text, bold) in [
            ("**接手后从这里开始：**在主会话中继续。", "接手后从这里开始：在主会话中继续。", ["接手后从这里开始："]),
            ("先看**结论：**这样做，再看`代码`与**原因**。", "先看结论：这样做，再看代码与原因。", ["结论：", "原因"]),
            ("看**「重点」**是这个", "看「重点」是这个", ["「重点」"]),
            ("2 ** 3 ** 4 and a**b**c", "2 ** 3 ** 4 and abc", ["b"]),
            ("x **y **z", "x **y **z", []),
            ("`a：**b**` 与 ```c：**d**```", "a：**b** 与 c：**d**", []),
        ] {
            let result = strong(source)
            precondition(result.text == text && result.bold == bold, "strong \(source): \(result)")
        }
        note("PASS source locations: absolute, relative, encoded/file URLs, literal colon filenames, remote isolation, inline link rendering, CJK strong")
        exit(0)
    }
    if CommandLine.arguments.contains("--experience-fork-only") { forkExperienceChecks(home: home); exit(0) }
    if CommandLine.arguments.contains("--experience-drag-provider-only") { dragProviderChecks() }
    if CommandLine.arguments.contains("--experience-workspace-only") {
        ConversationStore.checkColdOpeningRetryLifecycle(home: home)
        workspaceRecoveryChecks(home: home)
        exit(0)
    }
    if CommandLine.arguments.contains("--experience-native-row-order-only") { TranscriptTableController.checkMaterializedRowOrder(); exit(0) }
    if CommandLine.arguments.contains("--experience-appearance-zoom-only") { appearanceZoomChecks(home: home) }
    AppearancePreferences.shared.load(home: home) { fatalError($0) }
    if CommandLine.arguments.contains("--experience-admission-only") { DeviceStore.checkBackgroundSubmissionAdmission(); exit(0) }
    if CommandLine.arguments.contains("--experience-message-editor-offscreen") { messageEditorOffscreenChecks(home: home) }
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
        // Transcript chrome (activity line, attachment name, code label and
        // controls) must read its fonts on every apply: views are reused
        // across appearance changes, so a font snapshotted at creation would
        // keep a stale size after switching back.
        let transcriptDelegate = TranscriptTableController()
        let chromeMessage = DeliveredMessage(id: "chrome", role: .assistant, timestampUnixMs: nil, blocks: [
            .text(text: "```swift\nlet size = 1\n```"),
            .file(path: "/tmp/velune-typography-合成.txt"),
        ])
        let activityCell = TranscriptCellView()
        let chromeCell = TranscriptCellView()
        func chromeFonts(activity: Bool) -> [CGFloat] {
            let content: TranscriptRowContent = activity
                ? .activity(.init(text: "正在思考", isLive: true))
                : .message(chromeMessage, revision: 0, expanded: [], copies: false, edits: false, forks: false)
            let layout = TranscriptRowLayout.make(content, rendered: activity ? nil : RenderedMessage(chromeMessage), width: 640, measurer: TextMeasurer())
            let cell = activity ? activityCell : chromeCell
            cell.frame = NSRect(x: 0, y: 0, width: 640, height: layout.height)
            cell.apply(layout, rowID: activity ? "activity" : "chrome", outlineToggle: nil, edit: nil, fork: nil, toggleHelp: nil, textDelegate: transcriptDelegate, openArtifact: { _ in }, toggle: { _ in })
            func descendants(_ root: NSView) -> [NSView] { [root] + root.subviews.flatMap(descendants) }
            // Chrome labels now draw attributed strings baked at measurement
            // time; the check reads the baked font, falling back to the field's.
            return descendants(cell).compactMap { $0 as? NSTextField }.map { field in
                let attributes = field.attributedStringValue.length > 0 ? field.attributedStringValue.attributes(at: 0, effectiveRange: nil) : [:]
                return (attributes[.font] as? NSFont ?? field.font)?.pointSize ?? 0
            }.sorted()
        }
        // The composer leg above leaves the message size at .extraLarge.
        AppearancePreferences.shared.values.messages = .standard
        AppearancePreferences.shared.values.ui = .standard
        let chromeBase = chromeFonts(activity: true) + chromeFonts(activity: false)
        AppearancePreferences.shared.values.messages = .extraLarge
        AppearancePreferences.shared.values.ui = .extraLarge
        let chromeLarge = chromeFonts(activity: true) + chromeFonts(activity: false)
        AppearancePreferences.shared.values.messages = .standard
        AppearancePreferences.shared.values.ui = .standard
        let chromeRestored = chromeFonts(activity: true) + chromeFonts(activity: false)
        precondition(chromeBase == chromeRestored && chromeBase.count == chromeLarge.count && !chromeBase.isEmpty, "转录静态标签字体必须随外观恢复默认")
        precondition(zip(chromeBase, chromeLarge).allSatisfy { $1 > $0 }, "转录静态标签字体必须随超大增大")
        note("PASS live typography: native composer text, selection and undo survive message font changes without recreating the editor")
        note("PASS transcript chrome typography: activity, attachment, code label and control fonts follow appearance changes on reused cells")
        exit(0)
    }
    if !CommandLine.arguments.contains("--experience-visual-only") {
    ConversationStore.checkColdOpeningRetryLifecycle(home: home)
    let transport = Transport(stateDirectory: home.appendingPathComponent("application"), resourcesDirectory: Bundle.main.resourceURL!)
    try! transport.prepareForLaunch()
    MacNativePersistenceGate.open()
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
    let closedIdentity = UUID().uuidString
    let closing = ConversationWindowModel(application: device, kind: .single)
    precondition(!closing.acceptsWorkspacePersistence)
    closing.start(restoring: nil, seed: nil, persistenceID: closedIdentity)
    closing.persistWorkspace(); device.windowWorkspaceJournal.flush()
    let lastOpenLayout = device.windowWorkspaceJournal.restore(id: closedIdentity, recoverLatest: false, kind: .single)!
    closing.noteWindowClosed()
    precondition(!closing.acceptsWorkspacePersistence)
    closing.tools.move(.outline, to: .docked(.right))
    precondition(closing.savedLayout != lastOpenLayout)
    closing.persistWorkspace(); device.windowWorkspaceJournal.flush()
    precondition(device.windowWorkspaceJournal.restore(id: closedIdentity, recoverLatest: false, kind: .single) == lastOpenLayout,
                 "关闭后迟到的布局／草稿发布不得改写最后恢复点")
    closing.start(restoring: nil, seed: nil, persistenceID: closedIdentity)
    precondition(closing.acceptsWorkspacePersistence, "原生 scene 合法重用时可重新保存")
    closing.persistWorkspace(); device.windowWorkspaceJournal.flush()
    precondition(device.windowWorkspaceJournal.restore(id: closedIdentity, recoverLatest: false, kind: .single) == closing.savedLayout)
    closing.noteWindowClosed()
    note("PASS closed window persistence: late publications preserve the last open recovery point; reopening re-enables writes")
    let firstSeed = WindowSeed(tab: single.focused.id)
    let firstSeeded = ConversationWindowModel(application: device, kind: .single)
    firstSeeded.start(restoring: single.savedLayout, seed: firstSeed, persistenceID: UUID().uuidString)
    firstSeeded.persistWorkspace()
    firstSeeded.noteWindowClosed()
    let keptSeed = WindowSeed(tab: closing.focused.id)
    let keptSeeded = ConversationWindowModel(application: device, kind: .single)
    keptSeeded.start(restoring: closing.savedLayout, seed: keptSeed, persistenceID: UUID().uuidString)
    keptSeeded.persistWorkspace(); device.windowWorkspaceJournal.flush()
    let keptLayout = keptSeeded.savedLayout
    let archivedSeed = try! JSONDecoder().decode(WindowSeed.self, from: JSONEncoder().encode(keptSeed))
    let coldSeeded = ConversationWindowModel(application: device, kind: .single)
    coldSeeded.start(restoring: firstSeeded.savedLayout, seed: archivedSeed, persistenceID: UUID().uuidString)
    precondition(coldSeeded.savedLayout.panes == keptLayout.panes && coldSeeded.savedLayout.tools == keptLayout.tools,
                 "恢复值仍指向保留的窗口；直接给出的已关闭窗口布局不能改变归属")
    precondition(device.windowWorkspaceJournal.restore(id: keptSeed.id.uuidString, recoverLatest: false, kind: .single)?.panes == keptLayout.panes)
    let replacedStartingTab = ConversationWindowModel(application: device, kind: .single)
    replacedStartingTab.start(restoring: firstSeeded.savedLayout, seed: WindowSeed(id: keptSeed.id, tab: UUID()), persistenceID: UUID().uuidString)
    precondition(replacedStartingTab.savedLayout.panes == keptLayout.panes,
                 "canonical窗口身份仍可恢复已合法替换起始tab的布局")
    let legacyLayout = ConversationWindowModel(application: device, kind: .single).savedLayout
    let missingSeed = ConversationWindowModel(application: device, kind: .single)
    missingSeed.start(restoring: firstSeeded.savedLayout, seed: WindowSeed(tab: UUID()), persistenceID: UUID().uuidString)
    precondition(missingSeed.focused.store.reachingConversationID == nil,
                 "丢失handoff和恢复点时不得自动载入首个无关会话")
    note("PASS seeded window recovery: the seed's window name wins over a handed-in layout of another window; missing handoff stays blank")
    let draftSeed = WindowSeed(tab: legacyLayout.panes.values.first!.tabs[0])
    var draftLayout = legacyLayout
    let draftText = "恢复中的非空草稿\n保留执行前输入"
    draftLayout.composer = SavedComposerDraft(text: draftText, attachments: [])
    draftLayout.composerWorkspace = nil
    device.windowWorkspaceJournal.save(draftLayout, id: draftSeed.id.uuidString)
    device.windowWorkspaceJournal.flush()
    let firstDraftRestore = ConversationWindowModel(application: device, kind: .single)
    firstDraftRestore.start(restoring: nil, seed: draftSeed, persistenceID: UUID().uuidString)
    firstDraftRestore.noteWindowClosed()
    let afterDraftRestore = device.windowWorkspaceJournal.restore(id: draftSeed.id.uuidString, recoverLatest: false, kind: .single)!
    precondition(afterDraftRestore.panes == draftLayout.panes,
                 "非空草稿的同步发布不能在替换pane前覆盖canonical恢复点")
    let secondDraftRestore = ConversationWindowModel(application: device, kind: .single)
    secondDraftRestore.start(restoring: nil, seed: draftSeed, persistenceID: UUID().uuidString)
    precondition(secondDraftRestore.savedLayout.panes == draftLayout.panes,
                 "紧接着再次启动同一窗口时仍恢复原tab，不读取半恢复布局")
    let draftSaved = secondDraftRestore.savedLayout
    precondition(draftSaved.composer?.text == draftText || draftSaved.composerWorkspace?.drafts.contains(where: { $0.state.text == draftText }) == true,
                 "抑制恢复中写入后仍须完整保留草稿")
    note("PASS restoration write transaction: nonempty legacy draft publication cannot replace the original tab; immediate second mount retains panes and draft")
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

@MainActor private func appearanceZoomChecks(home: URL) -> Never {
    func require(_ condition: @autoclosure () -> Bool, _ message: @autoclosure () -> String) {
        guard condition() else {
            FileHandle.standardError.write(Data("FAIL appearance zoom: \(message())\n".utf8))
            exit(1)
        }
    }
    let foreground = NSWorkspace.shared.frontmostApplication?.processIdentifier
    func safe() {
        require(NSApp.activationPolicy() == .prohibited && !NSApp.isActive && !NSRunningApplication.current.isActive,
                "缩放检查进程不得激活")
        require(NSApp.windows.allSatisfy { !$0.isVisible && !$0.isKeyWindow && !$0.isMainWindow }, "缩放检查不得展示或聚焦窗口")
        require(NSWorkspace.shared.frontmostApplication?.processIdentifier == foreground, "缩放检查期间前台进程改变")
    }
    safe()

    let preferences = AppearancePreferences.shared
    let file = home.appendingPathComponent("mac-appearance.json")
    let oldValues = Data(#"{"ui":"large","messages":"small","code":"extraLarge","density":"compact","composerWidthRatio":0.65}"#.utf8)
    try! oldValues.write(to: file, options: .withoutOverwriting)
    var problems: [String] = []
    func reload() { preferences.load(home: home) { problems.append($0) } }
    reload()
    let baseline = preferences.values
    require(problems.isEmpty && baseline.zoomPercent == 100 && baseline.ui == .large && baseline.messages == .small
            && baseline.code == .extraLarge && baseline.density == .compact && baseline.composerWidthRatio == 0.65,
            "旧外观配置应默认 100%，并保留之前的字号、密度和宽度")
    let keys = KeyboardShortcutSettings.shared
    require(keys.shortcut(.zoomIn) == Shortcut("+") && keys.shortcut(.zoomOut) == Shortcut("-")
            && keys.shortcut(.resetZoom) == Shortcut("0", [.command, .option]) && keys.shortcut(.focusTranscript) == Shortcut("0"),
            "缩放默认键冲突或覆盖了转到消息")
    let equals = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 0,
                                  windowNumber: 0, context: nil, characters: "=", charactersIgnoringModifiers: "=",
                                  isARepeat: false, keyCode: 24)!
    require(KeyRouter.routeZoomInAlias(equals) && preferences.values.zoomPercent == 110, "⌘= 必须执行默认放大")
    ShortcutCommand.zoomOut.performZoom()
    require(preferences.values == baseline, "缩小命令必须和放大使用同一个缩放状态")
    keys.set(Shortcut("="), for: .reload)
    require(!KeyRouter.routeZoomInAlias(equals) && preferences.values == baseline, "⌘= 应保留给用户已有的显式绑定")
    keys.reset(.reload)
    for expected in [110, 125, 150] {
        require(preferences.canAdjustZoom(by: 1), "应允许放大至 \(expected)%")
        preferences.adjustZoom(by: 1)
        require(preferences.values.zoomPercent == expected, "未放大到 \(expected)%")
    }
    require(!preferences.canAdjustZoom(by: 1), "达到上限后仍允许放大")
    preferences.adjustZoom(by: 1)
    require(preferences.values.zoomPercent == 150, "放大超出上限")
    let enlarged = preferences.values
    reload()
    require(problems.isEmpty && preferences.values == enlarged, "重新载入外观时丢失缩放档位")
    for expected in [125, 110, 100, 90, 80] {
        require(preferences.canAdjustZoom(by: -1), "应允许缩小至 \(expected)%")
        preferences.adjustZoom(by: -1)
        require(preferences.values.zoomPercent == expected, "未缩小到 \(expected)%")
    }
    require(!preferences.canAdjustZoom(by: -1), "达到下限后仍允许缩小")
    preferences.adjustZoom(by: -1)
    require(preferences.values.zoomPercent == 80, "缩小超出下限")
    preferences.resetZoom()
    require(preferences.values == baseline, "恢复缩放重置了独立外观偏好")
    reload()
    require(problems.isEmpty && preferences.values == baseline, "重新载入后未恢复 100% 和原外观偏好")

    let dark = CommandLine.arguments.contains("--appearance-zoom-dark")
    let appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
    NSApp.appearance = appearance
    func layout(_ view: NSView) {
        view.layoutSubtreeIfNeeded(); view.display()
        RunLoop.main.run(until: Date().addingTimeInterval(0.15))
        view.layoutSubtreeIfNeeded(); view.display()
        safe()
    }
    func capture(_ view: NSView, as name: String) {
        guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { require(false, "无法绘制 \(name)"); return }
        view.cacheDisplay(in: view.bounds, to: bitmap)
        try! bitmap.representation(using: .png, properties: [:])!.write(to: home.appendingPathComponent(name + ".png"))
    }
    func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    func visibleMenus(in view: NSView) -> [NSPopUpButton] {
        descendants(view).compactMap { $0 as? NSPopUpButton }.filter { !$0.isHiddenOrHasHiddenAncestor }
    }

    preferences.values = AppearanceValues()
    let device = DeviceStore(preview: true)
    let permission = PermissionMode(id: "ask", label: "请求审批", detail: "合成权限模式")
    var runtimeType = device.runtimeTypes[0]
    runtimeType.permissionModes = [permission]
    let listed = device.conversations.map { value in
        ListedConversation(conversation: .init(id: value.id, title: .native(value.title),
            updatedAtUnixMs: value.updatedAtUnixMs, createdAtUnixMs: value.createdAtUnixMs, runtimeId: value.runtimeID,
            cwd: value.cwd, canRename: true, canDelete: true, parentConversationId: nil, nativeReadState: nil),
            nativeId: value.id, agentsUri: nil, pinned: false, unread: false, archived: false,
            project: value.project, moved: false, failed: false, group: value.group)
    }
    device.applyList(ConfigurationSnapshot(conversations: listed, conversationGroups: device.conversationGroups,
        conversationProjects: [], workingDirectoryProjects: [:], showsArchived: false, historyFailures: [], gateways: [],
        runtimeInstances: device.runtimeInstances, runtimeTypes: [runtimeType], modelTemplates: [],
        conversationBrowserLimits: device.conversationBrowserLimits, outlinePresentation: .popover, linkedConversationOpening: .currentTab,
        usageDisplay: .used, contextUsageDisplay: .used, conversationBrowserPreferences: ConversationBrowser().preferences,
        statusLine: [], providerImportTypes: [], protocols: device.protocols, selectedRuntimeInstanceId: device.selectedRuntimeInstanceID))
    let code = "let scale = 1.25"
    if var sample = device.previewSnapshots["sample-0"] {
        let messages: [DeliveredMessage] = [
            .init(id: "zoom-input", role: .user, blocks: [.text(text: "请调整界面字号，并保留正在编辑的输入。")]),
            .init(id: "zoom-reply", role: .assistant, blocks: [.text(text: "消息、代码和控件的字号会随缩放一起调整。\n\n```swift\n\(code)\n```")]),
        ]
        sample.messages = messages
        sample.transcriptItems = messages.map { .message(id: $0.id, messageId: $0.id) }
        sample.transcriptMessageIdentities = messages.map { .init(id: $0.id, messageId: $0.id) }
        sample.transcriptTurns = []
        sample.transcriptOutline = []
        device.keepPreviewSnapshot(sample)
    }
    let workspace = ConversationWindowModel(application: device, kind: .workspace)
    let tab = workspace.focused
    tab.store.selectConversation(id: "sample-0")
    workspace.tools.move(.composer, to: .builtIn)
    let model = workspace.composer(for: tab)
    let selectionModel = ModelSelection.native(providerId: "sample", modelId: "sample-native")
    workspace.composerContext(for: tab).selectModel(selectionModel)
    workspace.composerContext(for: tab).selectEffort("high")
    workspace.composerContext(for: tab).selectPermissionMode(permission.id)
    let draft = "缩放时保留这段草稿"
    model.restore(draft, attachments: [])
    let host = NSHostingView(rootView: HStack(spacing: 0) {
        ConversationListView(window: workspace, store: workspace.list, focused: tab.store).frame(width: 232)
        Divider()
        PaneGrid(window: workspace, previewEmpty: false)
    }.applicationAppearance().environment(\.colorScheme, dark ? .dark : .light)
        .background(Color(nsColor: .windowBackgroundColor)))
    let size = NSSize(width: 920, height: 560)
    host.frame = NSRect(origin: .zero, size: size)
    let window = OffscreenMessageEditorWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: true)
    window.appearance = appearance
    window.isRestorable = false
    window.contentView = host
    layout(host)
    guard let editor = descendants(host).compactMap({ $0 as? ComposerTextView }).first, let undo = editor.undoManager else {
        require(false, "未找到工作区原生编辑器和撤销记录")
        exit(1)
    }
    require(editor.string == draft, "工作区未显示草稿")
    undo.beginUndoGrouping()
    editor.insertText("和撤销记录", replacementRange: NSRange(location: (draft as NSString).length, length: 0))
    undo.endUndoGrouping()
    let typed = draft + "和撤销记录"
    let selection = NSRange(location: 2, length: 3)
    editor.setSelectedRange(selection)
    require(model.submission == typed && undo.canUndo, "原生编辑器未记录真实草稿修改")

    var measurements: [[String: Any]] = []
    func measure(_ percent: Int, name: String, wide: Bool = true) -> [CGFloat] {
        layout(host)
        require(preferences.values.zoomPercent == percent, "视图测量不在预期缩放档位")
        require(AppearancePreferences.rendering.read() == preferences.values, "原生排版快照必须来自当前外观设置")
        require(descendants(host).contains { $0 === editor } && model.submission == typed && editor.string == typed
                && editor.selectedRange() == selection && undo.canUndo, "缩放丢失输入、选区、撤销或原生编辑器")
        let bounds = host.convert(editor.bounds, from: editor)
        require(bounds.width > 0 && bounds.height > 0 && host.bounds.insetBy(dx: -1, dy: -1).contains(bounds),
                "缩放后输入框脱离工作区边界")
        guard let codeEditor = descendants(host).compactMap({ $0 as? TranscriptTextView }).first(where: { $0.string.contains(code) }),
              let codeSize = (codeEditor.textStorage?.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)?.pointSize,
              let editorSize = editor.font?.pointSize else {
            require(false, "未找到工作区代码文本和输入区字号")
            exit(1)
        }
        let caption = TranscriptTypography.caption.pointSize
        let spacing = TranscriptRowLayout.blockSpacing
        let context = workspace.composerContext(for: tab)
        require(context.chosenModel == selectionModel && context.nextTurnEffort == "high"
                && context.effectivePermissionMode?.id == permission.id, "缩放丢失执行配置")
        let menus = visibleMenus(in: host)
        let agent = menus.first { $0.title.contains("运行时示例模型") && $0.title.contains("high") }
        let access = menus.first { $0.title.contains(permission.label) }
        if wide {
            require(agent?.title.contains("示例运行时") == true && access != nil, "宽窗未显示完整的执行配置")
        }
        let executionControls = [agent, access].compactMap { $0 }
        require(agent != nil && executionControls.allSatisfy { host.bounds.contains(host.convert($0.bounds, from: $0)) },
                "执行配置超出工作区边界")
        let agentWidth = agent?.intrinsicContentSize.width ?? 0
        let permissionWidth = access?.intrinsicContentSize.width ?? 0
        measurements.append(["sample": name, "zoom": percent, "width": host.bounds.width, "composerFont": editorSize,
                             "codeFont": codeSize, "captionFont": caption, "messageSpacing": spacing,
                             "executionMenuWidth": agentWidth, "permissionMenuWidth": permissionWidth,
                             "composerFrame": NSStringFromRect(bounds)])
        capture(host, as: name)
        return [editorSize, codeSize, caption, spacing, agentWidth, permissionWidth]
    }
    let standard = measure(100, name: "zoom-workspace-100")
    preferences.adjustZoom(by: -1); preferences.adjustZoom(by: -1)
    let small = measure(80, name: "zoom-workspace-80")
    preferences.resetZoom()
    preferences.adjustZoom(by: 1); preferences.adjustZoom(by: 1); preferences.adjustZoom(by: 1)
    let large = measure(150, name: "zoom-workspace-150")
    require(zip(small, standard).allSatisfy { $0 < $1 } && zip(standard, large).allSatisfy { $0 < $1 },
            "字号或间距未随 80%、100%、150% 递增")
    window.setContentSize(NSSize(width: 720, height: size.height))
    _ = measure(150, name: "zoom-workspace-narrow-150", wide: false)
    preferences.resetZoom()
    layout(host)
    require(editor.font?.pointSize == standard[0], "恢复 100% 后输入区仍使用放大字号")
    undo.undo()
    require(model.submission == draft && editor.string == draft, "缩放后无法撤销原有输入")

    window.setContentSize(NSSize(width: 1_200, height: size.height))
    let cwd = "/synthetic/zoom-workspace"
    require(workspace.openNewConversationTab(runtimeID: "sample-instance", cwd: cwd) != nil, "无法创建带工作目录的合成草稿")
    func measureDirectory(_ percent: Int) -> CGFloat {
        layout(host)
        guard let menu = visibleMenus(in: host).first(where: { $0.title.contains("zoom-workspace") }) else {
            require(false, "未发送会话未显示工作目录")
            return 0
        }
        require(host.bounds.contains(host.convert(menu.bounds, from: menu)), "工作目录控件超出工作区边界")
        capture(host, as: "zoom-draft-\(percent)")
        return menu.intrinsicContentSize.width
    }
    let directoryWidth = measureDirectory(100)
    preferences.adjustZoom(by: 1); preferences.adjustZoom(by: 1); preferences.adjustZoom(by: 1)
    require(measureDirectory(150) > directoryWidth, "工作目录标签未随缩放加宽")
    window.contentView = nil

    let form = NSHostingView(rootView: Form { AppearanceSettingsSection() }.formStyle(.grouped)
        .applicationAppearance().environment(\.colorScheme, dark ? .dark : .light))
    form.frame = NSRect(x: 0, y: 0, width: 530, height: 515)
    let settings = OffscreenMessageEditorWindow(contentRect: form.frame, styleMask: [.borderless], backing: .buffered, defer: true)
    settings.appearance = appearance
    settings.isRestorable = false
    settings.contentView = form
    layout(form)
    capture(form, as: "zoom-settings-narrow-150")
    settings.contentView = nil
    try! JSONSerialization.data(withJSONObject: measurements, options: [.prettyPrinted, .sortedKeys])
        .write(to: home.appendingPathComponent("render-measurements.json"))
    safe()
    FileHandle.standardError.write(Data("PASS appearance zoom: shortcuts, persistence, live composer and undo, execution controls and working directory, narrow workspace, transcript and settings\n".utf8))
    exit(0)
}

@MainActor private func renderExperienceSamples(home: URL, window: ConversationWindowModel, model: ComposerModel, now: Int64) {
    func capture(_ view: NSView, name: String, size: NSSize, appearance: NSAppearance.Name = .aqua) {
        view.frame = NSRect(origin: .zero, size: size)
        let host = NSWindow(contentRect: view.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        host.appearance = NSAppearance(named: appearance)
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
    capture(NSHostingView(rootView: WindowRestorationPendingView()), name: "window-restoration-pending-light", size: NSSize(width: 520, height: 300))
    capture(NSHostingView(rootView: WindowRestorationPendingView().environment(\.colorScheme, .dark)), name: "window-restoration-pending-dark", size: NSSize(width: 520, height: 300), appearance: .darkAqua)
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
        add(.message(message, revision: 0, expanded: [], copies: true, edits: message.role == .user, forks: message.role == .assistant), rendered: RenderedMessage(message), id: message.id)
    }
    let editedMessage = DeliveredMessage(id: "user", role: .user, timestampUnixMs: now, blocks: [
        .text(text: "请把刚才的方案再简化一些。\n\n保留现有的消息编辑入口，让我能直接修改正文，再决定是在原会话继续，还是另开一个会话。运行参数不需要一直展开。\n\n另外，发送失败时请保留我正在编辑的内容。"),
    ])
    for (name, height, appearance, revision) in [
        ("message-editor", 450.0, NSAppearance.Name.aqua, String?.none),
        ("message-editor-dark", 450.0, NSAppearance.Name.darkAqua, String?.none),
        ("message-editor-dark-conflict", 520.0, NSAppearance.Name.darkAqua, Optional("synthetic-old-history")),
    ] {
        let request = MessageEditRequest(message: editedMessage, conversationID: nil, inPlace: true, isLast: false, revision: revision)
        let session = MessageEditingSession(source: window.focused.store, request: request, presentation: .sheet)
        capture(NSHostingView(rootView: MessageEditorView(session: session, window: window, source: window.focused)
            .applicationAppearance().background(Color(nsColor: .windowBackgroundColor))),
            name: name, size: NSSize(width: 620, height: height), appearance: appearance)
    }
    let attached = DeliveredMessage(id: "attachment-editor", role: .user, blocks:
        [.text(text: String(repeating: "编辑长正文保留换行与执行配置。\n", count: 40))] +
        (0..<12).map { .file(path: "/tmp/" + String(repeating: "very-long-attachment-name-", count: 5) + "\($0).md") })
    let attachedSession = MessageEditingSession(source: window.focused.store,
        request: MessageEditRequest(message: attached, conversationID: nil, inPlace: true, isLast: false, revision: nil), presentation: .sheet)
    capture(NSHostingView(rootView: MessageEditorView(session: attachedSession, window: window, source: window.focused)
        .applicationAppearance().background(Color(nsColor: .windowBackgroundColor))),
        name: "message-editor-long-attachments", size: NSSize(width: 620, height: 520))
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
