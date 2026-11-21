import AppKit
import SwiftUI
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
    let rightInput = window.composer(for: right)
    precondition(rightInput.id != x.id && rightInput.isEmpty)
    rightInput.restore("right content", attachments: [attachment])
    window.composerContext().selectNextTurnRuntime("sample-instance")
    window.selectTab(b.id)
    precondition(window.sharedComposer.id == y.id)
    let editor = ComposerTextView()
    let coordinator = ComposerEditor.Coordinator(model: y)
    coordinator.view = editor; editor.delegate = coordinator
    editor.replace(y.submission)
    let undo = coordinator.undoManager(for: editor)!
    // Navigation above and a later keystroke are separate native events.
    // Finish the navigation event's automatic undo group before typing.
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    undo.beginUndoGrouping()
    editor.insertText(" plus editing", replacementRange: NSRange(location: (editor.string as NSString).length, length: 0))
    undo.endUndoGrouping()
    precondition(undo.canUndo && y.submission == "B content plus editing")
    window.focus(rightPane)
    precondition(window.composer(for: right) === rightInput && rightInput.submission == "right content" && rightInput.attachments == [attachment])
    precondition(window.composerContext().nextTurnRuntimeID == "sample-instance")
    precondition(device.composerInputs.targetChoices(for: "sample-0") == aChoices)
    window.tools.move(.composer, to: .builtIn)
    window.tools.move(.composer, to: .docked(.bottom))
    precondition(window.composer(for: right).id == rightInput.id && window.composerContext().nextTurnRuntimeID == "sample-instance")
    precondition(coordinator.undoManager(for: editor) === undo && undo.canUndo)
    undo.undo()
    precondition(y.submission == "B content" && y.attachments == [attachment], "目的Tab切换不改源Tab的正文和撤销历史")
    let encoded = try! JSONSerialization.jsonObject(with: JSONEncoder().encode(y.savedDraft)) as! [String: Any]
    precondition(Set(encoded.keys) == ["text", "attachments"])
    note("PASS tabs and group focus restore destination content/choices; layout preserves identity; source edits stay independent")

    // A submission captures its input object. Later navigation changes the
    // visible object, not the identity or revision used by its failure callback.
    let failureWindow = ConversationWindowModel(application: DeviceStore(preview: true), kind: .workspace)
    failureWindow.focused.store.selectConversation(id: "sample-0")
    failureWindow.tools.move(.composer, to: .docked(.bottom))
    let sourceTab = failureWindow.focused
    let submittedInput = failureWindow.composer(for: sourceTab)
    submittedInput.restore("failed A", attachments: [attachment])
    submittedInput.clear(submitted: submittedInput.submission, attachments: submittedInput.attachments)
    let clearedRevision = submittedInput.revision
    failureWindow.openInNewTab("sample-2")
    let destinationTab = failureWindow.focused
    let destinationInput = failureWindow.composer(for: destinationTab)
    destinationInput.restore("edited B", attachments: [])
    submittedInput.restoreFailed("failed A", attachments: [attachment], revision: clearedRevision)
    precondition(failureWindow.composer(for: destinationTab) === destinationInput && destinationInput.submission == "edited B")
    failureWindow.selectTab(sourceTab.id)
    precondition(failureWindow.composer(for: sourceTab) === submittedInput && submittedInput.submission == "failed A" && submittedInput.attachments == [attachment])
    submittedInput.clear(submitted: submittedInput.submission, attachments: submittedInput.attachments)
    let nextClearedRevision = submittedInput.revision
    submittedInput.restore("new A editing", attachments: [])
    failureWindow.selectTab(destinationTab.id)
    submittedInput.restoreFailed("late failed A", attachments: [attachment], revision: nextClearedRevision)
    precondition(submittedInput.submission == "new A editing" && submittedInput.attachments.isEmpty && destinationInput.submission == "edited B")
    note("PASS submission A → navigate B → delayed failure restores only A's captured InputID; newer A edits and B input remain intact")

    let mirror = ConversationWindowModel(application: failureWindow.application, kind: .workspace)
    mirror.focused.store.selectConversation(id: "sample-0")
    mirror.tools.move(.composer, to: .docked(.bottom))
    let previousMirror = mirror.composer(for: mirror.focused)
    precondition(previousMirror.id == submittedInput.id && previousMirror !== submittedInput)
    let movingEditor = ComposerTextView()
    let movingCoordinator = ComposerEditor.Coordinator(model: submittedInput)
    movingCoordinator.view = movingEditor; movingEditor.delegate = movingCoordinator
    movingEditor.replace(submittedInput.submission)
    let movingUndo = movingCoordinator.undoManager(for: movingEditor)!
    movingUndo.beginUndoGrouping()
    movingEditor.insertText(" before moving", replacementRange: NSRange(location: (movingEditor.string as NSString).length, length: 0))
    movingUndo.endUndoGrouping()
    precondition(movingUndo.canUndo && previousMirror.submission == submittedInput.submission)
    mirror.composerDrafts.adopt(submittedInput, conversationID: "sample-0", tab: mirror.focused)
    let otherMirrorTab = ConversationTab(store: ConversationStore(device: failureWindow.application), start: {})
    otherMirrorTab.store.selectConversation(id: "sample-0")
    precondition(mirror.composer(for: mirror.focused) === submittedInput && mirror.composer(for: otherMirrorTab) === submittedInput)
    precondition(movingCoordinator.undoManager(for: movingEditor) === movingUndo && movingUndo.canUndo)
    movingUndo.undo()
    precondition(submittedInput.submission == "new A editing")
    let staleIncoming = ComposerModel(id: submittedInput.id)
    staleIncoming.restore("stale source", attachments: [])
    let latestShared = failureWindow.application.composerInputs.draft(submittedInput.id)!
    mirror.composerDrafts.adopt(staleIncoming, conversationID: "sample-0", tab: mirror.focused)
    precondition(staleIncoming.savedDraft == latestShared && failureWindow.application.composerInputs.draft(submittedInput.id) == latestShared)
    precondition(mirror.composer(for: otherMirrorTab) === staleIncoming)
    note("PASS window adoption selects one model per InputID for all tabs, keeps matching native undo, and reconciles stale incoming content from the shared client")

    let transferDevice = DeviceStore(preview: true)
    let transferSource = ConversationWindowModel(application: transferDevice, kind: .workspace)
    transferSource.focused.store.selectConversation(id: "sample-0")
    transferSource.tools.move(.composer, to: .docked(.bottom))
    let movingTab = transferSource.focused
    let movingInput = transferSource.composer(for: movingTab)
    movingInput.restore("cross-window content", attachments: [attachment])
    transferSource.openInNewTab("sample-2")
    transferSource.selectTab(movingTab.id)
    transferSource.openComposerSheet("sample-0")
    let remainingSheet = transferSource.composerSheetTab!
    precondition(transferSource.composer(for: remainingSheet) === movingInput)
    let transferEditor = ComposerTextView()
    let transferCoordinator = ComposerEditor.Coordinator(model: movingInput)
    transferCoordinator.view = transferEditor; transferEditor.delegate = transferCoordinator
    transferEditor.replace(movingInput.submission)
    let transferUndo = transferCoordinator.undoManager(for: transferEditor)!
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    transferUndo.beginUndoGrouping()
    transferEditor.insertText(" with undo", replacementRange: NSRange(location: (transferEditor.string as NSString).length, length: 0))
    transferUndo.endUndoGrouping()
    let transferTarget = ConversationWindowModel(application: transferDevice, kind: .single)
    transferTarget.focused.store.selectConversation(id: "sample-0")
    _ = transferTarget.composer(for: transferTarget.focused)
    transferTarget.drop(TabDragSource.link(for: movingTab.id, conversation: nil), on: transferTarget.focusedPaneID, zone: .center, from: transferSource)
    let remainingInput = transferSource.composer(for: remainingSheet)
    precondition(transferTarget.composer(for: movingTab) === movingInput && remainingInput !== movingInput && remainingInput.id == movingInput.id)
    precondition(remainingSheet.composer === remainingInput && remainingInput.savedDraft == movingInput.savedDraft)
    let remainingEditor = ComposerTextView()
    let remainingCoordinator = ComposerEditor.Coordinator(model: remainingInput)
    remainingCoordinator.view = remainingEditor; remainingEditor.delegate = remainingCoordinator
    remainingEditor.replace(remainingInput.submission)
    let remainingUndo = remainingCoordinator.undoManager(for: remainingEditor)!
    precondition(remainingUndo !== transferUndo && !remainingUndo.canUndo && transferUndo.canUndo)
    transferUndo.undo()
    precondition(movingInput.submission == "cross-window content" && remainingInput.submission == movingInput.submission)
    precondition(transferSource.composer(for: remainingSheet) === remainingInput && transferTarget.composer(for: movingTab) === movingInput)
    note("PASS actual cross-window tab drop leaves a distinct source editor for the same InputID; native undo follows the moved editor, source undo starts independently, and content still synchronizes")

    // Temporary side chats inherit once and own their choices in their Store.
    // They share runtime capabilities, never the ordinary target directory.
    let sourceChoices = device.composerInputs.targetChoices(for: "sample-0")
    let sourceNext = window.composerContext(for: a).nextTurn!
    let sideTarget = SideChatTarget(runtimeID: sourceNext.runtimeId, conversationID: "sample-0", title: "synthetic source", nextTurn: sourceNext)
    let sideStore = ConversationStore(device: device, sideChat: sideTarget)
    let sideTab = ConversationTab(store: sideStore, start: {})
    window.presentSideChat(sideTab, as: .sheet)
    let sideInput = window.composer(for: sideTab)
    sideInput.restore("temporary side input", attachments: [])
    let durableIDs = Set(window.composerDrafts.saved.drafts.map(\.id))
    precondition(window.composerOwner(for: sideTab) == nil && !durableIDs.contains(sideInput.id))
    precondition(device.composerInputs.draft(sideInput.id) == nil)
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
    window.presentSideChat(sideTab, as: .split)
    precondition(window.composer(for: sideTab) === sideInput && sideInput.submission == "temporary side input")
    window.presentSideChat(sideTab, as: .sheet)
    precondition(window.composer(for: sideTab) === sideInput)
    window.selectComposerDraft(y, for: sideTab)
    window.newComposerDraft(for: sideTab)
    precondition(window.composer(for: sideTab) === sideInput, "临时侧聊不能借用或更换持久InputID")
    sideInput.clear(submitted: sideInput.submission, attachments: [])
    sideInput.restore(y.submission, attachments: [])
    precondition(sideInput.id != y.id && device.composerInputs.draft(sideInput.id) == nil && y.submission == "B content")
    precondition(Set(window.composerDrafts.saved.drafts.map(\.id)) == durableIDs)
    window.closeSheet()
    let reopenedSide = ConversationStore(device: device, sideChat: sideTarget)
    let reopenedTab = ConversationTab(store: reopenedSide, start: {})
    window.presentSideChat(reopenedTab, as: .sheet)
    let reopenedInput = window.composer(for: reopenedTab)
    precondition(reopenedInput.id != sideInput.id && reopenedInput.isEmpty && reopenedInput.attachments.isEmpty)
    precondition(reopenedSide.keptComposerChoices.choices[sourceNext.runtimeId]?.model == sideTarget.selection)
    precondition(device.composerInputs.draft(reopenedInput.id) == nil && Set(window.composerDrafts.saved.drafts.map(\.id)) == durableIDs)
    window.closeSheet()
    note("PASS side-chat sheet/split moves preserve one temporary input; copied text and attempted persistent-input borrowing never write shared content; closing/reopening resets input and choices")

    // Reproduce the old seed timing: input resolves for Pi while Codex is
    // still the store's loaded target. Input lookup must persist no choices.
    let delayed = DeviceStore(preview: true)
    var codex = delayed.runtimeInstances.first { $0.id == "sample-instance" }!
    codex.typeId = .codex166
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
    retry.clear(submitted: retry.submission, attachments: [])
    let cleared = retry.revision
    retry.restoreFailed("failed Pi turn", attachments: [], revision: cleared)
    retry.restoreSavedDraft(.init(text: "retry", attachments: []))
    precondition(ComposerContext(tab: delayedTab, model: retry).nextTurnRuntimeID == "sample-review")
    precondition(delayed.composerInputs.targetChoices(for: "sample-2")?.runtimeId == nil
                 && delayed.composerInputs.targetChoices(for: "sample-2")?.choiceEdited == false)
    retry.restoreSavedDraft(.init(text: "new edit", attachments: []))
    retry.restoreFailed("late failed turn", attachments: [], revision: cleared)
    precondition(retry.submission == "new edit")
    note("PASS Codex-loaded → Pi-input-resolved → Pi-opened → failed input restored → typed retry stays Pi; late failure preserves newer editing")

    guard let path = ProcessInfo.processInfo.environment["VELUNE_COMPOSER_CHECK_HOME"], path.hasPrefix("/tmp/") else { fatalError("需要隔离的 /tmp VELUNE_COMPOSER_CHECK_HOME") }
    let home = URL(fileURLWithPath: path, isDirectory: true)
    try! FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
    let configuration = home.appendingPathComponent("generic-config.json")
    let transport = Transport(stateDirectory: home, resourcesDirectory: Bundle.main.resourceURL!)
    if !FileManager.default.fileExists(atPath: configuration.path) {
        try! FileManager.default.createDirectory(at: home.appendingPathComponent("agent", isDirectory: true), withIntermediateDirectories: true)
        // Enabled runtime identity is real configuration; its deliberately
        // unavailable executable keeps this catalog-only fixture offline.
        let defaults = try! transport.list()
        let gateway = defaults.gateways.first?.id ?? "default"
        _ = try! transport.upsertRuntime(RuntimeInstance(enabled: true, id: "pi", name: "synthetic Pi", typeId: .pi,
            gatewayId: gateway, settings: ["binary": home.appendingPathComponent("unavailable-pi").path,
                "nodeBinary": "/usr/bin/false", "agentDir": home.appendingPathComponent("agent").path]),
            execution: RuntimeExecution(transport: .local, home: .existing(path: home.path), remoteResourcesDirectory: nil))
    }
    let first = DeviceStore(transport: transport), second = DeviceStore(transport: transport)
    first.start(); second.start()
    first.composerInputs.poll(force: true); second.composerInputs.poll(force: true)
    wait { first.composerInputs.state != nil && second.composerInputs.state != nil && first.enabledRuntimeInstances.contains { $0.id == "pi" } && second.enabledRuntimeInstances.contains { $0.id == "pi" } }
    // Real catalog restoration, including bindings not yet opened in this window.
    let aliasSource = ComposerModel()
    aliasSource.restoreSavedDraft(.init(text: "persisted legacy shared", attachments: [attachment]))
    for id in ["synthetic-alias-a", "synthetic-alias-b"] {
        first.composerInputs.bind(id, input: aliasSource)
        wait { first.composerInputs.pendingBinding(id) == nil && first.composerInputs.inputID(for: id) == aliasSource.id }
    }
    let aliases = ComposerDrafts(device: first)
    let aliasTab = ConversationTab(store: ConversationStore(device: first), start: {})
    let recoveredA = aliases.model(for: .conversation("synthetic-alias-a"), tab: aliasTab)
    wait { first.composerInputs.pendingBinding("synthetic-alias-a") == nil && !first.composerInputs.isDirty(recoveredA.id) }
    let recoveredB = aliases.model(for: .conversation("synthetic-alias-b"), tab: aliasTab)
    precondition(recoveredA.id != recoveredB.id && recoveredA.attachments.count == 1 && recoveredB.attachments.count == 1)
    recoveredA.restoreSavedDraft(.init(text: "independent persisted A", attachments: []))
    wait { !first.composerInputs.isDirty(recoveredA.id) }
    second.composerInputs.poll(force: true)
    wait { second.composerInputs.draft(recoveredA.id)?.text == "independent persisted A" }
    precondition(second.composerInputs.inputID(for: "synthetic-alias-a") == recoveredA.id
                 && second.composerInputs.inputID(for: "synthetic-alias-b") == recoveredB.id
                 && second.composerInputs.draft(recoveredB.id)?.text == "persisted legacy shared")
    let mirrorAliases = ComposerDrafts(device: second)
    let mirrorTab = ConversationTab(store: ConversationStore(device: second), start: {})
    precondition(mirrorAliases.model(for: .conversation("synthetic-alias-a"), tab: mirrorTab).id == recoveredA.id,
                 "同一会话在其他窗口仍接续同一输入，不额外拆分")
    for id in ["synthetic-cold-a", "synthetic-cold-b"] {
        first.composerInputs.bind(id, input: aliasSource)
        wait { first.composerInputs.pendingBinding(id) == nil && first.composerInputs.inputID(for: id) == aliasSource.id }
    }
    let cold = DeviceStore(transport: transport)
    let coldAliases = ComposerDrafts(device: cold)
    var coldSaved = coldAliases.saved
    coldSaved.drafts = [.init(id: aliasSource.id, name: "cold legacy", state: aliasSource.savedDraft, conversationID: "synthetic-cold-a")]
    coldSaved.conversations = ["synthetic-cold-a": aliasSource.id, "synthetic-cold-b": aliasSource.id]
    coldSaved.shared = aliasSource.id
    coldAliases.restore(coldSaved)
    let coldTab = ConversationTab(store: ConversationStore(device: cold), start: {})
    let coldInput = coldAliases.model(for: .conversation("synthetic-cold-a"), tab: coldTab)
    precondition(coldInput.id != aliasSource.id)
    wait { cold.composerInputs.ownerID != nil && cold.composerInputs.pendingBinding("synthetic-cold-a") == nil
        && cold.composerInputs.inputID(for: "synthetic-cold-a") == coldInput.id }
    precondition(coldAliases.model(for: .conversation("synthetic-cold-a"), tab: coldTab) === coldInput
                 && coldInput.attachments.count == 1, "冷启动先恢复的副本不能被首次目录读取覆盖")
    let scopedDraft = ConversationStore.draftConversationPrefix + UUID().uuidString
    let creationInput = ComposerModel()
    first.composerInputs.bind(scopedDraft, input: creationInput)
    wait { first.composerInputs.pendingBinding(scopedDraft) == nil && first.composerInputs.inputID(for: scopedDraft) == creationInput.id }
    precondition(aliases.model(for: .conversation(scopedDraft), tab: aliasTab).id == creationInput.id)
    first.composerInputs.promote(creationInput, to: "synthetic-promoted-alias", runtimeID: "pi")
    wait { first.composerInputs.pendingPromotions["synthetic-promoted-alias"] == nil
        && first.composerInputs.state?.creations.contains { $0.conversationId == "synthetic-promoted-alias" } == true }
    precondition(aliases.model(for: .conversation("synthetic-promoted-alias"), tab: aliasTab).id == creationInput.id,
                 "同一Tab首次创建的草稿和原生身份保留InputID，不误判为跨会话共用")
    note("PASS real catalog: legacy aliases restored independently and synchronized to second client; same-context mirror and draft→created promotion preserve InputID")
    let owner = first.composerInputs.ownerID!
    let conflictStore = ConversationStore(device: first)
    let conflictTab = ConversationTab(store: conflictStore, start: {})
    let draftID = conflictStore.unsentContextID
    let seedModel = ComposerModel()
    first.composerInputs.ensure(draftID, model: seedModel) { _ in }
    first.composerInputs.saveTarget(draftID, choices: .init(runtimeId: nil, runtimeOwnerId: nil, choices: [], choiceEdited: false))
    let targetContext = ComposerContextKey.conversation(ownerId: owner, conversationId: draftID)
    wait { first.composerInputs.inputID(for: draftID) != nil && first.composerInputs.state?.targets.contains(where: { $0.context == targetContext }) == true }
    let contentID = first.composerInputs.inputID(for: draftID)!
    let targetRevision = first.composerInputs.state!.targets.first { $0.context == targetContext }!.revision
    first.composerInputs.save(contentID, draft: .init(text: "synchronized body", attachments: [attachment]))
    first.composerInputs.saveTarget(draftID, choices: .init(runtimeId: "pi", runtimeOwnerId: owner, choices: [], choiceEdited: true))
    wait { first.composerInputs.state?.targets.first(where: { $0.context == targetContext })?.revision == targetRevision + 1 && !first.composerInputs.isDirty(contentID) }
    second.composerInputs.poll(force: true)
    wait { second.composerInputs.draft(contentID)?.text == "synchronized body" && second.composerInputs.targetChoices(for: draftID)?.runtimeId == "pi" }
    precondition(second.composerInputs.draft(contentID)?.attachments.count == 1)
    let contentRevision = first.composerInputs.state!.inputs.first { $0.id == contentID.uuidString.lowercased() }!.revision
    first.composerInputs.saveTarget(draftID, choices: .init(runtimeId: "pi", runtimeOwnerId: owner, choices: [.init(ownerId: owner, runtimeId: "pi", choice: .init(model: nil, effort: "high", permissionMode: nil))], choiceEdited: true))
    wait { first.composerInputs.state?.targets.first(where: { $0.context == targetContext })?.revision == targetRevision + 2 }
    precondition(first.composerInputs.state!.inputs.first { $0.id == contentID.uuidString.lowercased() }!.revision == contentRevision)
    second.composerInputs.poll(force: true)
    wait { second.composerInputs.targetChoices(for: draftID)?.choices.first?.choice.effort == "high" }
    precondition(second.composerInputs.inputID(for: draftID) == contentID)
    note("PASS actual application clients synchronize content+attachment bytes and target choices separately; changing choices leaves content revision and scoped blank input identity unchanged")
    let localChoices = first.composerInputs.targetChoices(for: draftID)!
    second.composerInputs.saveTarget(draftID, choices: .init(runtimeId: "pi", runtimeOwnerId: owner, choices: [.init(ownerId: owner, runtimeId: "pi", choice: .init(model: nil, effort: "low", permissionMode: nil))], choiceEdited: true))
    wait { second.composerInputs.state?.targets.first(where: { $0.context == targetContext })?.revision == targetRevision + 3 }
    first.composerInputs.restoreTarget(draftID, value: .init(choices: localChoices, revision: targetRevision + 2))
    wait { first.composerInputs.targetIssue(for: draftID)?.contains("重新选择") == true }
    first.composerInputs.poll(force: true)
    wait { first.composerInputs.state?.targets.first(where: { $0.context == targetContext })?.revision == targetRevision + 3 }
    precondition(first.composerInputs.targetChoices(for: draftID) == localChoices, "后台读取保留冲突选择且不得重基线")
    conflictStore.restoreUnsentConversation(Conversation(id: draftID, title: "synthetic target", updatedAtUnixMs: nil, runtimeID: "pi", cwd: nil, canRename: false, canDelete: false))
    let conflictInput = ComposerModel(id: contentID)
    conflictInput.attach(conflictStore); conflictInput.restoreSavedDraft(first.composerInputs.draft(contentID))
    wait { ComposerContext(tab: conflictTab, model: conflictInput).nextTurn != nil && ComposerContext(tab: conflictTab, model: conflictInput).nextTurnSettled }
    let reselect = ComposerContext(tab: conflictTab, model: conflictInput)
    precondition(reselect.chosenModel == nil)
    reselect.selectModel(nil) // The same visible value is an explicit choice.
    wait { first.composerInputs.targetIssue(for: draftID) == nil && first.composerInputs.state?.targets.first(where: { $0.context == targetContext })?.revision == targetRevision + 4 }
    precondition(first.composerInputs.state!.inputs.first { $0.id == contentID.uuidString.lowercased() }!.revision == contentRevision)
    second.composerInputs.poll(force: true)
    wait { second.composerInputs.targetChoices(for: draftID)?.choices.first?.choice.effort == "high" }
    // Historical-edit acceptance uses this same authority, but captures its
    // baseline before the native send rather than adopting a later revision.
    let beforeEdit = first.composerInputs.targetRecord(for: draftID)!
    let editedChoices = conflictStore.messageEditingTargetChoices(
        .init(runtimeID: "pi", choices: ["pi": .init(model: .native(providerId: "synthetic", modelId: "edited-model"), effort: "low", permissionMode: "full-access")]),
        base: beforeEdit.choices)!
    first.composerInputs.saveEditedTarget(draftID, choices: editedChoices, expectedRevision: beforeEdit.revision)
    wait { first.composerInputs.targetRecord(for: draftID)?.choices == editedChoices && first.composerInputs.pendingTarget(for: draftID) == nil }
    precondition(first.composerInputs.inputID(for: draftID) == contentID && first.composerInputs.draft(contentID)?.text == "synchronized body" && first.composerInputs.draft(contentID)?.attachments.count == 1)
    precondition(first.composerInputs.state!.inputs.first { $0.id == contentID.uuidString.lowercased() }!.revision == contentRevision)
    let forkID = "synthetic-edited-fork"
    first.composerInputs.saveEditedTarget(forkID, choices: editedChoices, expectedRevision: nil)
    wait { first.composerInputs.targetRecord(for: forkID)?.choices == editedChoices && first.composerInputs.pendingTarget(for: forkID) == nil }
    precondition(first.composerInputs.inputID(for: forkID) == nil, "编辑执行配置不得新建或重绑定普通正文")
    precondition(first.composerInputs.targetRecord(for: draftID)?.choices == editedChoices, "分叉只更新结果会话")
    let forkRevision = first.composerInputs.targetRecord(for: forkID)!.revision
    let competingForkChoices = ComposerTargetChoices(runtimeId: "pi", runtimeOwnerId: owner, choices: [], choiceEdited: true)
    first.composerInputs.saveEditedTarget(forkID, choices: competingForkChoices, expectedRevision: nil)
    wait { first.composerInputs.targetIssue(for: forkID)?.contains("重新选择") == true }
    first.composerInputs.saveTarget(forkID, choices: competingForkChoices)
    var retriedForkTarget = false
    first.composerInputs.flush { retriedForkTarget = true }
    wait { retriedForkTarget }
    precondition(first.composerInputs.targetIssue(for: forkID)?.contains("重新选择") == true)
    precondition(first.composerInputs.targetRecord(for: forkID)?.revision == forkRevision && first.composerInputs.pendingTarget(for: forkID)?.revision == nil,
                 "不存在基线的失败重试不能采纳随后发现的目标修订")
    let editBaseline = first.composerInputs.targetRecord(for: draftID)!.revision
    second.composerInputs.poll(force: true)
    wait { second.composerInputs.targetRecord(for: draftID)?.revision == editBaseline }
    let newerChoices = ComposerTargetChoices(runtimeId: "pi", runtimeOwnerId: owner, choices: [], choiceEdited: true)
    second.composerInputs.saveTarget(draftID, choices: newerChoices, userEdit: true)
    wait { second.composerInputs.targetRecord(for: draftID)?.revision == editBaseline + 1 }
    first.composerInputs.poll(force: true)
    wait { first.composerInputs.targetRecord(for: draftID)?.revision == editBaseline + 1 }
    first.composerInputs.saveEditedTarget(draftID, choices: editedChoices, expectedRevision: editBaseline)
    wait { first.composerInputs.targetIssue(for: draftID)?.contains("重新选择") == true }
    precondition(first.composerInputs.targetRecord(for: draftID)?.choices == newerChoices && first.composerInputs.targetChoices(for: draftID) == editedChoices,
                 "编辑完成期间改选不能被成功回调静默覆盖，已发送的配置保留为待同步")
    precondition(first.composerInputs.draft(contentID)?.text == "synchronized body" && first.composerInputs.draft(contentID)?.attachments.count == 1)
    conflictStore.close()
    note("PASS accepted edit updates runtime/model/effort/permission target only; fork leaves source/input identity/content/attachments unchanged; post-send concurrent target update remains a visible CAS conflict")
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
        return context.nextTurnRuntimeID == "pi" && context.nextTurnSettled && context.inputIssue == nil && store.canSend
    }
    input.restore("retry", attachments: [])
    let before = store.acceptedSubmissions
    let context = ComposerContext(tab: tab, model: input)
    precondition(context.nextTurnRuntimeID == "pi" && context.submission().canSubmitDraft)
    context.send()
    wait { store.acceptedSubmissions > before && store.canSend && !store.isGenerating }
    precondition(ComposerContext(tab: tab, model: input).nextTurnRuntimeID == "pi")
    precondition(device.composerInputs.targetChoices(for: pi)?.runtimeId == nil
                 && device.composerInputs.targetChoices(for: pi)?.choiceEdited == false)
    FileHandle.standardError.write(Data("PASS native Codex → delayed Pi open → existing 429 failure → typed retry dispatched to Pi and completed; raw intent inherits and effective target remains Pi\n".utf8))
    store.close()
}

