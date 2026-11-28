#if VELUNE_DEV_CHECKS
import AppKit
import Foundation

/// Exercises journal selection and the real window-start candidate ordering.
/// All state stays in the caller's synthetic home; native navigation hosts stay hidden.
@MainActor func workspaceRecoveryChecks(home: URL) {
    workspaceNavigationChecks(home: home)
    func note(_ text: String) { FileHandle.standardError.write(Data((text + "\n").utf8)) }
    try! workspaceColdOpeningChecks(home: home)
    let device = DeviceStore(preview: true)
    do {
        let peer = DeviceStore(preview: true, peerID: "synthetic-peer")
        let window = ConversationWindowModel(application: device, kind: .workspace)
        window.start(restoring: nil, seed: WindowSeed(conversationID: "sample-1"))
        window.open("sample-1", device: peer)
        precondition(window.focused.store.device === peer && window.focused.store.loadedConversationID == "sample-1",
                     "另一台设备的同名会话编号不能复用本机标签")
        precondition(window.tab(showing: "sample-1", device: device) != nil)
        note("PASS conversation tabs distinguish identical IDs on different devices")
    }
    let local = ConversationWindowModel(application: device, kind: .workspace)
    let source = ConversationWindowModel(application: device, kind: .workspace, device: "synthetic-peer")
    let directory = home.appendingPathComponent("workspace-device-check", isDirectory: true)
    let journal = WindowWorkspaceJournal(directory: directory) { fatalError($0) }
    let localID = UUID().uuidString, remoteID = UUID().uuidString
    let localLayout = local.savedLayout, remoteLayout = source.savedLayout
    journal.save(localLayout, id: localID)
    journal.save(remoteLayout, id: remoteID)
    journal.flush()
    let snapshots = directory.appendingPathComponent("mac-window-workspaces", isDirectory: true)
    try! FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: 1)], ofItemAtPath: snapshots.appendingPathComponent(localID + ".json").path)
    try! FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: 2)], ofItemAtPath: snapshots.appendingPathComponent(remoteID + ".json").path)
    precondition(journal.restore(id: UUID().uuidString, recoverLatest: true) == localLayout,
                 "本机回退不能读取更新的远端窗口布局")
    precondition(journal.restore(id: localID, recoverLatest: false) == localLayout)
    precondition(journal.restore(id: remoteID, recoverLatest: false) == nil)
    precondition(journal.restore(id: remoteID, recoverLatest: false, device: "synthetic-peer") == remoteLayout)
    precondition(journal.restore(id: UUID().uuidString, recoverLatest: true, device: "another-peer") == nil)
    let leftPane = UUID(), rightPane = UUID()
    let leftTab = UUID(), rightTabs = [UUID(), UUID()]
    var multiPane = localLayout
    multiPane.layout = .split(PaneSplit(axis: .horizontal, children: [.pane(leftPane), .pane(rightPane)], fractions: [0.5, 0.5]))
    multiPane.panes = [
        leftPane: .init(tabs: [leftTab], selected: leftTab),
        rightPane: .init(tabs: rightTabs, selected: rightTabs[0]),
    ]
    multiPane.conversations = [leftTab: "synthetic-left", rightTabs[0]: "synthetic-right-a", rightTabs[1]: "synthetic-right-b"]
    multiPane.focused = rightPane
    let multiPaneID = UUID().uuidString
    let multiPaneDirectory = home.appendingPathComponent("workspace-multipane-check", isDirectory: true)
    let multiPaneJournal = WindowWorkspaceJournal(directory: multiPaneDirectory) { fatalError($0) }
    multiPaneJournal.save(multiPane, id: multiPaneID)
    multiPaneJournal.flush()
    precondition(multiPaneJournal.restore(id: multiPaneID, recoverLatest: false) == multiPane,
                 "不同标签数的多 pane UUID 字典经真实 journal 重开后仍保持身份与顺序")
    var legacyLayout = remoteLayout
    legacyLayout.windowDevice = nil
    let legacyID = UUID().uuidString
    journal.save(legacyLayout, id: legacyID)
    journal.flush()
    try! FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: 3)], ofItemAtPath: snapshots.appendingPathComponent(legacyID + ".json").path)
    precondition(journal.restore(id: UUID().uuidString, recoverLatest: true) == localLayout,
                 "旧快照缺设备身份时不能猜作本机latest")
    precondition(journal.restore(id: legacyID, recoverLatest: false, device: "synthetic-peer") == legacyLayout,
                 "旧快照保留显式scene身份恢复入口")
    let legacyFormatID = UUID().uuidString
    var legacyFormat = try! JSONSerialization.jsonObject(with: JSONEncoder().encode(localLayout)) as! [String: Any]
    legacyFormat.removeValue(forKey: "schemaVersion")
    let legacyBytes = try! JSONSerialization.data(withJSONObject: legacyFormat, options: [.sortedKeys])
    let legacyFile = snapshots.appendingPathComponent(legacyFormatID + ".json")
    try! legacyBytes.write(to: legacyFile)
    precondition(journal.restore(id: legacyFormatID, recoverLatest: false) == localLayout,
                 "已知未版本化恢复点仍可恢复")
    precondition(try! Data(contentsOf: snapshots.appendingPathComponent(legacyFormatID + ".json.before-beta.bak")) == legacyBytes,
                 "恢复点升级前保留逐字节副本")
    let migrated = try! JSONSerialization.jsonObject(with: Data(contentsOf: legacyFile)) as! [String: Any]
    precondition((migrated["schemaVersion"] as? Int) == 1)
    note("PASS workspace journal: exact and latest recovery stay within the execution device; unscoped legacy snapshots require a matching window UUID")


    // A structurally invalid newest or exact snapshot cannot hide a valid
    // older candidate; a valid empty layout remains an ordinary candidate.
    var invalidLayout = localLayout
    invalidLayout.focused = UUID()
    let invalidBytes = try! JSONEncoder().encode(invalidLayout)
    precondition((try? SavedWindowLayout.decodeKnown(invalidBytes)) == nil,
                 "直接给出与 journal 读出的布局共用的解码入口拒绝不完整窗口")
    let invalidID = UUID().uuidString
    journal.save(invalidLayout, id: invalidID)
    journal.flush()
    try! FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: 4)], ofItemAtPath: snapshots.appendingPathComponent(invalidID + ".json").path)
    let tolerantJournal = WindowWorkspaceJournal(directory: directory) { _ in }
    precondition(tolerantJournal.restore(id: UUID().uuidString, recoverLatest: true) == localLayout)
    precondition(tolerantJournal.restore(id: invalidID, recoverLatest: true) == localLayout)
    precondition(tolerantJournal.restore(id: invalidID, recoverLatest: false) == nil)
    precondition(tolerantJournal.restore(id: UUID().uuidString, recoverLatest: true, device: "synthetic-peer") == remoteLayout)
    let futureID = UUID().uuidString
    var future = try! JSONSerialization.jsonObject(with: JSONEncoder().encode(localLayout)) as! [String: Any]
    future["futureField"] = NSNull()
    let futureBytes = try! JSONSerialization.data(withJSONObject: future, options: [.sortedKeys])
    let futureFile = snapshots.appendingPathComponent(futureID + ".json")
    try! futureBytes.write(to: futureFile)
    precondition(tolerantJournal.restore(id: futureID, recoverLatest: false) == nil)
    tolerantJournal.save(localLayout, id: futureID)
    tolerantJournal.flush()
    precondition((try! Data(contentsOf: futureFile)) == futureBytes,
                 "未来恢复点在读取失败后仍保持原始字节，写回不会覆盖")
    note("PASS workspace journal filters incomplete layouts before exact/latest selection and keeps device isolation")

    var currentLayout = localLayout
    currentLayout.composerWorkspaceByDevice = ["synthetic-peer": localLayout.composerWorkspace!]
    let currentBytes = try! JSONEncoder().encode(currentLayout)
    precondition((try! SavedWindowLayout.decodeKnown(currentBytes)) == currentLayout,
                 "当前窗口的本机及远端草稿恢复点均可解码")
    let currentRecord = try! JSONSerialization.jsonObject(with: currentBytes) as! [String: Any]
    let currentWorkspace = currentRecord["composerWorkspace"] as! [String: Any]
    for (peer, version) in [(nil, 2), ("synthetic-peer", 4), (nil, nil)] as [(String?, Int?)] {
        var record = currentRecord
        var workspace = currentWorkspace
        workspace["schemaVersion"] = version
        if let peer { record["composerWorkspaceByDevice"] = [peer: workspace] }
        else { record["composerWorkspace"] = workspace }
        let bytes = try! JSONSerialization.data(withJSONObject: record, options: [.sortedKeys])
        let id = UUID().uuidString
        let file = snapshots.appendingPathComponent(id + ".json")
        try! bytes.write(to: file)
        tolerantJournal.save(currentLayout, id: id)
        tolerantJournal.flush()
        precondition((try! Data(contentsOf: file)) == bytes,
                     "首次保存也必须保留草稿格式不受支持的原始记录")
        precondition(tolerantJournal.restore(id: id, recoverLatest: false) == nil,
                     "不受支持的本机或远端草稿不能成为恢复候选")
        try! FileManager.default.removeItem(at: file)
    }
    note("PASS workspace journal accepts current drafts and preserves snapshots with unsupported local or remote draft schemas")

    do {
        let batchHome = home.appendingPathComponent("workspace-batch-failure-check", isDirectory: true)
        let batchSnapshots = batchHome.appendingPathComponent("mac-window-workspaces", isDirectory: true)
        let firstReport = DispatchSemaphore(value: 1)
        defer { firstReport.signal() }
        let entered = DispatchSemaphore(value: 0)
        let release = DispatchSemaphore(value: 0)
        let batchFailures = DispatchSemaphore(value: 0)
        let batchJournal = WindowWorkspaceJournal(directory: batchHome) { _ in
            if firstReport.wait(timeout: .now()) == .success {
                entered.signal()
                precondition(release.wait(timeout: .now() + 10) == .success)
            } else { batchFailures.signal() }
        }
        let failingIDs = (0..<3).map { _ in UUID().uuidString }
        for id in failingIDs { batchJournal.save(localLayout, id: id) }
        batchJournal.flush()
        // Replace already validated files with directories so the atomic write fails.
        for id in failingIDs {
            let target = batchSnapshots.appendingPathComponent(id + ".json")
            try! FileManager.default.removeItem(at: target)
            try! FileManager.default.createDirectory(at: target, withIntermediateDirectories: false)
        }
        batchJournal.save(localLayout, id: failingIDs[0])
        precondition(entered.wait(timeout: .now() + 10) == .success)
        // The first report holds the drain until both failures and a healthy write are pending.
        for id in failingIDs.dropFirst() { batchJournal.save(localLayout, id: id) }
        let healthyID = UUID().uuidString
        batchJournal.save(localLayout, id: healthyID)
        release.signal()
        batchJournal.flush()
        for _ in 0..<2 { precondition(batchFailures.wait(timeout: .now()) == .success, "同批次中每个窗口均要尝试写入") }
        precondition(batchJournal.restore(id: healthyID, recoverLatest: false) == localLayout)
        note("PASS workspace journal attempts every window in a batch after individual write failures")
    }

    let orderedDirectory = home.appendingPathComponent("workspace-legacy-order-check", isDirectory: true)
    let orderedJournal = WindowWorkspaceJournal(directory: orderedDirectory) { fatalError($0) }
    let orderedSnapshots = orderedDirectory.appendingPathComponent("mac-window-workspaces", isDirectory: true)
    try! FileManager.default.createDirectory(at: orderedSnapshots, withIntermediateDirectories: true)
    var olderLayout = localLayout
    olderLayout.pinned = false
    var newerLayout = localLayout
    newerLayout.pinned = true
    let olderID = UUID().uuidString, newerID = UUID().uuidString
    for (id, layout, timestamp) in [(olderID, olderLayout, 10.0), (newerID, newerLayout, 20.0)] {
        var raw = try! JSONSerialization.jsonObject(with: JSONEncoder().encode(layout)) as! [String: Any]
        raw.removeValue(forKey: "schemaVersion")
        let bytes = try! JSONSerialization.data(withJSONObject: raw, options: [.sortedKeys])
        try! bytes.write(to: orderedSnapshots.appendingPathComponent(id + ".json"))
        try! FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: timestamp)],
                                               ofItemAtPath: orderedSnapshots.appendingPathComponent(id + ".json").path)
    }
    let expectedDate = Date(timeIntervalSince1970: 20.0)
    precondition(orderedJournal.restore(id: olderID, recoverLatest: false) == olderLayout,
                 "系统重开的窗口按自己的 UUID 恢复，而不是选更近的窗口")
    let unrelatedBackup = orderedSnapshots.appendingPathComponent(newerID + ".json.before-beta.bak")
    precondition(!FileManager.default.fileExists(atPath: unrelatedBackup.path),
                 "按 UUID 恢复不能读取或迁移其他窗口的恢复点")
    precondition(orderedJournal.restore(id: UUID().uuidString, recoverLatest: true) == newerLayout,
                 "未版本化快照首次恢复按原始修改时间选最新窗口")
    precondition(orderedJournal.restore(id: UUID().uuidString, recoverLatest: true) == newerLayout,
                 "迁移后的第二次恢复仍选原始最新窗口")
    let actualDate = try! orderedSnapshots.appendingPathComponent(newerID + ".json")
        .resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate
    precondition(actualDate == expectedDate, "schema 标记不改变恢复点的原始排序时间")
    note("PASS legacy snapshot migration preserves mtime and latest-window selection")

    do {
        // Layouts saved while floating panels existed: an attached composer
        // panel and a free outline panel, plus a window's stale panel frame.
        let panel: [String: Any] = ["width": 640, "height": 120, "x": 0, "y": 0,
                                    "attachment": ["fromTrailing": false, "fromTop": false, "dx": 300, "dy": 24]]
        let free: [String: Any] = ["width": 280, "height": 500, "x": 900, "y": 100]
        func floatingTools(_ tools: ToolWindowLayout) -> [String: Any] {
            var raw = try! JSONSerialization.jsonObject(with: JSONEncoder().encode(tools)) as! [String: Any]
            var states = raw["tools"] as! [String: Any]
            states["composer"] = ["place": ["floating": [String: Any]()], "shown": true, "weight": 1, "panelFrame": panel, "pinned": false]
            states["outline"] = ["place": ["floating": [String: Any]()], "shown": true, "weight": 1, "panelFrame": free, "pinned": false]
            states["list"] = ["place": ["window": [String: Any]()], "shown": true, "weight": 1, "panelFrame": free, "windowFrame": panel, "pinned": false]
            raw["tools"] = states
            return raw
        }
        func expectMigrated(_ tools: ToolWindowLayout, _ context: String) {
            precondition(tools[.composer].place == .window && tools[.outline].place == .window && tools[.list].place == .window, "\(context)：浮动工具改为独立窗口")
            precondition(tools[.composer].windowFrame?.width == 640 && tools[.composer].windowFrame?.attachment?.dx == 300, "\(context)：保留面板尺寸与相对主窗口的位置")
            precondition(tools[.outline].windowFrame?.x == 900 && tools[.outline].windowFrame?.height == 500, "\(context)：自由面板保留原位置与尺寸")
            precondition(tools[.list].windowFrame?.width == 640, "\(context)：已是独立窗口的工具保留自己的窗口位置")
        }
        let migrationHome = home.appendingPathComponent("workspace-floating-migration", isDirectory: true)
        let migrationJournal = WindowWorkspaceJournal(directory: migrationHome) { fatalError($0) }
        let migrationSnapshots = migrationHome.appendingPathComponent("mac-window-workspaces", isDirectory: true)
        try! FileManager.default.createDirectory(at: migrationSnapshots, withIntermediateDirectories: true)
        var raw = try! JSONSerialization.jsonObject(with: JSONEncoder().encode(localLayout)) as! [String: Any]
        raw["tools"] = floatingTools(localLayout.tools)
        let original = try! JSONSerialization.data(withJSONObject: raw, options: [.sortedKeys])
        let snapshotID = UUID().uuidString
        let snapshot = migrationSnapshots.appendingPathComponent(snapshotID + ".json")
        try! original.write(to: snapshot)
        let date = Date(timeIntervalSince1970: 30)
        try! FileManager.default.setAttributes([.modificationDate: date], ofItemAtPath: snapshot.path)
        guard let restored = migrationJournal.restore(id: snapshotID, recoverLatest: false) else { fatalError("含浮动工具的恢复点必须可读") }
        expectMigrated(restored.tools, "恢复点")
        let backup = migrationSnapshots.appendingPathComponent(snapshotID + ".json.before-floating-tools.bak")
        precondition((try? Data(contentsOf: backup)) == original, "迁移前备份原恢复点")
        let rewritten = try! Data(contentsOf: snapshot)
        precondition(!String(decoding: rewritten, as: UTF8.self).contains("floating") && !String(decoding: rewritten, as: UTF8.self).contains("panelFrame"),
                     "恢复点改写为新格式")
        precondition((try? SavedWindowLayout.decodeKnown(rewritten)) == restored, "改写后的恢复点按新格式严格读回")
        precondition(try! snapshot.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate == date, "迁移保留恢复点修改时间")

        let toolHome = home.appendingPathComponent("workspace-floating-tool-layout", isDirectory: true)
        try! FileManager.default.createDirectory(at: toolHome, withIntermediateDirectories: true)
        let toolFile = toolHome.appendingPathComponent("mac-tool-layout.json")
        let toolOriginal = try! JSONSerialization.data(withJSONObject: ["schema": 1, "tools": floatingTools(ToolWindowLayout())], options: [.sortedKeys])
        try! toolOriginal.write(to: toolFile)
        let remembered = RememberedToolLayout(home: toolHome) { fatalError("工具布局迁移失败：\($0)") }
        expectMigrated(remembered.layout, "工具布局")
        precondition((try? Data(contentsOf: toolFile.appendingPathExtension("before-floating-tools.bak"))) == toolOriginal, "迁移前备份工具布局")
        precondition(RememberedToolLayout(home: toolHome) { fatalError("改写后的工具布局必须可读：\($0)") }.layout == remembered.layout, "工具布局改写为新格式")
        precondition(!String(decoding: try! Data(contentsOf: toolFile), as: UTF8.self).contains("floating"), "工具布局文件不再含浮动")
        note("PASS floating tools migrate to windows of their own: panel size and place kept, originals backed up, files rewritten strictly")
    }

    let startHome = home.appendingPathComponent("workspace-start-check", isDirectory: true)
    let transport = Transport(stateDirectory: startHome, resourcesDirectory: Bundle.main.resourceURL!)
    try! transport.prepareForLaunch()
    MacNativePersistenceGate.open()
    let startDevice = DeviceStore(transport: transport)
    startDevice.windowWorkspaceJournal.save(localLayout, id: UUID().uuidString)
    startDevice.windowWorkspaceJournal.flush()
    var wrongKind = localLayout
    wrongKind.windowKind = .single
    for rejectedScene in [remoteLayout, wrongKind, invalidLayout] {
        let restored = ConversationWindowModel(application: startDevice, kind: .workspace)
        restored.start(restoring: rejectedScene, seed: nil, persistenceID: UUID().uuidString)
        precondition(restored.savedLayout.layout == localLayout.layout && restored.savedLayout.panes == localLayout.panes,
                     "无效的直接给出布局不能阻止同设备同kind有效journal回退")
    }
    var validScene = remoteLayout
    validScene.windowDevice = .local
    let sceneWindow = ConversationWindowModel(application: startDevice, kind: .workspace)
    sceneWindow.start(restoring: validScene, seed: nil, persistenceID: UUID().uuidString)
    precondition(sceneWindow.savedLayout.layout == validScene.layout && sceneWindow.savedLayout.panes == validScene.panes,
                 "有效本scene布局仍优先于无关latest恢复点")
    let seeded = ConversationWindowModel(application: startDevice, kind: .workspace)
    seeded.start(restoring: remoteLayout, seed: WindowSeed(conversationID: "synthetic:explicit"), persistenceID: UUID().uuidString)
    precondition(seeded.focused.store.reachingConversationID == "synthetic:explicit" && seeded.savedLayout.panes != localLayout.panes,
                 "明确seed在无有效scene时导航，不套用无关latest")
    let restoredSeed = ConversationWindowModel(application: startDevice, kind: .workspace)
    restoredSeed.start(restoring: validScene, seed: WindowSeed(conversationID: "synthetic:original"), persistenceID: UUID().uuidString)
    precondition(restoredSeed.savedLayout.panes == validScene.panes,
                 "系统重开的seeded窗口仍恢复有效scene，不退回最初导航")
    precondition(!NSApp.isActive && NSApp.activationPolicy() == .prohibited)
    note("PASS actual window start validates scene device/kind/structure before latest fallback; valid scenes and seeded-window recovery retain precedence")
    workspaceShellChecks()
}

