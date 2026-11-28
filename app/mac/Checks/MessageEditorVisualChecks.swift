#if VELUNE_DEV_CHECKS
import AppKit
import SwiftUI
import VeluneBindings
import ObjectiveC

/// A rendering host can never present or become key/main, including if a view
/// accidentally requests it. No system input or foreground recovery is allowed.
@MainActor final class OffscreenMessageEditorWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    private func refusePresentation() -> Never {
        FileHandle.standardError.write(Data("FAIL offscreen editor: presentation requested\n".utf8))
        exit(1)
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

/// Draw production views in guarded, unordered hosts, without scenes, launch
/// completion or system event injection. Entry precedes SwiftUI App startup.
@MainActor func messageEditorOffscreenChecks(home: URL) -> Never {
    // Expected check failures exit normally, avoiding macOS crash-report UI.
    func fatalError(_ message: String) -> Never {
        FileHandle.standardError.write(Data("FAIL windowless editor: \(message)\n".utf8))
        exit(1)
    }
    func precondition(_ condition: @autoclosure () -> Bool, _ message: @autoclosure () -> String = "验收条件不满足", line: UInt = #line) {
        if !condition() { fatalError("line \(line): \(message())") }
    }
    if NSApp.activationPolicy() != .prohibited {
        precondition(NSApp.setActivationPolicy(.prohibited), "无法禁止应用激活")
    }
    precondition(NSApp.activationPolicy() == .prohibited, "检查进程必须禁止激活")
    // AppKit allocates TextInputUI helpers lazily, including before rendering.
    // Objects are not presentation: require every helper to remain offscreen,
    // and independently require each production view to have no host window.
    var frameworkWindowIDs: Set<ObjectIdentifier> = []
    var helperDetails: [[String: Any]] = []
    var hosts: [OffscreenMessageEditorWindow] = []
    func hiddenSystemHelpers() -> Bool {
        NSApp.windows.allSatisfy { window in
            if hosts.contains(where: { $0 === window }) { return !window.isVisible && !window.isKeyWindow && !window.isMainWindow }
            let image = class_getImageName(type(of: window)).map { String(cString: $0) } ?? ""
            if frameworkWindowIDs.insert(ObjectIdentifier(window)).inserted {
                let detail: [String: Any] = ["class": String(describing: type(of: window)), "image": image,
                    "visible": window.isVisible, "key": window.isKeyWindow, "main": window.isMainWindow]
                helperDetails.append(detail)
                FileHandle.standardError.write(Data("SYSTEM_HELPER \(detail)\n".utf8))
            }
            return image.hasPrefix("/System/Library/") && !window.isVisible && !window.isKeyWindow && !window.isMainWindow
        }
    }
    var activations = 0
    var keyWindows = 0
    var mainWindows = 0
    var systemActivations = 0
    let frontmostAtStart = NSWorkspace.shared.frontmostApplication?.processIdentifier ?? -1
    let active = NotificationCenter.default.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in activations += 1 }
    let keyed = NotificationCenter.default.addObserver(forName: NSWindow.didBecomeKeyNotification, object: nil, queue: .main) { _ in keyWindows += 1 }
    let main = NotificationCenter.default.addObserver(forName: NSWindow.didBecomeMainNotification, object: nil, queue: .main) { _ in mainWindows += 1 }
    let system = NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main) { notification in
        if (notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication)?.processIdentifier == ProcessInfo.processInfo.processIdentifier { systemActivations += 1 }
    }
    defer {
        NotificationCenter.default.removeObserver(active); NotificationCenter.default.removeObserver(keyed); NotificationCenter.default.removeObserver(main)
        NSWorkspace.shared.notificationCenter.removeObserver(system)
    }
    func safe() {
        precondition(NSApp.activationPolicy() == .prohibited && !NSApp.isActive && !NSRunningApplication.current.isActive &&
                     hiddenSystemHelpers() &&
                     activations == 0 && keyWindows == 0 && mainWindows == 0 && systemActivations == 0,
                     "离屏检查不得显示窗口、成为 key 或激活应用：policy=\(NSApp.activationPolicy().rawValue), active=\(NSApp.isActive), systemActive=\(NSRunningApplication.current.isActive), windows=\(NSApp.windows.map { String(describing: type(of: $0)) }), activationEvents=\(activations), keyEvents=\(keyWindows)")
    }
    func settle() {
        RunLoop.main.run(until: Date().addingTimeInterval(0.15))
        safe()
    }
    func wait(_ ready: () -> Bool) {
        let end = Date().addingTimeInterval(20)
        while !ready(), Date() < end { RunLoop.main.run(until: Date().addingTimeInterval(0.02)); safe() }
        precondition(ready(), "离屏编辑器检查超时")
    }
    func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    var measurements: [[String: Any]] = []
    func capture<V: View>(_ content: V, name: String, width: CGFloat, height: CGFloat? = nil) -> NSView {
        let host = NSHostingView(rootView: content.background(Color(nsColor: .windowBackgroundColor)))
        host.appearance = NSAppearance(named: CommandLine.arguments.contains("--editor-check-dark") ? .darkAqua : .aqua)
        host.frame = NSRect(x: 0, y: 0, width: width, height: height ?? 320)
        let window = OffscreenMessageEditorWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false
        window.isRestorable = false
        hosts.append(window)
        window.contentView = host
        host.layoutSubtreeIfNeeded(); settle(); host.layoutSubtreeIfNeeded()
        if height == nil {
            window.setContentSize(NSSize(width: width, height: max(160, host.fittingSize.height)))
            host.layoutSubtreeIfNeeded(); settle(); host.layoutSubtreeIfNeeded()
        }
        precondition(host.window === window && !window.isVisible && !window.canBecomeKey && !window.canBecomeMain)
        // A hidden host has no display-link paint cycle. Resolve the native
        // text layout after its final viewport resize before caching pixels.
        let nativeEditors = descendants(host).compactMap { $0 as? ComposerTextView }
        for editor in nativeEditors {
            if let container = editor.textContainer { editor.layoutManager?.ensureLayout(for: container) }
            editor.needsDisplay = true
            editor.enclosingScrollView?.needsDisplay = true
        }
        host.needsDisplay = true
        host.displayIfNeeded()
        guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { fatalError("无法绘制离屏原生视图") }
        host.cacheDisplay(in: host.bounds, to: bitmap)
        try! bitmap.representation(using: .png, properties: [:])!.write(to: home.appendingPathComponent(name + ".png"))
        let editors = descendants(host).compactMap { $0 as? ComposerTextView }
        precondition(editors.count == 1, "必须实际渲染同一个原生 ComposerTextView")
        measurements.append(["name": name, "width": host.bounds.width, "height": host.bounds.height,
                             "editorFontSize": editors[0].font?.pointSize ?? 0,
                             "editorFrame": NSStringFromRect(host.convert(editors[0].bounds, from: editors[0])),
                             "nativeTextLength": editors[0].string.utf16.count,
                             "laidOutGlyphCount": editors[0].layoutManager?.numberOfGlyphs ?? 0,
                             "documentFrame": NSStringFromRect(editors[0].frame),
                             "scrollOrigin": NSStringFromPoint(editors[0].enclosingScrollView?.contentView.bounds.origin ?? .zero)])
        try! JSONSerialization.data(withJSONObject: measurements, options: [.prettyPrinted, .sortedKeys])
            .write(to: home.appendingPathComponent("render-measurements.json"))
        safe()
        return host
    }
    safe()
    if !FileManager.default.fileExists(atPath: home.appendingPathComponent("fixture.json").path) {
        do { try prepareMessageEditorFixture(home: home, stateDirectory: MacExecutionContext.admitted.stateDirectory) }
        catch { fatalError("无法准备合成编辑fixture：\(error)") }
    }
    let fixture = try! JSONSerialization.jsonObject(with: Data(contentsOf: home.appendingPathComponent("fixture.json"))) as! [String: String]
    let conversationID = fixture["conversationID"]!
    let messageID = fixture["messageID"]!
    let runtimeID = fixture["runtimeID"]!
    precondition(MacExecutionContext.admitted.stateDirectory.path == home.appendingPathComponent("state").standardizedFileURL.resolvingSymlinksInPath().path,
                 "检查必须使用本次专用合成状态目录")
    let transport = Transport(stateDirectory: MacExecutionContext.admitted.stateDirectory, resourcesDirectory: Bundle.main.resourceURL!)
    let device = DeviceStore(transport: transport)
    device.start()
    let workspace = ConversationWindowModel(application: device, kind: .workspace)
    let source = workspace.focused
    source.store.start(loadFirstConversation: false, opening: conversationID)
    wait { source.store.loadedConversationID == conversationID && !source.store.isLoading && source.store.editability(of: messageID)?.inPlace == true && source.store.nextTurn != nil }
    let ordinary = workspace.composer(for: source)
    let draftFile = home.appendingPathComponent("ordinary-draft.txt")
    try! Data("ordinary draft attachment".utf8).write(to: draftFile)
    ordinary.restore("普通输入草稿必须保留", attachments: [draftFile])
    ComposerContext(tab: source, model: ordinary).selectNextTurnRuntime("review")
    wait { source.store.nextTurn?.runtimeId == "review" }
    let originalChoice = source.store.nextTurn?.choice
    let argumentDomain = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
    func preference(_ value: MessageEditPresentation) {
        var arguments = argumentDomain
        arguments[MessageEditPresentation.preferenceKey] = value.rawValue
        UserDefaults.standard.setVolatileDomain(arguments, forName: UserDefaults.argumentDomain)
    }
    func unchanged(checkExecution: Bool = true) {
        precondition(ordinary.submission == "普通输入草稿必须保留" && ordinary.attachments.count == 1 &&
                     ordinary.attachments.first?.lastPathComponent == draftFile.lastPathComponent &&
                     ordinary.attachments.first.flatMap { try? Data(contentsOf: $0) } == (try? Data(contentsOf: draftFile)),
                     "普通草稿发生变化：text=\(ordinary.submission), attachments=\(ordinary.attachments.map(\.path))")
        if checkExecution {
            precondition(source.store.nextTurn?.runtimeId == "review" && source.store.nextTurn?.choice == originalChoice,
                         "提交接受前，编辑不能污染普通输入的执行选择")
        }
    }
    func session(_ presentation: MessageEditPresentation) -> MessageEditingSession {
        let eligibility = source.store.editability(of: messageID)!
        let session = MessageEditingSession(source: source.store, request: .init(message: source.store.message(id: messageID)!, conversationID: conversationID,
            inPlace: eligibility.inPlace, isLast: source.store.isLastUserMessage(messageID), revision: eligibility.revision), presentation: presentation)
        session.start()
        wait { session.store.loadedConversationID == conversationID && !session.store.isLoading && session.next != nil && session.store.editability(of: messageID) != nil }
        return session
    }
    // A captured initial control check may offer fork-only while this private
    // Store is still opening. Read actual ready eligibility; do not fabricate
    // an in-place capability, and do not replace an explicit user fork choice.
    func preparingSession(explicitFork: Bool) -> MessageEditingSession {
        let result = MessageEditingSession(source: source.store, request: .init(message: source.store.message(id: messageID)!,
            conversationID: conversationID, inPlace: false, isLast: false,
            revision: source.store.editability(of: messageID)?.revision), presentation: .sheet)
        precondition(result.placement == .fork)
        if explicitFork { result.placement = .fork }
        result.start()
        wait { result.store.editability(of: messageID)?.inPlace == true && !result.store.isLoading }
        return result
    }
    let untouchedDefault = preparingSession(explicitFork: false)
    wait { untouchedDefault.placement == .inPlace }
    precondition(!untouchedDefault.historyChanged, "准备资格不能变成历史变化")
    untouchedDefault.cancel()
    let explicitFork = preparingSession(explicitFork: true)
    settle()
    precondition(explicitFork.placement == .fork, "资格就绪不能覆盖用户显式选择的新会话")
    explicitFork.cancel()
    let latestMessage = source.store.transcript.rows.last(where: { $0.message.role == .user })!.message
    let latest = MessageEditingSession(source: source.store, request: .init(message: latestMessage, conversationID: conversationID,
        inPlace: source.store.editability(of: latestMessage.id)!.inPlace, isLast: true,
        revision: source.store.editability(of: latestMessage.id)?.revision), presentation: .sheet)
    latest.start()
    wait { latest.store.loadedConversationID == conversationID && !latest.store.isLoading }
    latest.loadTargetCatalog()
    wait { latest.canSubmit }
    precondition(latest.placement == .inPlace && latest.needsPlacement && latest.target.submitTitle == "替换并发送" && !latest.historyChanged,
                 "最后一条消息必须明确默认原会话，并保留切换新会话的入口；placement=\(String(describing: latest.placement)), changed=\(latest.historyChanged)")
    let latestHost = capture(MessageEditorView(session: latest, window: workspace, source: source).applicationAppearance(),
                             name: "sheet-latest-default", width: 560)
    guard let placementPicker = descendants(latestHost).compactMap({ $0 as? NSPopUpButton })
        .first(where: { $0.itemTitles.contains("新会话（保留原会话）") }) else { fatalError("最后一条消息必须有原生继续方式picker") }
    // An unordered host does not run the system popup tracking loop. Exercise
    // the same Binding setter, then verify the real menu's selected display.
    latest.placement = .fork
    settle()
    precondition(placementPicker.titleOfSelectedItem == "新会话（保留原会话）", "原生picker必须显示新会话选择")
    wait { latest.placement == .fork && latest.canSubmit }
    precondition(latest.target.submitTitle == "新建并发送", "最后一条消息切fork后必须明确新建")
    latest.placement = .inPlace
    settle()
    precondition(placementPicker.titleOfSelectedItem == "原会话（替换后续内容）", "原生picker必须显示原会话选择")
    wait { latest.placement == .inPlace && latest.canSubmit }
    latest.cancel()
    // Image restoration is an input concern; no image is sent to the fake Codex.
    let pixel = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    pixel.bitmapData?.initialize(repeating: 255, count: pixel.bytesPerRow * pixel.pixelsHigh)
    let png = pixel.representation(using: .png, properties: [:])!
    func imageSession(_ base64: String) -> MessageEditingSession {
        var message = source.store.message(id: messageID)!
        message.blocks.append(.image(mediaType: "image/png", source: .data(data: .inline(text: base64))))
        let result = MessageEditingSession(source: source.store, request: .init(message: message, conversationID: conversationID,
            inPlace: true, isLast: false, revision: source.store.editability(of: messageID)?.revision), presentation: .sheet)
        result.placement = .inPlace
        result.start()
        return result
    }
    let imageEdit = imageSession(png.base64EncodedString())
    let restoredImage = imageEdit.editor.attachments[0]
    wait { (try? Data(contentsOf: restoredImage)) == png && imageEdit.canSubmit }
    precondition(restoredImage.path.contains("/message-edits/"))
    _ = capture(MessageEditorView(session: imageEdit, window: workspace, source: source).applicationAppearance(), name: "sheet-original-image", width: 560)
    imageEdit.cancel()
    let unreadable = imageSession("invalid base64 image")
    let unreadableURL = unreadable.editor.attachments[0]
    wait { unreadable.target.issue?.hasPrefix("无法保留原附件") == true }
    precondition(!unreadable.canSubmit && !FileManager.default.fileExists(atPath: unreadableURL.path))
    _ = capture(MessageEditorView(session: unreadable, window: workspace, source: source).applicationAppearance(), name: "sheet-unreadable-image", width: 560)
    unreadable.target.retryAttachment?(unreadableURL)
    precondition(unreadable.target.issue?.hasPrefix("正在保留原附件") == true)
    wait { unreadable.target.issue?.hasPrefix("无法保留原附件") == true }
    precondition(unreadable.editor.attachments == [unreadableURL], "重试必须沿用同一个附件 chip")
    unreadable.editor.removeAttachment(unreadableURL)
    wait { unreadable.canSubmit }
    unreadable.cancel()
    unchanged()
    let filePreview = ConversationWindowModel(application: DeviceStore(preview: true), kind: .workspace)
    let fileMessage = DeliveredMessage(id: "source-file", role: .user, blocks: [.text(text: "保留原文件"), .file(path: draftFile.path)])
    let fileEdit = MessageEditingSession(source: filePreview.focused.store,
        request: .init(message: fileMessage, conversationID: filePreview.focused.store.loadedConversationID,
                       inPlace: true, isLast: true, revision: nil), presentation: .sheet)
    fileEdit.start()
    let restoredFile = fileEdit.editor.attachments[0]
    wait { (try? Data(contentsOf: restoredFile)) == (try? Data(contentsOf: draftFile)) }
    precondition(restoredFile.path != draftFile.path, "编辑须持有原文件的独立副本")
    fileEdit.cancel()
    let edit = session(.sheet)
    preference(.floating)
    let configured = MessageEditingSession(source: source.store, request: edit.request)
    precondition(configured.presentation == .floating && edit.presentation == .sheet, "打开方式设置只影响新编辑器")
    configured.cancel()
    preference(.sheet)
    precondition(edit.placement == .inPlace, "较早消息也应默认在原会话替换后续")
    let initial = capture(MessageEditorView(session: edit, window: workspace, source: source).applicationAppearance(), name: "sheet-initial", width: 560)
    _ = capture(ComposerView(store: source.store, model: ordinary, layout: workspace, send: {}, steer: {},
        submission: { source.submission(using: ordinary) }, resume: {}, fillsHeight: true).applicationAppearance(),
        name: "ordinary-composer", width: 560, height: 240)
    guard let editor = edit.editor.textView else { fatalError("共享原生编辑器未创建") }
    let before = editor.string
    editor.insertText("\n补充：沿用原输入区的字号和控件。", replacementRange: NSRange(location: (editor.string as NSString).length, length: 0))
    precondition(edit.editor.submission == before + "\n补充：沿用原输入区的字号和控件。")
    let context = ComposerContext(tab: edit.tab, model: edit.editor, editingTarget: edit.target)
    context.selectNextTurnRuntime("review")
    precondition(edit.placement == .inPlace && edit.choices.runtimeID == "review")
    context.selectNextTurnRuntime(runtimeID)
    edit.placement = .inPlace
    wait { edit.canSubmit }
    let attachment = home.appendingPathComponent("review-notes.md")
    try! Data("synthetic review notes".utf8).write(to: attachment)
    edit.editor.addAttachments([attachment])
    unchanged()
    _ = capture(MessageEditorView(session: edit, window: workspace, source: source).applicationAppearance(), name: "sheet-ready", width: 560)
    _ = capture(MessageEditorView(session: edit, window: workspace, source: source, floating: true).applicationAppearance(), name: "floating", width: 560, height: 300)
    _ = capture(MessageEditorView(session: edit, window: workspace, source: source, floating: true).applicationAppearance(), name: "floating-minimum", width: 480, height: 180)
    let long = String(repeating: "较长正文应在输入框内滚动，目标选择和发送始终可达。\n", count: 32)
    editor.insertText(long, replacementRange: NSRange(location: 0, length: (editor.string as NSString).length))
    _ = capture(MessageEditorView(session: edit, window: workspace, source: source).applicationAppearance(), name: "sheet-long", width: 560)
    unchanged()
    // A second writer changes native history before this editor can poll it.
    // The production command must reject its captured revision and retain input.
    do {
        _ = try transport.sendTurn(sourceRuntimeID: runtimeID, conversationID: conversationID, targetRuntimeID: runtimeID, selection: nil,
                                   turn: ComposedTurn(text: "CONCURRENT NEW MESSAGE", effort: nil, permissionMode: nil, attachments: []))
    } catch { fatalError(error.localizedDescription) }
    var closed = false
    edit.onClose = { closed = true }
    precondition(!edit.historyChanged, "主线程尚未poll，必须实际覆盖Core先发现变化的竞争")
    edit.send(in: workspace)
    wait { edit.failure != nil }
    precondition(!closed && edit.editor.submission == long && !edit.submitting)
    wait { edit.historyChanged }
    wait { edit.canSubmit }
    precondition(edit.confirmationRevision != nil && !edit.submitting && !closed,
                 "CAS失败重读后应直接提供二次确认，并保留编辑副本")
    let confirmedRevision = edit.confirmationRevision
    _ = capture(MessageEditorView(session: edit, window: workspace, source: source).applicationAppearance(), name: "sheet-conflict", width: 560)
    // A further writer invalidates the offered confirmation instead of silently
    // advancing the CAS baseline under a previously confirmed action.
    do {
        _ = try transport.sendTurn(sourceRuntimeID: runtimeID, conversationID: conversationID, targetRuntimeID: runtimeID, selection: nil,
            turn: ComposedTurn(text: "SECOND CONCURRENT MESSAGE", effort: nil, permissionMode: nil, attachments: []))
    } catch { fatalError("第二次合成写入失败：\(error.localizedDescription)") }
    wait { edit.store.editability(of: messageID)?.revision != confirmedRevision && !edit.store.isGenerating }
    edit.send(in: workspace)
    precondition(edit.confirmationRevision != confirmedRevision && !edit.submitting && !closed,
                 "再次变化必须再次确认，不能无条件覆盖")
    unchanged()
    let fresh = edit
    let ready = capture(MessageEditorView(session: fresh, window: workspace, source: source).applicationAppearance(), name: "sheet-resend", width: 560)
    fresh.editor.textView!.insertText("OFFSCREEN EDITED MESSAGE", replacementRange: NSRange(location: 0, length: (fresh.editor.textView!.string as NSString).length))
    wait { fresh.canSubmit }
    closed = false
    fresh.onClose = { closed = true }
    // A conflict also has a copy button. Address the production submit control
    // explicitly: geometry alone could click Copy and disturb the clipboard.
    let buttons = descendants(ready).compactMap { $0 as? NSButton }
        .filter { !($0 is NSPopUpButton) && !$0.isHiddenOrHasHiddenAncestor }
    let sendButton = buttons.first { $0.accessibilityIdentifier() == "message-editor-submit" }
        ?? buttons.first { $0.toolTip == fresh.target.submitTitle }
    let sentViaNativeButton: Bool
    if let sendButton {
        precondition(sendButton.isEnabled, "可识别的原生发送按钮必须可用")
        sentViaNativeButton = true
        sendButton.performClick(nil)
    } else {
        // An unordered host can withhold native title/AX/tooltip metadata.
        // Use the exact production callback and the same submission gate;
        // never guess a neighbouring Copy button from its geometry.
        precondition(ComposerContext(tab: fresh.tab, model: fresh.editor, editingTarget: fresh.target).submission().canSubmitDraft,
                     "生产发送绑定必须可用")
        sentViaNativeButton = false
        fresh.send(in: workspace)
    }
    wait { closed }
    wait { source.store.transcript.rows.contains { $0.message.blocks.contains { if case .text(let text) = $0 { text == "OFFSCREEN EDITED MESSAGE" } else { false } } } == true && !source.store.isGenerating }
    precondition(source.store.loadedConversationID == conversationID)
    wait { source.store.nextTurn?.runtimeId == runtimeID }
    precondition((source.store.keptComposerChoices.choices[runtimeID] ?? TurnChoice()) == (fresh.choices.choices[runtimeID] ?? TurnChoice()),
                 "接受编辑后，执行配置必须传给继续的普通目标")
    unchanged(checkExecution: false)
    ConversationStore.inspectEditorVisualStates { workspace, session, state in
        let expected = session.editor.submission
        precondition(state == "empty" || !expected.isEmpty, "\(state) 的原编辑内容不能丢失")
        let sheet = capture(MessageEditorView(session: session, window: workspace, source: workspace.focused).applicationAppearance(),
                    name: "sheet-model-\(state)", width: 560)
        let floating = capture(MessageEditorView(session: session, window: workspace, source: workspace.focused, floating: true).applicationAppearance(),
                    name: "floating-model-\(state)", width: 480, height: 180)
        for host in [sheet, floating] {
            let actual = descendants(host).compactMap { $0 as? ComposerTextView }.first?.string
            precondition(actual == expected, "\(state) 的新原生输入框没有显示模型正文")
        }
        session.cancel()
    }
    withExtendedLifetime(initial) {}
    let result: [String: Any] = ["offscreenHostCount": hosts.count, "displayedHosts": hosts.filter(\.isVisible).count,
                               "systemHelperWindows": helperDetails, "activationEvents": activations, "keyWindowEvents": keyWindows,
                               "mainWindowEvents": mainWindows, "systemActivationEvents": systemActivations,
                               "frontmostAtStart": frontmostAtStart, "frontmostAtEnd": NSWorkspace.shared.frontmostApplication?.processIdentifier ?? -1,
                               "renderingMode": "guardedOffscreenHosts", "hiddenWindowTypes": NSApp.windows.map { String(describing: type(of: $0)) }, "actualNativeComposer": true, "draftAndTargetPreserved": true, "staleSubmitRetainsInput": true, "historyReconfirmationRetainsEditor": true, "repeatedChangesRequireConfirmation": true, "defaultInPlace": true, "lateInPlaceDefaultPromoted": true, "explicitForkPreserved": true, "placementBindingAndNativeMenuPresentation": true, "systemPopupInteractionCovered": false, "partialRewindFailurePriority": true,
                               "inPlaceSendViaNativeButton": sentViaNativeButton, "inPlaceSendViaProductionCallback": true, "newHostPreferenceApplied": true,
                               "inlineImagePreserved": true, "originalFileCopied": true,
                               "unreadableOriginalBlocksSend": true, "retryKeepsChipAndRemovalUnblocksSend": true,
                               "renders": measurements]
    try! JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys]).write(to: home.appendingPathComponent("acceptance.json"))
    FileHandle.standardError.write(Data("PASS offscreen native editor: shared composer, sheet/floating content, text/attachments, target/draft isolation, stale rejection and original-conversation send; no presentation or activation\n".utf8))
    safe()
    exit(0)
}

