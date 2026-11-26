#if VELUNE_DEV_CHECKS
import AppKit
import Foundation
import VeluneBindings

/// Entry for --check-aggregated-list, before ordinary application startup.
/// Preview devices, preferences and the isolated sidebar search use the
/// caller's temporary directory. This check never opens a window or an endpoint.
@MainActor func aggregatedListChecks() -> Never {
    NSApplication.shared.setActivationPolicy(.prohibited)
    MacNativePersistenceGate.close()
    guard let path = ProcessInfo.processInfo.environment["VELUNE_EXPERIENCE_CHECK_HOME"] else {
        fatalError("需要 VELUNE_EXPERIENCE_CHECK_HOME 合成检查目录")
    }
    let root = URL(fileURLWithPath: path, isDirectory: true).standardizedFileURL.resolvingSymlinksInPath()
    let temporary = URL(fileURLWithPath: "/tmp", isDirectory: true).resolvingSymlinksInPath()
    precondition(path.hasPrefix("/") && root.path.hasPrefix(temporary.path + "/"),
                 "聚合检查目录必须位于 /tmp 下")
    let home = root.appendingPathComponent("aggregated-list-\(UUID().uuidString)", isDirectory: true)
    try! FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
    runAggregatedListChecks(home: home)
    try! FileManager.default.removeItem(at: home)
    exit(0)
}

