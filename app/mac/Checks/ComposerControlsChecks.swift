#if VELUNE_DEV_CHECKS
import AppKit
import SwiftUI
import Combine
import VeluneBindings

/// Synthetic, hidden checks: no application transport, scenes, hotkeys or posted input.
@MainActor func composerControlsCheck() -> Never {
    NSApplication.shared.setActivationPolicy(.prohibited)
    func note(_ value: String) { FileHandle.standardError.write(Data((value + "\n").utf8)) }
    func settle() { RunLoop.main.run(until: Date().addingTimeInterval(0.1)) }
    let directory = URL(fileURLWithPath: ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"] ?? "/tmp/velune-next-ux/composer", isDirectory: true)
    try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

    if CommandLine.arguments.contains("--check-geometry") { composerGeometryChecks(directory: directory); exit(0) }

    if CommandLine.arguments.contains("--check-editor-visual") { messageEditorVisualCheck(directory: directory); exit(0) }

    if CommandLine.arguments.contains("--check-visual-states") { composerVisualStateChecks(directory: directory); exit(0) }

    var appearance = AppearanceValues()
    precondition(appearance.composerWidth(available: 420) == 420)
    precondition(appearance.composerWidth(available: 800) == 600)
    precondition(appearance.composerWidth(available: 1600) == 1200)
    precondition(appearance.composerWidth(available: 1600, reserved: 448) == 1152)
    appearance.composerWidthRatio = 0.5
    precondition(appearance.composerWidth(available: 1600) == 800)
    appearance.composerWidthRatio = 1
    precondition(appearance.composerWidth(available: 1600) == 1600)
    let encoded = try! JSONEncoder().encode(appearance)
    precondition(try! JSONDecoder().decode(AppearanceValues.self, from: encoded) == appearance)
    note("PASS composer width: 50/75/100%, narrow fit, symmetric side reservations, ratio round-trip")

    // AppKit owns the real editor and its coordinator. Invoke the view directly;
    // no event enters the system event queue or the user's focused window.
    KeyboardShortcutSettings.shared.load(home: nil) { _ in preconditionFailure("隐藏检查不写快捷键配置") }
    let model = ComposerModel()
    model.restore("synthetic input", attachments: [])
    var sent = 0, steered = 0
    let editor = NSHostingView(rootView: ComposerEditor(model: model, placeholder: "", submit: { sent += 1 }, steer: { steered += 1 }))
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 80), styleMask: [.borderless], backing: .buffered, defer: false)
    window.contentView = editor
    editor.layoutSubtreeIfNeeded(); settle()
    guard let text = model.textView else { fatalError("隐藏原生输入框未创建") }
    func key(_ modifiers: NSEvent.ModifierFlags = []) -> NSEvent {
        NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: modifiers, timestamp: 0,
                        windowNumber: window.windowNumber, context: nil, characters: "\r", charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36)!
    }
    KeyboardShortcutSettings.shared.setSendMode(.returnSends)
    text.keyDown(with: key())
    text.keyDown(with: key(.command))
    text.keyDown(with: key([.command, .option]))
    precondition(sent == 1 && steered == 2)
    KeyboardShortcutSettings.shared.setSendMode(.commandReturnSends)
    text.keyDown(with: key(.command))
    text.keyDown(with: key([.command, .option]))
    precondition(sent == 2 && steered == 3)
    text.setMarkedText("合成组词", selectedRange: NSRange(location: 0, length: 4), replacementRange: text.selectedRange())
    precondition(text.hasMarkedText())
    text.keyDown(with: key(.command))
    precondition(sent == 2 && steered == 3, "输入法组词不能触发提交")
    text.unmarkText()
    text.isEditable = false
    text.keyDown(with: key(.command))
    precondition(sent == 2 && steered == 3, "禁用输入框不能提交")
    note("PASS native composer keys: Return/⌘Return/⌥⌘Return in both modes; marked text and disabled editor preserved")

    // Keep native paste coverage after retiring the old two-owner UI check.
    // A named pasteboard leaves the user's clipboard untouched.
    let pasteStore = ConversationStore(device: DeviceStore(preview: true))
    model.attach(pasteStore); text.isEditable = true
    let beforePaste = model.submission
    let board = NSPasteboard(name: .init("local.velune.composer-check.\(UUID())"))
    defer { board.releaseGlobally() }
    let image = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    image.bitmapData?.initialize(repeating: 0, count: image.bytesPerRow * image.pixelsHigh)
    board.setData(image.representation(using: .png, properties: [:])!, forType: .png)
    board.setString("image fallback", forType: .string)
    precondition(text.readablePasteboardTypes.contains(.png) && text.readSelection(from: board, type: .png) && model.attachments.count == 1)
    let file = directory.appendingPathComponent("synthetic-paste-\(UUID()).txt")
    try! Data("synthetic attachment".utf8).write(to: file)
    board.clearContents(); board.writeObjects([file as NSURL])
    precondition(text.readSelection(from: board, type: .fileURL) && model.attachments.count == 2 && model.submission == beforePaste)
    for attachment in model.attachments { try? FileManager.default.removeItem(at: attachment) }
    note("PASS native PNG and Finder file paste use input attachments without replacing text or the user's clipboard")

    for phase in [ComposerSubmission.Phase.sending, .resuming, .stopping, .blocked] {
        let submission = ComposerSubmission(phase: phase, hasInput: true, isComposing: false, canSend: true, canQueue: true, canResume: true)
        precondition(!submission.canSubmitDraft)
    }
    precondition(!ComposerSubmission(phase: .running, hasInput: true, isComposing: true, canSend: true, canQueue: true, canResume: false).canSubmitDraft)
    precondition(!ComposerSubmission(phase: .running, hasInput: true, isComposing: false, canSend: true, canQueue: false, canResume: false).canSubmitDraft)
    precondition(!ComposerSubmission(phase: .running, hasInput: false, isComposing: false, canSend: true, canQueue: true, canResume: false).canSubmitDraft)
    note("PASS submission admission: pending/stopping/blocked/composing/no-queue/empty states reject draft submission")

    ConversationStore.checkPendingSendCancellation { workspace in
        let store = workspace.focused.store
        let composer = ComposerView(store: store, model: workspace.composer(for: workspace.focused), layout: workspace,
            send: {}, steer: {}, submission: { ComposerSubmission(phase: .sending, hasInput: true, isComposing: false, canSend: false, canQueue: false, canResume: false) }, resume: {})
        let host = NSHostingView(rootView: composer.applicationAppearance())
        let pendingWindow = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 800, height: 230), styleMask: [.borderless], backing: .buffered, defer: false)
        pendingWindow.contentView = host
        host.layoutSubtreeIfNeeded(); settle(); host.displayIfNeeded()
        if let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) {
            host.cacheDisplay(in: host.bounds, to: bitmap)
            try! bitmap.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent("pending-send.png"))
        }
        var nativeNodes: [String] = []
        @MainActor func findStop(_ node: Any) -> (enabled: Bool, press: @MainActor () -> Bool)? {
            var visited = Set<ObjectIdentifier>()
            @MainActor func find(_ node: Any) -> (enabled: Bool, press: @MainActor () -> Bool)? {
                guard let object = node as? NSObject, visited.insert(ObjectIdentifier(object)).inserted else { return nil }
                if let button = node as? NSButton {
                    nativeNodes.append("button: \(button.title)")
                    if button.title == "停止" { return (button.isEnabled, { button.performClick(nil); return true }) }
                }
                if let element = node as? NSAccessibilityProtocol {
                    nativeNodes.append("\(type(of: object)): \(element.accessibilityIdentifier() ?? "-") / \(element.accessibilityLabel() ?? "-")")
                    if element.accessibilityIdentifier() == "composer-stop" { return (element.isAccessibilityEnabled(), { element.accessibilityPerformPress() }) }
                    for child in element.accessibilityChildren() ?? [] { if let found = find(child) { return found } }
                }
                if let view = node as? NSView {
                    for child in view.subviews { if let found = find(child) { return found } }
                }
                return nil
            }
            return find(node)
        }
        // In an unshown SwiftUI host the label is drawn inside the button and
        // neither its title nor virtual AX identifier is exposed. The status
        // row has stop immediately before send; use their actual native frames.
        @MainActor func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        let trailingButtons = descendants(host).compactMap { $0 as? NSButton }
            .filter { !($0 is NSPopUpButton) && !$0.isHiddenOrHasHiddenAncestor }
            .sorted { host.convert($0.bounds, from: $0).midX < host.convert($1.bounds, from: $1).midX }
        let nativeStop: (enabled: Bool, press: @MainActor () -> Bool)? = trailingButtons.count >= 2 ? {
            let button = trailingButtons[trailingButtons.count - 2]
            return (button.isEnabled, { button.performClick(nil); return true })
        }() : nil
        guard let stop = findStop(host) ?? nativeStop else { fatalError("未找到发送等待态的原生停止按钮：\(nativeNodes.joined(separator: " | "))") }
        precondition(stop.enabled, "发送等待态停止按钮不能被isLoading禁用")
        precondition(stop.press(), "原生停止按钮不能执行取消")
        precondition(!pendingWindow.isVisible)
    }

    ConversationStore.checkSubAgentControls { workspace, child in
        let context = workspace.composerContext(for: workspace.focused)
        let store = context.store
        let composer = ComposerView(store: store, model: context.model, layout: workspace,
            send: context.send, steer: context.steer, submission: { context.submission() }, resume: context.resume)
        let content = VStack {
            if !child { LinkedAgentsView(store: store, inPopover: false, open: { _, _ in }).frame(height: 220) }
            composer
        }.applicationAppearance().background(Color(nsColor: .windowBackgroundColor))
        let host = NSHostingView(rootView: content)
        let native = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 800, height: child ? 160 : 390), styleMask: [.borderless], backing: .buffered, defer: false)
        native.contentView = host
        host.layoutSubtreeIfNeeded(); settle(); host.layoutSubtreeIfNeeded(); host.displayIfNeeded()
        if !child && store.inputAccess == .readOnly {
            guard let editor = context.model.textView, editor.isEditable else {
                fatalError("只读普通会话必须保留可编辑的原生草稿输入框")
            }
            editor.insertText("停留在相同会话的草稿", replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
            precondition(context.model.submission == "停留在相同会话的草稿")
            precondition(!context.submission().canSubmitDraft)
            context.send()
            precondition(context.model.submission == "停留在相同会话的草稿" && store.composerRequest == nil)
            note("PASS native composer edits a read-only parent draft and keeps submission blocked")
            return
        }
        guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { fatalError("无法绘制Pi子会话检查") }
        host.cacheDisplay(in: host.bounds, to: bitmap)
        try! bitmap.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent(child ? "pi-child-controls.png" : "pi-parent-controls.png"))
        precondition(!native.isVisible)
    }

    let device = DeviceStore(preview: true)
    let workspace = ConversationWindowModel(application: device, kind: .workspace)
    let input = workspace.composer(for: workspace.focused)
    let beforeSelection = ModelSelection.managed(modelRecordKey: "sample-model")
    let target = ComposerContext(tab: workspace.focused, model: input)
    target.selectModel(beforeSelection)
    let beforeRequest = ComposerContext(tab: workspace.focused, model: input).composerQuotaRequest
    precondition(beforeRequest.selection == beforeSelection)
    let afterSelection = ModelSelection.native(providerId: "sample", modelId: "sample-native")
    target.selectModel(afterSelection)
    let afterRequest = ComposerContext(tab: workspace.focused, model: input).composerQuotaRequest
    precondition(afterRequest.selection == afterSelection && beforeRequest != afterRequest)
    device.invalidateProviderReports()
    let reboundRequest = ComposerContext(tab: workspace.focused, model: input).composerQuotaRequest
    precondition(reboundRequest != afterRequest && reboundRequest.selection == afterSelection,
                 "换绑应失效旧额度请求并保留当前模型")
    note("PASS quota request follows model selection and invalidates a prior provider binding")
    composerQuotaPresentationChecks()

    let observation = QuotaObservation(freshness: .fresh, windows: [
        QuotaWindowObservation(id: "kimi:limit_5h", modelIds: nil, usedPercent: 35, resetAtMs: 1_800_000_000_000, windowMs: 18_000_000, observedAtMs: 1_799_999_000_000, expiresAtMs: 1_800_000_000_000, pace: nil),
        QuotaWindowObservation(id: "kimi:limit_7d", modelIds: nil, usedPercent: 20, resetAtMs: 1_800_500_000_000, windowMs: 604_800_000, observedAtMs: 1_799_999_000_000, expiresAtMs: 1_800_000_000_000, pace: nil)
    ], balances: [], available: true, refreshing: false, problem: nil)
    let account = ComposerQuotaEntry(id: "provider:synthetic:kimi", name: "Kimi For Coding", origin: .provider, observation: observation, limits: [], credits: nil)
    let details = NSHostingView(rootView: StatusDetails(status: ConversationStatus(), accounts: [account], branch: nil, display: .used, quotaDisplay: .remaining)
        .applicationAppearance().background(Color(nsColor: .windowBackgroundColor)))
    let detailWindow = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 380, height: 520), styleMask: [.borderless], backing: .buffered, defer: false)
    detailWindow.contentView = details
    details.layoutSubtreeIfNeeded(); settle()
    details.layoutSubtreeIfNeeded(); details.displayIfNeeded()
    if let bitmap = details.bitmapImageRepForCachingDisplay(in: details.bounds) {
        details.cacheDisplay(in: details.bounds, to: bitmap)
        try! bitmap.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent("selected-provider-quota.png"))
    } else { fatalError("无法绘制合成额度详情") }
    precondition(!window.isVisible && !detailWindow.isVisible && NSApp.activationPolicy() == .prohibited)
    note("PASS generic quota details rendered with synthetic Kimi 5h/7d observations; no visible windows or application transport")
    exit(0)
}