/// The visible path runs a real AppKit loop until an explicit operator marker.
/// The matrix path renders the same production view without ordering windows.
@MainActor func messageEditorVisualCheck(directory: URL) {
    _ = NSApplication.shared
    if CommandLine.arguments.contains("--editor-visual-core") {
        guard let path = ProcessInfo.processInfo.environment["VELUNE_EXPERIENCE_CHECK_HOME"], path.hasPrefix("/tmp/velune-editor-acceptance-") else { preconditionFailure("需要独立合成core fixture") }
        messageEditorOffscreenChecks(home: URL(fileURLWithPath: path))
    }
    let matrix = CommandLine.arguments.contains("--editor-visual-matrix")
    if !matrix && !CommandLine.arguments.contains("--allow-foreground-check") {
        FileHandle.standardError.write(Data("此检查会显示并激活窗口；仅在用户明确要求前台验收时使用 --allow-foreground-check。\n".utf8))
        exit(2)
    }
    NSApplication.shared.setActivationPolicy(matrix ? .prohibited : .regular)
    if !matrix { NSApplication.shared.finishLaunching() }
    var observations: [String] = []
    ConversationStore.inspectEditorVisualStates { workspace, session, state in
        let floating = session.presentation == .floating
        for width: CGFloat in matrix ? [480, 800] : [680] {
            let content: AnyView
            if matrix { content = AnyView(MessageEditorView(session: session, window: workspace, source: workspace.focused, floating: floating)) }
            else { content = AnyView(PaneGrid(window: workspace, previewEmpty: false)) }
            let host = NSHostingView(rootView: content.applicationAppearance().background(Color(nsColor: .windowBackgroundColor)))
            let window = NSWindow(contentRect: NSRect(x: 160, y: 120, width: matrix ? width : 940, height: matrix ? (floating ? 300 : 500) : 780), styleMask: matrix ? [.borderless] : [.titled, .closable, .resizable], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.title = "Velune 编辑配置视觉验收（\(state)，合成数据）"
            window.contentView = host
            @MainActor func settle() { host.layoutSubtreeIfNeeded(); RunLoop.main.run(until: Date().addingTimeInterval(0.12)); host.layoutSubtreeIfNeeded() }
            @MainActor func capture(_ suffix: String) {
                host.displayIfNeeded()
                guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { fatalError("无法绘制编辑器") }
                host.cacheDisplay(in: host.bounds, to: bitmap)
                try! bitmap.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent("editor-\(state)-\(Int(width))-\(suffix).png"))
            }
            if matrix {
                settle(); capture("initial")
                let before = session.editor.textView.map { host.convert($0.bounds, from: $0) }
                if let editor = session.editor.textView, editor.isEditable { precondition(window.makeFirstResponder(editor)) }
                settle(); capture("focus")
                let after = session.editor.textView.map { host.convert($0.bounds, from: $0) }
                if let before, let after { precondition(abs(before.height - after.height) < 1 && abs(before.minY - after.minY) < 1, "编辑器聚焦改变布局：\(state)") }
                if state.hasPrefix("busy") { precondition(session.submitting && !session.canSubmit && !session.target.editable) }
                if state == "missing-runtime" { precondition(session.next == nil && session.target.issue != nil && !session.canSubmit) }
                if state == "missing-model" { precondition(session.next?.chosenModelUnavailable == true && !session.canSubmit) }
                observations.append("\(state) \(Int(width)): focus stable; height \(Int(host.bounds.height)); editable \(session.editor.textView?.isEditable == true)")
                precondition(!window.isVisible)
            } else {
                let marker = directory.appendingPathComponent("continue-editor-\(state)")
                try? FileManager.default.removeItem(at: marker)
                let timer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { timer in
                    MainActor.assumeIsolated {
                        guard FileManager.default.fileExists(atPath: marker.path) else { return }
                        timer.invalidate(); NSApplication.shared.stop(nil)
                        if let event = NSEvent.otherEvent(with: .applicationDefined, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, subtype: 0, data1: 0, data2: 0) { NSApplication.shared.postEvent(event, atStart: false) }
                    }
                }
                let display = Timer.scheduledTimer(withTimeInterval: 0.01, repeats: false) { _ in
                    MainActor.assumeIsolated {
                        NSApplication.shared.unhide(nil); window.orderFrontRegardless(); window.makeKeyAndOrderFront(nil)
                        NSApplication.shared.activate(ignoringOtherApps: true)
                        let source = workspace.focused
                        let identity = session.id
                        session.onClose = { [weak source] in source?.finishMessageEditing(identity) }
                        source.editing = session
                        FileHandle.standardError.write(Data("READY editor \(state) window=\(window.windowNumber) visible=\(window.isVisible) pane=true\n".utf8))
                    }
                }
                NSApplication.shared.run()
                display.invalidate(); timer.invalidate()
                guard workspace.focused.editing == nil else {
                    FileHandle.standardError.write(Data("FAIL editor: close the actual sheet/panel through its Cancel action before finishing\n".utf8)); exit(1)
                }
                window.orderOut(nil)
            }
            window.contentView = nil
        }
    }
    if matrix { try! observations.joined(separator: "\n").write(to: directory.appendingPathComponent("editor-measurements.txt"), atomically: true, encoding: .utf8) }
    FileHandle.standardError.write(Data("PASS editor visual check \(matrix ? "matrix" : "visible")\n".utf8))
}