@MainActor private func runAggregatedListChecks(home: URL) {
    func note(_ value: String) { FileHandle.standardError.write(Data((value + "\n").utf8)) }
    func wait(_ condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(3)
        while !condition(), Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
        precondition(condition(), "等待聚合列表投影／设置落盘超时")
    }
    func group(_ key: ConversationGroupKey) -> ConversationGroup {
        ConversationGroup(key: key, name: nil, total: 0, loaded: 0, latestActivityUnixMs: nil, collapsed: false)
    }
    func snapshot(_ keys: [ConversationGroupKey]) -> ConfigurationSnapshot {
        ConfigurationSnapshot(conversations: [], conversationGroups: keys.map(group), conversationProjects: [],
            workingDirectoryProjects: [:], showsArchived: false, historyFailures: [], gateways: [],
            runtimeInstances: [], runtimeTypes: [], modelTemplates: [],
            conversationBrowserLimits: ConversationBrowserLimits(page: 20, total: 200),
            outlinePresentation: .popover, linkedConversationOpening: .currentTab,
            usageDisplay: .used, contextUsageDisplay: .used, conversationBrowserPreferences: ConversationBrowser().preferences,
            statusLine: [], providerImportTypes: [], protocols: [], selectedRuntimeInstanceId: nil)
    }
    let a = ConversationGroupKey.project(path: "/synthetic-a")
    let b = ConversationGroupKey.runtime(runtimeId: "synthetic-b")
    let c = ConversationGroupKey.runtime(runtimeId: "synthetic-c")
    let localStore = DeviceStore(preview: true)
    let peerStore = DeviceStore(preview: true, peerID: "synthetic-peer")
    localStore.applyList(snapshot([.pinned, a, b, c]))
    peerStore.applyList(snapshot([.pinned, a, b]))
    let local = AggregatedListModel.Device(peerID: nil, store: localStore, name: "合成本机")
    let peer = AggregatedListModel.Device(peerID: peerStore.peerID, store: peerStore, name: "合成远端")
    let persistence = AggregatedListPersistence(home: home) { fatalError($0.localizedDescription) }
    let model = AggregatedListModel(local: localStore, hub: nil, persistence: persistence)
    model.updateDevices([local, peer])
    func section(_ device: AggregatedListModel.Device, _ key: ConversationGroupKey) -> AggregatedListModel.Section {
        model.sections.first { $0.device.id == device.id && $0.key == key }!
    }
    func orderedIDs(_ value: AggregatedListModel) -> [String] {
        value.sections.filter { ConversationBrowser.orderable($0.key) }.map(\.id)
    }
    let localA = section(local, a), localB = section(local, b), localC = section(local, c)
    let peerA = section(peer, a), peerB = section(peer, b)
    precondition(localA.id != peerA.id, "不同设备的同名组必须有独立身份")
    precondition(localA.displayTitle(multipleDevices: true) == localA.title)
    precondition(peerA.displayTitle(multipleDevices: false) == peerA.title)
    let pinnedSlots = model.sections.indices.filter { model.sections[$0].key == .pinned }
    model.move(peerB, before: localA)
    precondition(orderedIDs(model) == [peerB.id, localA.id, localB.id, localC.id, peerA.id],
                 "首次手动排序应立即采用移动后的顺序")
    precondition(model.groupOrder == orderedIDs(model))
    precondition(pinnedSlots.allSatisfy { model.sections[$0].key == .pinned }, "固定组不能参与手动移动")
    let firstOrder = model.groupOrder
    model.move(localA, before: localA)
    precondition(model.groupOrder == firstOrder, "拖到自身不能把组移到最后")
    note("PASS aggregated first move: moved order takes effect immediately; same-key device groups stay distinct; fixed groups and self drops are stable")

    // Simulate a project/runtime filter hiding B while the all-device view
    // remains active. Restoring it must restore B to its saved slot.
    localStore.applyList(snapshot([.pinned, a, c]))
    wait { !model.sections.contains { $0.id == localB.id } }
    model.move(localC, before: localA)
    let expected = [peerB.id, localC.id, localB.id, localA.id, peerA.id]
    precondition(model.groupOrder == expected, "移动可见组必须保留未显示组的原槽位")
    localStore.applyList(snapshot([.pinned, a, b, c]))
    wait { orderedIDs(model) == expected }
    model.setCollapsed(localA, true)
    precondition(model.isCollapsed(localA) && !model.isCollapsed(peerA))
    model.setFilter(.local)
    precondition(model.sections.map(\.key) == [.pinned, a, b, c], "本机筛选不能套用跨设备顺序")
    model.move(localC, before: localA)
    precondition(model.groupOrder == expected, "单设备移动由原浏览器负责")
    model.setFilter(.peer(peerStore.peerID!))
    precondition(model.sections.map(\.key) == [.pinned, a, b], "远端筛选须保留执行端返回顺序")
    let file = home.appendingPathComponent("mac-aggregated-list.json")
    wait {
        guard let data = try? Data(contentsOf: file),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return false }
        return json["filter"] as? String == "peer:synthetic-peer"
            && json["groupOrder"] as? [String] == expected && json["collapsed"] as? [String] == [localA.id]
    }
    let restored = AggregatedListModel(local: localStore, hub: nil,
        persistence: AggregatedListPersistence(home: home) { fatalError($0.localizedDescription) })
    restored.updateDevices([local, peer])
    precondition(restored.filter == .peer("synthetic-peer") && restored.isCollapsed(localA))
    precondition(restored.sections.map(\.key) == [.pinned, a, b])
    // Avoid scheduling another disk write on the restored instance.
    precondition(restored.groupOrder == expected)
    model.setFilter(.all)
    precondition(orderedIDs(model) == expected)
    note("PASS aggregated filtering/persistence: hidden slots survive moves; each single-device view keeps its own order; filter, qualified collapse and manual order survive reopening")

    let search = AggregatedListModel(local: localStore, hub: nil)
    search.updateDevices([local, peer])
    let peerList = search.listStore(for: peer)
    search.setSearchText("  synthetic needle  ")
    precondition(peerList.searchText == "synthetic needle" && peerList.searchRead.results == nil)
    precondition(search.sections.isEmpty, "等待搜索结果时不能把普通列表冒充搜索结果")
    let extraStore = DeviceStore(preview: true, peerID: "synthetic-new-peer")
    let extra = AggregatedListModel.Device(peerID: extraStore.peerID, store: extraStore, name: "新上线合成设备")
    search.updateDevices([local, peer, extra])
    precondition(search.listStore(for: extra).searchText == "synthetic needle", "搜索期间新增设备也必须查询")
    search.setFilter(.peer("synthetic-peer"))
    precondition(search.listStore(for: local).searchText.isEmpty && peerList.searchText == "synthetic needle")
    search.setSearchText("")
    precondition(peerList.searchText.isEmpty && peerList.searchRead.results == nil)
    precondition(search.sections.map(\.key) == [.pinned, a, b])
    note("PASS aggregated search scope: new devices join active query; filter changes and clearing invalidate previous queries")

    let searchHome = home.appendingPathComponent("isolated-search-core", isDirectory: true)
    let transport = Transport(stateDirectory: searchHome, resourcesDirectory: Bundle.main.resourceURL!)
    let runtimeID = "synthetic-sidebar-search"
    let agent = searchHome.appendingPathComponent("agent", isDirectory: true)
    let projects = agent.appendingPathComponent("projects/synthetic", isDirectory: true)
    try! FileManager.default.createDirectory(at: projects, withIntermediateDirectories: true)
    let nativeID = "00000000-0000-4000-8000-000000000001"
    let record: [String: Any] = ["type": "user", "uuid": "search-input", "parentUuid": NSNull(),
        "sessionId": nativeID, "cwd": "/synthetic", "timestamp": "2026-01-01T00:00:00Z",
        "message": ["role": "user", "content": "Synthetic sidebar search anchor"]]
    var bytes = try! JSONSerialization.data(withJSONObject: record)
    bytes.append(10)
    try! bytes.write(to: projects.appendingPathComponent(nativeID + ".jsonl"))
    let configuration = try! transport.list()
    for var runtime in configuration.runtimeInstances where runtime.enabled {
        runtime.enabled = false
        _ = try! transport.upsertRuntime(runtime)
    }
    _ = try! transport.upsertRuntime(RuntimeInstance(enabled: true, id: runtimeID, name: "搜索合成源",
        typeId: .claudeCode, gatewayId: configuration.gateways.first?.id ?? "default",
        settings: ["binary": "/synthetic/unused-agent", "nodeBinary": "/usr/bin/python3", "agentDir": agent.path]),
        execution: RuntimeExecution(transport: .local, home: .existing(path: searchHome.path), remoteResourcesDirectory: nil))
    let localSearchStore = DeviceStore(transport: transport)
    localSearchStore.applyList(try! transport.list())
    let secondHome = home.appendingPathComponent("recovering-search-core", isDirectory: true)
    let resources = Bundle.main.resourceURL!.path
    var firstOpen = true
    let disconnected = Transport(stateDirectory: secondHome, telemetry: transport.telemetryResult, remote: {
        if firstOpen {
            firstOpen = false
            throw TransportError.rejected("合成设备暂时不可达")
        }
        return try VeluneApplication.open(options: BindingOptions(homeDirectory: secondHome.path, resourcesDirectory: resources))
    })
    precondition((try? disconnected.prepareForLaunch()) == nil, "预置缓存连接错误")
    let disconnectedStore = DeviceStore(transport: disconnected, peerID: "synthetic-disconnected")
    let sources = AggregatedListModel(local: localSearchStore, hub: nil)
    let localSearch = sources.devices[0]
    let failedSearch = AggregatedListModel.Device(peerID: disconnectedStore.peerID, store: disconnectedStore, name: "合成离线设备")
    sources.updateDevices([localSearch, failedSearch])
    sources.setSearchText("sidebar search anchor")
    precondition(sources.searchLoading)
    wait { !sources.searchLoading && sources.sections.flatMap(\.conversations).contains { $0.id == "\(runtimeID):\(nativeID)" } }
    precondition(sources.searchFailures.map(\.id) == [failedSearch.id])
    sources.retrySearch(on: failedSearch)
    precondition(sources.searchFailures.first?.retrying == true)
    wait { !sources.searchLoading && sources.searchFailures.isEmpty }
    precondition(sources.sections.flatMap(\.conversations).contains { $0.id == "\(runtimeID):\(nativeID)" }, "部分失败与重试不能遮住其他设备的结果")
    try! transport.close()
    sources.retrySearch(on: localSearch)
    wait { !sources.searchLoading && sources.searchFailures.map(\.id) == [localSearch.id] }
    precondition(sources.sections.flatMap(\.conversations).contains { $0.id == "\(runtimeID):\(nativeID)" }, "同一搜索重试失败应保留已有结果")
    sources.setSearchText("other query")
    precondition(sources.sections.isEmpty, "换词后不能继续显示上一个搜索词的结果")
    wait { !sources.searchLoading }
    try! disconnected.close()
    note("PASS aggregated search recovery: real sidebar query, explicit retry bypasses cached open failure, successful-source result retention, same-query failure and new-query isolation")

    let unloadedStore = DeviceStore(preview: true, peerID: "synthetic-unloaded")
    unloadedStore.deleteInPreview(Set(unloadedStore.conversations.map(\.id)))
    let unloaded = AggregatedListModel.Device(peerID: unloadedStore.peerID, store: unloadedStore, name: "未加载设备")
    precondition(unloadedStore.listRevision == 0 && unloadedStore.conversations.isEmpty)
    let phases: [ControllerPhase] = [.disconnected, .suspended, .discovering, .connecting, .connected, .recovering, .blocked]
    for phase in phases {
        let status = ControllerStatus(peerId: "synthetic-peer", phase: phase, selected: false, foreground: false,
            paused: false, intentRevision: 0, connectionEpoch: 1, viaDeviceIds: [], errorCode: nil)
        precondition(unloaded.needsConnectionStatus(status), "未取得列表时所有连接阶段均须显示状态行")
        precondition(peer.needsConnectionStatus(status) == (phase != .connected), "已有列表掉线时也必须显示状态")
        precondition(!local.needsConnectionStatus(status))
    }
    precondition(unloaded.needsConnectionStatus(nil) && peer.needsConnectionStatus(nil))
    search.setFilter(.all)
    search.updateDevices([local, peer, unloaded])
    precondition(search.pendingDevices().map(\.id) == [peer.id, unloaded.id])
    search.setFilter(.peer("synthetic-unloaded"))
    precondition(search.pendingDevices().map(\.id) == [unloaded.id])
    note("PASS aggregated device status: no-list devices remain visible in every phase; retained lists show offline state; local and filtered devices are handled independently")

    // Exercise the production projection with two execution devices, including
    // a source project whose unrelated cwd must remain in its original group.
    let projectHome = home.appendingPathComponent("cross-projects", isDirectory: true)
    try! FileManager.default.createDirectory(at: projectHome, withIntermediateDirectories: true)
    let catalog = CrossDeviceProjectCatalog(home: projectHome)
    let linked = CrossDeviceProject(name: "共同项目", folders: [
        .init(peerID: nil, path: "/work/shared"),
        .init(peerID: "synthetic-windows", path: "C:\\work\\shared")])
    precondition(catalog.save([linked]))
    var duplicateFolder = linked
    duplicateFolder.folders.append(duplicateFolder.folders[0])
    precondition(!catalog.save([duplicateFolder]), "同项目重复目录身份不能被接受")
    duplicateFolder.folders[2].id = UUID()
    precondition(!catalog.save([duplicateFolder]), "同设备重复目录不能被接受")
    precondition(catalog.save([linked]))
    let projectLocal = DeviceStore(preview: true)
    let windows = DeviceStore(preview: true, peerID: "synthetic-windows")
    let localSource = ConversationGroupKey.project(path: "/source-project")
    let windowsSource = ConversationGroupKey.project(path: "C:\\source-project")
    func projectSnapshot(_ store: DeviceStore, key: ConversationGroupKey,
                         rows: [(String, String)]) -> ConfigurationSnapshot {
        var value = snapshot([key])
        value.runtimeInstances = store.runtimeInstances
        value.runtimeTypes = store.runtimeTypes
        value.conversationGroups = [ConversationGroup(key: key, name: nil, total: 8,
            loaded: UInt32(rows.count), latestActivityUnixMs: nil, collapsed: false)]
        value.conversations = rows.map { id, cwd in
            ListedConversation(conversation: ConversationSummary(id: id, title: .native(id),
                updatedAtUnixMs: nil, createdAtUnixMs: nil, runtimeId: "sample-instance", cwd: cwd,
                canRename: true, canDelete: true, parentConversationId: nil, nativeReadState: nil),
                nativeId: id, agentsUri: nil, pinned: false, unread: false, archived: false,
                project: "/source-project", moved: false, failed: false, group: key)
        }
        return value
    }
    projectLocal.applyList(projectSnapshot(projectLocal, key: localSource,
        rows: [("local-linked", "/work/shared/src"), ("local-unrelated", "/different")]))
    windows.applyList(projectSnapshot(windows, key: windowsSource,
        rows: [("windows-linked", "c:/WORK/shared/src")]))
    let projectDevices = [AggregatedListModel.Device(peerID: nil, store: projectLocal, name: "Mac"),
        AggregatedListModel.Device(peerID: windows.peerID, store: windows, name: "Windows")]
    let projectPreferences = AggregatedListPersistence(home: projectHome) { fatalError($0.localizedDescription) }
    let unified = AggregatedListModel(local: projectLocal, hub: nil, persistence: projectPreferences, crossProjects: catalog)
    unified.updateDevices(projectDevices)
    let shards = unified.sections.filter { $0.crossProjectID == linked.id }
    precondition(shards.count == 2 && shards.filter { unified.showsHeader($0) }.count == 1,
                 "本机和 Windows 必须共用一个逻辑项目 header")
    precondition(shards.first { $0.device.peerID == nil }?.conversations.map(\.id) == ["local-linked"])
    precondition(shards.first { $0.device.peerID == windows.peerID }?.conversations.map(\.id) == ["windows-linked"])
    precondition(unified.sections.first { $0.crossProjectID == nil && $0.device.peerID == nil }?.conversations.map(\.id) == ["local-unrelated"],
                 "原项目包含的其他 cwd 不能随附属目录一起迁入")
    for device in projectDevices {
        precondition(unified.sections.filter { $0.device.id == device.id && $0.hasMore }.count == 1,
                     "每个来源组只显示一个加载更多入口")
    }
    unified.setCollapsed(shards[0], true)
    precondition(shards.allSatisfy { unified.isCollapsed($0) }, "折叠必须覆盖整个逻辑项目")
    let window = ConversationWindowModel(application: projectLocal, kind: .workspace)
    for device in projectDevices {
        let folder = linked.folders.first { $0.peerID == device.peerID }!
        let tab = window.openNewConversationTab(device: device.store, runtimeID: nil, cwd: folder.path)!
        precondition(tab.store.device === device.store && tab.store.unsentConversation?.cwd == folder.path,
                     "项目新会话必须保留目标设备及其原始目录")
    }
    let projectPreferencesFile = projectHome.appendingPathComponent("mac-aggregated-list.json")
    wait { FileManager.default.fileExists(atPath: projectPreferencesFile.path) }
    let reopenedCatalog = CrossDeviceProjectCatalog(home: projectHome)
    precondition(reopenedCatalog.projects == [linked])
    let reopened = AggregatedListModel(local: projectLocal, hub: nil,
        persistence: AggregatedListPersistence(home: projectHome) { fatalError($0.localizedDescription) },
        crossProjects: reopenedCatalog)
    reopened.updateDevices(projectDevices)
    let reopenedShards = reopened.sections.filter { $0.crossProjectID == linked.id }
    precondition(reopenedShards.count == 2 && reopenedShards.allSatisfy { reopened.isCollapsed($0) })
    precondition(reopenedShards.filter { reopened.showsHeader($0) }.count == 1)
    note("PASS cross-device production projection: one header and collapse across Mac/Windows; cwd-qualified ownership; one pagination per source; new tabs retain device/cwd; catalog and collapse survive reopening")

    // Drain the model's final save before the temporary directory is removed.
    wait {
        guard let data = try? Data(contentsOf: file),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return false }
        return json["filter"] as? String == "all"
    }
    precondition(!NSApp.isActive && NSApp.activationPolicy() == .prohibited && !MacNativePersistenceGate.isEnabled)
    note("PASS aggregated list checks: isolated fixture; no windows, endpoints or model calls")
}
#endif
