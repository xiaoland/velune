import AppKit
import SwiftUI

/// Offscreen production-view coverage. Events enter the same local handler as
/// the mouse monitor; this does not post system mouse events or exercise AX.
/// The caller supplies two selected tabs in separate groups, synthetic input
/// and execution choices, and a process already using prohibited activation.
@MainActor func composerGroupPresentationChecks(window: ConversationWindowModel, left: ConversationTab, right: ConversationTab) {
    precondition(NSApp.activationPolicy() == .prohibited, "离屏呈现检查必须使用 prohibited 激活策略")
    precondition(window.tools[.composer].place == .docked(.bottom), "此呈现检查覆盖底部共享输入面板")
    guard let output = ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"], !output.isEmpty,
          let leftPane = window.pane(holding: left.id), let rightPane = window.pane(holding: right.id) else {
        fatalError("需要 VELUNE_CHECK_OUTPUT 与两个已挂载的会话组")
    }
    precondition(leftPane.id != rightPane.id && leftPane.selectedID == left.id && rightPane.selectedID == right.id,
                 "两个样本 Tab 必须分别是各自组的当前选择")
    let leftInput = window.composer(for: left), rightInput = window.composer(for: right)
    let leftDraft = leftInput.savedDraft, rightDraft = rightInput.savedDraft
    let leftContext = window.composerContext(for: left), rightContext = window.composerContext(for: right)
    precondition(leftInput !== rightInput && leftDraft.text != rightDraft.text, "需要可区分的独立输入正文")
    precondition(leftContext.nextTurnRuntimeID != nil && rightContext.nextTurnRuntimeID != nil
                 && leftContext.nextTurnRuntimeID != rightContext.nextTurnRuntimeID,
                 "需要两个不同且已准备完成的执行目标")
    for attachment in leftInput.attachments + rightInput.attachments {
        precondition(FileManager.default.isReadableFile(atPath: attachment.path), "样本附件必须是可读的合成文件")
    }
    let directory = URL(fileURLWithPath: output, isDirectory: true)
    try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

    // No substitute anchors: PaneView and TabContentView register their own
    // production PaneAnchor only when this hierarchy moves into the NSWindow.
    let host = NSHostingView(rootView:
        VStack(spacing: 0) {
            PaneGrid(window: window, previewEmpty: false)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            Divider()
            ToolContent(kind: .composer, window: window, side: .bottom)
                .frame(height: 250)
        }
        .applicationAppearance()
        .background(Color(nsColor: .windowBackgroundColor)))
    let native = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 900),
                          styleMask: [.borderless], backing: .buffered, defer: false)
    native.isReleasedWhenClosed = false
    native.contentView = host
    defer { native.contentView = nil }

    func assertOffscreen() {
        precondition(NSApp.activationPolicy() == .prohibited && !native.isVisible && !native.isKeyWindow && !native.isMainWindow,
                     "呈现检查不得显示或激活宿主窗口")
    }
    func settle() {
        assertOffscreen()
        host.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.02))
        host.layoutSubtreeIfNeeded()
        assertOffscreen()
    }
    func wait(_ message: String, until condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(5)
        repeat {
            settle()
            if condition() { return }
        } while Date() < deadline
        preconditionFailure(message)
    }
    func editors(in view: NSView) -> [ComposerTextView] {
        (view as? ComposerTextView).map { [$0] } ?? view.subviews.flatMap { editors(in: $0) }
    }
    func liveAnchor(_ pane: Pane) -> NSView? {
        guard let anchor = window.view(of: pane.id), anchor.window === native,
              anchor.isDescendant(of: host), anchor.bounds.width > 100, anchor.bounds.height > 100 else { return nil }
        return anchor
    }
    wait("生产 PaneAnchor 或 ComposerTextView 未进入离屏宿主") {
        liveAnchor(leftPane) != nil && liveAnchor(rightPane) != nil && editors(in: host).count == 1
    }
    precondition(window.window === native, "窗口命中测试必须读取实际生产视图注册的锚点")
    var observations = [
        "覆盖：真实 PaneGrid + ToolContent(.composer) 的离屏 NSHostingView/NSWindow；1280×900。",
        "输入：构造 NSEvent，仅调用生产 handleMouseDown 本地事件入口；没有物理鼠标、系统事件派发或 AX 操作。",
        "窗口：prohibited；未调用 orderFront、makeKey、show 或激活应用。",
    ]

    func selectAndCheck(_ tab: ConversationTab, pane: Pane, label: String, expected: ComposerContext) {
        guard let anchor = liveAnchor(pane) else { fatalError("生产组锚点已失效") }
        let location = anchor.convert(NSPoint(x: anchor.bounds.midX, y: anchor.bounds.midY), to: nil)
        guard let event = NSEvent.mouseEvent(with: .leftMouseDown, location: location, modifierFlags: [],
                                            timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: native.windowNumber,
                                            context: nil, eventNumber: 1, clickCount: 1, pressure: 1) else {
            fatalError("无法构造本地组点击事件")
        }
        precondition(event.window === native && anchor.bounds.contains(anchor.convert(event.locationInWindow, from: nil)),
                     "构造事件必须属于离屏宿主且命中真实生产锚点")
        precondition(window.handleMouseDown(event) === event, "普通组点击应保留事件继续传递")
        precondition(window.focusedPaneID == pane.id && window.focused === tab, "本地左键入口没有切换到目的组")
        wait("SwiftUI/AppKit 输入绑定未跟随组焦点") {
            let mounted = editors(in: host)
            guard mounted.count == 1, let editor = mounted.first,
                  let coordinator = editor.delegate as? ComposerEditor.Coordinator else { return false }
            return editor.window === native && coordinator.model === expected.model
                && editor.string == expected.model.submission && expected.model.textView === editor
        }
        let actual = window.composerContext()
        precondition(actual.tab === tab && actual.model === expected.model
                     && actual.nextTurnRuntimeID == expected.nextTurnRuntimeID
                     && actual.nextTurnModel == expected.nextTurnModel
                     && actual.chosenModel == expected.chosenModel
                     && actual.nextTurnEffort == expected.nextTurnEffort,
                     "目的组的执行配置没有恢复")
        precondition(leftInput.savedDraft == leftDraft && rightInput.savedDraft == rightDraft,
                     "切换组不得改变任一输入正文或附件")
        guard let editor = editors(in: host).first else { fatalError("缺少生产原生编辑器") }
        let editorFrame = host.convert(editor.bounds, from: editor)
        precondition(editorFrame.width > 100 && editorFrame.height > 0 && host.bounds.intersects(editorFrame),
                     "原生编辑器必须实际排版在完整布局中")
        host.displayIfNeeded()
        guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { fatalError("无法生成完整布局位图") }
        host.cacheDisplay(in: host.bounds, to: bitmap)
        guard let png = bitmap.representation(using: .png, properties: [:]) else { fatalError("无法编码完整布局 PNG") }
        try! png.write(to: directory.appendingPathComponent("composer-group-\(label).png"))
        observations.append("\(label)：本地组点击 → focused Tab \(tab.id)；生产 Coordinator.model 与输入对象一致；原生正文 \(editor.string.count) 字，附件 \(actual.model.attachments.count)；runtime=\(actual.nextTurnRuntimeID ?? "nil")，effort=\(actual.nextTurnEffort ?? "default")。")
        assertOffscreen()
    }

    // Visit both sides twice so the checks prove restoration after departure,
    // rather than only the state of the first mounted editor.
    for _ in 0..<2 {
        selectAndCheck(left, pane: leftPane, label: "left", expected: leftContext)
        selectAndCheck(right, pane: rightPane, label: "right", expected: rightContext)
    }
    try! observations.joined(separator: "\n").write(to: directory.appendingPathComponent("composer-group-presentation.txt"), atomically: true, encoding: .utf8)
    FileHandle.standardError.write(Data("PASS offscreen production pane clicks restore native composer text, attachments and execution choices across groups; left/right layout PNGs saved (local event handler coverage, not system mouse or AX)\n".utf8))
}