@MainActor private func composerQuotaPresentationChecks() {
    let windows = [
        QuotaWindowObservation(id: "5h", modelIds: nil, usedPercent: 54, resetAtMs: 1_800_003_600_000,
                               windowMs: 18_000_000, observedAtMs: 1_800_000_000_000, expiresAtMs: nil, pace: nil),
        QuotaWindowObservation(id: "opus", modelIds: ["opus"], usedPercent: 20, resetAtMs: nil,
                               windowMs: nil, observedAtMs: 1_800_000_000_000, expiresAtMs: nil, pace: nil),
    ]
    let observation = QuotaObservation(freshness: .fresh, windows: windows, balances: [],
                                       available: true, refreshing: false, problem: nil)
    let native = ComposerQuotaEntry(id: "login", name: "运行时", origin: .runtimeLogin, observation: observation,
                                    limits: [QuotaLimit(windowId: "5h", maxUsedPercent: 80)], credits: nil)
    precondition(native.windows == windows && native.limit(for: "5h") == 80 && native.limit(for: "opus") == nil)
    precondition(native.title == "运行时的登录额度" && !native.isRecorded)
    let recorded = ComposerQuotaEntry(id: "record", name: "运行时", origin: .conversationRecord,
                                      observation: observation, limits: [], credits: nil)
    precondition(recorded.isRecorded && recorded.title == "运行时会话记录中的限额")

    let zone = TimeZone(secondsFromGMT: 8 * 3600)!
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = zone
    let reset = calendar.date(from: DateComponents(year: 2026, month: 11, day: 3, hour: 9, minute: 7))!
    let text = QuotaResetFormat.text(reset, timeZone: zone)
    precondition(text.contains("2026") && text.contains("11") && text.contains("03") && text.contains("09:07"), text)
    precondition(QuotaResetFormat.text(reset, timeZone: TimeZone(secondsFromGMT: 0)!).contains("01:07"), "Use the viewer's local timezone")
    precondition(QuotaResetFormat.text(nil) == "重置时间未知")

    let balance = ProviderAccountCredits(hasCredits: true, unlimited: false, balance: "12.7500")
    let zero = ProviderAccountCredits(hasCredits: false, unlimited: false, balance: "0")
    let unlimited = ProviderAccountCredits(hasCredits: true, unlimited: true, balance: nil)
    let fixtures: [(String, ComposerQuotaCredits, String, String)] = [
        ("balance", .init(observation: balance, stale: false, refreshing: false, failed: false), "12.7500", "12.7500 credits"),
        ("zero", .init(observation: zero, stale: false, refreshing: false, failed: false), "0", "0 credits"),
        ("unlimited", .init(observation: unlimited, stale: false, refreshing: false, failed: false), "不限量", "credits 不限量"),
        ("stale-unlimited", .init(observation: unlimited, stale: true, refreshing: false, failed: false), "不限量（上次余额）", "credits 不限量（上次余额）"),
        ("stale", .init(observation: balance, stale: true, refreshing: true, failed: false), "12.7500（上次余额）", "12.7500 credits（上次）"),
        ("loading", .init(observation: nil, stale: false, refreshing: true, failed: false), "读取中…", "credits 读取中…"),
        ("unknown", .init(observation: nil, stale: false, refreshing: false, failed: false), "未报告", "credits 未报告"),
        ("failed", .init(observation: nil, stale: false, refreshing: false, failed: true), "暂时无法读取", "credits 暂时无法读取")
    ]
    for (name, credits, detail, compact) in fixtures {
        precondition(credits.detailValue == detail && credits.compactValue == compact, "credits presentation: \(name)")
    }
    FileHandle.standardError.write(Data("PASS quota origin, limits, local reset times and credits presentation\n".utf8))
}

