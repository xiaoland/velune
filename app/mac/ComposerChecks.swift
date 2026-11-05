import AppKit
import VeluneBindings

extension ConversationWindowModel {
    /// Local invariant check; UI navigation and restoration are verified in
    /// the running preview separately. No runtime request is issued here.
    func checkComposerSharing() {
        let store = ConversationStore(device: application)
        let tab = ConversationTab(store: store, start: {})
        let first = ComposerDrafts(device: application), second = ComposerDrafts(device: application)
        let owner = ComposerDrafts.Owner.conversation("check-composer-sharing")
        let a = first.model(for: owner, tab: tab), b = second.model(for: owner, tab: tab)
        a.restore("SHARED LOCAL DRAFT", attachments: [])
        precondition(b.submission == a.submission, "同一会话的不同编辑器必须同步输入与明确选择")
        a.clear(submitted: a.submission, attachments: [])
        precondition(b.isEmpty, "消费输入必须在其他编辑器中原子清空")
        a.restore("MOVED LOCAL DRAFT", attachments: [])
        first.move(a, to: .shared, tab: tab)
        precondition(first.shared === a && b.id == a.id && b.submission == a.submission, "移动保留同一输入身份")
        a.clear(submitted: a.submission, attachments: [])
        precondition(b.isEmpty, "消费后所有引用必须一同清空，不能留下独立副本")
        FileHandle.standardError.write(Data("CHECK composer-sharing: session editors share input + intent; atomic consumption; move leaves no duplicate PASSED\n".utf8))
    }
}