/// Native model/undo check for tab-scoped input, with no runtime or UI activation.
@MainActor func composerTabStateCheck() -> Never {
    NSApplication.shared.setActivationPolicy(.prohibited)
    let device = DeviceStore(preview: true)
    let window = ConversationWindowModel(application: device, kind: .workspace)
    window.tools.move(.composer, to: .docked(.bottom))
    window.openInNewTab()
    let first = window.focused
    let a = window.composer(for: first)
    a.restoreSavedDraft(.init(text: "first blank", attachments: [URL(fileURLWithPath: "/tmp/synthetic-first.txt")]))
    window.composerContext().selectNextTurnRuntime("sample-instance")
    window.composerContext().selectEffort("high")
    let firstChoices = first.store.composerTargetChoices
    let firstNext = first.store.nextTurn
    window.openInNewTab()
    let second = window.focused
    let b = window.composer(for: second)
    precondition(first.store.loadedConversationID != second.store.loadedConversationID && a.id != b.id)
    precondition(b.isEmpty && b.attachments.isEmpty)
    b.restoreSavedDraft(.init(text: "second blank", attachments: []))
    window.composerContext().selectNextTurnRuntime("sample-review")
    let secondChoices = second.store.composerTargetChoices
    let secondNext = second.store.nextTurn
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    window.selectTab(first.id)
    precondition(window.composer(for: first) === a && a.submission == "first blank" && first.store.composerTargetChoices == firstChoices)
    let editor = ComposerTextView()
    let coordinator = ComposerEditor.Coordinator(model: a)
    editor.delegate = coordinator
    let undo = coordinator.undoManager(for: editor)!
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    precondition(undo.canUndo)
    undo.undo()
    precondition(a.submission == "second blank" && a.attachments.isEmpty && first.store.nextTurn?.runtimeId == secondNext?.runtimeId && first.store.nextTurn?.choice == secondNext?.choice)
    precondition(b.submission == "second blank" && second.store.composerTargetChoices == secondChoices)
    undo.redo()
    precondition(a.submission == "first blank" && a.attachments.count == 1 && first.store.nextTurn?.model == firstNext?.model && first.store.nextTurn?.runtimeId == firstNext?.runtimeId)
    window.split(.horizontal)
    let rightPane = window.focusedPaneID
    let rightInput = window.composer(for: window.focused)
    precondition(rightInput.id != a.id && rightInput.id != b.id && rightInput.isEmpty)
    rightInput.restoreSavedDraft(.init(text: "right group", attachments: []))
    window.selectTab(second.id)
    window.focus(rightPane)
    precondition(window.composer(for: window.focused) === rightInput && rightInput.submission == "right group")
    let saved = window.savedLayout
    let restored = ConversationWindowModel(application: device, kind: .workspace)
    restored.start(restoring: saved, seed: nil)
    for original in [first, second] {
        restored.selectTab(original.id)
        precondition(restored.focused.id == original.id && restored.focused.store.loadedConversationID == original.store.loadedConversationID, "Tab恢复身份不一致：\(restored.focused.id) / \(restored.focused.store.loadedConversationID ?? "nil")；原Tab \(original.id) / \(original.store.loadedConversationID ?? "nil")")
        precondition(restored.composer(for: restored.focused).savedDraft == window.composerDrafts.existingModel(for: .conversation(original.store.loadedConversationID!))?.savedDraft)
    }
    let inheritedDevice = DeviceStore(preview: true)
    let modelA = ModelSelection.native(providerId: "sample", modelId: "sample-native")
    let modelB = ModelSelection.managed(modelRecordKey: "sample-model")
    for (id, selection) in [("sample-0", modelA), ("sample-1", modelB)] {
        var snapshot = inheritedDevice.previewSnapshots[id]!
        snapshot.transcriptTurns[snapshot.transcriptTurns.count - 1].settings = TurnSettings(runtimeId: "sample-instance", model: nil, effort: selection == modelA ? "high" : nil, permissionMode: nil, modelSelection: selection)
        inheritedDevice.keepPreviewSnapshot(snapshot)
    }
    let inherited = ConversationWindowModel(application: inheritedDevice, kind: .workspace)
    inherited.tools.move(.composer, to: .docked(.bottom))
    inherited.focused.store.selectConversation(id: "sample-0")
    let inheritedA = inherited.focused
    precondition(inheritedA.store.nextTurn?.choice.model == nil && inheritedA.store.nextTurn?.model == modelA)
    inherited.openInNewTab("sample-1")
    let inheritedB = inherited.focused
    let inheritedInput = inherited.composer(for: inheritedB)
    precondition(inheritedB.store.nextTurn?.choice.model == nil && inheritedB.store.nextTurn?.model == modelB)
    let inheritedEditor = ComposerTextView()
    let inheritedCoordinator = ComposerEditor.Coordinator(model: inheritedInput)
    let inheritedUndo = inheritedCoordinator.undoManager(for: inheritedEditor)!
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    inheritedUndo.undo()
    precondition(inheritedB.store.nextTurn?.model == modelA && inheritedB.store.nextTurn?.effort == "high")
    precondition(inheritedA.store.nextTurn?.choice.model == nil && inheritedA.store.nextTurn?.model == modelA)
    inheritedUndo.redo()
    precondition(inheritedB.store.nextTurn?.model == modelB)
    composerRestoredGroupStateCheck()
    if let output = ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"] {
        let directory = URL(fileURLWithPath: output, isDirectory: true)
        try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let question = RuntimeInteraction(id: "synthetic-async-question", kind: .asyncUserInput(questions: [
            InteractionQuestion(id: "scope", text: "希望先处理哪一项？", options: [InteractionOption(id: "runtime", label: "运行时问题（推荐）"), InteractionOption(id: "workspace", label: "工作区体验")], secret: false),
            InteractionQuestion(id: "note", text: "有哪些补充约束？", options: [], secret: false, defaultText: "保留现有数据\n先验证再交付", multiline: true),
            InteractionQuestion(id: "checks", text: "需要哪些验证？（可多选）", options: [InteractionOption(id: "core", label: "核心行为"), InteractionOption(id: "ui", label: "原生界面")], secret: false, allowOther: false, multiple: true)
        ], delivery: .conversationInput))
        for (name, width, appearance) in [("wide", 760.0, NSAppearance.Name.aqua), ("narrow-dark", 420.0, NSAppearance.Name.darkAqua)] {
            let view = NSHostingView(rootView: RuntimeInteractionView(store: first.store, tab: first, interaction: question))
            let nativeWindow = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 520), styleMask: [.borderless], backing: .buffered, defer: false)
            nativeWindow.appearance = NSAppearance(named: appearance)
            nativeWindow.contentView = view
            view.layoutSubtreeIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.1))
            let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds)!
            view.cacheDisplay(in: view.bounds, to: bitmap)
            try! bitmap.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent("async-question-\(name).png"))
            nativeWindow.contentView = nil
        }
    }
    FileHandle.standardError.write(Data("PASS tab composer: independent blank content/choices; navigation undo+redo copies full state; group focus restores destination; workspace restores stable tab identities\n".utf8))
    exit(0)
}