/// Exercise the production inline editor over repeated layout/run-loop passes.
@MainActor private func composerGeometryChecks(directory: URL) {
    let device = DeviceStore(preview: true)
    let workspace = ConversationWindowModel(application: device, kind: .workspace)
    let model = workspace.composer(for: workspace.focused)
    let composer = ComposerView(store: workspace.focused.store, model: model, layout: workspace,
        send: {}, steer: {}, submission: { workspace.focused.submission(using: model) }, resume: {})
    let host = NSHostingView(rootView: VStack(spacing: 0) { Spacer(minLength: 0); composer }
        .applicationAppearance().background(Color(nsColor: .windowBackgroundColor)))
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 800, height: 600), styleMask: [.borderless], backing: .buffered, defer: false)
    window.contentView = host
    var observations: [String] = []
    var failures: [String] = []
    var frameChanges = 0
    var primaryText: ComposerTextView?
    var settledHeights: [String: CGFloat] = [:]

    func capture(_ phase: String, host: NSView) {
        // Hidden SwiftUI hosts may commit their first backing layer only after
        // a display pass. Flush it before reading the offscreen bitmap.
        host.layoutSubtreeIfNeeded(); host.display()
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        host.display()
        guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { fatalError("Missing native geometry bitmap") }
        host.cacheDisplay(in: host.bounds, to: bitmap)
        try! bitmap.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent("geometry-\(phase).png"))
    }
    @MainActor func findEditor(_ view: NSView) -> ComposerTextView? {
        if let text = view as? ComposerTextView { return text }
        return view.subviews.lazy.compactMap { findEditor($0) }.first
    }
    func sample(_ phase: String, secondHost: NSView? = nil, secondText: ComposerTextView? = nil, secondChanges: () -> Int = { 0 }) {
        var settled: [String: [(height: CGFloat, y: CGFloat, expected: CGFloat)]] = [:]
        let initialChanges = frameChanges
        let initialSecondChanges = secondChanges()
        var warmChanges = frameChanges
        var warmSecondChanges = secondChanges()
        for pass in 0..<30 {
            host.layoutSubtreeIfNeeded()
            secondHost?.layoutSubtreeIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.015))
            func record(_ text: ComposerTextView, host: NSView, phase: String, changes: Int) {
                guard let scroll = text.enclosingScrollView, let container = text.textContainer, let manager = text.layoutManager else { fatalError("Missing production native scroll/text layout") }
                manager.ensureLayout(for: container)
                let usedHeight = manager.usedRect(for: container).height
                let expected = min(max(ceil(usedHeight + text.textContainerInset.height * 2), ComposerTextView.minimumHeight), ComposerTextView.maximumHeight)
                let y = host.convert(scroll.bounds, from: scroll).minY
                observations.append("\(phase),\(pass),\(scroll.frame.height),\(scroll.contentSize.width),\(text.frame.width),\(container.containerSize.width),\(usedHeight),\(y),\(changes)")
                if pass >= 10 { settled[phase, default: []].append((scroll.frame.height, y, expected)) }
            }
            if let text = primaryText ?? model.textView { record(text, host: host, phase: phase, changes: frameChanges - initialChanges) }
            if let secondHost, let secondText { record(secondText, host: secondHost, phase: phase + "-secondary", changes: secondChanges() - initialSecondChanges) }
            if pass == 9 { warmChanges = frameChanges; warmSecondChanges = secondChanges() }
        }
        for (name, frames) in settled {
            guard let last = frames.last else { continue }
            settledHeights[name] = last.height
            if frames.contains(where: { abs($0.height - last.height) > 0.5 || abs($0.y - last.y) > 0.5 }) {
                failures.append("\(name): geometry kept changing after initial settling")
            }
            if abs(last.height - last.expected) > 0.5 { failures.append("\(name): viewport \(last.height) disagrees with content \(last.expected)") }
        }
        if frameChanges - warmChanges > 2 { failures.append("\(phase): \(frameChanges - warmChanges) late native frame changes") }
        if secondChanges() - warmSecondChanges > 2 { failures.append("\(phase): \(secondChanges() - warmSecondChanges) late secondary native frame changes") }
        if ["typed-3", "deleted-1", "paste-long", "width-380"].contains(phase) { capture(phase, host: host) }
    }
    sample("empty")
    guard let text = model.textView, let scroll = text.enclosingScrollView else { fatalError("Missing inline production editor") }
    primaryText = text
    scroll.postsFrameChangedNotifications = true
    let observation = NotificationCenter.default.publisher(for: NSView.frameDidChangeNotification, object: scroll).sink { _ in frameChanges += 1 }
    precondition(window.makeFirstResponder(text))
    for line in 1...5 {
        if line > 1 { text.insertText("\n", replacementRange: text.selectedRange()); sample("newline-\(line)") }
        for character in line == 1 ? "first line" : "line \(line)" {
            text.insertText(String(character), replacementRange: text.selectedRange())
            RunLoop.main.run(until: Date().addingTimeInterval(0.005))
        }
        sample("typed-\(line)")
    }
    for line in (1...4).reversed() {
        let string = text.string as NSString
        let range = string.range(of: "\n", options: .backwards)
        text.setSelectedRange(NSRange(location: range.location, length: string.length - range.location))
        text.insertText("", replacementRange: text.selectedRange())
        sample("deleted-\(line)")
    }
    let board = NSPasteboard(name: .init("local.velune.geometry.\(UUID())"))
    defer { board.releaseGlobally() }
    board.setString(String(repeating: "Long synthetic wrapped content with native paste. ", count: 160), forType: .string)
    precondition(text.readSelection(from: board, type: .string))
    sample("paste-long")
    for width: CGFloat in [380, 1000, 520, 800] {
        window.setContentSize(NSSize(width: width, height: 600))
        sample("width-\(Int(width))")
    }
    window.makeFirstResponder(nil); sample("blur")
    window.makeFirstResponder(text); sample("focus")
    // A sheet or another pane may render the same input at another width.
    // Both visible hosts need their own fitting height, regardless of focus.
    model.clear(submitted: model.submission, attachments: [])
    text.insertText(String(repeating: "native wrapped words ", count: 12), replacementRange: text.selectedRange())
    sample("before-second-host")
    let secondHost = NSHostingView(rootView: composer.applicationAppearance())
    let secondWindow = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 360, height: 600), styleMask: [.borderless], backing: .buffered, defer: false)
    secondWindow.contentView = secondHost
    secondHost.layoutSubtreeIfNeeded()
    guard let secondText = findEditor(secondHost), let secondScroll = secondText.enclosingScrollView else { fatalError("Missing second production editor") }
    var secondaryFrameChanges = 0
    secondScroll.postsFrameChangedNotifications = true
    let secondaryObservation = NotificationCenter.default.publisher(for: NSView.frameDidChangeNotification, object: secondScroll).sink { _ in secondaryFrameChanges += 1 }
    for width: CGFloat in [360, 520, 420] {
        secondWindow.setContentSize(NSSize(width: width, height: 600))
        sample("second-host-\(Int(width))", secondHost: secondHost, secondText: secondText, secondChanges: { secondaryFrameChanges })
    }
    precondition(secondWindow.makeFirstResponder(secondText))
    sample("second-host-focused", secondHost: secondHost, secondText: secondText, secondChanges: { secondaryFrameChanges })
    precondition(window.makeFirstResponder(text))
    sample("first-host-refocused", secondHost: secondHost, secondText: secondText, secondChanges: { secondaryFrameChanges })
    capture("second-host-primary", host: host); capture("second-host-secondary", host: secondHost)
    secondWindow.contentView = nil
    sample("second-host-removed")
    let csv = "phase,pass,viewportHeight,viewportWidth,textWidth,containerWidth,usedHeight,viewportY,frameChanges\n" + observations.joined(separator: "\n")
    try! csv.write(to: directory.appendingPathComponent("geometry.csv"), atomically: true, encoding: .utf8)
    precondition(!window.isVisible && !secondWindow.isVisible && NSApp.activationPolicy() == .prohibited)
    _ = observation; _ = secondaryObservation
    if (settledHeights["typed-3"] ?? 0) <= (settledHeights["typed-1"] ?? 0) { failures.append("Third typed line did not grow inline editor") }
    if settledHeights["paste-long"] != ComposerTextView.maximumHeight { failures.append("Long native paste did not reach bounded scrolling height") }
    if (settledHeights["second-host-360-secondary"] ?? 0) <= (settledHeights["second-host-360"] ?? 0) { failures.append("Narrow simultaneous host did not get its own larger fitting height") }
    try! failures.joined(separator: "\n").write(to: directory.appendingPathComponent("geometry-failures.txt"), atomically: true, encoding: .utf8)
    if !failures.isEmpty {
        FileHandle.standardError.write(Data((failures.joined(separator: "\n") + "\n").utf8))
        exit(1)
    }
    FileHandle.standardError.write(Data("PASS inline native geometry: per-character typing/newlines/deletion/native paste/width/focus; simultaneous hosts fit independently at three widths, 30 layout passes per phase, no late native frame churn\n".utf8))
    window.contentView = nil
}
#endif