/// Self-contained native fixture for this check; no installed Harness or account
/// is used. Persisted legacy turns are served through the real Codex adapter.
@MainActor func prepareMessageEditorFixture(home: URL, stateDirectory: URL) throws {
    let peer = home.appendingPathComponent("synthetic-editor-codex")
    let script = #"""
#!/usr/bin/python3
import fcntl, json, pathlib, sys, uuid
root = pathlib.Path(__file__).parent
path = root / 'editor-native.json'
if '--version' in sys.argv:
    print('codex-cli 0.166.0'); sys.exit(0)
def lock_native():
    handle = (root / 'editor-native.lock').open('a')
    fcntl.flock(handle, fcntl.LOCK_EX)
    return handle
def save(db):
    temp = path.with_suffix('.partial'); temp.write_text(json.dumps(db)); temp.replace(path)
def turn(n, text):
    return {'id':'turn-' + str(n), 'status':'completed', 'startedAt':n, 'completedAt':n + 1,
            'items':[{'id':'user-' + str(n), 'type':'userMessage', 'content':[{'type':'text','text':text}]},
                     {'id':'assistant-' + str(n), 'type':'agentMessage', 'text':'Synthetic reply ' + str(n)}]}
with lock_native():
    if not path.exists(): save({'11111111-1111-4111-8111-111111111111':[turn(1,'EDIT THIS EARLIER MESSAGE'),turn(2,'KEEP LATER MESSAGE UNTIL CONFIRMED')]})
def emit(value): print(json.dumps(value), flush=True)
def notify(method, params): emit({'method':method,'params':params})
def thread(db, identity):
    return {'id':identity,'name':'Synthetic editor history','cwd':str(root),'source':'appServer',
            'historyMode':'legacy','createdAt':1,'updatedAt':len(db[identity]) + 1,
            'modelProvider':'openai','status':{'type':'idle'},'turns':db[identity]}
current = '11111111-1111-4111-8111-111111111111'
for line in sys.stdin:
    request = json.loads(line)
    with (root / 'editor-wire.jsonl').open('a') as log: log.write(json.dumps(request) + '\n')
    if 'id' not in request: continue
    with lock_native():
        db = json.loads(path.read_text()); method = request.get('method'); p = request.get('params', {})
        identity = p.get('threadId', current); result = {}
        if method == 'config/read': result = {'config':{'model_provider':'openai','model':'synthetic-editor-model'}}
        elif method == 'model/list':
            result = {'data':[{'id':'synthetic-editor-model','model':'synthetic-editor-model','displayName':'Synthetic editor model',
                               'supportedReasoningEfforts':[],'defaultReasoningEffort':'medium'}]}
        elif method == 'project/list': result = {'data':[], 'nextCursor':None}
        elif method == 'thread/list': result = {'data':[thread(db,key) for key in db], 'nextCursor':None}
        elif method in ('thread/read','thread/resume'):
            current = identity; result = {'thread':thread(db,identity),'model':'synthetic-editor-model'}
        elif method == 'thread/start':
            current = str(uuid.uuid4()); db[current] = []; save(db)
            result = {'thread':thread(db,current),'model':'synthetic-editor-model'}
        elif method == 'thread/revert':
            index = next(i for i,t in enumerate(db[identity]) if t['id'] == p['beforeTurnId'])
            db[identity] = db[identity][:index]; save(db); result = {'thread':thread(db,identity)}
        elif method == 'turn/start':
            n = max([int(t['id'].split('-')[-1]) for t in db[identity]] + [0]) + 1
            text = '\n'.join(i.get('text','') for i in p.get('input',[]) if i.get('type') == 'text')
            value = turn(n,text); value['items'][0]['content'] = p.get('input',[]); db[identity].append(value); save(db)
            emit({'id':request['id'],'result':{'turn':{'id':value['id']}}})
            notify('turn/started',{'threadId':identity,'turn':{'id':value['id']}})
            for item in value['items']:
                notify('item/completed',{'threadId':identity,'turnId':value['id'],'item':item})
            notify('turn/completed',{'threadId':identity,'turn':{'id':value['id'],'status':'completed'}})
            continue
    emit({'id':request['id'],'result':result})
"""#
    try Data(script.utf8).write(to: peer)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: peer.path)
    let transport = Transport(stateDirectory: stateDirectory, resourcesDirectory: Bundle.main.resourceURL!)
    let defaults = try transport.list()
    for runtime in ["editor", "review"] {
        _ = try transport.upsertRuntime(RuntimeInstance(enabled: true, id: runtime, name: "Synthetic " + runtime,
            typeId: .codex166, gatewayId: defaults.gateways.first?.id ?? "default",
            settings: ["binary": peer.path, "nodeBinary": "/usr/bin/false", "agentDir": home.appendingPathComponent("native").path]),
            execution: RuntimeExecution(transport: .local, home: .existing(path: home.path), remoteResourcesDirectory: nil))
    }
    try transport.close(stoppingWork: true)
    let fixture = ["conversationID": "editor:11111111-1111-4111-8111-111111111111", "messageID": "user-1", "runtimeID": "editor"]
    try JSONSerialization.data(withJSONObject: fixture, options: [.sortedKeys]).write(to: home.appendingPathComponent("fixture.json"))
}
#endif
