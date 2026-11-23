import AppKit
import Combine
import Foundation
import SwiftUI
import VeluneBindings

/// Never presents or activates a window. Native history and state are synthetic.
@MainActor private final class ArchivedCheckWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    private func refuse() -> Never { fatalError("Archive content check must not present a window") }
    override func makeKey() { refuse() }
    override func makeMain() { refuse() }
    override func orderFront(_ sender: Any?) { refuse() }
    override func makeKeyAndOrderFront(_ sender: Any?) { refuse() }
    override func orderFrontRegardless() { refuse() }
    override func order(_ place: NSWindow.OrderingMode, relativeTo otherWin: Int) {
        if place != .out { refuse() }
        super.order(place, relativeTo: otherWin)
    }
}

@MainActor func archivedConversationsChecks() -> Never {
    NSApplication.shared.setActivationPolicy(.prohibited)
    guard let path = ProcessInfo.processInfo.environment["VELUNE_COMPOSER_CHECK_HOME"], path.hasPrefix("/tmp/"),
          let outputPath = ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"], outputPath.hasPrefix("/tmp/") else {
        fatalError("归档检查需要私有合成目录")
    }
    let home = URL(fileURLWithPath: path, isDirectory: true)
    let output = URL(fileURLWithPath: outputPath, isDirectory: true)
    let front = NSWorkspace.shared.frontmostApplication?.processIdentifier
    func safe() {
        precondition(NSApp.activationPolicy() == .prohibited && !NSApp.isActive)
        precondition(NSWorkspace.shared.frontmostApplication?.processIdentifier == front)
        precondition(NSApp.windows.allSatisfy { !$0.isVisible && !$0.isKeyWindow && !$0.isMainWindow })
    }
    func wait(_ label: String, _ condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(30)
        while !condition(), Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)); safe() }
        precondition(condition(), "归档检查超时：\(label)")
    }
    func executable(_ name: String) -> String {
        for folder in (ProcessInfo.processInfo.environment["PATH"] ?? "").split(separator: ":") {
            let candidate = URL(fileURLWithPath: String(folder)).appendingPathComponent(name).path
            if FileManager.default.isExecutableFile(atPath: candidate) { return candidate }
        }
        fatalError("归档检查需要已安装的 \(name)")
    }
    let pi = executable("pi"), node = executable("node")
    let agent = home.appendingPathComponent("agent", isDirectory: true)
    try! FileManager.default.createDirectory(at: agent, withIntermediateDirectories: true)
    var files: [String: URL] = [:]
    for index in 0..<9 {
        let folder = agent.appendingPathComponent("sessions/project-\(index % 2)", isDirectory: true)
        try! FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let id = String(format: "10000000-0000-4000-8000-%012d", index)
        let title = index == 8 ? "ArchiveNeedle 未加载页的合成归档会话" : "合成归档 \(index)：" + String(repeating: "长标题用于检查窄窗截断与布局 ", count: 12)
        let records: [[String: Any]] = [
            ["type": "session", "version": 3, "id": id, "cwd": "/synthetic/project-\(index % 2)", "timestamp": "2026-01-01T00:00:00Z"],
            ["type": "message", "id": "user-\(index)", "parentId": NSNull(), "timestamp": "2026-01-01T00:00:00Z",
             "message": ["role": "user", "content": [["type": "text", "text": title]], "timestamp": 1767225600000]],
            ["type": "session_info", "id": "name-\(index)", "parentId": "user-\(index)", "timestamp": "2026-01-01T00:00:00Z", "name": title]
        ]
        var bytes = Data()
        for record in records { bytes.append(try! JSONSerialization.data(withJSONObject: record)); bytes.append(10) }
        let file = folder.appendingPathComponent(id + ".jsonl")
        try! bytes.write(to: file); files[id] = file
    }
    let transport = Transport(stateDirectory: home, resourcesDirectory: Bundle.main.resourceURL!)
    let bootstrap = try! transport.list()
    for var runtime in bootstrap.runtimeInstances where runtime.enabled {
        runtime.enabled = false; _ = try! transport.upsertRuntime(runtime)
    }
    let runtimeID = "synthetic-archives"
    _ = try! transport.upsertRuntime(RuntimeInstance(enabled: true, id: runtimeID, name: "合成 Pi", typeId: .pi,
        gatewayId: bootstrap.gateways.first?.id ?? "default",
        settings: ["binary": pi, "nodeBinary": node, "agentDir": agent.path]),
        execution: RuntimeExecution(transport: .local, home: .existing(path: home.path), remoteResourcesDirectory: nil))
    let all = try! transport.list().conversations.map(BindingMapping.listed).filter { $0.runtimeID == runtimeID }
    precondition(all.count == 9, "必须发现九个合成原生会话")
    _ = try! transport.organizeConversations(all.map { ConversationReference(runtimeId: $0.runtimeID, conversationId: $0.id, nativeReadState: $0.nativeReadState) }, .archive)
    _ = try! transport.setConversationBrowserLimits(ConversationBrowserLimits(page: 2, total: 2))
    let device = DeviceStore(transport: transport)
    device.applyList(try! transport.list())
    let originalPreferences = device.conversationBrowserPreferences
    precondition(device.conversations.isEmpty && !device.showsArchived)
    let model = ArchivedConversationsModel(source: device)
    var completed: [Set<String>] = []
    let watch = model.completed.sink { completed.append($0) }
    do {
        let releaseNextRead = DispatchSemaphore(value: 0)
        defer { releaseNextRead.signal() }
        var firstReadDelivered = false
        model.start()
        device.performOnQueue {
            DispatchQueue.main.async { firstReadDelivered = true }
            _ = releaseNextRead.wait(timeout: .now() + 15)
        }
        model.refreshOnActivation()
        wait("首次归档回执", { firstReadDelivered })
        precondition(model.ready && !model.isLoading, "窗口激活应复用正在读取的首屏")
    }
    precondition(model.conversations.count == 2 && model.conversations.allSatisfy(\.archived))
    let firstPage = Set(model.conversations.map(\.id))
    let cold = all.first { !firstPage.contains($0.id) }!
    let coldTitle = cold.title
    model.searchText = coldTitle
    model.search(); wait("跨页标题搜索", { !model.isLoading })
    precondition(model.conversations.contains { $0.id == cold.id }, "标题搜索必须找到尚未加载的归档")
    model.searchText = ""; model.search(); wait("清空搜索", { !model.isLoading })
    let filterEntered = DispatchSemaphore(value: 0), filterRelease = DispatchSemaphore(value: 0)
    device.performOnQueue { filterEntered.signal(); _ = filterRelease.wait(timeout: .now() + 15) }
    precondition(filterEntered.wait(timeout: .now() + 3) == .success)
    model.browser.project = .path("/synthetic/project-1"); model.setPreferences()
    // Clear/search while preferences are still queued: the final delivery
    // must observe the preceding change rather than replay a cached listing.
    model.search(); filterRelease.signal()
    wait("筛选与搜索串行回执", { !model.isLoading })
    precondition(model.browser.project == .path("/synthetic/project-1"), "迟到搜索不能回滚筛选偏好")
    precondition(!model.conversations.isEmpty && model.conversations.allSatisfy { $0.project == "/synthetic/project-1" || $0.cwd == "/synthetic/project-1" })
    model.browser.project = .all; model.setPreferences(); wait("清除筛选", { !model.isLoading })
    let group = model.groups.first { $0.loaded < $0.total }!
    model.loadMore(group.key); wait("分页", { !model.isLoading })
    precondition(model.conversations.count > 2 && firstPage.isSubset(of: Set(model.conversations.map(\.id))))
    precondition(device.conversations.isEmpty && !device.showsArchived && device.conversationBrowserPreferences == originalPreferences,
                 "独立归档的模式、筛选、搜索和分页不能污染主列表")

    let cancelled = model.conversations[0]
    let blocked = DispatchSemaphore(value: 0), release = DispatchSemaphore(value: 0)
    device.performOnQueue { blocked.signal(); _ = release.wait(timeout: .now() + 15) }
    precondition(blocked.wait(timeout: .now() + 3) == .success)
    model.unarchive([cancelled])
    let request = device.conversationManagement.last!
    precondition(!request.running && device.cancelConversationManagement(request.id))
    release.signal()
    model.refresh(); wait("取消后重读", { !model.isLoading })
    precondition(model.conversations.contains { $0.id == cancelled.id } && completed.isEmpty,
                 "取消排队操作应保留归档与选中完成事件")

    let unarchived = model.conversations[0]
    model.unarchive([unarchived])
    wait("取消归档与主列表刷新", { device.conversationManagement.isEmpty && !model.isLoading && device.conversations.contains { $0.id == unarchived.id } })
    precondition(completed.last == [unarchived.id] && !model.conversations.contains { $0.id == unarchived.id })
    let deleted = model.conversations[0]
    precondition(model.canDelete(deleted), "原生合成会话应可删除")
    model.delete([deleted])
    wait("原生删除", { device.conversationManagement.isEmpty && !model.isLoading && !model.conversations.contains { $0.id == deleted.id } })
    precondition(completed.last == [deleted.id] && device.deletedConversationIDs.contains(deleted.id))
    // Pi native IDs carry the source path; verify its actual JSONL is gone.
    let deletedFile = files.values.first { deleted.id.hasSuffix($0.path) }!
    precondition(!FileManager.default.fileExists(atPath: deletedFile.path))
    let failureTarget = model.conversations[0]
    var unavailable = failureTarget; unavailable.runtimeID = "synthetic-runtime-removed"
    let completedBeforeFailure = completed
    model.delete([unavailable])
    wait("删除失败", { device.conversationManagement.isEmpty && !model.isLoading && model.operationError != nil })
    precondition(model.conversations.contains { $0.id == failureTarget.id } && completed == completedBeforeFailure,
                 "失败应保留归档，且不能清除失败项的选择")

    let dark = CommandLine.arguments.contains("--archived-check-dark")
    NSApp.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
    var states: [String] = []
    func render(_ name: String, model renderedModel: ArchivedConversationsModel, width: CGFloat) {
        let host = NSHostingView(rootView: ArchivedConversationsContent(model: renderedModel).applicationAppearance()
            .environment(\.colorScheme, dark ? .dark : .light).background(Color(nsColor: .windowBackgroundColor)))
        host.frame = NSRect(x: 0, y: 0, width: width, height: 520)
        let window = ArchivedCheckWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false; window.isRestorable = false; window.contentView = host
        host.layoutSubtreeIfNeeded(); RunLoop.main.run(until: Date().addingTimeInterval(0.15)); host.layoutSubtreeIfNeeded(); safe()
        let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds)!
        host.cacheDisplay(in: host.bounds, to: bitmap)
        try! bitmap.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent("archived-\(name)-\(dark ? "dark" : "light").png"))
        states.append(name); window.contentView = nil; window.close()
        renderedModel.stop(); RunLoop.main.run(until: Date().addingTimeInterval(0.05)); safe()
    }
    // Preview-backed models cannot open a core from onAppear; projection is
    // copied from the production archive view for stable content captures.
    let preview = DeviceStore(preview: true)
    let projected = ArchivedConversationsModel(source: preview)
    let visualTransport = transport.clientView("archive-visual-snapshot")
    let archiveSnapshot = try! visualTransport.showArchivedConversations(true)
    preview.applyList(archiveSnapshot)
    projected.apply(archiveSnapshot)
    try! visualTransport.close()
    render("content-wide", model: projected, width: 900)
    render("content-narrow", model: projected, width: 600)
    let empty = ArchivedConversationsModel(source: preview)
    render("empty", model: empty, width: 600)
    empty.searchText = "无匹配的合成标题"
    render("no-matches", model: empty, width: 600)
    render("operation-error", model: model, width: 760)
    model.start(); wait("重开独立归档", { model.ready && !model.isLoading })
    let closingTarget = model.conversations[0]
    let closingEntered = DispatchSemaphore(value: 0), closingRelease = DispatchSemaphore(value: 0)
    device.performOnQueue { closingEntered.signal(); _ = closingRelease.wait(timeout: .now() + 15) }
    precondition(closingEntered.wait(timeout: .now() + 3) == .success)
    model.unarchive([closingTarget]); model.stop(); closingRelease.signal()
    wait("关窗后排队管理完成", { device.conversationManagement.isEmpty && device.conversations.contains { $0.id == closingTarget.id } })
    precondition(completed.last == [closingTarget.id], "关窗不应丢弃设备管理完成回执")
    var mainAfterClose: ConfigurationSnapshot?
    device.performOnQueue {
        let value = try! transport.list()
        DispatchQueue.main.async { mainAfterClose = value }
    }
    wait("关闭独立视图后主核心读取", { mainAfterClose != nil })
    precondition(mainAfterClose!.conversations.contains { $0.conversation.id == unarchived.id })
    precondition(!model.ready, "关窗后的迟到管理回执不能重新打开归档视图")
    safe(); withExtendedLifetime(watch) {}
    let receipt: [String: Any] = ["states": states, "dark": dark, "realModelCalls": 0, "piBinary": pi, "nodeBinary": node,
        "crossPageTitleSearch": true, "mainBrowserIsolation": true, "queuedCancellationRetainsArchive": true,
        "unarchiveRefreshesMain": true, "nativeDeleteRemovesJSONL": true, "failurePreservesRowAndCompletion": true,
        "pendingManagementSurvivesClose": true, "derivedClosePreservesMainCore": true, "foregroundUnchanged": true, "systemChromeVerified": false]
    try! JSONSerialization.data(withJSONObject: receipt, options: [.prettyPrinted, .sortedKeys]).write(to: output.appendingPathComponent("archived-check.json"))
    print("PASS archived conversations: real native fixture, cross-page title search, isolated browsing, queued cancel, unarchive, native delete, failure, derived close, \(states.count) offscreen states")
    exit(0)
}
