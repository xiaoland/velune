import AppKit
import SwiftUI
import VeluneBindings

@MainActor private final class QueuedEditorCheckWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    private func refuse() -> Never {
        FileHandle.standardError.write(Data("FAIL queued editor: presentation requested\n".utf8))
        exit(1)
    }
    override func makeKey() { refuse() }
    override func makeMain() { refuse() }
    override func makeKeyAndOrderFront(_ sender: Any?) { refuse() }
    override func orderFront(_ sender: Any?) { refuse() }
    override func orderBack(_ sender: Any?) { refuse() }
    override func orderFrontRegardless() { refuse() }
    override func order(_ place: NSWindow.OrderingMode, relativeTo otherWin: Int) {
        if place != .out { refuse() }
        super.order(place, relativeTo: otherWin)
    }
}

/// Exercise the production editor with queue projections and render its contents
/// without presenting a sheet, starting a native runtime, or injecting input.
@MainActor func queuedMessageEditorChecks() -> Never {
    func fail(_ message: String) -> Never {
        FileHandle.standardError.write(Data("FAIL queued editor: \(message)\n".utf8))
        exit(1)
    }
    func require(_ condition: @autoclosure () -> Bool, _ message: String) {
        if !condition() { fail(message) }
    }
    guard let directory = ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"] else { fail("missing private output") }
    let output = URL(fileURLWithPath: directory).standardizedFileURL.resolvingSymlinksInPath()
    require(output.path.hasPrefix("/private/tmp/") || output.path.hasPrefix("/tmp/"), "requires a private temporary output directory")
    _ = NSApplication.shared
    if NSApp.activationPolicy() != .prohibited { _ = NSApp.setActivationPolicy(.prohibited) }
    let frontmost = NSWorkspace.shared.frontmostApplication?.processIdentifier
    var activated = false
    let observers = [NSApplication.didBecomeActiveNotification, NSWindow.didBecomeKeyNotification, NSWindow.didBecomeMainNotification].map { event in
        NotificationCenter.default.addObserver(forName: event, object: nil, queue: .main) { _ in activated = true }
    }
    defer { for observer in observers { NotificationCenter.default.removeObserver(observer) } }
    func safe() {
        require(!activated && NSApp.activationPolicy() == .prohibited && !NSApp.isActive && !NSRunningApplication.current.isActive
                && NSWorkspace.shared.frontmostApplication?.processIdentifier == frontmost, "check must remain in the background")
        require(NSApp.windows.allSatisfy { !$0.isVisible && !$0.isKeyWindow && !$0.isMainWindow }, "check must not present a window")
    }
    safe()
    let dark = CommandLine.arguments.contains("--queued-editor-dark")
    var states: [String] = []
    func composer(in view: NSView) -> ComposerTextView? {
        if let editor = view as? ComposerTextView { return editor }
        for child in view.subviews {
            if let editor = composer(in: child) { return editor }
        }
        return nil
    }
    func capture(_ name: String, _ session: QueuedMessageEditingSession, _ workspace: ConversationWindowModel) {
        let view = QueuedMessageEditor(session: session, window: workspace, tab: workspace.focused)
            .environment(\.colorScheme, dark ? .dark : .light)
            .background(Color(nsColor: .windowBackgroundColor))
        let host = NSHostingView(rootView: view)
        host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        host.frame = NSRect(x: 0, y: 0, width: 560, height: 300)
        let window = QueuedEditorCheckWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false
        window.isRestorable = false
        window.contentView = host
        host.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.1))
        window.setContentSize(NSSize(width: 560, height: max(150, host.fittingSize.height)))
        host.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.08))
        host.layoutSubtreeIfNeeded()
        safe()
        guard let editor = composer(in: host), editor.window === window,
              editor.string == session.editor.submission else { fail("\(name): production composer did not mount with the current draft") }
        if let container = editor.textContainer { editor.layoutManager?.ensureLayout(for: container) }
        editor.needsDisplay = true
        host.needsDisplay = true
        host.displayIfNeeded()
        guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { fail("failed to allocate screenshot") }
        host.cacheDisplay(in: host.bounds, to: bitmap)
        guard let png = bitmap.representation(using: .png, properties: [:]) else { fail("failed to encode screenshot") }
        do { try png.write(to: output.appendingPathComponent("queued-editor-\(name)-\(dark ? "dark" : "light").png")) }
        catch { fail(error.localizedDescription) }
        states.append(name)
        window.contentView = nil
        window.close()
        safe()
    }

    ConversationStore.inspectQueuedEditor { workspace, message, project, setCatalog in
        let tab = workspace.focused
        let store = tab.store
        guard let conversationID = store.loadedConversationID, let context = message.editContext else { fail("missing queue fixture identity") }
        let ordinaryFile = output.appendingPathComponent("ordinary-draft.md")
        do { try Data("Ordinary draft attachment".utf8).write(to: ordinaryFile) }
        catch { fail(error.localizedDescription) }
        let ordinary = workspace.composer(for: tab)
        ordinary.restore("普通草稿保留", attachments: [ordinaryFile])
        let originalDraft = ordinary.savedDraft
        let originalExecution = store.nextTurn
        let session = QueuedMessageEditingSession(store: store, conversationID: conversationID, message: message, context: context)
        require(session.editor.submission == message.text && session.target.nextTurn?.choice.model == context.selection
                && session.target.nextTurn?.choice.effort == context.effort, "editor must start from the queue's own instruction")
        require(session.retained.map(\.id) == [0] && session.retained.first?.name == "notes.md"
                && session.editor.attachments.isEmpty, "host-only attachment must be retained without a local upload")
        capture("initial", session, workspace)

        let catalog = store.modelCatalogs[context.runtimeId]
        setCatalog(.loading)
        var retainedModel = ComposerContext(tab: tab, model: session.editor, editingTarget: session.target)
        require(retainedModel.selectedModelName == "sample-native" && !retainedModel.chosenModelUnavailable,
                "loading a model catalog must keep the captured model visible")
        setCatalog(.ready([]))
        retainedModel = ComposerContext(tab: tab, model: session.editor, editingTarget: session.target)
        require(retainedModel.selectedModelName == "sample-native" && retainedModel.chosenModelUnavailable
                && retainedModel.submission().canSubmitDraft,
                "a model missing from the catalog must retain its identity without blocking content edits")
        guard case let .replace(_, _, _, savedSelection, _, _)? = session.operation else { fail("missing retained-model replacement") }
        require(savedSelection == context.selection, "saving content must keep the original queued model")
        capture("retained-model", session, workspace)
        session.target.chooseEffort("high")
        require(!session.canSave && session.operation == nil && session.target.recordedModelName == nil,
                "editing execution settings still requires a model offered by the current catalog")
        session.target.chooseEffort(context.effort)
        require(session.canSave, "restoring the queued execution settings must allow editing content again")
        setCatalog(catalog)
        require(ComposerContext(tab: tab, model: session.editor, editingTarget: session.target).selectedModelName
                == catalog?.models?.first(where: { $0.selection == context.selection })?.name,
                "use the regular model name when the catalog offers the captured selection")

        session.editor.receiveDraft(.init(text: "", attachments: []))
        let attachmentOnly = ComposerContext(tab: tab, model: session.editor, editingTarget: session.target)
        require(attachmentOnly.submission().canSubmitDraft, "retained attachments also count as input")
        guard case let .replace(_, _, _, _, keeps, retainedOnly)? = session.operation else { fail("missing attachment-only replacement") }
        require(keeps == [0] && retainedOnly.attachments.isEmpty, "original attachments must travel as indices, not uploaded local paths")
        session.removeRetained(0)
        require(!session.canSave, "empty queued instructions must not be saved")

        let added = output.appendingPathComponent("new-input.md")
        do { try Data("Synthetic new attachment".utf8).write(to: added) }
        catch { fail(error.localizedDescription) }
        session.editor.restore("修改排队消息，附上这份轮次检查结果。", attachments: [])
        session.editor.addAttachments([added])
        let targetRuntime = "sample-review"
        let newSelection = ModelSelection.native(providerId: "sample", modelId: "sample-native")
        session.target.chooseModel(nil)
        require(session.target.nextTurn?.choice.model == nil, "editor must allow clearing the captured model")
        session.target.chooseRuntime(targetRuntime)
        session.target.chooseModel(newSelection)
        session.target.chooseEffort("high")
        let permission = session.target.nextTurn?.permissionModes.first?.id
        if let permission { session.target.choosePermissionMode(permission) }
        guard case let .replace(id, revision, runtime, selection, retained, turn)? = session.operation else { fail("missing replacement instruction") }
        require(id == message.id && revision == context.revision && runtime == targetRuntime && selection == newSelection
                && retained.isEmpty && turn.text == session.editor.submission && turn.attachments == [added.path]
                && turn.effort == "high" && turn.permissionMode == permission, "replacement must encode edited input, target and revision")
        require(ordinary.savedDraft == originalDraft && store.nextTurn == originalExecution, "queued edit must not touch ordinary input")
        capture("edited", session, workspace)

        project(message, true)
        require(!session.canSave && session.operation == nil, "another pending operation must block saving")
        capture("busy", session, workspace)
        project(message, false)
        let disconnected = QueuedMessageEditingSession(store: store, conversationID: conversationID, message: message, context: context)
        disconnected.editor.receiveDraft(.init(text: "提交失败也保留这一版", attachments: [added]))
        disconnected.save { fail("a disconnected edit must not report success") }
        require(disconnected.failure != nil && disconnected.editor.attachments == [added] &&
                disconnected.editor.submission == "提交失败也保留这一版", "a failed receipt must retain the edit")
        capture("failed", disconnected, workspace)
        var changed = message
        changed.editContext?.revision += 1
        project(changed, false)
        require(!session.canSave && session.operation == nil && session.issue != nil, "a concurrent edit must block stale replacement")
        capture("stale", session, workspace)
        project(nil, false)
        require(!session.canSave && session.operation == nil && session.issue != nil, "an item that left the queue must not be resubmitted")
        require(session.editor.submission == turn.text && session.editor.attachments == [added] && ordinary.savedDraft == originalDraft,
                "stale or dispatched item must leave the edit and ordinary composer intact")
        capture("dispatched", session, workspace)
    }
    safe()
    do { try Data(("PASS queued editor: " + states.joined(separator: ", ") + "\n").utf8).write(to: output.appendingPathComponent("queued-editor-check.txt")) }
    catch { fail(error.localizedDescription) }
    print("PASS queued editor: \(states.joined(separator: ", "))")
    exit(0)
}