/// Older shared composers could checkpoint one InputID into several contexts.
/// Restoring those bindings must not keep the conversations joined together.
@MainActor private func composerRestoredGroupStateCheck() {
    let device = DeviceStore(preview: true)
    let shared = ComposerModel()
    let attachment = FileManager.default.temporaryDirectory.appendingPathComponent("synthetic-restored-\(UUID()).txt")
    try! Data("synthetic restored attachment".utf8).write(to: attachment)
    defer { try? FileManager.default.removeItem(at: attachment) }
    shared.restoreSavedDraft(.init(text: "legacy shared input", attachments: [attachment]))
    for id in ["sample-0", "sample-1"] { device.composerInputs.bind(id, input: shared) }
    let window = ConversationWindowModel(application: device, kind: .workspace)
    window.tools.move(.composer, to: .docked(.bottom))
    var legacy = window.composerDrafts.saved
    legacy.drafts = [
        .init(id: shared.id, name: "legacy shared", state: shared.savedDraft, conversationID: "sample-0")
    ]
    legacy.conversations = ["sample-0": shared.id, "sample-1": shared.id]
    legacy.shared = shared.id
    legacy.activeContext = "sample-0"
    legacy.targetContext = "sample-1"
    window.composerDrafts.restore(legacy)
    window.focused.store.selectConversation(id: "sample-0")
    let left = window.focused
    let leftInput = window.composer(for: left)
    window.composerContext().selectNextTurnRuntime("sample-instance")
    window.composerContext().selectModel(.native(providerId: "sample", modelId: "sample-native"))
    window.composerContext().selectEffort("high")
    let leftNext = window.composerContext().nextTurn!
    precondition(leftNext.model != nil && leftNext.effort == "high", "源样本必须实际设置模型和推理强度")
    window.split(.horizontal)
    let right = window.focused
    right.store.selectConversation(id: "sample-1")
    let rightPane = window.focusedPaneID
    let rightInput = window.composer(for: right)
    precondition(leftInput.id != rightInput.id, "旧窗口的多重输入绑定必须恢复为独立会话草稿")
    precondition(leftInput.savedDraft == shared.savedDraft && rightInput.savedDraft == shared.savedDraft,
                 "隔离旧关联必须保留每份正文和附件")
    rightInput.restoreSavedDraft(.init(text: "right edited input", attachments: []))
    window.composerContext().selectNextTurnRuntime("sample-review")
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    window.selectTab(left.id)
    precondition(leftInput.savedDraft == shared.savedDraft && window.composerContext().nextTurnRuntimeID == "sample-instance")
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    window.focus(rightPane)
    precondition(window.composer(for: right) === rightInput && rightInput.submission == "right edited input"
                 && window.composerContext().nextTurnRuntimeID == "sample-review")
    let editor = ComposerTextView()
    let coordinator = ComposerEditor.Coordinator(model: rightInput)
    let undo = coordinator.undoManager(for: editor)!
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    undo.undo()
    precondition(rightInput.savedDraft == leftInput.savedDraft, "跨组撤销复制正文和附件")
    let undoNext = window.composerContext().nextTurn!
    precondition(undoNext.runtimeId == leftNext.runtimeId && undoNext.model == leftNext.model
                 && (undoNext.effort ?? undoNext.currentEffort) == (leftNext.effort ?? leftNext.currentEffort)
                 && (undoNext.permissionMode ?? undoNext.currentPermissionMode) == (leftNext.permissionMode ?? leftNext.currentPermissionMode),
                 "跨组撤销复制完整执行配置：\(undoNext.runtimeId) / \(undoNext.effort ?? undoNext.currentEffort ?? "nil")；预期\(leftNext.runtimeId) / \(leftNext.effort ?? leftNext.currentEffort ?? "nil")")
    undo.redo()
    precondition(rightInput.submission == "right edited input" && window.composerContext().nextTurnRuntimeID == "sample-review")
    composerGroupPresentationChecks(window: window, left: left, right: right)
    let saved = window.savedLayout
    let restored = ConversationWindowModel(application: device, kind: .workspace)
    restored.start(restoring: saved, seed: nil)
    restored.selectTab(left.id)
    precondition(restored.composer(for: restored.focused).savedDraft == leftInput.savedDraft)
    restored.selectTab(right.id)
    precondition(restored.composer(for: restored.focused).savedDraft == rightInput.savedDraft)
    precondition(device.composerInputs.draft(shared.id) != nil && leftInput.savedDraft == shared.savedDraft,
                 "拆分不删除原输入，另一会话仍保留未改写的正文和附件")
    var legacyOnly = saved
    legacyOnly.composerWorkspace = nil
    legacyOnly.composer = .init(text: "legacy unbound recovery", attachments: [attachment])
    let legacyWindow = ConversationWindowModel(application: device, kind: .workspace)
    legacyWindow.start(restoring: legacyOnly, seed: nil)
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    precondition(legacyWindow.composer(for: legacyWindow.focused).submission == "legacy unbound recovery"
                 && leftInput.savedDraft == shared.savedDraft,
                 "无目录的旧窗口草稿必须暂存在独立输入，不能写穿初始Tab的已有会话草稿")
    FileHandle.standardError.write(Data("PASS restored group composer: legacy aliases split without losing content/attachments; focus and undo/redo restore full state; restart preserves independent drafts\n".utf8))
}
