import AppKit
import VeluneBindings

/// State checks use native window models without displaying or activating windows.
@MainActor func composerContentTargetCheck() -> Never {
    NSApplication.shared.setActivationPolicy(.prohibited)
    func note(_ value: String) { FileHandle.standardError.write(Data((value + "\n").utf8)) }
    func wait(_ condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(15)
        while !condition(), Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
        precondition(condition(), "等待独立内容／目标配置同步超时")
    }
    let device = DeviceStore(preview: true)
    let window = ConversationWindowModel(application: device, kind: .workspace)
    window.focused.store.selectConversation(id: "sample-0")
    window.tools.move(.composer, to: .docked(.bottom))
    let a = window.focused
    let x = window.composer(for: a)
    let attachment = FileManager.default.temporaryDirectory.appendingPathComponent("velune-target-check-\(UUID()).txt")
    try! Data("synthetic attachment".utf8).write(to: attachment)
    defer { try? FileManager.default.removeItem(at: attachment) }
    x.restore("A content", attachments: [attachment])
    window.composerContext().selectNextTurnRuntime("sample-instance")
    window.composerContext().selectEffort("high")
    let aChoices = device.composerInputs.targetChoices(for: "sample-0")!
    window.openInNewTab("sample-2")
    let b = window.focused
    let y = window.composer(for: b)
    y.restore("B content", attachments: [attachment])
    window.composerContext().selectNextTurnRuntime("sample-review")
    precondition(y.id != x.id && window.composerContext().nextTurnRuntimeID == "sample-review")
    window.selectTab(a.id)
    precondition(window.sharedComposer.id == x.id && window.composerContext().nextTurnRuntimeID == "sample-instance")
    window.split(.horizontal)
    let right = window.focused
    let rightPane = window.focusedPaneID
    precondition(window.composer(for: right).id == x.id)
    window.selectTab(b.id)
    precondition(window.sharedComposer.id == y.id)
    window.focus(rightPane)
    precondition(window.composer(for: right).id == y.id && y.submission == "B content" && y.attachments == [attachment])
    precondition(window.composerContext().nextTurnRuntimeID == "sample-instance")
    precondition(device.composerInputs.targetChoices(for: "sample-0") == aChoices)
    window.tools.move(.composer, to: .builtIn)
    window.tools.move(.composer, to: .docked(.bottom))
    precondition(window.composer(for: right).id == y.id && window.composerContext().nextTurnRuntimeID == "sample-instance")
    let encoded = try! JSONSerialization.jsonObject(with: JSONEncoder().encode(y.savedDraft!)) as! [String: Any]
    precondition(Set(encoded.keys) == ["text", "attachments"])
    note("PASS same-group tabs restore their content and target choices; cross-group focus keeps text/attachments and takes destination choices; layout preserves both; content cache contains no execution choices")

    // Temporary side chats inherit once and own their choices in their Store.
    // They share runtime capabilities, never the ordinary target directory.
    let sourceChoices = device.composerInputs.targetChoices(for: "sample-0")
    let sourceNext = window.composerContext(for: a).nextTurn!
    let sideTarget = SideChatTarget(runtimeID: sourceNext.runtimeId, conversationID: "sample-0", title: "synthetic source", nextTurn: sourceNext)
    let sideStore = ConversationStore(device: device, sideChat: sideTarget)
    let sideTab = ConversationTab(store: sideStore, start: {})
    let sideInput = ComposerModel(followsConversation: false)
    sideInput.attach(sideStore); sideInput.restore("temporary side input", attachments: [])
    ComposerContext(tab: sideTab, model: sideInput).prepare()
    let native = ModelSelection.native(providerId: "sample", modelId: "sample-native")
    ComposerContext(tab: sideTab, model: sideInput).selectModel(native)
    precondition(ComposerContext(tab: sideTab, model: sideInput).chosenModel == native)
    ComposerContext(tab: sideTab, model: sideInput).selectNextTurnRuntime("sample-review")
    precondition(ComposerContext(tab: sideTab, model: sideInput).nextTurnRuntimeID == "sample-review")
    ComposerContext(tab: sideTab, model: sideInput).selectNextTurnRuntime("sample-instance")
    precondition(ComposerContext(tab: sideTab, model: sideInput).chosenModel == native && sideInput.submission == "temporary side input")
    device.saveRuntimeInstance(RuntimeInstance(id: "no-side-chat", name: "unsupported", typeId: .acp, gatewayId: device.runtimeInstances[0].gatewayId, settings: [:]))
    ComposerContext(tab: sideTab, model: sideInput).selectNextTurnRuntime("no-side-chat")
    precondition(ComposerContext(tab: sideTab, model: sideInput).nextTurnRuntimeID == "sample-instance")
    precondition(device.composerInputs.targetChoices(for: "sample-0") == sourceChoices)
    let reopenedSide = ConversationStore(device: device, sideChat: sideTarget)
    precondition(reopenedSide.keptComposerChoices.choices[sourceNext.runtimeId]?.model == sideTarget.selection)
    note("PASS temporary side-chat Store owns model/runtime memory; changing it preserves input and source choices, rejects unsupported runtimes and does not survive reopening")

    // Reproduce the old seed timing: input resolves for Pi while Codex is
    // still the store's loaded target. Input lookup must persist no choices.
    let delayed = DeviceStore(preview: true)
    var codex = delayed.runtimeInstances.first { $0.id == "sample-instance" }!
    codex.typeId = .codex
    delayed.saveRuntimeInstance(codex)
    let delayedStore = ConversationStore(device: delayed)
    delayedStore.selectConversation(id: "sample-0")
    let delayedTab = ConversationTab(store: delayedStore, start: {})
    let drafts = ComposerDrafts(device: delayed)
    let retry = drafts.model(for: .conversation("sample-2"), tab: delayedTab)
    precondition(delayedStore.nextTurnRuntimeID == "sample-instance")
    precondition(delayed.composerInputs.targetChoices(for: "sample-2") == nil)
    delayedStore.selectConversation(id: "sample-2")
    let piContext = ComposerContext(tab: delayedTab, model: retry)
    precondition(piContext.nextTurnRuntimeID == "sample-review")
    retry.restore("failed Pi turn", attachments: [])
    retry.clear(submitted: retry.submission, attachments: [], conversationID: nil)
    let cleared = retry.draftRevision(for: nil)
    retry.restoreFailed("failed Pi turn", attachments: [], for: nil, revision: cleared)
    retry.restoreSavedDraft(.init(text: "retry", attachments: []))
    precondition(ComposerContext(tab: delayedTab, model: retry).nextTurnRuntimeID == "sample-review")
    precondition(delayed.composerInputs.targetChoices(for: "sample-2")?.runtimeId == "sample-review")
    retry.restoreSavedDraft(.init(text: "new edit", attachments: []))
    retry.restoreFailed("late failed turn", attachments: [], for: nil, revision: cleared)
    precondition(retry.submission == "new edit")
    note("PASS Codex-loaded → Pi-input-resolved → Pi-opened → failed input restored → typed retry stays Pi; late failure preserves newer editing")

    guard let path = ProcessInfo.processInfo.environment["VELUNE_COMPOSER_CHECK_HOME"], path.hasPrefix("/tmp/") else { fatalError("需要隔离的 /tmp VELUNE_COMPOSER_CHECK_HOME") }
    let home = URL(fileURLWithPath: path, isDirectory: true)
    try! FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
    let configuration = home.appendingPathComponent("generic-config.json")
    if !FileManager.default.fileExists(atPath: configuration.path) {
        try! FileManager.default.createDirectory(at: home.appendingPathComponent("agent", isDirectory: true), withIntermediateDirectories: true)
        // Enabled runtime identity is real configuration; its deliberately
        // unavailable executable keeps this catalog-only fixture offline.
        let execution = try! JSONSerialization.data(withJSONObject: ["transport": "local", "home": ["existing": ["path": home.path]], "remoteResourcesDirectory": NSNull()])
        let seed: [String: Any] = ["schemaVersion": 11, "gateways": [["id": "g", "name": "synthetic", "providers": [], "routes": []]],
            "runtimeInstances": [["id": "pi", "name": "synthetic Pi", "enabled": true, "typeId": "pi-1.0", "gatewayId": "g", "settings": ["binary": home.appendingPathComponent("unavailable-pi").path, "nodeBinary": "/usr/bin/false", "agentDir": home.appendingPathComponent("agent").path, "execution": String(decoding: execution, as: UTF8.self)]]], "modelTemplates": []]
        try! JSONSerialization.data(withJSONObject: seed).write(to: configuration)
    }
    let transport = Transport(stateDirectory: home, resourcesDirectory: Bundle.main.resourceURL!)
    let first = DeviceStore(transport: transport), second = DeviceStore(transport: transport)
    first.start(); second.start()
    first.composerInputs.poll(force: true); second.composerInputs.poll(force: true)
    wait { first.composerInputs.state != nil && second.composerInputs.state != nil && first.enabledRuntimeInstances.contains { $0.id == "pi" } && second.enabledRuntimeInstances.contains { $0.id == "pi" } }
    let owner = first.composerInputs.ownerID!
    let draftID = ConversationStore.draftConversationPrefix + "new-conversation"
    let contentID = first.composerInputs.inputID(for: draftID)!
    first.composerInputs.save(contentID, draft: .init(text: "synchronized body", attachments: [attachment]))
    first.composerInputs.saveTarget("target-pi", choices: .init(runtimeId: "pi", runtimeOwnerId: owner, choices: [], choiceEdited: true))
    wait { first.composerInputs.state?.targets.contains(where: { $0.context == .conversation(ownerId: owner, conversationId: "target-pi") }) == true && !first.composerInputs.isDirty(contentID) }
    second.composerInputs.poll(force: true)
    wait { second.composerInputs.draft(contentID)?.text == "synchronized body" && second.composerInputs.targetChoices(for: "target-pi")?.runtimeId == "pi" }
    precondition(second.composerInputs.draft(contentID)?.attachments.count == 1)
    let contentRevision = first.composerInputs.state!.inputs.first { $0.id == contentID.uuidString.lowercased() }!.revision
    first.composerInputs.saveTarget("target-pi", choices: .init(runtimeId: "pi", runtimeOwnerId: owner, choices: [.init(ownerId: owner, runtimeId: "pi", choice: .init(model: nil, effort: "high", permissionMode: nil))], choiceEdited: true))
    wait { first.composerInputs.state?.targets.first(where: { $0.context == .conversation(ownerId: owner, conversationId: "target-pi") })?.revision == 2 }
    precondition(first.composerInputs.state!.inputs.first { $0.id == contentID.uuidString.lowercased() }!.revision == contentRevision)
    second.composerInputs.poll(force: true)
    wait { second.composerInputs.targetChoices(for: "target-pi")?.choices.first?.choice.effort == "high" }
    precondition(second.composerInputs.inputID(for: draftID) == contentID)
    note("PASS actual application clients synchronize content+attachment bytes and target choices separately; changing choices leaves content revision and global new input identity unchanged")
    let localChoices = first.composerInputs.targetChoices(for: "target-pi")!
    second.composerInputs.saveTarget("target-pi", choices: .init(runtimeId: "pi", runtimeOwnerId: owner, choices: [.init(ownerId: owner, runtimeId: "pi", choice: .init(model: nil, effort: "low", permissionMode: nil))], choiceEdited: true))
    wait { second.composerInputs.state?.targets.first(where: { $0.context == .conversation(ownerId: owner, conversationId: "target-pi") })?.revision == 3 }
    first.composerInputs.restoreTarget("target-pi", value: .init(choices: localChoices, revision: 2))
    wait { first.composerInputs.targetIssue(for: "target-pi")?.contains("重新选择") == true }
    first.composerInputs.poll(force: true)
    wait { first.composerInputs.state?.targets.first(where: { $0.context == .conversation(ownerId: owner, conversationId: "target-pi") })?.revision == 3 }
    precondition(first.composerInputs.targetChoices(for: "target-pi") == localChoices, "后台读取保留冲突选择且不得重基线")
    let conflictStore = ConversationStore(device: first)
    conflictStore.restoreUnsentConversation(Conversation(id: "target-pi", title: "synthetic target", updatedAtUnixMs: nil, runtimeID: "pi", cwd: nil, canRename: false, canDelete: false))
    let conflictTab = ConversationTab(store: conflictStore, start: {})
    let conflictInput = ComposerModel(followsConversation: false, id: contentID)
    conflictInput.attach(conflictStore); conflictInput.restoreSavedDraft(first.composerInputs.draft(contentID))
    ComposerContext(tab: conflictTab, model: conflictInput).prepare()
    wait { ComposerContext(tab: conflictTab, model: conflictInput).nextTurn != nil && ComposerContext(tab: conflictTab, model: conflictInput).nextTurnSettled }
    let reselect = ComposerContext(tab: conflictTab, model: conflictInput)
    precondition(reselect.chosenModel == nil)
    reselect.selectModel(nil) // The same visible value is an explicit choice.
    wait { first.composerInputs.targetIssue(for: "target-pi") == nil && first.composerInputs.state?.targets.first(where: { $0.context == .conversation(ownerId: owner, conversationId: "target-pi") })?.revision == 4 }
    precondition(first.composerInputs.state!.inputs.first { $0.id == contentID.uuidString.lowercased() }!.revision == contentRevision)
    second.composerInputs.poll(force: true)
    wait { second.composerInputs.targetChoices(for: "target-pi")?.choices.first?.choice.effort == "high" }
    conflictStore.close()
    note("PASS real two-client target CAS conflict preserves local choices; background poll does not overwrite/rebase; explicitly reselecting the same model accepts the latest revision and recovers without editing content")
    composerNativeTargetRetryCheck()
    exit(0)
}