/// Hold the device queue before startup. Tab queues can read the addressed
/// histories, but no configuration has reached the Mac store yet.
@MainActor private func workspaceColdOpeningChecks(home: URL) throws {
    let directory = home.appendingPathComponent("cold-workspace-" + UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let state = directory.appendingPathComponent("state")
    try prepareMessageEditorFixture(home: directory, stateDirectory: state)
    let transport = Transport(stateDirectory: state, resourcesDirectory: Bundle.main.resourceURL!)
    let device = DeviceStore(transport: transport)
    let fixture = try JSONDecoder().decode([String: String].self,
        from: Data(contentsOf: directory.appendingPathComponent("fixture.json")))
    let id = fixture["conversationID"]!
    let otherID = "review:" + id.drop(while: { $0 != ":" }).dropFirst()
    let release = DispatchSemaphore(value: 0)
    device.performOnQueue { precondition(release.wait(timeout: .now() + 60) == .success) }
    func wait(_ step: String, _ condition: () -> Bool) {
        let started = Date(), deadline = started.addingTimeInterval(25)
        while !condition(), Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
        precondition(condition(), "工作区冷启动检查超时：\(step)，等待 \(Int(Date().timeIntervalSince(started)))s")
    }
    let known = ConversationStore(device: device), other = ConversationStore(device: device)
    let first = ConversationStore(device: device), draft = ConversationStore(device: device)
    let closed = ConversationStore(device: device), list = ConversationStore(device: device)
    list.start(loadFirstConversation: false)
    first.start()
    closed.start()
    closed.close()
    draft.start(loadFirstConversation: false, newConversation: true)
    known.start(opening: id)
    other.start()
    other.openLinkedConversation(id: otherID)

    wait("两个指定来源打开") { known.loadedConversationID == id && other.loadedConversationID == otherID }
    precondition(known.conversation(id: id)?.runtimeID == fixture["runtimeID"]
                 && other.conversation(id: otherID)?.runtimeID == "review")
    precondition(device.listRevision == 0 && first.loadedConversationID == nil && draft.loadedConversationID == nil,
                 "两个来源的详情应在设备首列表返回前打开；草稿须等待运行时配置")
    let peer = DeviceStore(transport: Transport(stateDirectory: directory.appendingPathComponent("other-device"),
                                               resourcesDirectory: Bundle.main.resourceURL!), peerID: "synthetic-other")
    let switched = ConversationStore(device: peer)
    switched.start(opening: id)
    wait("另一设备解析失败") { switched.openingFailure != nil }
    precondition(switched.loadedConversationID == nil && known.loadedConversationID == id,
                 "切换设备后同一 ID 必须由目标设备解析，不能借用旧设备的详情")
    switched.close()
    release.signal()
    wait("首列表后初始选择与草稿") { first.loadedConversationID != nil && draft.isUnsentConversation }
    precondition(device.listRevision == 1 && first.loadedConversationID == first.sidebarConversations.first?.id)
    precondition(closed.loadedConversationID == nil && list.loadedConversationID == nil
                 && other.loadedConversationID == otherID, "迟到的首列表不能改变显式导航或关闭的标签")
    for tab in [known, other, first, draft, list] { tab.close() }
    FileHandle.standardError.write(Data("PASS 同设备一次首列表，两个指定来源先打开，初始选择、草稿、导航与关闭均正确\n".utf8))
}

/// Model checks for presentation ownership; no windows are shown or activated.
@MainActor func workspaceContinuityChecks(home: URL) {
    func note(_ text: String) { FileHandle.standardError.write(Data((text + "\n").utf8)) }
    let device = DeviceStore(preview: true)
    let source = ConversationWindowModel(application: device, kind: .workspace, device: "synthetic-peer")
    let document = ArtifactDocument.mermaid(source: "graph TD; A-->B", conversationID: "sample-0", sourceID: "message")
    source.openArtifact(document)
    let artifact = source.artifactSheet!
    artifact.showsSource = true
    var seed: WindowSeed?
    source.openWindow = { kind, value in precondition(kind == .single); seed = value }
    source.moveArtifactSheetToWindow()
    precondition(seed?.tab == artifact.id && seed?.device == "synthetic-peer")
    let target = ConversationWindowModel(application: device, kind: .single, device: seed!.device)
    target.start(restoring: nil, seed: seed)
    precondition(target.focusedEditor === artifact && artifact.showsSource)
    source.openArtifact(document)
    precondition(source.artifactSheet == nil && target.focusedEditor === artifact && artifact.showsSource,
                 "再次查看必须复用移到另一个窗口的 Artifact")
    source.openInNewWindow("sample-1")
    precondition(seed?.conversationID == "sample-1" && seed?.device == "synthetic-peer")

    let otherDevice = DeviceStore(preview: true)
    let local = ConversationWindowModel(application: otherDevice, kind: .workspace)
    let count = local.tabCount
    local.drop(TabDragSource.link(for: artifact.id, conversation: nil), on: local.focusedPaneID, zone: .center)
    precondition(target.focusedEditor === artifact && local.tabCount == count,
                 "跨执行设备拖放不能把远端标签接到本机 core")
    local.drop(ConversationLink.url(runtimeID: "synthetic-runtime", conversationID: "sample-1"), on: local.focusedPaneID, zone: .center, from: source)
    precondition(local.tabCount == count, "列表拖放也必须保留执行设备身份")
    local.openArtifact(document)
    precondition(local.artifactSheet != nil && local.artifactSheet !== artifact,
                 "不同执行设备中相同消息 ID 不代表同一个 Artifact")
    note("PASS window ownership: remote seeds preserve device; Artifact reuse survives window handoff and source selection; foreign-device drops preserve both windows")

    workspaceRecoveryChecks(home: home)

    let notifiedDevice = DeviceStore(preview: true)
    var openings: [WindowSeed] = []
    notifiedDevice.openConversationWindow = { openings.append($0) }
    notifiedDevice.showConversation("sample-1")
    precondition(openings.count == 1 && openings[0].conversationID == "sample-1",
                 "所有会话窗口关闭时也必须请求打开通知目标")
    let notifiedWindow = ConversationWindowModel(application: notifiedDevice, kind: .workspace)
    notifiedWindow.start(restoring: nil, seed: openings[0])
    precondition(notifiedWindow.focused.store.loadedConversationID == "sample-1")
    precondition(!WindowPresentation.notificationTargets.contains(openings[0].id))
    let host = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 520, height: 420), styleMask: [.borderless], backing: .buffered, defer: false)
    let anchor = NSView(frame: host.contentView!.bounds)
    host.contentView = anchor
    notifiedWindow.register(notifiedWindow.focusedPaneID) { anchor }
    precondition(notifiedDevice.isShowingConversation("sample-1", keyWindow: host, active: true))
    precondition(!notifiedDevice.isShowingConversation("sample-1", keyWindow: host, active: false))
    let settings = NSWindow(contentRect: .zero, styleMask: [.borderless], backing: .buffered, defer: false)
    precondition(!notifiedDevice.isShowingConversation("sample-1", keyWindow: settings, active: true),
                 "设置窗口成为key时不能仍把上一个会话当作正在查看")
    notifiedWindow.openArtifact(document)
    precondition(!notifiedDevice.isShowingConversation("sample-1", keyWindow: host, active: true))
    notifiedWindow.moveArtifactSheetToTab()
    precondition(!notifiedDevice.isShowingConversation("sample-1", keyWindow: host, active: true),
                 "聚焦Artifact后不能继续抑制旧会话通知")
    notifiedWindow.noteWindowClosed()
    notifiedDevice.showConversation("sample-2")
    precondition(openings.count == 2 && openings[1].conversationID == "sample-2",
                 "已关闭但尚被scene保留的模型不能吞掉开窗请求")
    WindowPresentation.notificationTargets.remove(openings[1].id)
    precondition(!host.isVisible && !settings.isVisible)
    note("PASS notification presentation: no-window scene dispatch preserves target; closed models are excluded; key settings and Artifact content do not suppress a conversation's notification")
    precondition(!NSApp.isActive)
}
#endif
