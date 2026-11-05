import AppKit
import SwiftUI
import VeluneBindings

/// Synthetic, hidden checks: no application transport, scenes, hotkeys or posted input.
@MainActor func composerControlsCheck() -> Never {
    NSApplication.shared.setActivationPolicy(.prohibited)
    func note(_ value: String) { FileHandle.standardError.write(Data((value + "\n").utf8)) }
    func settle() { RunLoop.main.run(until: Date().addingTimeInterval(0.1)) }
    let directory = URL(fileURLWithPath: ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"] ?? "/tmp/velune-next-ux/composer", isDirectory: true)
    try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

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

    let device = DeviceStore(preview: true)
    let workspace = ConversationWindowModel(application: device, kind: .workspace)
    let input = workspace.composer(for: workspace.focused)
    let beforeSelection = ModelSelection.managed(modelRecordKey: "sample-model")
    var target = ComposerContext(tab: workspace.focused, model: input)
    target.prepare()
    target.selectModel(beforeSelection)
    target = ComposerContext(tab: workspace.focused, model: input)
    let beforeRequest = target.composerQuotaRequest
    precondition(beforeRequest.selection == beforeSelection)
    let afterSelection = ModelSelection.native(providerId: "sample", modelId: "sample-native")
    target.selectModel(afterSelection)
    let afterRequest = ComposerContext(tab: workspace.focused, model: input).composerQuotaRequest
    precondition(afterRequest.selection == afterSelection && beforeRequest != afterRequest)
    note("PASS quota request follows the current target model selection without carrying an earlier account request")

    let observation = QuotaObservation(freshness: .fresh, windows: [
        QuotaWindowObservation(id: "kimi:limit_5h", modelIds: nil, usedPercent: 35, resetAtMs: 1_800_000_000_000, windowMs: 18_000_000, observedAtMs: 1_799_999_000_000, expiresAtMs: 1_800_000_000_000, pace: nil),
        QuotaWindowObservation(id: "kimi:limit_7d", modelIds: nil, usedPercent: 20, resetAtMs: 1_800_500_000_000, windowMs: 604_800_000, observedAtMs: 1_799_999_000_000, expiresAtMs: 1_800_000_000_000, pace: nil)
    ], balances: [], available: true, refreshing: false, problem: nil)
    let account = ComposerQuotaEntry(id: "provider:synthetic:kimi", name: "Kimi For Coding", origin: .provider, observation: observation, limits: [])
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