/// Isolated, hidden native checks. Never registers hotkeys, creates scenes or activates a window.
@MainActor func composerOwnershipCheck() -> Never {
    NSApplication.shared.setActivationPolicy(.prohibited)
    if CommandLine.arguments.contains("--composer-check-content-target") { composerContentTargetCheck() }
    func wait(_ condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(15)
        while !condition(), Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
        precondition(condition(), "等待共享输入超时")
    }
    func note(_ value: String) { FileHandle.standardError.write(Data((value + "\n").utf8)) }
    if CommandLine.arguments.contains("--check-new-tab") {
    // New conversation opens its own tab; only an unsent one is restored in place.
    let device = DeviceStore(preview: true)
    let window = ConversationWindowModel(application: device, kind: .workspace)
    window.focused.store.selectConversation(id: "sample-0")
    window.tools.move(.composer, to: .docked(.bottom))
    let source = window.focused
    let baseTabs = window.tabCount
    window.openInNewTab()
    let draftInput = window.composer(for: window.focused)
    draftInput.restore("GLOBAL NEW INPUT", attachments: [])
    window.selectTab(source.id)
    note("step: before createConversation")
    precondition(window.createConversation() != nil, "有可用运行时的新建必须成功")
    note("step: created tab")
    precondition(window.tabCount == baseTabs + 2 && window.focused !== source, "新建会话必须新开并选中自己的标签")
    precondition(source.store.loadedConversationID == "sample-0", "原会话标签的内容不变")
    precondition(window.focused.store.loadedConversationID == ConversationStore.draftConversationPrefix + "new-conversation", "新标签承载未发送的新会话")
    note("step: tab state ok")
    precondition(window.composer(for: window.focused).id == draftInput.id && draftInput.submission == "GLOBAL NEW INPUT", "新标签恢复唯一的全局新建输入")
    note("step: global input ok")
    let runtime = device.enabledRuntimeInstances.last!
    window.selectTab(source.id)
    window.createConversation(in: .runtime(runtimeId: runtime.id))
    precondition(window.tabCount == baseTabs + 3 && window.composerContext().nextTurnRuntimeID == runtime.id, "组创建在新标签中应用显式运行时")
    note("step: group target ok")
    let tabsBeforeFailure = window.tabCount
    window.createConversation(in: .runtime(runtimeId: "missing-synthetic-runtime"))
    precondition(window.tabCount == tabsBeforeFailure, "不可用的运行时不新建标签、不改变窗口")
    note("step: invalid runtime ok")
    let reuse = ConversationWindowModel(application: device, kind: .workspace)
    reuse.focused.store.restoreUnsentConversation(Conversation(id: ConversationStore.draftConversationPrefix + "new-conversation", title: "新会话", updatedAtUnixMs: nil, runtimeID: runtime.id, cwd: nil, canRename: false, canDelete: false))
    let reuseTab = reuse.focused
    precondition(reuse.createConversation() === reuseTab && reuse.tabCount == 1, "未发送的新会话原地恢复，不叠加空标签")
    note("step: reuse ok")
    // Reopen closed tab (⇧⌘T): its conversation returns; a closed unsent
    // tab reopens as a fresh new one; deleted conversations are skipped.
    let reopen = ConversationWindowModel(application: device, kind: .workspace)
    reopen.focused.store.selectConversation(id: "sample-0")
    reopen.openInNewTab("sample-1")
    precondition(reopen.tabCount == 2)
    reopen.closeTab(reopen.focused.id)
    precondition(reopen.tabCount == 1, "关闭标签页")
    reopen.reopenClosedTab()
    precondition(reopen.tabCount == 2 && reopen.focused.store.loadedConversationID == "sample-1", "重新打开关闭的标签页恢复其会话")
    reopen.createConversation()
    let closedDraft = reopen.focused
    reopen.closeTab(closedDraft.id)
    reopen.reopenClosedTab()
    precondition(reopen.focused !== closedDraft && reopen.focused.store.loadedConversationID == ConversationStore.draftConversationPrefix + "new-conversation", "关闭的未发送新会话重开为新的新会话")
    reopen.openInNewTab("sample-2")
    reopen.closeTab(reopen.focused.id)
    device.deleteInPreview(["sample-2"])
    let tabsBeforeDead = reopen.tabCount
    reopen.reopenClosedTab()
    precondition(reopen.tabCount == tabsBeforeDead, "已删除会话的重开条目被跳过")
    note("step: reopen ok")

    // A blank tab opened while a first send is in flight never follows that
    // send's conversation: consuming the global new input rebinds the entry
    // to the next empty input at once.
    let raceHome = FileManager.default.temporaryDirectory.appendingPathComponent("velune-new-tab-\(UUID().uuidString)", isDirectory: true)
    try! FileManager.default.createDirectory(at: raceHome, withIntermediateDirectories: true)
    try! Data(#"{"schemaVersion":11,"gateways":[{"id":"g","name":"g","providers":[],"routes":[]}],"runtimeInstances":[{"enabled":true,"id":"synthetic-runtime","name":"合成","typeId":"pi-1.0","gatewayId":"g","settings":{}}],"modelTemplates":[]}"#.utf8).write(to: raceHome.appendingPathComponent("generic-config.json"))
    let raceTransport = Transport(stateDirectory: raceHome, resourcesDirectory: Bundle.main.resourceURL!)
    let raceDevice = DeviceStore(transport: raceTransport)
    let raceWindow = ConversationWindowModel(application: raceDevice, kind: .workspace)
    raceWindow.focused.store.restoreUnsentConversation(Conversation(id: ConversationStore.draftConversationPrefix + "new-conversation", title: "新会话", updatedAtUnixMs: nil, runtimeID: "synthetic-runtime", cwd: nil, canRename: false, canDelete: false))
    raceWindow.tools.move(.composer, to: .docked(.bottom))
    let sending = raceWindow.focused
    let sentInput = raceWindow.composer(for: sending)
    sentInput.restore("FIRST MESSAGE IN FLIGHT", attachments: [])
    raceDevice.composerInputs.saveTarget(ConversationStore.draftConversationPrefix + "new-conversation", choices: .init(runtimeId: "synthetic-runtime", runtimeOwnerId: raceDevice.composerInputs.ownerID, choices: [], choiceEdited: true))
    wait { raceDevice.composerInputs.inputID(for: ConversationStore.draftConversationPrefix + "new-conversation") == sentInput.id }
    // The send consumes the input; the global new entry takes the next one.
    sentInput.clear(submitted: sentInput.submission, attachments: [])
    raceDevice.composerInputs.consumeNewConversationInput(sentInput)
    wait { !raceDevice.enabledRuntimeInstances.isEmpty }
    // A restored background tab holds its conversation's identity before it
    // is first shown: its title follows the list, and the already-open
    // checks find it. (Preview stores always start on sample-0, so the
    // seeding is exercised on the real-transport device.)
    let paneID = UUID(), frontTabID = UUID(), backTabID = UUID()
    let arrangement = SavedWindowLayout(layout: .pane(paneID), panes: [paneID: .init(tabs: [frontTabID, backTabID], selected: frontTabID)], conversations: [backTabID: "synthetic-restored-b"], focused: paneID, tools: ToolWindowLayout(), pinned: false)
    let restoredWindow = ConversationWindowModel(application: raceDevice, kind: .workspace)
    restoredWindow.start(restoring: arrangement, seed: nil)
    let background = restoredWindow.allTabs.compactMap { $0 as? ConversationTab }.first { $0.id == backTabID }!
    precondition(background.store.selectedConversationID == "synthetic-restored-b", "恢复的后台标签在首次显示前即持有会话身份")
    precondition(restoredWindow.tab(showing: "synthetic-restored-b")?.1 === background, "恢复的后台标签被已打开检查识别，不会重复打开")
    raceWindow.openInNewTab()
    let blank = raceWindow.focused
    precondition(blank !== sending, "新标签未选中")
    wait { blank.store.isUnsentConversation }
    wait { raceWindow.composer(for: blank).id != sentInput.id }
    let freshInput = raceWindow.composer(for: blank)
    precondition(freshInput.submission.isEmpty && raceDevice.composerInputs.targetChoices(for: ConversationStore.draftConversationPrefix + "new-conversation")?.runtimeId == "synthetic-runtime", "发送期间新建的标签应取到下一份空输入并保留选择")
    // A failure before native creation can return the consumed input while
    // the replacement is still empty.
    let entryID = ConversationStore.draftConversationPrefix + "new-conversation"
    raceDevice.composerInputs.restoreConsumedNewConversationInput(sentInput, whileCreating: sending.store)
    precondition(raceDevice.composerInputs.inputID(for: entryID) == sentInput.id, "原生创建前失败应换回原输入")
    raceDevice.composerInputs.consumeNewConversationInput(sentInput)
    let fresh2 = raceDevice.composerInputs.inputID(for: entryID)!
    precondition(fresh2 != sentInput.id, "再次消费应换入另一份输入")
    wait { !raceDevice.composerInputs.isDirty(fresh2) }
    raceDevice.composerInputs.restorePending(fresh2, draft: SavedComposerDraft(text: "USER TYPED", attachments: []), revision: nil)
    raceDevice.composerInputs.restoreConsumedNewConversationInput(sentInput, whileCreating: sending.store)
    precondition(raceDevice.composerInputs.inputID(for: entryID) == fresh2, "新输入已有内容时发送失败不得顶替")
    _ = try! raceTransport.composerInputs(.promote(inputId: sentInput.id.uuidString.lowercased(), ownerId: raceDevice.composerInputs.ownerID!, conversationId: "synthetic-created-race", runtimeId: "synthetic-runtime"))
    raceDevice.composerInputs.poll(force: true)
    wait { raceDevice.composerInputs.state?.creations.contains(where: { $0.conversationId == "synthetic-created-race" }) == true }
    precondition(blank.store.isUnsentConversation && blank.store.loadedConversationID == ConversationStore.draftConversationPrefix + "new-conversation", "发送期间新建的空白标签不得变成刚创建的会话")
    // Native creation commits ownership even when its first dispatch fails.
    // Test an empty replacement too, so content protection cannot hide it.
    let afterCreation = ComposerModel()
    raceDevice.composerInputs.bind(entryID, input: afterCreation)
    let creation = raceDevice.composerInputs.state!.creations.first { $0.conversationId == "synthetic-created-race" }!
    sending.store.followComposerCreation(creation)
    precondition(!sending.store.isUnsentConversation, "原生交接立即保留创建身份，不等待首次发送")
    raceDevice.composerInputs.restoreConsumedNewConversationInput(sentInput, whileCreating: sending.store)
    precondition(raceDevice.composerInputs.inputID(for: entryID) == afterCreation.id, "原生已创建后首发失败应留在已有会话，不得重新占用全局新建输入")
    let reusedStore = ConversationStore(device: raceDevice)
    reusedStore.restoreUnsentConversation(Conversation(id: entryID, title: "新会话", updatedAtUnixMs: nil, runtimeID: "synthetic-runtime", cwd: nil, canRename: false, canDelete: false))
    raceDevice.composerInputs.bind(entryID, input: sentInput)
    raceDevice.composerInputs.consumeNewConversationInput(sentInput)
    precondition(raceDevice.composerInputs.inputID(for: entryID) != sentInput.id)
    raceDevice.composerInputs.restoreConsumedNewConversationInput(sentInput, whileCreating: reusedStore)
    precondition(raceDevice.composerInputs.inputID(for: entryID) == sentInput.id, "复用同一输入创建后续会话时，旧交接记录不能禁止当前创建失败恢复")
    note("PASS new conversation: own tab after the current one; source tab untouched; global input restored; group target applied; unavailable runtime is a no-op; unsent tab restored in place; restored background tabs keep titles; in-flight send does not capture a later blank tab; reopen closed tab restores conversations, drafts and skips deleted")
    exit(0)
    }
    if !CommandLine.arguments.contains("--composer-check-sync-only"), !CommandLine.arguments.contains("--composer-check-reopen"), !CommandLine.arguments.contains("--composer-check-targets") {
    let device = DeviceStore(preview: true)
    let window = ConversationWindowModel(application: device, kind: .workspace)
    window.focused.store.selectConversation(id: "sample-0")
    window.tools.move(.composer, to: .docked(.bottom))
    let a = window.focused
    let x = window.composer(for: a)
    x.restore("A 的未发送输入", attachments: [])
    window.composerContext(for: a).selectNextTurnRuntime(device.enabledRuntimeInstances.last!.id)
    window.openInNewTab("sample-1")
    let b = window.focused
    let y = window.composer(for: b)
    precondition(y.id != x.id && y.isEmpty, "同组导航必须切换输入")
    y.restore("B 的输入", attachments: [])
    window.selectTab(a.id)
    precondition(window.sharedComposer.id == x.id && window.sharedComposer.submission == "A 的未发送输入")
    window.openInNewTab()
    let n = window.composer(for: window.focused)
    n.restore("全局新建输入", attachments: [])
    window.selectTab(a.id)
    window.split(.horizontal)
    let right = window.focused, rightPane = window.focusedPaneID
    precondition(window.isSplit && window.composer(for: right).id == x.id, "拆分布局延续输入")
    window.selectTab(b.id)
    precondition(window.sharedComposer.id == y.id)
    window.focus(rightPane)
    precondition(window.composer(for: right).id == y.id, "跨组聚焦只换目标")
    precondition(right.store.createConversation(), "显式新建必须成功")
    precondition(window.composer(for: right).id == n.id && window.sharedComposer.submission == "全局新建输入", "显式新建恢复唯一新建项")
    window.selectTab(a.id)
    window.focus(rightPane)
    precondition(window.composer(for: right).id == x.id)
    // The exact submitted model, not the new target's parked input, is promoted.
    window.composerDrafts.reassociate(x, from: ConversationStore.draftConversationPrefix + "new-conversation", to: "native-created")
    precondition(device.composerInputs.inputID(for: ConversationStore.draftConversationPrefix + "new-conversation") == n.id)
    precondition(window.sharedComposer.id == x.id)
    window.closePane(rightPane)
    precondition(!window.isSplit && window.composer(for: window.focused).id == x.id)
    note("PASS native window actions: tab navigation; group focus; split/close continuity; explicit new; borrowed input identity")
    }

    guard let path = ProcessInfo.processInfo.environment["VELUNE_COMPOSER_CHECK_HOME"], path.hasPrefix("/tmp/") else { fatalError("必须指定隔离的 /tmp 检查目录") }
    let home = URL(fileURLWithPath: path, isDirectory: true)
    let transport = Transport(stateDirectory: home, resourcesDirectory: Bundle.main.resourceURL!)
    let firstDevice = DeviceStore(transport: transport)
    let secondDevice = DeviceStore(transport: transport)
    let first = ComposerDrafts(device: firstDevice), second = ComposerDrafts(device: secondDevice)
    let store = ConversationStore(device: firstDevice), otherStore = ConversationStore(device: secondDevice)
    let tab = ConversationTab(store: store, start: {}), otherTab = ConversationTab(store: otherStore, start: {})
    firstDevice.composerInputs.poll(force: true); secondDevice.composerInputs.poll(force: true)
    wait { firstDevice.composerInputs.state != nil && secondDevice.composerInputs.state != nil }
    if CommandLine.arguments.contains("--composer-check-targets") {
        let device = firstDevice
        let window = ConversationWindowModel(application: device, kind: .workspace)
        let unsent = Conversation(id: ConversationStore.draftConversationPrefix + "new-conversation", title: "新会话", updatedAtUnixMs: nil, runtimeID: "synthetic-runtime", cwd: "/tmp/creation-a", canRename: false, canDelete: false)
        window.focused.store.restoreUnsentConversation(unsent)
        window.tools.move(.composer, to: .docked(.bottom))
        let firstTab = window.focused
        let firstInput = window.composer(for: firstTab)
        firstInput.restore("one global new input", attachments: [])
        let secondStore = ConversationStore(device: device)
        window.focusedPane.insert(ConversationTab(store: secondStore, start: { secondStore.restoreUnsentConversation(unsent) }))
        let secondTab = window.focused
        secondTab.store.setUnsentWorkingDirectory("/tmp/creation-b")
        precondition(window.composer(for: secondTab).id == firstInput.id)
        let saved = try! JSONDecoder().decode(SavedWindowLayout.self, from: JSONEncoder().encode(window.savedLayout))
        precondition(saved.creationTargets?[firstTab.id]?.cwd == "/tmp/creation-a" && saved.creationTargets?[secondTab.id]?.cwd == "/tmp/creation-b")
        let restored = ConversationWindowModel(application: device, kind: .workspace)
        restored.start(restoring: saved, seed: nil)
        precondition(restored.focused.store.loadedConversationWorkingDirectory == "/tmp/creation-b")
        restored.selectTab(firstTab.id)
        let restoredInput = restored.composer(for: restored.focused)
        note("CHECK restored new tab: cwd=\(restored.focused.store.loadedConversationWorkingDirectory ?? "nil"); input=\(restoredInput.id); expected=\(firstInput.id)")
        precondition(restored.focused.store.loadedConversationWorkingDirectory == "/tmp/creation-a")
        precondition(restoredInput.id == firstInput.id)
        note("PASS new tabs: one shared InputID; separate creation targets survive native window snapshot roundtrip")
        exit(0)
    }
    let snapshot = home.appendingPathComponent("check-workspace.json")
    if CommandLine.arguments.contains("--composer-check-lifecycle") {
        let origin = ConversationWindowModel(application: firstDevice, kind: .workspace)
        let mirror = ConversationWindowModel(application: secondDevice, kind: .workspace)
        let unsent = Conversation(id: ConversationStore.draftConversationPrefix + "new-conversation", title: "新会话", updatedAtUnixMs: nil, runtimeID: "synthetic-runtime", cwd: nil, canRename: false, canDelete: false)
        origin.focused.store.restoreUnsentConversation(unsent); mirror.focused.store.restoreUnsentConversation(unsent)
        origin.tools.move(.composer, to: .docked(.bottom)); mirror.tools.move(.composer, to: .docked(.bottom))
        let persistence = UUID().uuidString
        origin.start(restoring: nil, seed: nil, persistenceID: persistence)
        origin.focused.store.setUnsentWorkingDirectory("/tmp/creation-before")
        origin.persistWorkspace(); firstDevice.windowWorkspaceJournal.flush()
        origin.focused.store.setUnsentWorkingDirectory("/tmp/creation-after")
        firstDevice.windowWorkspaceJournal.flush()
        let savedTarget = firstDevice.windowWorkspaceJournal.restore(id: persistence, recoverLatest: false)
        precondition(savedTarget?.creationTargets?[origin.focused.id]?.cwd == "/tmp/creation-after", "创建目标变化须独立持久化")
        let work = origin.composer(for: origin.focused)
        work.restore("first-send recovery", attachments: [])
        wait { !firstDevice.composerInputs.isDirty(work.id) }
        secondDevice.composerInputs.poll(force: true)
        wait { secondDevice.composerInputs.draft(work.id)?.text == "first-send recovery" }
        let reflected = mirror.composer(for: mirror.focused)
        precondition(reflected.id == work.id)
        var invalidations = 0
        let watch = reflected.objectWillChange.sink { invalidations += 1 }
        defer { watch.cancel() }
        secondDevice.composerInputs.poll(force: true)
        wait { invalidations > 0 }
        _ = try! transport.composerInputs(.promote(inputId: work.id.uuidString.lowercased(), ownerId: firstDevice.composerInputs.ownerID!, conversationId: "synthetic-created-native", runtimeId: "synthetic-runtime"))
        secondDevice.composerInputs.poll(force: true)
        wait { mirror.focused.store.selectedConversationID == "synthetic-created-native" && !mirror.focused.store.isUnsentConversation }
        precondition(mirror.sharedComposer.id == work.id && mirror.sharedComposer.submission == "first-send recovery")
        precondition(secondDevice.composerInputs.inputID(for: unsent.id) != work.id)
        note("PASS actual window model: creation target persisted without input edits; unchanged content refreshes submission access; another new editor adopts native identity and keeps its input")
        exit(0)
    }
    if CommandLine.arguments.contains("--composer-check-reopen") {
        let saved = try! JSONDecoder().decode(SavedComposerWorkspace.self, from: Data(contentsOf: snapshot))
        first.restore(saved)
        let restored = first.model(for: .conversation("synthetic-A"), tab: tab)
        precondition(restored.id == saved.conversations["synthetic-A"] && restored.submission == "跨设备完整输入" && restored.attachments.count == 1 && firstDevice.composerInputs.targetChoices(for: "synthetic-A")?.runtimeId == "synthetic-runtime")
        precondition(FileManager.default.isReadableFile(atPath: restored.attachments[0].path))
        note("PASS new process: same InputID, text, attachment bytes and execution choices restored")
        exit(0)
    }
    let source = home.appendingPathComponent("input.png")
    try! Data("synthetic attachment bytes".utf8).write(to: source)
    let input = first.model(for: .conversation("synthetic-A"), tab: tab)
    wait { firstDevice.composerInputs.inputID(for: "synthetic-A") != nil }
    let resolved = first.model(for: .conversation("synthetic-A"), tab: tab)
    resolved.restoreSavedDraft(.init(text: "跨设备完整输入", attachments: [source]))
    firstDevice.composerInputs.saveTarget("synthetic-A", choices: .init(runtimeId: "synthetic-runtime", runtimeOwnerId: firstDevice.composerInputs.ownerID, choices: [.init(ownerId: firstDevice.composerInputs.ownerID!, runtimeId: "synthetic-runtime", choice: .init(model: .native(providerId: "synthetic-provider", modelId: "synthetic-model"), effort: "high", permissionMode: nil))], choiceEdited: true))
    wait { !firstDevice.composerInputs.isDirty(resolved.id) }
    secondDevice.composerInputs.poll(force: true)
    wait { secondDevice.composerInputs.draft(resolved.id)?.text == "跨设备完整输入" }
    let remote = second.model(for: .conversation("synthetic-A"), tab: otherTab)
    precondition(remote.id == resolved.id && remote.attachments.count == 1 && secondDevice.composerInputs.targetChoices(for: "synthetic-A") == firstDevice.composerInputs.targetChoices(for: "synthetic-A"))
    precondition(try! Data(contentsOf: remote.attachments[0]) == Data("synthetic attachment bytes".utf8))
    try! JSONEncoder().encode(first.saved).write(to: snapshot)
    // Remove both transport cache and authoritative blob; text still hydrates.
    let metadata = firstDevice.composerInputs.state!.inputs.first { UUID(uuidString: $0.id) == resolved.id }!.content.attachments[0]
    let blob = home.appendingPathComponent("composer-attachments/" + metadata.id + "/" + metadata.name)
    try! FileManager.default.removeItem(at: blob)
    try! FileManager.default.removeItem(at: remote.attachments[0])
    var refreshes = 0
    let refreshWatch = secondDevice.composerInputs.changes.sink { refreshes += 1 }
    defer { refreshWatch.cancel() }
    secondDevice.composerInputs.poll(force: true)
    wait { refreshes > 0 }
    precondition(secondDevice.composerInputs.draft(resolved.id)?.text == "跨设备完整输入")
    precondition(!FileManager.default.isReadableFile(atPath: remote.attachments[0].path))
    try! Data("synthetic attachment bytes".utf8).write(to: blob)
    secondDevice.composerInputs.poll(force: true)
    wait { FileManager.default.isReadableFile(atPath: remote.attachments[0].path) }
    note("PASS actual shared application clients: full content synchronization; missing attachment isolates failure; forced retry restores bytes")
    _ = input
    exit(0)
}
