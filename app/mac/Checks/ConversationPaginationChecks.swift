#if VELUNE_DEV_CHECKS
import AppKit
import Foundation
import SwiftUI
import VeluneBindings

/// Exercises the production Mac queue, transport and source-fold preparation.
/// All native records and runtime configuration live in the check's private home.
@MainActor func conversationPaginationChecks() -> Never {
    NSApplication.shared.setActivationPolicy(.prohibited)
    func wait(_ condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(20)
        while !condition(), Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
        precondition(condition(), "会话分页检查等待超时")
    }
    guard let path = ProcessInfo.processInfo.environment["VELUNE_COMPOSER_CHECK_HOME"], path.hasPrefix("/tmp/") else {
        fatalError("分页检查需要专用临时目录")
    }
    let home = URL(fileURLWithPath: path, isDirectory: true)
    let agent = home.appendingPathComponent("agent", isDirectory: true)
    try! FileManager.default.createDirectory(at: agent, withIntermediateDirectories: true)
    for group in 0..<6 {
        let folder = agent.appendingPathComponent("projects/group-\(group)", isDirectory: true)
        try! FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        for row in 0..<6 {
            let id = String(format: "00000000-0000-4000-8000-%06d%06d", group, row)
            let record: [String: Any] = ["type": "user", "uuid": "u-\(group)-\(row)", "parentUuid": NSNull(),
                "sessionId": id, "cwd": "/synthetic/project-\(group)", "timestamp": "2026-01-01T00:00:00Z",
                "message": ["role": "user", "content": "Synthetic group \(group) row \(row)"]]
            var bytes = try! JSONSerialization.data(withJSONObject: record)
            bytes.append(10)
            try! bytes.write(to: folder.appendingPathComponent(id + ".jsonl"))
        }
    }
    let transport = Transport(stateDirectory: home, resourcesDirectory: Bundle.main.resourceURL!)
    let bootstrap = try! transport.list()
    let gateway = bootstrap.gateways.first?.id ?? "default"
    for var runtime in bootstrap.runtimeInstances where runtime.enabled {
        runtime.enabled = false
        _ = try! transport.upsertRuntime(runtime)
    }
    _ = try! transport.upsertRuntime(RuntimeInstance(enabled: true, id: "synthetic-pagination", name: "分页合成源",
        typeId: .claudeCode, gatewayId: gateway,
        settings: ["binary": "/synthetic/unused-agent", "nodeBinary": "/usr/bin/python3", "agentDir": agent.path]),
        execution: RuntimeExecution(transport: .local, home: .existing(path: home.path), remoteResourcesDirectory: nil))
    let keys = (0..<6).map { ConversationGroupKey.project(path: "/synthetic/project-\($0)") }
    _ = try! transport.setConversationBrowserLimits(ConversationBrowserLimits(page: 2, total: 4))
    _ = try! transport.setConversationBrowserPreferences(ConversationBrowserPreferences(grouping: .project,
        sort: .updated, oldestFirst: false, runtimeId: "synthetic-pagination", project: .all,
        collapsed: [keys[3], keys[4]], groupSort: .manual, groupOrder: keys))
    let initial = try! transport.list()
    let device = DeviceStore(transport: transport)
    device.applyList(initial)
    func loaded(_ index: Int) -> UInt32 { device.conversationGroups.first { $0.key == keys[index] }!.loaded }
    precondition(loaded(0) == 2 && loaded(1) == 2 && loaded(4) == 0 && loaded(5) == 0)
    let original = Set(device.conversations.map(\.id))
    let started = ProcessInfo.processInfo.systemUptime
    device.loadMoreConversations(in: keys[4])
    device.loadMoreConversations(in: keys[4])
    precondition(device.loadingConversationGroups.contains(keys[4]), "点击后必须立即显示加载状态")
    wait { !device.loadingConversationGroups.contains(keys[4]) }
    precondition(loaded(4) == 2, "快速重复点击只能受理一页，不能排队两次")
    precondition(device.conversationBrowserPreferences?.collapsed.contains(keys[4]) == false
                 && device.conversationBrowserPreferences?.collapsed.contains(keys[3]) == true,
                 "只展开被点击的来源组，保留其他组的折叠偏好")
    precondition(original.isSubset(of: Set(device.conversations.map(\.id))), "加载另一组不能丢弃已有会话")
    let elapsed = ProcessInfo.processInfo.systemUptime - started
    let duplicate = agent.appendingPathComponent("projects/duplicate", isDirectory: true)
    try! FileManager.default.createDirectory(at: duplicate, withIntermediateDirectories: true)
    try! Data("{}\n".utf8).write(to: duplicate.appendingPathComponent("00000000-0000-4000-8000-000000000000.jsonl"))
    device.loadMoreConversations(in: keys[5])
    wait { !device.loadingConversationGroups.contains(keys[5]) }
    precondition(loaded(5) == 0 && !device.problems.isEmpty, "冷读失败应结束加载并披露问题")
    try! FileManager.default.removeItem(at: duplicate)
    device.loadMoreConversations(in: keys[5])
    wait { !device.loadingConversationGroups.contains(keys[5]) }
    precondition(loaded(5) > 0 && loaded(4) == 2
                 && original.isSubset(of: Set(device.conversations.map(\.id))), "失败后可重试并保留其他组")
    if let output = ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"] {
        for dark in [false, true] {
            let host = NSHostingView(rootView: VStack(alignment: .leading, spacing: 18) {
                ConversationPaginationButton(loaded: 0, total: 36, isLoading: false, load: {})
                ConversationPaginationButton(loaded: 0, total: 36, isLoading: true, load: {})
                ConversationPaginationButton(loaded: 2, total: 36, isLoading: false, sourceName: "合成设备", load: {})
            }.padding(16).frame(width: 360, height: 140, alignment: .leading)
                .environment(\.colorScheme, dark ? .dark : .light)
                .background(Color(nsColor: .windowBackgroundColor)))
            host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            host.frame = NSRect(x: 0, y: 0, width: 360, height: 140)
            host.layoutSubtreeIfNeeded()
            let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds)!
            host.cacheDisplay(in: host.bounds, to: bitmap)
            try! bitmap.representation(using: .png, properties: [:])!.write(to:
                URL(fileURLWithPath: output).appendingPathComponent("pagination-\(dark ? "dark" : "light").png"))
        }
    }
    precondition(!NSApp.isActive && NSApp.windows.isEmpty, "分页检查不能创建或激活界面")
    FileHandle.standardError.write(Data("PASS Mac pagination: source-fold preparation, immediate loading, duplicate click coalescing, existing rows retained, failure/retry; first page \(elapsed)s; no windows or model calls\n".utf8))
    exit(0)
}
#endif
