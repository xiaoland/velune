import AppKit
import Foundation

/// Exercises journal selection and the real window-start candidate ordering.
/// All state stays in the caller's synthetic home; no windows are created.
@MainActor func workspaceRecoveryChecks(home: URL) {
    func note(_ text: String) { FileHandle.standardError.write(Data((text + "\n").utf8)) }
    let device = DeviceStore(preview: true)
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
    note("PASS workspace journal: exact and latest recovery stay within the execution device; unscoped legacy snapshots require an explicit scene identity")


    // A structurally invalid newest or exact snapshot cannot hide a valid
    // older candidate; a valid empty layout remains an ordinary candidate.
    var invalidLayout = localLayout
    invalidLayout.focused = UUID()
    let invalidID = UUID().uuidString
    journal.save(invalidLayout, id: invalidID)
    journal.flush()
    try! FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: 4)], ofItemAtPath: snapshots.appendingPathComponent(invalidID + ".json").path)
    let tolerantJournal = WindowWorkspaceJournal(directory: directory) { _ in }
    precondition(tolerantJournal.restore(id: UUID().uuidString, recoverLatest: true) == localLayout)
    precondition(tolerantJournal.restore(id: invalidID, recoverLatest: true) == localLayout)
    precondition(tolerantJournal.restore(id: invalidID, recoverLatest: false) == nil)
    precondition(tolerantJournal.restore(id: UUID().uuidString, recoverLatest: true, device: "synthetic-peer") == remoteLayout)
    note("PASS workspace journal filters incomplete layouts before exact/latest selection and keeps device isolation")

    let startHome = home.appendingPathComponent("workspace-start-check", isDirectory: true)
    let transport = Transport(stateDirectory: startHome, resourcesDirectory: Bundle.main.resourceURL!)
    let startDevice = DeviceStore(transport: transport)
    startDevice.windowWorkspaceJournal.save(localLayout, id: UUID().uuidString)
    startDevice.windowWorkspaceJournal.flush()
    var wrongKind = localLayout
    wrongKind.windowKind = .single
    for rejectedScene in [remoteLayout, wrongKind, invalidLayout] {
        let restored = ConversationWindowModel(application: startDevice, kind: .workspace)
        restored.start(restoring: rejectedScene, seed: nil, persistenceID: UUID().uuidString)
        precondition(restored.savedLayout.layout == localLayout.layout && restored.savedLayout.panes == localLayout.panes,
                     "无效SceneStorage不能阻止同设备同kind有效journal回退")
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
