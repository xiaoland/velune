#if VELUNE_DEV_CHECKS
import AppKit
import SwiftUI
import VeluneBindings

/// Real pane composition and native responder transitions, with synthetic data.
@MainActor func runtimeInteractionChecks() -> Never {
    runtimeRecoveryLifecycleChecks()
    let visible = CommandLine.arguments.contains("--interaction-check-visible")
    guard !visible || CommandLine.arguments.contains("--allow-foreground-check") else {
        FileHandle.standardError.write(Data("可见检查会激活应用；仅在用户明确要求前台验收时使用 --allow-foreground-check。\n".utf8))
        exit(2)
    }
    let visibleCase = ProcessInfo.processInfo.environment["VELUNE_VISUAL_CASE"] ?? "async"
    NSApplication.shared.setActivationPolicy(visible ? .regular : .prohibited)
    if CommandLine.arguments.contains("--interaction-check-dark") { NSApplication.shared.appearance = NSAppearance(named: .darkAqua) }
    if visible { NSApplication.shared.finishLaunching() }
    let directory = URL(fileURLWithPath: ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"] ?? "/tmp/velune-interaction-visual", isDirectory: true)
    try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    func settle() { RunLoop.main.run(until: Date().addingTimeInterval(0.12)) }
    @MainActor func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    @MainActor func capture(_ host: NSView, _ name: String) {
        host.layoutSubtreeIfNeeded(); settle(); host.layoutSubtreeIfNeeded(); host.displayIfNeeded()
        let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds)!
        host.cacheDisplay(in: host.bounds, to: bitmap)
        try! bitmap.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent(name + ".png"))
    }
    var lifecycleWindow: NSWindow?
    var lifecycleHost: NSHostingView<AnyView>?
    ConversationStore.checkInteractionDraftLifetime { workspace, phase in
        if lifecycleHost == nil {
            let content = PaneLayoutView(window: workspace, layout: workspace.layout, path: [], previewEmpty: false).applicationAppearance().background(Color(nsColor: .windowBackgroundColor))
            let host = NSHostingView(rootView: AnyView(content))
            let native = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 800, height: 620), styleMask: [.borderless], backing: .buffered, defer: false)
            native.contentView = host
            lifecycleWindow = native; lifecycleHost = host
        }
        guard let host = lifecycleHost else { preconditionFailure() }
        capture(host, "lifetime-" + phase)
        if ["returned", "context-returned", "failed"].contains(phase) {
            let editors = descendants(host).compactMap { $0 as? NSTextView }
            precondition(editors.contains { $0.string == "切换标签后仍保留的回答\n第二行" }, "真实Pane重新挂载后应显示已填写的回答和所在题目")
        }
    }
    lifecycleWindow?.contentView = nil
    lifecycleHost = nil; lifecycleWindow = nil
    let questions = [
        InteractionQuestion(id: "scope", text: "这次先处理哪一项？", options: [InteractionOption(id: "runtime", label: "运行时与会话历史（推荐）"), InteractionOption(id: "workspace", label: "工作区与标签页体验")], secret: false),
        InteractionQuestion(id: "notes", text: "有哪些需要保留的约束？", options: [], secret: false, defaultText: "保留已有会话和附件\n先验证聚焦、输入与提交后的状态", multiline: true),
        InteractionQuestion(id: "checks", text: "需要覆盖哪些检查？（可多选）", options: [InteractionOption(id: "core", label: "核心行为与历史恢复"), InteractionOption(id: "ui", label: "完整窗口中的原生界面")], secret: false, allowOther: true, multiple: true)
    ]
    let variants: [(String, InteractionKind, Bool)] = [
        ("async", .asyncUserInput(questions: questions, delivery: .conversationInput), false),
        ("sync", .userInput(questions: [InteractionQuestion(id: "text", text: "请输入项目名称", options: [], secret: false)]), false),
        ("closed-default", .userInput(questions: [
            InteractionQuestion(id: "closed", text: "只允许从以下选项中选择", options: [InteractionOption(id: "yes", label: "允许"), InteractionOption(id: "no", label: "拒绝")], secret: false, defaultText: "不可提交的隐藏自填值", allowOther: false),
            InteractionQuestion(id: "closed-multiple", text: "只允许勾选现有选项", options: [InteractionOption(id: "a", label: "Alpha"), InteractionOption(id: "b", label: "Beta")], secret: false, defaultText: "不可提交的隐藏自填值", allowOther: false, multiple: true)
        ]), false),
        ("multiline", .userInput(questions: [InteractionQuestion(id: "multiline", text: "补充需要保留的约束", options: [], secret: false, defaultText: "第一行默认值\n第二行默认值", multiline: true)]), false),
        ("multiple", .asyncUserInput(questions: [questions[2]], delivery: .conversationInput), false),
        ("secret", .userInput(questions: [InteractionQuestion(id: "secret", text: "请输入临时密码", options: [], secret: true)]), false),
        ("secret-options", .userInput(questions: [InteractionQuestion(id: "secret-other", text: "使用哪一种凭据？", options: [InteractionOption(id: "existing", label: "使用现有凭据")], secret: true, defaultText: "synthetic-hidden-default")]), false),
        ("readonly-secret", .asyncUserInput(questions: [InteractionQuestion(id: "readonly-secret", text: "需要父会话提供凭据", options: [], secret: true, defaultText: "synthetic-hidden-default")], delivery: .conversationInput), true),
        ("long", .asyncUserInput(questions: [InteractionQuestion(id: "long", text: String(repeating: "在保留现有输入、执行配置和会话历史的前提下，你希望如何安排接下来的验证？", count: 3), options: (1...5).map { InteractionOption(id: "opt-\($0)", label: "选项 \($0)：" + String(repeating: "逐项核对完整界面和焦点转换，保留未完成工作。", count: 2)) }, secret: false)], delivery: .conversationInput), false),
        ("readonly", .asyncUserInput(questions: questions, delivery: .conversationInput), true),
        ("native-child", .asyncUserInput(questions: questions, delivery: .nativeReply), true),
        ("approval", .approval(title: "允许读取这个目录？", detail: String(repeating: "仅检查合成项目目录中的源文件和配置。\n", count: 9), options: [InteractionOption(id: "allow", label: "允许一次"), InteractionOption(id: "deny", label: "拒绝")]), false)
    ]
    var metrics: [[String: String]] = []
    for (sizeName, width, height) in [("wide", 980.0, 740.0), ("narrow", 360.0, 560.0), ("short", 620.0, 380.0)] {
        for (name, kind, child) in variants {
            let device = DeviceStore(preview: true)
            var snapshot = device.previewSnapshots["sample-0"]!
            snapshot.pendingInteractions = [RuntimeInteraction(id: "visual-\(name)", kind: kind)]
            if child { snapshot.conversation.parentConversationID = "synthetic-parent" }
            device.keepPreviewSnapshot(snapshot)
            let workspace = ConversationWindowModel(application: device, kind: .workspace)
            workspace.focused.store.selectConversation(id: "sample-0")
            workspace.tools.move(.composer, to: .builtIn)
            let tab = workspace.focused
            if visible && name == visibleCase && sizeName == "wide" {
                workspace.openInNewTab("sample-1")
                workspace.selectTab(tab.id)
            }
            let host = NSHostingView(rootView: PaneLayoutView(window: workspace, layout: workspace.layout, path: [], previewEmpty: false).applicationAppearance().background(Color(nsColor: .windowBackgroundColor)))
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: height), styleMask: visible ? [.titled, .closable, .resizable] : [.borderless], backing: .buffered, defer: false)
            window.title = "Velune 提问视觉验收（合成数据）"
            window.contentView = host
            let prefix = "\(sizeName)-\(name)"
            if visible && sizeName == "wide" && name == visibleCase {
                let ready = directory.appendingPathComponent("continue-visible-check")
                try? FileManager.default.removeItem(at: ready)
                let limit = Date().addingTimeInterval(600)
                let timer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { _ in
                    MainActor.assumeIsolated {
                        if FileManager.default.fileExists(atPath: ready.path) || Date() >= limit {
                            NSApplication.shared.stop(nil)
                            let wake = NSEvent.otherEvent(with: .applicationDefined, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, subtype: 0, data1: 0, data2: 0)!
                            NSApplication.shared.postEvent(wake, atStart: false)
                        }
                    }
                }
                DispatchQueue.main.async {
                    NSApplication.shared.unhide(nil)
                    window.makeKeyAndOrderFront(nil)
                    NSApplication.shared.activate(ignoringOtherApps: true)
                }
                NSApplication.shared.run()
                timer.invalidate()
                window.orderOut(nil)
            }
            capture(host, prefix + "-initial")
            let before = descendants(host).compactMap { $0 as? NSScrollView }.map { NSStringFromRect(host.convert($0.bounds, from: $0)) }
            tab.focusInteraction(); settle()
            capture(host, prefix + "-focus")
            let after = descendants(host).compactMap { $0 as? NSScrollView }.map { NSStringFromRect(host.convert($0.bounds, from: $0)) }
            precondition(before == after, "聚焦改变了表单/会话/输入滚动区的布局：\(prefix)\n\(before)\n\(after)")
            for field in descendants(host).compactMap({ $0 as? NSTextField }).filter({ $0.isEditable && $0.isEnabled && !$0.isHiddenOrHasHiddenAncestor }) {
                precondition(window.makeFirstResponder(field), "无法聚焦回答字段"); settle()
                let focused = descendants(host).compactMap { $0 as? NSScrollView }.map { NSStringFromRect(host.convert($0.bounds, from: $0)) }
                precondition(focused == after, "直接聚焦字段改变布局：\(prefix)")
            }
            if let editor = descendants(host).compactMap({ $0 as? NSTextView }).first(where: { $0.isEditable && !($0 is ComposerTextView) }) {
                precondition(window.makeFirstResponder(editor), "无法聚焦多行回答")
                editor.insertText("\n新增约束：" + String(repeating: "输入增长时表单不挤压其他区域。\n", count: 8), replacementRange: NSRange(location: (editor.string as NSString).length, length: 0))
                capture(host, prefix + "-typing")
                let typed = descendants(host).compactMap { $0 as? NSScrollView }.map { NSStringFromRect(host.convert($0.bounds, from: $0)) }
                precondition(typed == after, "多行内容增长改变布局：\(prefix)")
            }
            let buttons = descendants(host).compactMap { $0 as? NSButton }.filter { !$0.isHiddenOrHasHiddenAncestor }
            if let radio = buttons.first(where: { $0.title.contains("运行时与会话历史") }) {
                radio.performClick(nil); settle()
                precondition(radio.state == .on, "原生单选应选中")
                capture(host, prefix + "-selected")
            }
            if name == "closed-default" {
                precondition(!buttons.contains { $0.state == .on }, "封闭选项不能从隐藏默认文本伪造选择")
            }
            // In the actual Pane order: transcript, question form, optional
            // TextEditor and composer. SwiftUI radio labels are not NSButton.title.
            let scrolls = descendants(host).compactMap { $0 as? NSScrollView }
            precondition(scrolls.count >= 2, "未找到实际问题滚动区")
            let scroller = scrolls[1]
            if let document = scroller.documentView {
                let bottom = max(0, document.bounds.height - scroller.contentView.bounds.height)
                scroller.contentView.scroll(to: NSPoint(x: 0, y: bottom))
                scroller.reflectScrolledClipView(scroller.contentView)
                capture(host, prefix + "-scrolled")
            }
            if name == "async" {
                // Show a filled form through busy/failure, rather than an
                // empty form paired with a misleading retention notice.
                let draft = tab.store.interactionDraft(for: snapshot.pendingInteractions[0])
                draft.selectedOptions[questions[0].id] = [questions[0].options[0].id]
                draft.values[questions[1].id] = "已填写且在失败后保留的回答\n第二行"
                capture(host, prefix + "-answered")
                ConversationStore.checkInteractionVisualBusy(tab.store, true, replying: false)
                precondition(tab.store.interactionReplyStatus(snapshot.pendingInteractions[0].id) == nil,
                             "普通刷新不能被投影为正在提交回答")
                capture(host, prefix + "-refreshing")
                ConversationStore.checkInteractionVisualBusy(tab.store, true)
                precondition(tab.store.interactionReplyStatus(snapshot.pendingInteractions[0].id) == "正在提交回答")
                capture(host, prefix + "-submitting")
                let busy = descendants(host).compactMap { $0 as? NSTextField }.filter { $0.isEditable && !$0.isHiddenOrHasHiddenAncestor }
                precondition(busy.allSatisfy { !$0.isEnabled }, "提交时回答字段应禁用")
                ConversationStore.checkInteractionVisualBusy(tab.store, false)
                ConversationStore.checkInteractionVisualFailure(tab.store, "visual-\(name)", "连接暂时不可用，请重试；已填写的回答会保留。")
                capture(host, prefix + "-failed")
                precondition(draft.selectedOptions[questions[0].id] == [questions[0].options[0].id] && draft.values[questions[1].id]?.hasPrefix("已填写") == true, "提交中和失败后应保留已填答案")
                snapshot.pendingInteractions = []
                device.keepPreviewSnapshot(snapshot); tab.store.selectConversation(id: "sample-0")
                capture(host, prefix + "-removed")
            }
            metrics.append(["case": prefix, "focusStable": "true", "scrollRegions": before.joined(separator: ";")])
            precondition(!window.isVisible)
            window.contentView = nil
            tab.store.close()
        }
    }
    try! JSONSerialization.data(withJSONObject: metrics, options: [.prettyPrinted, .sortedKeys]).write(to: directory.appendingPathComponent("geometry.json"))
    FileHandle.standardError.write(Data("PASS runtime interaction: full Pane composition at wide/narrow/short sizes; async/sync/secret/long/readonly/native-child/approval; focus and multiline input keep layout stable\n".utf8))
    exit(0)
}
#endif