/// Optional native fixture: the caller supplies only isolated synthetic homes.
@MainActor private func composerNativeTargetRetryCheck() {
    let environment = ProcessInfo.processInfo.environment
    guard let path = environment["VELUNE_COMPOSER_NATIVE_HOME"] else { return }
    guard path.hasPrefix("/tmp/"), let codex = environment["VELUNE_COMPOSER_NATIVE_CODEX_ID"], let pi = environment["VELUNE_COMPOSER_NATIVE_PI_ID"] else { fatalError("原生回归需要隔离目录与两个合成会话") }
    func wait(_ condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(30)
        while !condition(), Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
        precondition(condition(), "等待原生 Codex → Pi 回归步骤超时")
    }
    let transport = Transport(stateDirectory: URL(fileURLWithPath: path, isDirectory: true), resourcesDirectory: Bundle.main.resourceURL!)
    let device = DeviceStore(transport: transport)
    device.start()
    wait { device.composerInputs.ownerID != nil && device.composerInputs.state != nil && device.enabledRuntimeInstances.count == 2 && device.conversations.contains(where: { $0.id == codex }) && device.conversations.contains(where: { $0.id == pi }) }
    let store = ConversationStore(device: device)
    store.start(loadFirstConversation: false, newConversation: false)
    let tab = ConversationTab(store: store, start: {})
    let drafts = ComposerDrafts(device: device)
    store.selectConversation(id: codex)
    wait { store.loadedConversationID == codex && !store.isLoading && store.nextTurnRuntimeID == "codex" }
    let input = drafts.model(for: .shared, tab: tab)
    ComposerContext(tab: tab, model: input).selectNextTurnRuntime("codex")
    store.selectConversation(id: pi)
    // Focus keeps the current content; resolving the destination's parked
    // empty input while the old transcript is loaded must not seed Codex.
    drafts.focus(tab)
    let parked = drafts.model(for: .conversation(pi), tab: tab)
    precondition(parked.id != input.id)
    precondition(device.composerInputs.targetChoices(for: pi) == nil, "Pi 打开前不写入旧 Codex 配置")
    precondition(ComposerContext(tab: tab, model: input).nextTurn == nil, "目标正在打开时不可解析旧会话的下一轮配置")
    wait { store.loadedConversationID == pi && !store.isLoading }
    wait {
        let context = ComposerContext(tab: tab, model: input)
        context.prepare()
        return context.nextTurnRuntimeID == "pi" && context.nextTurnSettled && context.inputIssue == nil && store.canSend
    }
    input.restore("retry", attachments: [])
    let before = store.acceptedSubmissions
    let context = ComposerContext(tab: tab, model: input)
    precondition(context.nextTurnRuntimeID == "pi" && context.submission().canSubmitDraft)
    context.send()
    wait { store.acceptedSubmissions > before && store.canSend && !store.isGenerating }
    precondition(ComposerContext(tab: tab, model: input).nextTurnRuntimeID == "pi")
    precondition(device.composerInputs.targetChoices(for: pi)?.runtimeId == "pi")
    FileHandle.standardError.write(Data("PASS native Codex → delayed Pi open → existing 429 failure → typed retry dispatched to Pi and completed; target choices remain Pi\n".utf8))
    store.close()
}
