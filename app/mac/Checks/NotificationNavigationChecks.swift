#if VELUNE_DEV_CHECKS
import AppKit
import UserNotifications
import VeluneBindings

/// Uses real notification payloads, store navigation and window models with
/// synthetic conversations. No system notifications or messages are sent.
@MainActor func notificationNavigationChecks() -> Never {
    NSApplication.shared.setActivationPolicy(.prohibited)
    MacNativePersistenceGate.close()
    guard let path = ProcessInfo.processInfo.environment["VELUNE_EXPERIENCE_CHECK_HOME"], path.hasPrefix("/tmp/") else {
        fatalError("需要 /tmp 下的合成检查目录")
    }
    let home = URL(fileURLWithPath: path, isDirectory: true)
    try! FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
    func note(_ value: String) { FileHandle.standardError.write(Data((value + "\n").utf8)) }

    let local = DeviceStore(preview: true)
    let peer = DeviceStore(preview: true, peerID: "synthetic-peer")
    var submitted: [UNNotificationRequest] = []
    let localNotifier = ConversationNotifier(application: local) { submitted.append($0) }
    let router = ConversationNotificationRouter.shared
    let delegate = UNUserNotificationCenter.current().delegate
    let peerNotifier = ConversationNotifier(application: peer)
    precondition(delegate === router && UNUserNotificationCenter.current().delegate === router,
                 "启动另一设备不能替换进程级通知 delegate")
    let localTarget = ConversationNotificationTarget(conversationID: "sample-1", peerID: nil)
    let peerTarget = ConversationNotificationTarget(conversationID: "sample-1", peerID: peer.peerID)
    precondition(localTarget.requestIdentifier != peerTarget.requestIdentifier,
                 "不同设备的同编号会话不能互相替换通知")
    for target in [localTarget, peerTarget] {
        precondition(ConversationNotificationTarget(userInfo: target.userInfo) == target,
                     "通知载荷必须保留完整设备和会话身份")
    }
    let first = ConversationNotificationTarget(conversationID: "b:c", peerID: "a")
    let second = ConversationNotificationTarget(conversationID: "c", peerID: "a:b")
    precondition(first.requestIdentifier != second.requestIdentifier)
    precondition(ConversationNotificationTarget(userInfo: ["conversationID": "sample-1"]) == nil)
    precondition(!router.open(userInfo: ["conversationID": "sample-1", "deviceID": 1]))
    precondition(!router.open(userInfo: ["conversationID": "", "deviceID": ""]))
    note("PASS notification identity: process delegate survives multiple devices; payload and replacement IDs include device and conversation")

    let localWindow = ConversationWindowModel(application: local, kind: .workspace)
    localWindow.start(restoring: nil, seed: WindowSeed(conversationID: "sample-0"))
    let peerWindow = ConversationWindowModel(application: peer, kind: .single, device: peer.peerID)
    peerWindow.start(restoring: nil, seed: WindowSeed(conversationID: "sample-0", device: peer.peerID))
    precondition(router.open(userInfo: localTarget.userInfo))
    precondition(localWindow.focused.store.device === local && localWindow.focused.store.loadedConversationID == "sample-1")
    precondition(peerWindow.focused.store.loadedConversationID == "sample-0",
                 "最后启动的是远端 notifier 时，本机通知仍导航本机")
    precondition(router.open(userInfo: peerTarget.userInfo))
    precondition(peerWindow.focused.store.device === peer && peerWindow.focused.store.loadedConversationID == "sample-1")
    let count = peerWindow.tabCount
    precondition(router.open(userInfo: peerTarget.userInfo) && peerWindow.tabCount == count,
                 "重复点击通知应复用目标设备的同一会话标签")
    precondition(!router.open(userInfo: ["conversationID": "sample-1"]),
                 "旧通知缺设备身份且会话编号歧义时不能猜测设备")
    peer.deleteInPreview(["sample-2"])
    precondition(router.open(userInfo: ["conversationID": "sample-2"])
                 && localWindow.focused.store.loadedConversationID == "sample-2",
                 "旧通知仍能恢复唯一可确定设备的会话")
    let untouched = localWindow.tabCount
    precondition(!router.open(userInfo: ConversationNotificationTarget(conversationID: "sample-2", peerID: "unknown-peer").userInfo))
    precondition(localWindow.tabCount == untouched, "失效设备通知不能回退本机会话")
    note("PASS notification response routing: local and remote same-ID targets stay separate; repeated clicks reuse tabs; ambiguous legacy and unknown devices cannot misroute")

    var openings: [WindowSeed] = []
    peer.openConversationWindow = { openings.append($0) }
    peerWindow.noteWindowClosed()
    precondition(router.open(userInfo: peerTarget.userInfo))
    precondition(openings.count == 1 && openings[0].conversationID == peerTarget.conversationID
                 && openings[0].device == peer.peerID,
                 "被 scene 保留的已关闭窗口不能吞掉通知")
    WindowPresentation.notificationTargets.remove(openings[0].id)
    localWindow.noteWindowClosed()
    note("PASS closed notification windows: retained closed models are excluded from selection")

    var stale = local.previewSnapshots["sample-1"]!
    stale.runState = .running
    local.keepPreviewSnapshot(stale)
    let activityWindow = ConversationWindowModel(application: local, kind: .workspace)
    activityWindow.start(restoring: nil, seed: WindowSeed(conversationID: stale.conversation.id))
    let background = activityWindow.focused
    activityWindow.openInNewTab("sample-0", device: local)
    precondition(activityWindow.focused !== background && background.store.isGenerating)
    let running = ActiveConversation(contextRuntimeId: stale.contextRuntimeID, conversationId: stale.conversation.id,
        runState: .running, pendingInteractions: 1, parentConversationId: nil, blocksIdleSubmission: true)
    local.applyConversationActivity(.init(active: [running], finished: [], resumable: [], cursor: 0))
    precondition(background.store.tabIsRunning && background.store.tabIsWaiting)
    let completed = FinishedTurn(silent: false, sequence: 1, contextRuntimeId: stale.contextRuntimeID,
        conversationId: stale.conversation.id, outcome: .completed)
    let cancelled = FinishedTurn(silent: false, sequence: 2, contextRuntimeId: stale.contextRuntimeID,
        conversationId: "sample-0", outcome: .cancelled)
    let silent = FinishedTurn(silent: true, sequence: 3, contextRuntimeId: stale.contextRuntimeID,
        conversationId: "sample-2", outcome: .completed)
    let finished = ConversationActivity(active: [], finished: [completed, cancelled, silent], resumable: [], cursor: 3)
    local.applyConversationActivity(finished)
    localNotifier.observe(finished)
    precondition(activityWindow.focused !== background && background.store.isGenerating,
                 "验收应保留未选中标签的旧正文快照")
    precondition(!background.store.tabIsRunning && !background.store.tabIsWaiting,
                 "已结束的标签必须立即跟随设备状态，不能继续显示等待或运行")
    precondition(submitted.count == 1 && submitted[0].content.body == "已完成"
                 && ConversationNotificationTarget(userInfo: submitted[0].content.userInfo) == localTarget,
                 "后台完成应提交正确的通知，取消和静默轮次应跳过")
    activityWindow.noteWindowClosed()
    note("PASS background completion: unselected tab clears its running and waiting states from device activity, and one completion notification targets that tab")

    // A synthetic hub supplies real peer-store identity without starting its
    // endpoint. Queued reads cannot reach any device or personal data.
    let transport = Transport(stateDirectory: home.appendingPathComponent("hub"), resourcesDirectory: Bundle.main.resourceURL!)
    let hub = DeviceHub(local: transport)
    hub.localStore = local
    let remote = hub.store(for: "synthetic-notification-peer")
    let aggregate = ConversationWindowModel(application: local, kind: .workspace, hub: hub)
    aggregate.start(restoring: nil, seed: WindowSeed(conversationID: "sample-1"))
    let host = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 520, height: 420), styleMask: [.borderless], backing: .buffered, defer: false)
    let anchor = NSView(frame: host.contentView!.bounds)
    host.contentView = anchor
    aggregate.register(aggregate.focusedPaneID) { anchor }
    precondition(local.isShowingConversation("sample-1", keyWindow: host, active: true))
    precondition(!remote.isShowingConversation("sample-1", keyWindow: host, active: true),
                 "可见的本机同编号会话不能抑制远端通知")
    let oldRemoteWindow = ConversationWindowModel(application: remote, kind: .single, device: remote.peerID, hub: hub)
    oldRemoteWindow.noteWindowClosed()
    precondition(ConversationWindowModel.openNotificationConversation("sample-1", application: remote) === aggregate,
                 "旧远端窗口关闭后，通知应选中仍打开的聚合窗口")
    precondition(aggregate.focused.store.device === remote && aggregate.tab(showing: "sample-1", device: local) != nil,
                 "聚合窗口需添加属于远端的目标标签并保留本机同编号标签")
    let aggregateCount = aggregate.tabCount
    precondition(ConversationWindowModel.openNotificationConversation("sample-1", application: remote) === aggregate)
    precondition(aggregate.tabCount == aggregateCount)
    note("PASS aggregated notification navigation: closing a dedicated window routes into the live workspace; same-ID local tab cannot suppress or replace the remote tab")

    let saved = aggregate.savedLayout
    aggregate.noteWindowClosed()
    var hubOpenings: [WindowSeed] = []
    hub.openWorkspaceWindow = { hubOpenings.append($0) }
    remote.showConversation("synthetic-notification-target")
    precondition(hubOpenings.count == 1 && hubOpenings[0].device == remote.peerID,
                 "无窗口时经 hub 打开聚合窗口的 seed 必须携带远端身份")
    let seed = hubOpenings[0]
    precondition(WindowPresentation.notificationTargets.contains(seed.id))
    let restored = ConversationWindowModel(application: local, kind: .workspace, hub: hub)
    restored.start(restoring: saved, seed: seed)
    precondition(restored.focused.store.device === remote
                 && restored.focused.store.reachingConversationID == seed.conversationID,
                 "有效旧布局不能覆盖本次通知的设备和会话目标")
    precondition(restored.tab(showing: "sample-1", device: local) != nil
                 && restored.tab(showing: "sample-1", device: remote) != nil,
                 "通知打开目标时保留恢复布局中的两台设备标签")
    precondition(!WindowPresentation.notificationTargets.contains(seed.id))
    let restoredCount = restored.tabCount
    remote.showConversation("synthetic-notification-target")
    precondition(restored.tabCount == restoredCount && hubOpenings.count == 1)
    let backgroundLocal = restored.tab(showing: "sample-1", device: local)!.1
    restored.selectTab(backgroundLocal.id)
    precondition(backgroundLocal.store.loadedConversationID == "sample-1",
                 "预览后台标签激活后加载恢复目标的正文")
    backgroundLocal.store.noteRestoredConversation("sample-2")
    precondition(backgroundLocal.store.selectedConversationID == "sample-1",
                 "恢复身份入口不能改写已启动标签的当前选择")
    note("PASS notification seed after restoration: no-window hub dispatch preserves device; restored layout retains old tabs and opens the requested new tab exactly once")

    let unsentWindow = ConversationWindowModel(application: remote, kind: .workspace, hub: hub)
    precondition(unsentWindow.focused.store.selectedConversationID == nil
                 && unsentWindow.savedLayout.conversationDevices?[unsentWindow.focused.id] == remote.peerID,
                 "未发送且 selectedConversationID 为空的标签也必须保存设备身份")
    let paneID = UUID(), existingTab = UUID(), draftTab = UUID(), blankTab = UUID()
    peerWindow.sharedComposer.restore("SYNTHETIC LEGACY REMOTE INPUT", attachments: [])
    let legacyComposer = peerWindow.composerDrafts.saved
    let draft = Conversation(id: draftConversationId(tabIdentity: "legacy"), title: "合成远端草稿", updatedAtUnixMs: nil,
                             runtimeID: "synthetic-runtime", cwd: nil, canRename: false, canDelete: false)
    let legacy = SavedWindowLayout(layout: .pane(paneID), panes: [paneID: .init(tabs: [existingTab, draftTab, blankTab], selected: existingTab)],
                                   conversations: [existingTab: "synthetic-existing", draftTab: draft.id], focused: paneID,
                                   tools: ToolWindowLayout(), pinned: false, composer: peerWindow.sharedComposer.savedDraft,
                                   composerWorkspace: legacyComposer, creationTargets: [draftTab: draft],
                                   windowKind: .workspace, windowDevice: .remote(remote.peerID!))
    let migrated = legacy.migratingToAggregatedWorkspace()
    precondition(migrated.windowDevice == .local && migrated.composer == nil && migrated.composerWorkspace == nil)
    for tab in [existingTab, draftTab, blankTab] {
        precondition(migrated.conversationDevices?[tab] == remote.peerID,
                     "旧远端窗口中的已存在、未发送和空白标签均归原执行设备")
    }
    precondition(migrated.composerByDevice?[remote.peerID!] == legacy.composer
                 && migrated.composerWorkspaceByDevice?[remote.peerID!] == legacyComposer,
                 "旧窗口 composer 的输入身份、关联和未完成文字应整体移入原设备")
    precondition(migrated.migratingToAggregatedWorkspace() == migrated)
    precondition(try! SavedWindowLayout.decodeKnown(JSONEncoder().encode(migrated)) == migrated,
                 "多个 UUID 设备字段经真实编解码往返必须保持可恢复")
    var single = legacy
    single.windowKind = .single
    precondition(single.migratingToAggregatedWorkspace() == single)
    let migratedWindow = ConversationWindowModel(application: local, kind: .workspace, hub: hub)
    migratedWindow.start(restoring: legacy, seed: nil)
    precondition(migratedWindow.allTabs.compactMap { $0 as? ConversationTab }.allSatisfy { $0.store.device === remote })
    precondition(migratedWindow.drafts(for: remote).shared.id == legacyComposer.shared
                 && migratedWindow.drafts(for: remote).shared.submission == "SYNTHETIC LEGACY REMOTE INPUT"
                 && migratedWindow.sharedComposer.submission.isEmpty,
                 "旧远端 composer 恢复到远端输入目录，本机目录保持独立")
    let journal = WindowWorkspaceJournal(directory: home.appendingPathComponent("legacy-journal")) { fatalError($0) }
    let legacyID = UUID().uuidString
    journal.save(legacy, id: legacyID)
    journal.flush()
    let original = home.appendingPathComponent("legacy-journal/mac-window-workspaces/\(legacyID).json")
    let bytes = try! Data(contentsOf: original)
    precondition(journal.restore(id: legacyID, recoverLatest: false, aggregatingDevices: true) == migrated,
                 "没有直接给出布局时，journal 的同窗口旧远端恢复点仍会迁移")
    precondition(try! Data(contentsOf: original.appendingPathExtension("before-aggregation.bak")) == bytes)
    journal.save(migrated, id: legacyID)
    journal.flush()
    precondition(try! Data(contentsOf: original.appendingPathExtension("before-aggregation.bak")) == bytes,
                 "迁移后的保存不改写旧窗口的可恢复备份")
    note("PASS aggregated restoration: unsent tabs retain devices; old remote conversation/draft/blank tabs and composer identities migrate together; journal keeps a byte-exact backup")

    precondition(!host.isVisible && !NSApp.isActive && NSApp.activationPolicy() == .prohibited)
    withExtendedLifetime([localNotifier, peerNotifier]) {}
    note("PASS notification navigation checks: synthetic data only; no activation, system notifications or message submission")
    exit(0)
}
#endif
