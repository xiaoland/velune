import AppKit
import SwiftUI
import VeluneBindings

@MainActor private final class BackgroundCheckWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    private func refuse() -> Never { fatalError("Background content check must not show a window") }
    override func makeKey() { refuse() }
    override func makeMain() { refuse() }
    override func orderFront(_ sender: Any?) { refuse() }
    override func makeKeyAndOrderFront(_ sender: Any?) { refuse() }
    override func orderFrontRegardless() { refuse() }
    override func order(_ place: NSWindow.OrderingMode, relativeTo otherWin: Int) { if place != .out { refuse() }; super.order(place, relativeTo: otherWin) }
}

@MainActor func backgroundTaskChecks() -> Never {
    NSApplication.shared.setActivationPolicy(.prohibited)
    guard let directory = ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"], directory.hasPrefix("/tmp/") else { exit(2) }
    let output = URL(fileURLWithPath: directory, isDirectory: true)
    try! FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
    let front = NSWorkspace.shared.frontmostApplication?.processIdentifier
    let dark = CommandLine.arguments.contains("--background-check-dark")
    NSApp.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
    func safe() {
        precondition(!NSApp.isActive && NSApp.activationPolicy() == .prohibited && NSWorkspace.shared.frontmostApplication?.processIdentifier == front)
        precondition(NSApp.windows.allSatisfy { !$0.isVisible && !$0.isKeyWindow && !$0.isMainWindow })
    }
    var states: [String] = []
    func render<V: View>(_ name: String, _ content: V) {
        let host = NSHostingView(rootView: content.applicationAppearance().background(Color(nsColor: .windowBackgroundColor)))
        host.frame = NSRect(x: 0, y: 0, width: 360, height: 400)
        let window = BackgroundCheckWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false; window.isRestorable = false; window.contentView = host
        host.layoutSubtreeIfNeeded(); RunLoop.main.run(until: Date().addingTimeInterval(0.08)); host.layoutSubtreeIfNeeded(); safe()
        let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds)!
        host.cacheDisplay(in: host.bounds, to: bitmap)
        try! bitmap.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent("background-\(name)-\(dark ? "dark" : "light").png"))
        states.append(name); window.contentView = nil; window.close(); safe()
    }
    let store = ConversationStore(device: DeviceStore(preview: true))
    let reset = BackgroundTasksModel(); reset.loading = true; reset.reset(store)
    precondition(!reset.loading && reset.tasks.isEmpty && reset.issue == nil, "切换会话必须清除旧任务加载态")
    render("agents-empty", LinkedAgentsView(store: store, inPopover: false, open: { _, _ in }))
    for state in ["empty", "loading", "unknown", "unsupported"] {
        let model = BackgroundTasksModel()
        model.loading = state == "loading"
        if state == "unknown" { model.issue = "运行时连接暂时不可用。保留已有任务，恢复后重新同步状态。" }
        if state == "unsupported" { model.issue = "请选择支持后台任务的 Pi 或 Claude Code 会话。" }
        render(state, BackgroundTasksContent(model: model, inPopover: false, open: { _, _ in }))
    }
    // Older windows know only subAgents. Adding a task tool must preserve
    // their placement and frame and keep the two tools independent.
    var old = ToolWindowLayout()
    old[.agents].place = .window; old[.agents].pinned = true
    old[.agents].windowFrame = ToolFrame(NSRect(x: 12, y: 24, width: 350, height: 500), attachedTo: nil)
    var restored = try! JSONDecoder().decode(ToolWindowLayout.self, from: JSONEncoder().encode(old))
    precondition(restored[.agents] == old[.agents] && restored[.backgroundTasks].place == .builtIn)
    restored.move(.backgroundTasks, to: .docked(.right))
    precondition(restored[.agents] == old[.agents] && restored.tools(in: .right) == [.backgroundTasks])
    restored.hide(.backgroundTasks)
    precondition(restored.isOpen(.agents) && !restored.isOpen(.backgroundTasks))
    precondition(!store.hasLinkedAgents && store.canShowBackgroundTasks, "后台任务入口不能依赖关联 Agent 存在")
    let statuses: [BackgroundTaskStatus] = [.running, .stopping, .completed, .failed, .cancelled, .unknown]
    let tasks = statuses.enumerated().map { index, status in
        BackgroundTaskInfo(id: "synthetic-\(index)", kind: index == 2 ? "delegate" : "shell", title: index == 2 ? "汇总构建结果并检查各平台兼容性，准备一份可供下一轮使用的报告" : "检查工作区构建与测试结果", status: status, conversationId: index == 2 ? "pi:synthetic-child" : nil, childSessionId: nil, canStop: status == .running, canReadOutput: status == .completed || status == .failed)
    }
    let listModel = BackgroundTasksModel(); listModel.tasks = tasks
    render("states", BackgroundTasksContent(model: listModel, inPopover: false, open: { _, _ in }))
    render("sidebar-states", BackgroundTasksContent(model: listModel, inPopover: false, inSidebar: true, open: { _, _ in }))
    for task in tasks {
        let model = BackgroundTasksModel(); model.tasks = [task]
        if task.canReadOutput { model.output = BackgroundTaskOutput(requestId: "synthetic", taskId: task.id, status: .ready, text: task.status == .failed ? "合成任务输出：构建失败，缺少测试依赖。" : "合成任务输出：构建完成，所有已选择的检查通过。", truncated: false, issue: nil) }
        render(task.status.label, BackgroundTaskDetail(initial: task, model: model, back: {}, open: { _, _ in }))
    }
    for state in ["empty-output", "long-output", "output-error"] {
        let task = tasks[2], model = BackgroundTasksModel(); model.tasks = [task]
        if state == "output-error" {
            model.outputIssue = "任务输出暂时不可用。刷新后可重试，任务不会重新执行。"
            model.output = BackgroundTaskOutput(requestId: "synthetic", taskId: task.id, status: .failed, text: "", truncated: false, issue: model.outputIssue)
        }
        else { model.output = BackgroundTaskOutput(requestId: "synthetic", taskId: task.id, status: .ready, text: state == "empty-output" ? "" : String(repeating: "合成输出 0123456789\n", count: 200), truncated: state == "long-output", issue: nil) }
        render(state, BackgroundTaskDetail(initial: task, model: model, back: {}, open: { _, _ in }))
    }
    safe()
    store.close()
    let receipt: [String: Any] = ["states": states, "dark": dark, "realModelCalls": 0, "systemChromeVerified": false, "foregroundUnchanged": true,
                                "independentLayoutRestoration": true, "loadingClearedOnConversationChange": true]
    try! JSONSerialization.data(withJSONObject: receipt, options: [.prettyPrinted, .sortedKeys]).write(to: output.appendingPathComponent("background-check.json"))
    print("PASS background task native content states: \(states.count)"); exit(0)
}
