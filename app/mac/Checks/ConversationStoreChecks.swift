#if VELUNE_DEV_CHECKS
import AppKit
import Combine
import VeluneBindings

extension ConversationStore {
    static func checkInteractionDraftLifetime(_ inspect: (ConversationWindowModel, String) -> Void) {
        let device = DeviceStore(preview: true)
        var value = device.previewSnapshots["sample-0"]!
        let request = RuntimeInteraction(id: "draft-lifetime", kind: .asyncUserInput(questions: [
            InteractionQuestion(id: "choice", text: "选择接下来的一步", options: [InteractionOption(id: "a", label: "检查界面"), InteractionOption(id: "b", label: "核对数据")], secret: false),
            InteractionQuestion(id: "text", text: "需要保留的约束", options: [], secret: false, defaultText: "默认正文", multiline: true)
        ], delivery: .conversationInput))
        value.pendingInteractions = [request]
        device.keepPreviewSnapshot(value)
        let window = ConversationWindowModel(application: device, kind: .workspace)
        let source = window.focused
        let store = source.store
        store.selectConversation(id: value.conversation.id)
        window.tools.move(.composer, to: .builtIn)
        inspect(window, "initial")
        let draft = store.interactionDraft(for: request)
        draft.selectedOptions["choice"] = ["a"]
        draft.values["text"] = "切换标签后仍保留的回答\n第二行"
        draft.page = 1
        inspect(window, "edited")
        window.openInNewTab("sample-1")
        inspect(window, "other-tab")
        window.selectTab(source.id)
        inspect(window, "returned")
        precondition(store.interactionDraft(for: request) === draft && draft.page == 1 && draft.selectedOptions["choice"] == ["a"])
        store.selectConversation(id: "sample-1")
        inspect(window, "other-context")
        store.selectConversation(id: value.conversation.id)
        inspect(window, "context-returned")
        precondition(store.interactionDraft(for: request) === draft && draft.page == 1)
        let key = InteractionDraftKey(conversationID: value.conversation.id, interactionID: request.id)
        store.checkAccess.interactionReplyFailures[key] = "连接暂时不可用，已保留回答。"
        store.selectConversation(id: "sample-1")
        precondition(store.interactionReplyFailure(request.id) == nil, "回答错误不能串到其他会话")
        store.selectConversation(id: value.conversation.id)
        precondition(store.interactionReplyFailure(request.id) != nil, "返回会话应保留该请求的错误")
        inspect(window, "failed")
        precondition(store.interactionDraft(for: request) === draft && draft.values["text"]?.hasPrefix("切换标签") == true)
        value.pendingInteractions = []
        store.checkAccess.apply(value)
        inspect(window, "resolved")
        precondition(store.checkAccess.interactionDrafts.isEmpty && store.checkAccess.interactionReplyFailures.isEmpty, "已完成/忽略/失效的问题应清理内存草稿：drafts=\(store.checkAccess.interactionDrafts.keys) pending=\(store.pendingInteractions.map(\.id)) failures=\(store.checkAccess.interactionReplyFailures)")
        precondition(draft.values.isEmpty && draft.selectedOptions.isEmpty && draft.otherAnswers.isEmpty && draft.page == 0, "已移除的草稿引用也应清除回答")
        value.pendingInteractions = [request]
        store.checkAccess.apply(value)
        let deletedDraft = store.interactionDraft(for: request)
        deletedDraft.values["text"] = "删除会话时应清理的私有回答"
        store.checkAccess.interactionReplyFailures[key] = "仅属于被删除会话的错误"
        store.checkAccess.resetProjection()
        inspect(window, "deleted")
        precondition(store.checkAccess.interactionDrafts.isEmpty && store.checkAccess.interactionReplyFailures.isEmpty && deletedDraft.values.isEmpty, "删除/缺失会话应清理回答及错误")
    }
    static func checkInteractionVisualBusy(_ store: ConversationStore, _ busy: Bool, replying: Bool = true) {
        precondition(store.isPreview, "视觉检查只能使用合成预览")
        store.checkAccess.isLoading = busy
        if busy, replying, let conversationID = store.loadedConversationID, let request = store.pendingInteractions.first {
            store.checkAccess.interactionReplyInFlight = (InteractionDraftKey(conversationID: conversationID, interactionID: request.id), "正在提交回答", UUID(), request)
        } else { store.checkAccess.interactionReplyInFlight = nil }
    }
    static func checkInteractionVisualFailure(_ store: ConversationStore, _ id: String, _ detail: String) {
        precondition(store.isPreview)
        guard let conversationID = store.loadedConversationID else { return }
        store.checkAccess.interactionReplyFailures[InteractionDraftKey(conversationID: conversationID, interactionID: id)] = detail
    }
}

extension ConversationStore {
    /// The normal periodic poll is deliberately absent in this check. The
    /// ordinary Send path must observe the runtime event after its receipt.
    static func checkLateSendEcho(transport: Transport, seed: SnapshotResult, home: URL) throws {
        let runtimeID = "synthetic-codex"
        let native = ModelSelection.native(providerId: "openai", modelId: "synthetic-native")
        let model = RuntimeModel(selection: native, name: "Synthetic", reasoningEfforts: [])
        let device = DeviceStore(transport: transport)
        device.applyList(try transport.list())
        let store = ConversationStore(device: device)
        store.checkAccess.modelCatalogs[runtimeID] = .ready([model])
        store.adopt(seed)
        store.checkAccess.started = true
        defer { store.close() }

        func wait(until ready: () -> Bool) {
            let deadline = ProcessInfo.processInfo.systemUptime + 2
            while !ready(), ProcessInfo.processInfo.systemUptime < deadline {
                RunLoop.main.run(until: Date().addingTimeInterval(0.01))
            }
        }
        store.checkAccess.pollProjection()
        wait { !store.checkAccess.pollPending && store.checkAccess.snapshot?.runState == .idle }
        let target = resolveNextTurn(input: NextTurnInput(runtimeId: runtimeID,
            runtimeType: device.runtimeTypes.first { $0.id == .codex166 }, providers: [],
            catalog: [model], turnSettings: [], conversationRuntimeId: runtimeID, modelRecordKey: nil,
            kept: TurnChoice(model: native), runtimeChoices: nil, newConversation: .chosen(choice: TurnChoice(model: native))))
        let prompt = "synthetic late user echo"
        func lastUserRowID() -> String? {
            store.transcript.rows.last(where: { $0.message.role == .user })?.id
        }
        let previous = lastUserRowID()
        guard previous != nil, store.checkAccess.snapshot?.runState == .idle, store.send(text: prompt, next: target) else {
            throw TransportError.rejected("late echo fixture could not send into the warm conversation")
        }
        wait { store.checkAccess.acceptedSubmissions == 1 && !store.checkAccess.isLoading }
        device.nativeUserMessageWakes.send("unrelated-conversation")
        guard !store.checkAccess.pollPending else {
            throw TransportError.rejected("a wake for another conversation must not start a read")
        }
        // Release after both of the previous implementation's reads would
        // have fired. This Store has no periodic timer.
        RunLoop.main.run(until: Date().addingTimeInterval(0.5))
        guard store.checkAccess.acceptedSubmissions == 1, lastUserRowID() == previous, store.isGenerating else {
            throw TransportError.rejected("an ACK alone must not add a transcript row")
        }
        let released = ProcessInfo.processInfo.systemUptime
        try Data().write(to: home.appendingPathComponent("release-user-echo"))
        wait { lastUserRowID() != previous }
        let matching = store.transcript.rows.filter { $0.message.role == .user && $0.message.copiedText == prompt }
        guard matching.count == 1, lastUserRowID() == matching.first?.id, store.isGenerating else {
            throw TransportError.rejected("expected one native input row before any assistant reply")
        }
        let elapsedMS = Int((ProcessInfo.processInfo.systemUptime - released) * 1000)
        print("PASS Core native wake delivered one user row \(elapsedMS)ms after release, with no periodic or manual poll")
    }

    /// Real Store → Transport → Core → warm Codex owner. No AppKit lifecycle.
    static func checkIndependentRequests(transport: Transport, seed: SnapshotResult, home: URL) throws {
        func note(_ text: String) { FileHandle.standardError.write(Data(("PASS " + text + "\n").utf8)) }
        func wait(_ label: String, _ condition: () -> Bool) {
            let deadline = Date().addingTimeInterval(12)
            while !condition(), Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
            guard condition() else {
                FileHandle.standardError.write(Data(("FAIL independent requests timeout: \(label)\n").utf8))
                exit(1)
            }
        }
        func require(_ condition: @autoclosure () -> Bool, _ detail: String = "synthetic invariant", line: UInt = #line) {
            guard condition() else {
                FileHandle.standardError.write(Data(("FAIL independent requests at line \(line): \(detail)\n").utf8))
                exit(1)
            }
        }
        let device = DeviceStore(transport: transport)
        device.applyList(try transport.list())
        let store = ConversationStore(device: device)
        let native = ModelSelection.native(providerId: "openai", modelId: "synthetic-native")
        store.checkAccess.modelCatalogs["synthetic-codex"] = .ready([RuntimeModel(selection: native, name: "Synthetic", reasoningEfforts: [])])
        store.adopt(seed)
        store.checkAccess.started = true
        // Reuse the provider-confirmed owner from the completed seed turn.
        wait("idle seed") { store.checkAccess.pollProjection(); return store.checkAccess.snapshot?.runState == .idle }
        let target = resolveNextTurn(input: NextTurnInput(runtimeId: "synthetic-codex",
            runtimeType: device.runtimeTypes.first { $0.id == .codex166 }, providers: [],
            catalog: [RuntimeModel(selection: native, name: "Synthetic", reasoningEfforts: [])], turnSettings: [],
            conversationRuntimeId: "synthetic-codex", modelRecordKey: nil, kept: TurnChoice(model: native), runtimeChoices: nil,
            newConversation: .chosen(choice: TurnChoice(model: native))))
        var accepted = 0
        require(store.send(text: "one held warm submission", next: target, onAccepted: { _ in accepted += 1 }), "warm Send must be admitted")
        require(store.checkAccess.isLoading, "ordinary Send must remain pending")
        wait("send reached peer") { FileManager.default.fileExists(atPath: home.appendingPathComponent("send-entered").path) }
        wait("poll installs question during ordinary ACK") {
            store.checkAccess.pollProjection()
            return !store.pendingInteractions.isEmpty
        }
        require(store.checkAccess.isLoading, "poll must finish before ordinary ACK")
        let deferredIDs = store.checkAccess.snapshot!.messages.filter(\.hasDeferredPayload).map(\.id)
        require(!deferredIDs.isEmpty, "real helper must deliver deferred payload")
        store.loadPayloads(messageIDs: deferredIDs)
        require(store.checkAccess.payloadPending && store.checkAccess.payloadDemand.count == max(0, deferredIDs.count - 32), "one payload batch must contain at most 32 messages")
        for _ in 0..<20 { store.loadPayloads(messageIDs: deferredIDs) }
        require(store.checkAccess.payloadPending && store.checkAccess.payloadDemand.count <= deferredIDs.count, "payload admission must coalesce")
        wait("payload during held ACK") { !store.checkAccess.payloadPending && store.checkAccess.payloadDemand.isEmpty }
        require(store.checkAccess.isLoading && store.checkAccess.snapshot!.messages.allSatisfy { !$0.hasDeferredPayload }, "shown message payload must read independently")
        guard let interaction = store.pendingInteractions.first else { throw TransportError.rejected("synthetic question disappeared before reply") }
        require(store.canReply(to: interaction), "card buttons/submit must remain admitted while ordinary loading")
        var replaced = interaction
        replaced.id += "-obsolete"
        require(!store.canReply(to: replaced), "expired interaction must not be admitted")
        store.replyInteraction(interaction, reply: .decision(optionId: "accept"))
        store.replyInteraction(interaction, reply: .decision(optionId: "accept"))
        require(!store.canReply(to: interaction), "independent reply admission must prevent double-submit")
        wait("reply ACK consumed while ordinary ACK held") {
            store.checkAccess.pollProjection()
            return store.checkAccess.replyAttempt == nil && FileManager.default.fileExists(atPath: home.appendingPathComponent("reply-wire").path)
        }
        require(store.checkAccess.isLoading && accepted == 0, "ordinary ACK must still be held after reply settles")
        let lines = try String(contentsOf: home.appendingPathComponent("reply-wire"), encoding: .utf8).split(separator: "\n")
        require(lines.count == 1, "reply must reach wire exactly once")
        store.cancel()
        wait("native cancellation before send ACK") { FileManager.default.fileExists(atPath: home.appendingPathComponent("cancel-wire").path) }
        require(store.checkAccess.isLoading, "native stop must reach peer before ordinary ACK release")
        try Data().write(to: home.appendingPathComponent("release-send"))
        wait("ordinary ACK release") { !store.checkAccess.isLoading }
        require(accepted == 1 && store.checkAccess.acceptedSubmissions == 1, "Send receipt must settle exactly once")
        let promptProcesses = try String(contentsOf: home.appendingPathComponent("prompt-wire"), encoding: .utf8).split(separator: "\n")
        require(promptProcesses.count == 2 && Set(promptProcesses).count == 1, "warm Send must reuse the confirmed owner without replay")
        note("Store/Core/helper: held warm Send ACK does not block poll/question, coalesced payload, one-shot reply or native stop; one accepted receipt, no replay")
        store.close()
        require(!store.canReply(to: interaction), "closed Store cannot reply")

        // A real payload request captured before refresh must be retried using
        // the new scope without another viewport notification. Only the refresh
        // completion is gated/replayed from an actual identical Core delivery.
        let payloadSeed = try transport.snapshot(runtimeID: "synthetic-codex", conversationID: seed.snapshot!.conversation.id)
        let refreshing = ConversationStore(device: device)
        refreshing.checkAccess.modelCatalogs["synthetic-codex"] = .ready([])
        refreshing.adopt(payloadSeed)
        guard let payloadMessage = refreshing.checkAccess.snapshot?.messages.first(where: \.hasDeferredPayload) else { throw TransportError.rejected("refresh gate needs real deferred content") }
        let payloadScopeBeforeRefresh = refreshing.checkAccess.projectionScope()
        let releaseOldPayload = DispatchSemaphore(value: 0), releaseRefresh = DispatchSemaphore(value: 0)
        refreshing.checkAccess.payloadQueue.async { _ = releaseOldPayload.wait(timeout: .now() + 12) }
        refreshing.loadPayloads(messageIDs: [payloadMessage.id])
        require(refreshing.checkAccess.payloadPending && refreshing.checkAccess.payloadDemand.isEmpty, "sole demand must be owned by the held batch")
        require(refreshing.canReloadFromRuntimes, "same-conversation refresh must be admitted during a payload read")
        refreshing.checkAccess.reloadProjection {
            guard releaseRefresh.wait(timeout: .now() + 12) == .success else { throw TransportError.rejected("refresh completion gate timed out") }
            return payloadSeed
        }
        require(refreshing.checkAccess.generation != payloadScopeBeforeRefresh.storeGeneration && refreshing.checkAccess.navigationID == payloadScopeBeforeRefresh.navigationID, "refresh changes request generation without navigating")
        releaseOldPayload.signal()
        wait("payload automatically reread under refreshed scope") {
            !refreshing.checkAccess.payloadPending && refreshing.checkAccess.payloadDemand.isEmpty
                && refreshing.message(id: payloadMessage.id)?.hasDeferredPayload == false
        }
        require(refreshing.checkAccess.isLoading, "new-scope payload read must finish before refresh receipt")
        releaseRefresh.signal()
        wait("identical deferred refresh projection") { !refreshing.checkAccess.isLoading }
        require(refreshing.message(id: payloadMessage.id)?.hasDeferredPayload == false, "identical refreshed projection must retain the independently loaded payload")
        refreshing.checkAccess.payloadDemand.removeAll()
        refreshing.checkAccess.generation += 1; refreshing.checkAccess.navigationID = UUID()
        refreshing.checkAccess.loadedPayloads.removeAll()
        if let delivered = payloadSeed.snapshot { refreshing.checkAccess.apply(BindingMapping.snapshot(delivered)) }
        require(refreshing.message(id: payloadMessage.id)?.hasDeferredPayload == true, "navigation counterexample must still have deferred content to request")
        refreshing.checkAccess.consumePayloads(.success([]), scope: payloadScopeBeforeRefresh, stamps: [payloadMessage.id: payloadMessage.stamp])
        require(refreshing.checkAccess.payloadDemand.isEmpty, "navigation must not resurrect an old viewport demand")
        refreshing.close()
        note("held sole payload → same-conversation refresh → old callback → automatic new-scope read; no viewport repeat or cross-navigation demand")

        device.composerInputs.poll(force: true)
        wait("composer authority for operation-owned promotion") { device.composerInputs.ownerID != nil && device.composerInputs.state != nil }

        // Hold the real createAndSend boundary after Core creation, before its
        // Send dispatch. This exercises the production two-stage publisher and
        // queue with an observable Core poll between creation and final receipt.
        let creating = ConversationStore(device: device)
        creating.checkAccess.modelCatalogs["synthetic-codex"] = .ready([RuntimeModel(selection: native, name: "Synthetic", reasoningEfforts: [])])
        guard let seedSnapshot = seed.snapshot else { throw TransportError.rejected("synthetic seed omitted its projection") }
        var creationDraft = BindingMapping.snapshot(seedSnapshot)
        creationDraft.conversation = Conversation(id: "draft:real-create-gate", title: "Synthetic creation", updatedAtUnixMs: nil, runtimeID: "synthetic-codex", cwd: home.path)
        creationDraft.messages = []; creationDraft.transcriptTurns = []; creationDraft.transcriptItems = []
        creationDraft.transcriptOutline = []; creationDraft.transcriptMessageIdentities = []
        creating.checkAccess.isDraft = true
        creating.checkAccess.apply(creationDraft)
        creating.checkAccess.started = true
        let creationDraftID = creationDraft.conversation.id
        let creationInput = ComposerModel()
        device.composerInputs.bind(creationDraftID, input: creationInput)
        wait("creation input binding") { device.composerInputs.inputID(for: creationDraftID) == creationInput.id }
        let creationTicket = SendProjectionTicket(scope: creating.checkAccess.projectionScope(), input: creationInput, device: device)
        let releaseCreation = DispatchSemaphore(value: 0)
        var creationEvents = 0, creationReceipts = 0
        let creationWatch = creating.composerEvents.sink { if case .created = $0 { creationEvents += 1 } }
        creating.checkAccess.enqueue({
            try transport.createAndSend(runtimeID: "synthetic-codex", cwd: home.path, selection: native,
                turn: ComposedTurn(text: "one created submission", effort: nil, permissionMode: nil, attachments: [])) { created in
                    DispatchQueue.main.async {
                        creating.checkAccess.publishCreatedIdentity(created, ticket: creationTicket, previous: creationDraftID)
                    }
                    _ = releaseCreation.wait(timeout: .now() + 20)
                }
        }, consumeReceipt: { result in
            if case .success = result { creationReceipts += 1; creating.checkAccess.acceptedSubmissions &+= 1 }
        }) { value in creating.checkAccess.applySnapshotResult(value, scope: creationTicket.scope) }
        wait("created identity before Send") { !creating.checkAccess.isDraft && creationEvents == 1 }
        let createdEpoch = creating.checkAccess.projectionEpoch
        creating.checkAccess.needsResync = true
        creating.checkAccess.pollProjection()
        wait("Core poll between onCreated and Send") { !creating.checkAccess.pollPending && creating.checkAccess.projectionEpoch > createdEpoch }
        require(creating.checkAccess.isLoading && creationReceipts == 0, "created identity and a consumed Core poll must precede Send receipt")
        let promptsBeforeCreatedSend = try String(contentsOf: home.appendingPathComponent("prompt-wire"), encoding: .utf8).split(separator: "\n")
        require(promptsBeforeCreatedSend.count == promptProcesses.count, "onCreated and Core poll must finish before created Send reaches the wire")
        releaseCreation.signal()
        wait("created Send receipt") { !creating.checkAccess.isLoading }
        require(creationReceipts == 1 && creating.checkAccess.acceptedSubmissions == 1 && !creating.checkAccess.isDraft
                && creating.loadedConversationID == creationTicket.createdID && creationEvents == 1, "created identity and Accepted must survive the intermediate Core poll")
        let promptsAfterCreation = try String(contentsOf: home.appendingPathComponent("prompt-wire"), encoding: .utf8).split(separator: "\n")
        require(promptsAfterCreation.count == promptProcesses.count + 1, "created Send must reach wire once, never replay")
        creationWatch.cancel(); creating.close()
        note("real createAndSend: onCreated identity → completed Core poll → exactly one Send wire/receipt")

        let client = device.composerInputs
        let newEntry = Self.draftConversationPrefix + "new-conversation"
        let submittedChoices = ComposerTargetChoices(runtimeId: "synthetic-codex", runtimeOwnerId: client.ownerID, choices: [], choiceEdited: true)
        let closedInput = ComposerModel()
        closedInput.restoreSavedDraft(.init(text: "closed-create", attachments: []))
        client.bind(newEntry, input: closedInput)
        wait("closed creation source binding") { client.inputID(for: newEntry) == closedInput.id }
        client.captureTarget(closedInput.id, choices: submittedChoices)
        let closedCreation = ConversationStore(device: device)
        closedCreation.checkAccess.modelCatalogs["synthetic-codex"] = .ready([])
        closedCreation.checkAccess.isDraft = true; closedCreation.checkAccess.apply(creationDraft)
        let closedTicket = SendProjectionTicket(scope: closedCreation.checkAccess.projectionScope(), input: closedInput, device: device)
        let releaseClosedSend = DispatchSemaphore(value: 0)
        var closedEvents = 0, closedReceipts = 0, promotionStagedOnce = false
        let closedWatch = closedCreation.composerEvents.sink { if case .created = $0 { closedEvents += 1 } }
        closedCreation.checkAccess.enqueue({
            try transport.createAndSend(runtimeID: "synthetic-codex", cwd: home.path, selection: native,
                turn: ComposedTurn(text: "closed-create", effort: nil, permissionMode: nil, attachments: [])) { created in
                    DispatchQueue.main.async {
                        closedCreation.close()
                        closedCreation.checkAccess.publishCreatedIdentity(created, ticket: closedTicket, previous: creationDraftID)
                        closedCreation.checkAccess.publishCreatedIdentity(created, ticket: closedTicket, previous: creationDraftID)
                        if let id = closedTicket.createdID { promotionStagedOnce = client.pendingPromotions[id]?.choices == submittedChoices }
                    }
                    _ = releaseClosedSend.wait(timeout: .now() + 20)
                }
        }, consumeReceipt: { result in
            if case .success = result { closedReceipts += 1; closedCreation.checkAccess.acceptedSubmissions &+= 1 }
        }) { value in closedCreation.checkAccess.applySnapshotResult(value, scope: closedTicket.scope) }
        closedInput.clear(submitted: "closed-create", attachments: [])
        client.consumeNewConversationInput(closedInput)
        wait("closed onCreated promoted without a window subscriber") {
            guard let id = closedTicket.createdID else { return false }
            return client.state?.creations.contains { $0.conversationId == id && $0.inputId == closedInput.id.uuidString.lowercased() } == true
                && client.pendingPromotions[id] == nil
        }
        guard let closedID = closedTicket.createdID else { throw TransportError.rejected("closed create lost identity") }
        require(promotionStagedOnce && client.targetChoices(for: closedID) == submittedChoices, "duplicate created callback must not consume/overwrite submitted choices twice")
        require(closedEvents == 0 && closedCreation.loadedConversationID == creationDraftID, "closed creation must not reopen the old view or emit navigation")
        releaseClosedSend.signal()
        wait("closed created Send receipt") { !closedCreation.checkAccess.isLoading }
        require(closedReceipts == 1 && closedCreation.checkAccess.acceptedSubmissions == 1
                && client.state?.creations.filter { $0.conversationId == closedID }.count == 1, "closed operation must retain one accepted receipt and one promotion")
        closedWatch.cancel()

        let failureInput = ComposerModel()
        let failureText = "created-failure"
        failureInput.restoreSavedDraft(.init(text: failureText, attachments: []))
        client.bind(newEntry, input: failureInput)
        wait("failed creation source binding") { client.inputID(for: newEntry) == failureInput.id }
        client.captureTarget(failureInput.id, choices: submittedChoices)
        let failedCreation = ConversationStore(device: device)
        failedCreation.checkAccess.modelCatalogs["synthetic-codex"] = .ready([])
        var failureDraft = creationDraft
        failureDraft.runState = .idle
        failureDraft.actions = ConversationActions(inputAccess: .editable, canSend: true, canCancel: false, canSwitch: true)
        failedCreation.checkAccess.isDraft = true; failedCreation.checkAccess.apply(failureDraft)
        var failedID: String?, failedReceipts = 0
        var clearedRevision: UInt64?
        require(failedCreation.send(text: failureText, next: target, input: failureInput, onFailure: { createdID in
            failedID = createdID; failedReceipts += 1
            failureInput.restoreFailed(failureText, attachments: [], revision: clearedRevision)
            client.save(failureInput.id, draft: failureInput.savedDraft)
            client.restoreConsumedNewConversationInput(failureInput, whileCreating: failedCreation, createdConversationID: createdID)
        }), "created-failure Send must be admitted")
        failureInput.clear(submitted: failureText, attachments: []); clearedRevision = failureInput.revision
        client.consumeNewConversationInput(failureInput)
        let untouchedNewInput = client.inputID(for: newEntry)
        failedCreation.close()
        wait("created then native Send rejection") { !failedCreation.checkAccess.isLoading && failedReceipts == 1 }
        guard let failedID else { throw TransportError.rejected("failed Send omitted its created identity") }
        wait("failed created input remains associated and saved") {
            client.state?.creations.contains { $0.conversationId == failedID && $0.inputId == failureInput.id.uuidString.lowercased() } == true
                && client.pendingPromotions[failedID] == nil && !client.isDirty(failureInput.id)
        }
        require(failedCreation.isUnsentConversation && failedCreation.loadedConversationID == creationDraftID, "closed failure must not reopen presentation")
        require(client.inputID(for: newEntry) == untouchedNewInput && untouchedNewInput != failureInput.id,
                "created failure must not restore the consumed input to a fresh creation entry even when the closed Store looks draft")
        require(client.inputID(for: failedID) == failureInput.id && client.targetChoices(for: failedID) == submittedChoices
                && client.draft(failureInput.id)?.text == failureText && failedCreation.checkAccess.acceptedSubmissions == 0,
                "created identity, submitted choices and failed draft must survive independently of the closed view")
        let promptsAfterClosedCases = try String(contentsOf: home.appendingPathComponent("prompt-wire"), encoding: .utf8).split(separator: "\n")
        require(promptsAfterClosedCases.count == promptsAfterCreation.count + 2, "closed success and created rejection each dispatch once without recreation or resend")
        note("closed before onCreated: one operation-owned input/choice promotion; created Send rejection preserves primary identity and failed draft, not a new creation")

        let fixture = ConversationStore(device: DeviceStore(preview: true))
        var base = fixture.checkAccess.snapshot!
        base.cursor = TranscriptCursor(generation: 41, sequence: 10)
        fixture.checkAccess.apply(base)
        let old = fixture.checkAccess.projectionScope()
        var newer = base; newer.cursor = TranscriptCursor(generation: 41, sequence: 12)
        fixture.checkAccess.consumePolledSnapshot(.complete(newer), scope: old)
        fixture.checkAccess.consumePolledSnapshot(.complete(base), scope: old)
        require(fixture.checkAccess.snapshot?.cursor == newer.cursor, "late full cannot roll back sequence")
        var delta = newer; delta.cursor = TranscriptCursor(generation: 41, sequence: 13)
        fixture.checkAccess.consumePolledSnapshot(.changes(base: newer.cursor!, header: delta, order: newer.messages.map(\.id), changed: []), scope: fixture.checkAccess.projectionScope())
        fixture.checkAccess.consumePolledSnapshot(.changes(base: base.cursor!, header: base, order: [], changed: []), scope: old)
        require(fixture.checkAccess.snapshot?.cursor == delta.cursor && fixture.checkAccess.needsResync, "late delta must not merge into replaced base")
        var otherGeneration = base; otherGeneration.cursor = TranscriptCursor(generation: 1, sequence: 99)
        require(!fixture.checkAccess.installProjection(otherGeneration, scope: old), "opaque generations cannot be numerically ordered")
        require(!fixture.checkAccess.installProjection(nil, scope: old) && fixture.checkAccess.snapshot != nil, "stale nil cannot reset projection")
        fixture.checkAccess.needsResync = false
        fixture.checkAccess.consumePolledSnapshot(.absent, scope: old)
        require(fixture.checkAccess.snapshot?.cursor == delta.cursor && fixture.checkAccess.needsResync, "stale polled absence must preserve the newer view and request resync")
        let absentFixture = ConversationStore(device: DeviceStore(preview: true))
        var absentBase = base; absentBase.pendingInteractions = [interaction]
        absentFixture.checkAccess.apply(absentBase)
        let absentScope = absentFixture.checkAccess.projectionScope()
        absentFixture.checkAccess.consumePolledSnapshot(.unchanged, scope: absentScope)
        require(absentFixture.loadedConversationID != nil && absentFixture.pendingInteractions == [interaction], "unchanged must retain projection and interaction")
        absentFixture.checkAccess.consumePolledSnapshot(.absent, scope: absentScope)
        require(absentFixture.loadedConversationID == nil && absentFixture.pendingInteractions.isEmpty, "fresh polled absence must clear projection and interaction")
        absentFixture.close()
        let fresh = fixture.checkAccess.projectionScope()
        otherGeneration.contextRuntimeID = "synthetic-new-tail"
        require(fixture.checkAccess.installProjection(otherGeneration, scope: fresh), "unchanged base permits Core context/generation transition")
        require(fixture.checkAccess.projectionScope().source == base.conversation.runtimeID && fixture.projectionRuntimeID == "synthetic-new-tail", "poll/resync addresses logical source, not obsolete tail")
        note("full/delta ordering, stale nil, opaque-generation CAS, logical-source versus tail context")

        var draft = base
        draft.conversation = Conversation(id: "draft:synthetic-two-phase", title: "Synthetic draft", updatedAtUnixMs: nil, runtimeID: base.conversation.runtimeID, cwd: nil)
        fixture.checkAccess.isDraft = true
        fixture.checkAccess.apply(draft)
        let ticket = SendProjectionTicket(scope: fixture.checkAccess.projectionScope())
        fixture.checkAccess.publishCreatedIdentity(seed, ticket: ticket, previous: draft.conversation.id)
        let createdBase = ticket.scope
        var polled = fixture.checkAccess.snapshot!
        polled.cursor = TranscriptCursor(generation: createdBase.cursor!.generation, sequence: createdBase.cursor!.sequence + 5)
        fixture.checkAccess.consumePolledSnapshot(.complete(polled), scope: createdBase)
        fixture.checkAccess.applySnapshotResult(seed, scope: ticket.scope)
        require(!fixture.checkAccess.isDraft && fixture.loadedConversationID == seed.snapshot!.conversation.id
                     && ticket.createdID == fixture.loadedConversationID && fixture.checkAccess.snapshot?.cursor == polled.cursor,
                     "created identity survives onCreated → poll → final Send without using draft or completion-time base")
        note("Create two-stage ticket: created identity is published before poll; late Send snapshot cannot revert poll")

        let deferred = DeliveredMessage(stamp: 7, completed: true, id: "synthetic-payload", role: .assistant, timestampUnixMs: nil,
            blocks: [.reasoning(summary: "synthetic", text: .deferred(bytes: 100))])
        func setPayloadFixtureMessages(_ messages: [DeliveredMessage], in value: inout ConversationSnapshot) {
            // These independent reasoning messages have no turn. Like a Core
            // delivery, each still needs a stable presentation ID and item.
            value.messages = messages
            value.transcriptTurns = []
            value.transcriptOutline = []
            value.transcriptMessageIdentities = messages.map { .init(id: "payload-presentation:" + $0.id, messageId: $0.id) }
            value.transcriptItems = messages.map { .message(id: "payload-presentation:" + $0.id, messageId: $0.id) }
        }
        var payloadView = polled
        setPayloadFixtureMessages([deferred], in: &payloadView)
        fixture.checkAccess.apply(payloadView)
        let payloadScope = fixture.checkAccess.projectionScope(), payloadEpoch = fixture.checkAccess.projectionEpoch
        var loaded = deferred; loaded.blocks = [.reasoning(summary: "synthetic", text: .inline(text: "complete payload"))]
        fixture.checkAccess.consumePayloads(.success([loaded]), scope: payloadScope, stamps: [deferred.id: 7])
        require(fixture.checkAccess.snapshot?.messages == [loaded] && fixture.checkAccess.projectionEpoch == payloadEpoch)
        var changed = deferred; changed.stamp = 8
        setPayloadFixtureMessages([changed], in: &payloadView)
        fixture.checkAccess.apply(payloadView)
        fixture.checkAccess.consumePayloads(.success([loaded]), scope: payloadScope, stamps: [deferred.id: 7])
        require(fixture.checkAccess.snapshot?.messages == [changed], "payload stamp changes cannot receive old content")
        setPayloadFixtureMessages([deferred], in: &payloadView)
        payloadView.cursor = TranscriptCursor(generation: 987, sequence: 1)
        fixture.checkAccess.apply(payloadView)
        fixture.checkAccess.consumePayloads(.success([loaded]), scope: payloadScope, stamps: [deferred.id: 7])
        require(fixture.checkAccess.snapshot?.messages == [deferred] && fixture.checkAccess.payloadDemand.contains(deferred.id), "new delivery generation may reuse a stamp and must retain viewport demand")
        let previousContext = fixture.checkAccess.projectionScope()
        payloadView.contextRuntimeID = "synthetic-payload-tail"
        fixture.checkAccess.apply(payloadView)
        fixture.checkAccess.consumePayloads(.success([loaded]), scope: previousContext, stamps: [deferred.id: 7])
        require(fixture.checkAccess.snapshot?.messages == [deferred] && fixture.checkAccess.payloadDemand.contains(deferred.id), "old context payload cannot cross into new tail or discard the sole viewport demand")
        note("payload: content stamp, delivery generation and context isolation; filling payload does not advance projection CAS epoch")

        let bounded = ConversationStore(device: device)
        bounded.checkAccess.modelCatalogs["synthetic-codex"] = .ready([])
        bounded.checkAccess.payloadPending = true
        var changingViewport = payloadView
        for replacement in 0..<8 {
            let messages = (0..<40).map { index in
                var message = deferred
                message.id = "viewport-\(replacement)-\(index)"
                return message
            }
            setPayloadFixtureMessages(messages, in: &changingViewport)
            bounded.checkAccess.apply(changingViewport)
            require(bounded.checkAccess.payloadDemand.isEmpty, "replacing messages must remove obsolete queued demand even while a batch is in flight")
            bounded.loadPayloads(messageIDs: changingViewport.messages.map(\.id) + ["not-in-projection"])
            require(bounded.checkAccess.payloadDemand.count == 40, "busy demand must be bounded by the current projection, not accumulated request history")
        }
        setPayloadFixtureMessages([], in: &changingViewport)
        bounded.checkAccess.apply(changingViewport)
        require(bounded.checkAccess.payloadDemand.isEmpty, "removed messages cannot retain queued payload demand")
        bounded.checkAccess.payloadPending = false; bounded.close()
        note("payload demand: busy replacement retains only current deferred IDs; batch size is bounded at 32")

        // Equal cursor header changes and payload fills have different CAS effects.
        let equalScope = fixture.checkAccess.projectionScope()
        var headerOnly = payloadView; headerOnly.conversation.title = "new header"
        fixture.checkAccess.apply(headerOnly)
        require(!fixture.checkAccess.installProjection(payloadView, scope: equalScope) && fixture.checkAccess.snapshot?.conversation.title == "new header")
        let epoch = fixture.checkAccess.projectionEpoch
        fixture.checkAccess.apply(otherGeneration, coreDelivery: false)
        require(fixture.checkAccess.projectionEpoch == epoch)
        fixture.checkAccess.apply(otherGeneration)
        require(!fixture.checkAccess.baseIsCurrent(fresh))
        let nav = fixture.checkAccess.projectionScope()
        fixture.checkAccess.generation += 1
        require(!fixture.checkAccess.installProjection(base, scope: nav))

        // A late reply completion cannot clear a newer attempt for the same key.
        let key = InteractionDraftKey(conversationID: fixture.loadedConversationID!, interactionID: interaction.id)
        let oldAttempt = UUID(), newAttempt = UUID()
        fixture.checkAccess.interactionReplyInFlight = (key, "new attempt", newAttempt, interaction)
        fixture.checkAccess.replyAttempt = newAttempt
        fixture.checkAccess.finishInteractionReply(.failure(TransportError.rejected("old failure")), interaction: interaction,
            key: key, scope: fixture.checkAccess.projectionScope(), attempt: oldAttempt)
        require(fixture.checkAccess.replyAttempt == newAttempt && fixture.checkAccess.interactionReplyInFlight?.attempt == newAttempt && fixture.checkAccess.interactionReplyFailures[key] == nil)
        note("navigation invalidates late projections; old same-key reply attempt cannot clear or fail a newer attempt")

        // Receipts settle even when a newer poll makes the operation's opaque
        // generation snapshot unusable, and even when closure drops presentation.
        let receiptStore = ConversationStore(device: device)
        receiptStore.checkAccess.apply(base)
        let release = DispatchSemaphore(value: 0)
        var receipts = 0
        receiptStore.checkAccess.enqueueSnapshot({
            guard release.wait(timeout: .now() + 5) == .success else { throw TransportError.rejected("synthetic receipt release timed out") }
            return SnapshotResult(snapshot: nil)
        }, scope: receiptStore.checkAccess.projectionScope(), consumeReceipt: { result in
            if case .success = result { receipts += 1 }
        })
        receiptStore.checkAccess.apply(newer)
        receiptStore.close()
        release.signal()
        wait("late receipt") { !receiptStore.checkAccess.isLoading }
        require(receipts == 1 && receiptStore.checkAccess.snapshot?.cursor == newer.cursor)
        fixture.close()
        note("business receipt settles exactly once before closed-view/stale-projection rejection")
    }

    /// Native state/queue regression for cold-open recovery. All input is
    /// synthetic; the queue case owns only a transport in the supplied home.
    static func checkColdOpeningRetryLifecycle(home: URL) {
        func note(_ value: String) { FileHandle.standardError.write(Data((value + "\n").utf8)) }
        func wait(_ condition: () -> Bool) {
            let deadline = Date().addingTimeInterval(5)
            while !condition(), Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
            precondition(condition(), "冷打开生命周期检查超时")
        }
        let sample = DeviceStore(preview: true).previewSnapshots["sample-2"]!
        let transport = Transport(stateDirectory: home.appendingPathComponent("cold-opening-queue"), resourcesDirectory: Bundle.main.resourceURL!)
        let store = ConversationStore(device: DeviceStore(transport: transport))
        store.checkAccess.started = true
        store.checkAccess.unopenedConversationID = sample.conversation.id
        store.checkAccess.selectedConversationID = sample.conversation.id
        let unavailable = TransportError.operation("openConversation", .diagnostic(kind: "unavailable", detail: "会话历史正在读取，请稍后重试", code: "unavailable", phase: "read", operationID: "synthetic"))
        store.checkAccess.noteOpeningFailure(unavailable)
        precondition(store.checkAccess.snapshot == nil && store.isOpeningConversation && store.openingFailure == nil)
        precondition(store.checkAccess.takeOpeningRetry() == nil && store.checkAccess.openingRetry.failures == 1,
                     "暂态失败不能在下一个通知中立即热重试")
        wait { store.checkAccess.openingRetry.ready }
        store.checkAccess.isLoading = true
        precondition(store.checkAccess.takeOpeningRetry() == nil)
        store.checkAccess.isLoading = false
        store.checkAccess.linkedConversationRequest = UUID()
        precondition(store.checkAccess.takeOpeningRetry() == nil, "在途lookup不能被重试取代")
        store.checkAccess.abandonLinkedConversationLookup()
        precondition(store.checkAccess.takeOpeningRetry() == sample.conversation.id && store.checkAccess.takeOpeningRetry() == nil,
                     "snapshot尚不存在时仍应取出原目标，而且只取一次")
        store.checkAccess.noteOpeningFailure(unavailable)
        precondition(store.checkAccess.openingRetry.failures == 2 && !store.checkAccess.openingRetry.ready)
        store.checkAccess.apply(sample)
        precondition(store.loadedConversationID == sample.conversation.id && !store.checkAccess.openingNeedsRetry && store.checkAccess.unopenedConversationID == nil)
        precondition(store.transcript.rows.map(\.message) == sample.messages)
        note("PASS cold open: typed temporary failure retains destination; backoff, loading/lookup dedup, single retry admission, second failure backoff and successful original transcript")

        for error in [TransportError.closed, .quitting, .rejected("synthetic missing"),
                      .diagnostic(kind: "history", detail: "synthetic invalid history", code: "invalid_history", phase: "read", operationID: "")] {
            store.checkAccess.resetProjection()
            store.checkAccess.unopenedConversationID = sample.conversation.id
            store.checkAccess.noteOpeningFailure(error)
            precondition(!store.checkAccess.openingNeedsRetry && store.openingFailure != nil && store.checkAccess.takeOpeningRetry() == nil)
        }
        store.checkAccess.noteOpeningFailure(unavailable)
        store.checkAccess.offListConversations = [sample.conversation]
        store.selectConversation(id: sample.conversation.id + "-other")
        precondition(store.checkAccess.unopenedConversationID == nil && !store.checkAccess.openingNeedsRetry,
                     "切换目的地必须撤销旧重试，即使新目的地尚未读到列表")
        store.checkAccess.unopenedConversationID = sample.conversation.id
        store.checkAccess.noteOpeningFailure(unavailable)
        let lookup = UUID()
        store.checkAccess.linkedConversationRequest = lookup
        store.checkAccess.finishLinkedConversationLookup(.success(nil), id: sample.conversation.id, move: .new, request: lookup)
        precondition(store.checkAccess.unopenedConversationID == nil && !store.checkAccess.openingNeedsRetry && store.openingFailure != nil)
        note("PASS permanent failures and missing lookup stop retries; navigation cannot return to the abandoned destination")

        store.checkAccess.unopenedConversationID = sample.conversation.id
        store.checkAccess.noteOpeningFailure(TransportError.diagnostic(kind: "history", detail: "synthetic read error",
                                                          code: "provider_read", phase: "native_read", operationID: "synthetic-read"))
        precondition(store.coldOpeningProblem?.detail == "synthetic read error"
                     && store.coldOpeningProblem?.operationID == "synthetic-read" && store.canRetryFailedOpening,
                     "冷打开失败时，正文必须保留诊断和原会话的重试入口")

        let manual = ConversationStore(device: DeviceStore(transport: transport))
        manual.checkAccess.offListConversations = [sample.conversation]
        manual.selectConversation(id: sample.conversation.id)
        precondition(manual.checkAccess.isLoading && manual.checkAccess.unopenedConversationID == sample.conversation.id,
                     "空Tab从列表首次打开也必须登记目标，不能只照顾start(opening:)种子")
        manual.close()
        wait { !manual.checkAccess.isLoading }
        let linked = ConversationStore(device: DeviceStore(transport: transport))
        linked.openLinkedConversation(id: "synthetic-off-list")
        precondition(linked.checkAccess.unopenedConversationID == "synthetic-off-list" && linked.checkAccess.linkedConversationRequest != nil)
        linked.close()
        precondition(linked.checkAccess.unopenedConversationID == nil && linked.checkAccess.linkedConversationRequest == nil)
        note("PASS manual first opens: listed and off-list link destinations are remembered before their asynchronous read; closure cancels both")

        // An operation may already be on the background queue when its tab
        // closes. Its completion must not resurrect that closed tab's state.
        let closing = ConversationStore(device: DeviceStore(transport: transport))
        closing.checkAccess.started = true
        closing.checkAccess.unopenedConversationID = sample.conversation.id
        closing.checkAccess.noteOpeningFailure(unavailable)
        let release = DispatchSemaphore(value: 0)
        var applied = false
        closing.checkAccess.enqueue({
            _ = release.wait(timeout: .now() + 5)
            return true
        }, onFailure: { _ in preconditionFailure("synthetic operation cannot fail") }) { _ in applied = true }
        precondition(closing.checkAccess.isLoading)
        let beforeClose = closing.checkAccess.generation
        closing.close()
        release.signal()
        wait { !closing.checkAccess.isLoading }
        precondition(!applied && closing.checkAccess.generation != beforeClose && !closing.checkAccess.openingNeedsRetry && closing.checkAccess.takeOpeningRetry() == nil)
        note("PASS close: queued successful completion is discarded and no retry survives tab closure")
    }

    /// Historical execution settings and a synthetic catalog drive the real
    /// message-editor popover without reading or mutating a runtime.
    static func inspectEditorVisualStates(_ inspect: (ConversationWindowModel, MessageEditingSession, String) -> Void) {
        let matrix = CommandLine.arguments.contains("--editor-visual-matrix") || CommandLine.arguments.contains("--experience-message-editor-offscreen")
        let requested = CommandLine.arguments.first(where: { $0.hasPrefix("--editor-visual-state=") }).map { String($0.dropFirst("--editor-visual-state=".count)) }
        for state in ["empty", "long", "failed", "missing-model", "missing-runtime", "fork-only", "legacy-model", "legacy-model-loading", "legacy-model-missing", "catalog-loading", "busy", "failure", "failure-target-changed", "placement", "history-changed", "long-floating", "busy-floating", "failure-floating"] {
            if let requested { if state != requested { continue } }
            else if !matrix, state != "long", state != "failed" { continue }
            let device = DeviceStore(preview: true)
            let workspace = ConversationWindowModel(application: device, kind: .workspace)
            let source = workspace.focused.store
            var value = device.previewSnapshots["sample-0"]!
            let message = value.messages.first(where: { $0.role == .user })!
            let legacy = state.hasPrefix("legacy-model")
            let selection: ModelSelection = legacy ? .managed(modelRecordKey: "synthetic-editor-route")
                : .native(providerId: "sample", modelId: "visual-editor-model")
            let recordedModel = legacy ? "GPT-6.1-Sol（ChatGPT1）" : "visual-editor-model"
            value.transcriptTurns = [.init(id: "editor-turn", userMessageId: message.id, workMessageIds: [], lastMessageId: nil, replyStartBlock: 0, durationMs: nil, isRunning: false, canFork: false, settings: .init(runtimeId: value.contextRuntimeID, model: recordedModel, modelSelection: legacy ? nil : selection), usage: nil)]
            device.keepPreviewSnapshot(value)
            source.checkAccess.isDraft = false
            source.checkAccess.apply(value)
            let session = MessageEditingSession(source: source, request: .init(message: message, conversationID: value.conversation.id, inPlace: state != "fork-only", isLast: state != "placement", revision: ["history-changed", "failure-target-changed"].contains(state) ? "synthetic-old-revision" : nil), presentation: state.hasSuffix("floating") ? .floating : .sheet)
            session.start()
            let store = session.store
            if state == "failed" { store.checkAccess.modelCatalogs[value.contextRuntimeID] = .failed("模型目录读取失败（合成错误）。请重新读取模型后重试。") }
            else { store.checkAccess.modelCatalogs[value.contextRuntimeID] = .ready([.init(selection: selection, name: "视觉验收长模型 " + String(repeating: "Very Long Model Name ", count: 10), reasoningEfforts: ["low", "medium", "high"])]) }
            store.checkAccess.refreshNextTurn()
            if legacy {
                store.checkAccess.modelCatalogs[value.contextRuntimeID] = .loading
                precondition(session.next?.chosenModelUnavailable == false, "目录加载不能被判成模型不可用")
                precondition(session.target.issue?.hasPrefix("正在确认原消息的模型") == true,
                             "尚未恢复历史模型时不可悄悄按默认模型发送")
                store.checkAccess.modelCatalogs[value.contextRuntimeID] = .ready([.init(selection: selection, name: recordedModel, reasoningEfforts: [])])
                precondition(session.next?.model == selection && session.next?.chosenModelUnavailable == false,
                             "历史显示名应恢复为目录中的可靠选择，而非伪造原生ID")
                if state == "legacy-model-loading" {
                    store.checkAccess.modelCatalogs[value.contextRuntimeID] = .loading
                    let context = ComposerContext(tab: session.tab, model: session.editor, editingTarget: session.target)
                    precondition(context.selectedModelName == recordedModel && !context.chosenModelUnavailable,
                                 "确认原消息模型期间应保留记录名称，不能声称默认模型或不可用")
                }
                if state == "legacy-model-missing" {
                    store.checkAccess.modelCatalogs[value.contextRuntimeID] = .ready([])
                    precondition(session.next?.chosenModelUnavailable == true, "无法恢复历史模型时不得默认发送")
                }
            }
            if state == "catalog-loading" {
                let chosen = ModelSelection.native(providerId: "sample", modelId: "user-selected-model")
                session.target.chooseModel(chosen)
                store.checkAccess.modelCatalogs[value.contextRuntimeID] = .loading
                precondition(session.next?.choice.model == chosen && session.next?.chosenModelUnavailable == false,
                             "异步目录准备必须保留用户选择，loading不能判不可用")
            }
            if state == "empty" {
                session.editor.receiveDraft(.init(text: "", attachments: []))
                precondition(session.editor.isEmpty)
            }
            if state.hasPrefix("long") {
                let text = String(repeating: "长正文保持原生字体与内边距，增长到上限后滚动。\n", count: 30)
                session.editor.receiveDraft(.init(text: text, attachments: []))
                precondition(session.editor.submission == text)
            }
            if state == "missing-model" {
                let other = ModelSelection.native(providerId: "sample", modelId: "visual-removed-model")
                store.checkAccess.modelCatalogs[value.contextRuntimeID] = .ready([.init(selection: other, name: "即将移除的模型", reasoningEfforts: [])])
                session.target.chooseModel(other)
                precondition(session.choices.choices[value.contextRuntimeID]?.model == other)
                store.checkAccess.modelCatalogs[value.contextRuntimeID] = .ready([])
                precondition(session.next?.chosenModelUnavailable == true)
            }
            if state == "missing-runtime" {
                precondition(session.next != nil)
                device.deleteRuntimeInstance(id: value.contextRuntimeID)
                precondition(session.next == nil, "Removed runtimes must not resolve an editor execution target")
                precondition(!session.canSubmit)
            }
            session.prepareVisualCheckState(state)
            if state == "failure-target-changed" {
                precondition(session.historyNotice != nil && session.failure != nil)
                precondition(session.noticeText == session.failure && session.noticeHelp == session.failure,
                             "回退后目标变化不能遮住尚未发送的真实失败详情")
            }
            inspect(workspace, session, state)
        }
    }

    /// Isolated fixture for the queue editor's attachment and receipt states.
    static func inspectQueuedEditor(_ inspect: (ConversationWindowModel, QueuedMessage, (QueuedMessage?, ContextPreparation?, Bool) -> Void, (RuntimeModelCatalog?) -> Void) -> Void) {
        let window = ConversationWindowModel(application: DeviceStore(preview: true), kind: .workspace)
        let store = window.focused.store
        var snapshot = store.device.previewSnapshots["sample-0"]!
        let runtimeID = snapshot.contextRuntimeID
        let message = QueuedMessage(id: "synthetic-queue", submissionId: "synthetic-submission", text: "补充验收记录并保留附件。",
                                    attachments: ["/synthetic-host/notes.md"],
                                    editContext: .init(revision: 3, runtimeId: runtimeID,
                                                       selection: .native(providerId: "sample", modelId: "sample-native"),
                                                       effort: "medium", permissionMode: nil))
        store.checkAccess.isDraft = false
        snapshot.followUp = .init()
        snapshot.followUp.queue = [message]
        snapshot.followUp.paused = .turnStopped
        store.checkAccess.apply(snapshot)
        store.loadModelCatalog(runtimeID: runtimeID)
        inspect(window, message, { item, preparation, busy in
            snapshot.followUp.queue = item.map { [$0] } ?? []
            snapshot.followUp.contextPreparation = preparation
            store.checkAccess.apply(snapshot)
            store.checkAccess.isLoading = busy
        }, { store.checkAccess.modelCatalogs[runtimeID] = $0 })
    }

    /// Full composer projections for native visual state-transition checks.
    static func inspectComposerVisualStates(_ inspect: (ConversationWindowModel, String, ConversationSnapshot) -> Void) {
        let window = ConversationWindowModel(application: DeviceStore(preview: true), kind: .workspace)
        let store = window.focused.store
        let base = store.device.previewSnapshots["sample-0"]!
        for name in ["idle", "running", "compacting", "queue", "paused", "background", "external", "child", "compacted", "model-missing", "model-failed"] {
            var value = base
            value.conversation.title = "较长会话标题：检查标签截断后完整标题仍可查看，Composer 视觉验收"
            value.followUp = .init()
            value.runState = .idle
            value.actions = .init(inputAccess: .editable, canSend: true, canCancel: false, canSwitch: true)
            if ["running", "compacting", "queue", "paused", "background"].contains(name) {
                value.followUp.canQueue = true
                value.followUp.queue = (0..<5).map { .init(id: "visual-\($0)", text: "排队消息 \($0)：" + String(repeating: "较长的内容用于检查截断与布局。", count: 8), attachments: $0 == 1 ? ["/tmp/synthetic-attachment.md"] : []) }
            }
            if ["running", "queue", "child"].contains(name) { value.runState = .running; value.actions.canCancel = true }
            if name == "compacting" { value.followUp.isCompacting = true }
            if name == "paused" { value.followUp.paused = .sendFailed }
            if name == "background" { value.followUp.waiting = .subAgents; value.followUp.canStopSubAgents = true }
            if name == "external" { value.runState = .elsewhere; value.actions.inputAccess = .readOnly; value.actions.canSend = false }
            if name == "child" { value.conversation.parentConversationID = "synthetic-parent"; value.actions.inputAccess = .readOnly; value.actions.canSend = false }
            value.usage.contextTokens = name == "compacted" ? 12_000 : 150_000
            value.usage.contextWindow = 200_000
            if name == "compacted" || name == "child" {
                value.messages = [.init(id: "parent-input", role: .user, blocks: [.text(text: "父 Agent 指令：核对这份设计并指出遗漏。")])]
                if name == "compacted" {
                    value.messages += [.init(id: "compact-notice", role: .client, blocks: [.text(text: "上下文已压缩")]), .init(id: "after-compact", role: .assistant, blocks: [.text(text: "压缩后继续核对，之前的用户消息仍保留。")])]
                }
                value.transcriptTurns = []
                value.transcriptItems = value.messages.map { .message(id: $0.id, messageId: $0.id) }
                value.transcriptMessageIdentities = value.messages.map { .init(id: $0.id, messageId: $0.id) }
            }
            store.checkAccess.isDraft = false
            store.checkAccess.apply(value)
            if name == "model-failed" {
                ComposerContext(tab: window.focused, model: window.composer(for: window.focused)).selectModel(nil)
                store.checkAccess.modelCatalogs[value.contextRuntimeID] = .failed("无法读取模型目录（合成失败），请重试。")
                store.checkAccess.refreshNextTurn()
            }
            if name == "model-missing" {
                let selection = ModelSelection.native(providerId: "sample", modelId: "synthetic-retired-model")
                store.checkAccess.modelCatalogs[value.contextRuntimeID] = .ready([.init(selection: selection, name: String(repeating: "Long model name ", count: 8), reasoningEfforts: [])])
                store.checkAccess.refreshNextTurn()
                ComposerContext(tab: window.focused, model: window.composer(for: window.focused)).selectModel(selection)
                precondition(store.nextTurn?.choice.model == selection, "Must select an actually offered model before removal")
                store.checkAccess.modelCatalogs[value.contextRuntimeID] = .ready([])
                store.checkAccess.refreshNextTurn()
                precondition(store.nextTurn?.chosenModelUnavailable == true, "Removed selected model must retain unavailable state")
            }
            inspect(window, name, value)
        }
    }

    /// Synthetic child projections exercise the same parent/child controls as
    /// the core's delivered snapshots, without opening a runtime or history.
    static func checkSubAgentControls(_ inspect: (ConversationWindowModel, Bool) -> Void) {
        let device = DeviceStore(preview: true)
        let window = ConversationWindowModel(application: device, kind: .workspace)
        let store = window.focused.store
        var parent = device.previewSnapshots["sample-0"]!
        parent.runState = .idle
        parent.actions.canCancel = false
        parent.followUp.canStopSubAgents = true
        parent.subAgents = [.init(conversationId: "synthetic-pi-child", title: "Pi advisor", failed: false)]
        parent.collaborators = []
        store.checkAccess.isDraft = false
        store.checkAccess.apply(parent)
        precondition(store.hasLinkedAgents && LinkedAgent.all(of: store).contains { $0.id == "synthetic-pi-child" })
        precondition(!store.canCancel)
        inspect(window, false)

        var readOnlyParent = parent
        readOnlyParent.actions.inputAccess = .readOnly
        store.checkAccess.apply(readOnlyParent)
        let readOnlyContext = ComposerContext(tab: window.focused, model: window.composer(for: window.focused))
        precondition(!readOnlyContext.inputEditable && readOnlyContext.executionChoicesEditable && readOnlyContext.canSwitchModel,
                     "非子会话的文本只读不能收紧原有的本地执行配置资格")
        store.checkAccess.apply(parent)
        store.selectComposerRuntime("sample-review")
        var child = parent
        child.conversation = Conversation(id: "synthetic-pi-child", title: "Pi advisor", updatedAtUnixMs: nil,
            runtimeID: parent.conversation.runtimeID, cwd: parent.conversation.cwd,
            parentConversationID: parent.conversation.id)
        child.subAgents = []
        child.followUp = .init()
        child.actions.inputAccess = .readOnly
        child.actions.canSend = false
        child.actions.canCancel = true
        child.runState = .running
        let recordedModel = ModelSelection.native(providerId: "sample", modelId: "child-recorded-model")
        child.transcriptTurns = [.init(id: "child-recorded-turn", userMessageId: "child-input", workMessageIds: [],
            lastMessageId: nil, replyStartBlock: 0, durationMs: nil, isRunning: true, canFork: false,
            outcome: nil, settings: .init(runtimeId: child.contextRuntimeID, model: "child-recorded-model", effort: "child-recorded-effort", modelSelection: recordedModel), usage: nil)]
        store.checkAccess.apply(child)
        precondition(store.hasLinkedAgents && store.canCancel && !store.canSend && !store.draftEditable)
        let input = window.composer(for: window.focused)
        let context = ComposerContext(tab: window.focused, model: input)
        precondition(window.composerOwner(for: window.focused) == nil && input === window.focused.composer)
        precondition(context.nextTurnRuntimeID == child.contextRuntimeID && context.nextTurnModel == recordedModel)
        precondition(context.currentEffort == "child-recorded-effort" && !context.canSwitchModel && !context.inputEditable)
        let before = device.composerInputs.state
        context.selectNextTurnRuntime("sample-review")
        context.selectModel(.managed(modelRecordKey: "sample-model"))
        context.selectEffort("high")
        context.selectPermissionMode(nil)
        precondition(device.composerInputs.state == before && store.nextTurn == context.nextTurn,
                     "只读子会话必须只显示自己的记录，不保存输入或执行选择")
        inspect(window, true)
        child.actions.canCancel = false
        child.runState = .elsewhere
        store.checkAccess.apply(child)
        precondition(!store.canCancel, "外部进程或没有精确停止能力的子会话不能伪造停止入口")
        FileHandle.standardError.write(Data("PASS child controls: parent links, running child stop, read-only input, external child not stoppable\n".utf8))
    }

    /// Synthetic pending operation; the observer presses the production native
    /// stop button while the ordinary command queue is occupied.
    static func checkPendingSendCancellation(_ inspect: (ConversationWindowModel) -> Void) {
        let window = ConversationWindowModel(application: DeviceStore(preview: true), kind: .workspace)
        let store = window.focused.store
        let cancellation = VeluneSendCancellation()
        store.checkAccess.pendingSendCancellation = cancellation
        store.checkAccess.executionStarting = true
        store.checkAccess.isLoading = true
        store.checkAccess.composerRequest = .sending
        precondition(store.canCancel && !store.canSend)
        inspect(window)
        precondition(cancellation.isCancelled() && store.isStopping && !store.canCancel,
                     "发送等待期间原生停止按钮必须直接设置取消信号，不能等待操作队列")
        if let value = store.checkAccess.snapshot { store.checkAccess.apply(value) }
        precondition(store.isStopping, "创建会话的迟到空闲快照不能撤销取消意图")
        store.checkAccess.pendingSendCancellation = nil
        store.checkAccess.executionStarting = false
        store.checkAccess.isLoading = false
        store.checkAccess.stopRequested = false
        store.checkAccess.composerRequest = nil
        FileHandle.standardError.write(Data("PASS pending send: native stop remains enabled during loading, cancellation immediate and late idle snapshot preserved\n".utf8))
    }

    /// Check-only execution facts for a non-preview submission fixture. The
    /// in-memory input authority and shared resolver prepare a real target
    /// projection; empty native catalogs prevent any runtime transport call.
    func prepareSyntheticComposerTargetForAdmissionCheck() {
        precondition(!isPreview && !checkAccess.started && device.composerInputs.ownerID == "preview"
                     && device.composerInputs.state == nil && checkAccess.composerTargetRequestID == nil
                     && checkAccess.modelCatalogRequests.isEmpty,
                     "合成提交检查不能混用真实输入权威或在途运行时请求")
        precondition(device.runtimeTypes.allSatisfy { !$0.sessionOptions },
                     "合成提交检查不能触发原生会话选项请求")
        for runtime in device.enabledRuntimeInstances { checkAccess.modelCatalogs[runtime.id] = .ready([]) }
        guard let request = checkAccess.composerTargetRequest else {
            preconditionFailure("合成提交检查必须先采纳完整会话快照")
        }
        checkAccess.preparePreviewComposerTarget(request)
        precondition(composerTargetIssue == nil && checkAccess.composerTargetRequestID == nil
                     && checkAccess.modelCatalogRequests.isEmpty,
                     "合成目标必须通过原有执行配置解析且不启动运行时请求")
    }

    /// The frontend must dispatch while the source runs, not retain a hidden
    /// intent until a later snapshot. The private empty backend rejects this
    /// synthetic identity, which also checks failure restores only loading state.
    static func checkConcurrentForkDispatch(home: URL) {
        let transport = Transport(stateDirectory: home.appendingPathComponent("fork-dispatch"), resourcesDirectory: Bundle.main.resourceURL!)
        let store = ConversationStore(device: DeviceStore(transport: transport))
        store.checkAccess.composerTargetClosed = true
        for state in [RunState.running, .elsewhere] {
            let source = ConversationSnapshot(revision: 1,
                conversation: Conversation(id: "synthetic-running-source", title: "Synthetic", updatedAtUnixMs: nil,
                    runtimeID: "synthetic", cwd: nil, canRename: false, canDelete: false),
                contextRuntimeID: "synthetic", modelRecordKey: nil, runState: state, messages: [],
                actions: ConversationActions(inputAccess: .editable, canSend: false, canCancel: false, canSwitch: true))
            store.checkAccess.apply(source)
            precondition(store.canFork, "源运行态不能禁止独立 fork")
            store.forkConversation(through: "completed-user") { _ in preconditionFailure("合成空后端不应找到会话") }
            precondition(store.isForking && store.checkAccess.isLoading, "必须立即派发，不能等待源结束")
            let deadline = Date().addingTimeInterval(15)
            while store.checkAccess.isLoading, Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
            precondition(!store.checkAccess.isLoading && !store.isForking, "实际失败必须结束复制加载态")
            precondition(store.checkAccess.snapshot?.runState == state && store.loadedConversationID == source.conversation.id,
                         "失败不得替换或停止源会话投影")
        }
        // A completed fork republishes the core list (noteForkCompleted):
        // the sidebar shows the fork only as the core's grouped entry, with
        // no provisional row of its own. The empty backend answers the list
        // read, so the refresh applying is observable through listRevision.
        let revision = store.device.listRevision
        store.device.noteForkCompleted()
        let refreshDeadline = Date().addingTimeInterval(15)
        while store.device.listRevision == revision, Date() < refreshDeadline { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
        precondition(store.device.listRevision > revision, "fork 后必须由核心列表刷新提供新会话条目")
        store.close()
        FileHandle.standardError.write(Data("PASS fork dispatch: running and elsewhere sources dispatch immediately; backend failure clears loading without changing source; a completed fork republishes the core list with no provisional entry\n".utf8))
    }

    /// Hidden native check of the production navigation/catalog admission path.
    /// A retained source snapshot must not launch any catalog before open finishes.
    static func checkOpeningCatalogAdmission() {
        let device = DeviceStore(preview: true)
        let store = ConversationStore(device: device)
        let source = device.previewSnapshots["sample-0"]!
        let destination = device.previewSnapshots["sample-2"]!
        let targetRuntime = destination.contextRuntimeID
        precondition(targetRuntime != source.contextRuntimeID, "目录准入检查必须区分来源和目标运行时")
        store.checkAccess.apply(source)
        store.checkAccess.modelCatalogs.removeAll()
        // Mirror production navigation: the selected destination changes while
        // the source snapshot remains visible until the open result arrives.
        store.checkAccess.selectedConversationID = destination.conversation.id
        store.checkAccess.pendingConversationID = destination.conversation.id
        store.loadModelCatalog(runtimeID: targetRuntime)
        precondition(store.checkAccess.modelCatalogs.isEmpty, "打开会话前不应启动旧输入触发的模型目录")
        store.selectComposerRuntime(targetRuntime)
        precondition(store.checkAccess.modelCatalogs.isEmpty && store.nextTurn == nil,
                     "加载中的目标选择不应阻塞历史打开或借用来源会话的执行配置")
        store.checkAccess.apply(destination)
        precondition(store.checkAccess.modelCatalogs.isEmpty, "目标快照到达但打开尚未结束时不能提前启动模型目录")
        store.checkAccess.deferredSelection = ("synthetic-next-destination", .new)
        store.checkAccess.finishConversationLoading()
        precondition(store.checkAccess.modelCatalogs.isEmpty, "已有更新导航时不能先排入旧目标模型目录")
        store.checkAccess.deferredSelection = nil
        store.checkAccess.finishConversationLoading()
        precondition(store.loadedConversationID == destination.conversation.id && store.nextTurn?.runtimeId == targetRuntime,
                     "打开完成后必须通过目标权威配置解析实际执行运行时")
        guard case .ready? = store.checkAccess.modelCatalogs[targetRuntime] else {
            preconditionFailure("会话打开完成后应加载实际目标运行时的模型目录")
        }
        precondition(store.checkAccess.modelCatalogs[source.contextRuntimeID] == nil, "完成目标打开不能补启动来源会话的模型目录")
        FileHandle.standardError.write(Data("PASS conversation opening: retained snapshot cannot start a catalog ahead of open; destination catalog loads after opening\n".utf8))
        checkOpeningFailureRecovery()
    }

    private static func checkOpeningFailureRecovery() {
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("velune-open-failure-\(UUID())", isDirectory: true)
        let transport = Transport(stateDirectory: home, telemetry: .failure(TransportError.unavailable), remote: {
            throw TransportError.rejected("synthetic opening failure")
        })
        let device = DeviceStore(transport: transport)
        let window = ConversationWindowModel(application: device, kind: .workspace)
        window.tools.move(.composer, to: .docked(.bottom))
        let tab = window.focused
        let store = tab.store
        let destination = Conversation(id: "missing:destination", title: "Synthetic destination", updatedAtUnixMs: nil, runtimeID: "missing", cwd: nil, canRename: false, canDelete: false)
        store.checkAccess.offListConversations = [destination]
        func wait() {
            let deadline = Date().addingTimeInterval(5)
            while store.checkAccess.isLoading, Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
            precondition(!store.checkAccess.isLoading, "合成打开失败检查超时")
        }
        func source(_ id: String, isDraft: Bool) {
            store.checkAccess.isDraft = isDraft
            store.checkAccess.apply(ConversationSnapshot(revision: 1, conversation: Conversation(id: id, title: "Synthetic source", updatedAtUnixMs: nil, runtimeID: "missing", cwd: nil, canRename: false, canDelete: false), contextRuntimeID: "missing", modelRecordKey: nil, runState: .idle, messages: [], actions: ConversationActions(inputAccess: .editable, canSend: true, canCancel: false, canSwitch: true)))
            store.composerEvents.send(isDraft ? .newConversation : .conversation(id))
        }
        for isDraft in [false, true] {
            let id = isDraft ? draftConversationPrefix + "new-conversation" : "missing:source"
            source(id, isDraft: isDraft)
            let input = window.composer(for: tab)
            input.restoreSavedDraft(.init(text: isDraft ? "unsent source" : "existing source", attachments: []))
            store.selectConversation(id: destination.id)
            precondition(window.sharedComposer.id != input.id, "发起导航应切换目标输入")
            wait()
            precondition(store.checkAccess.selectedConversationID == id && store.loadedConversationID == id)
            precondition(store.isUnsentConversation == isDraft, "打开失败必须恢复原草稿身份")
            precondition(window.composer(for: tab).id == input.id && window.sharedComposer.submission == input.submission, "打开失败必须恢复原会话输入")
        }
        // A second navigation is deferred until the first settles; a failed
        // opening must not prevent that newer intent from taking its turn.
        let latest = Conversation(id: "missing:latest", title: "Latest destination", updatedAtUnixMs: nil, runtimeID: "missing", cwd: nil, canRename: false, canDelete: false)
        store.checkAccess.offListConversations.append(latest)
        var destinations: [String] = []
        let watch = store.composerEvents.sink { event in if case .conversation(let id) = event { destinations.append(id) } }
        store.selectConversation(id: destination.id)
        store.selectConversation(id: latest.id)
        wait()
        precondition(destinations.contains(latest.id) && store.checkAccess.deferredSelection == nil, "较早失败不得吞掉之后的导航")
        watch.cancel()
        FileHandle.standardError.write(Data("PASS failed opening: source input and unsent identity restored; newer deferred navigation still executes\n".utf8))

        let linkedID = "missing:linked"
        let found = ListedConversation(conversation: ConversationSummary(id: linkedID, title: .native("Synthetic linked"), updatedAtUnixMs: nil, createdAtUnixMs: nil, runtimeId: "missing", cwd: nil, canRename: false, canDelete: false, parentConversationId: nil, nativeReadState: nil), nativeId: "synthetic-native", agentsUri: nil, pinned: false, unread: false, archived: false, project: nil, moved: false, failed: false, group: nil)
        store.openLinkedConversation(id: linkedID)
        let oldRequest = store.checkAccess.linkedConversationRequest!
        store.selectConversation(id: latest.id)
        store.checkAccess.finishLinkedConversationLookup(.success(found), id: linkedID, move: .new, request: oldRequest)
        precondition(store.checkAccess.selectedConversationID == latest.id && store.checkAccess.deferredSelection == nil && store.conversation(id: linkedID) == nil,
                     "已选择新目标后，旧离表查找成功也不能夺回导航")
        wait()

        // Choosing the same link again is a new request too; an older
        // response cannot consume its intent or navigation-history move.
        store.openLinkedConversation(id: linkedID, move: .back)
        let firstRequest = store.checkAccess.linkedConversationRequest!
        store.openLinkedConversation(id: linkedID)
        let currentRequest = store.checkAccess.linkedConversationRequest!
        precondition(firstRequest != currentRequest)
        store.checkAccess.finishLinkedConversationLookup(.success(found), id: linkedID, move: .back, request: firstRequest)
        precondition(store.checkAccess.linkedConversationRequest == currentRequest && store.conversation(id: linkedID) == nil)
        store.checkAccess.finishLinkedConversationLookup(.success(found), id: linkedID, move: .new, request: currentRequest)
        precondition(store.checkAccess.selectedConversationID == linkedID && store.checkAccess.linkedConversationRequest == nil)
        wait()
        FileHandle.standardError.write(Data("PASS linked navigation: stale lookup cannot replace a newer destination or consume a newer request for the same link\n".utf8))
    }
}

extension ConversationStore {
    /// Runs the production serial receipt-before-projection runner with a
    /// preview store. Synthetic operations never create or open a backend.
    static func checkOutgoingAdmissionLifecycle(home: URL, inspect: (ConversationWindowModel, String) -> Void) {
        precondition(!MacNativePersistenceGate.isEnabled)
        precondition(home.path.hasPrefix("/tmp/") || home.path.hasPrefix("/private/tmp/"))
        let device = DeviceStore(preview: true)
        let window = ConversationWindowModel(application: device, kind: .workspace)
        let store = window.focused.store
        let summary = ConversationSummary(id: "queue-fixture", title: .native("合成提交与队列"), updatedAtUnixMs: nil,
            createdAtUnixMs: nil, runtimeId: "sample-instance", cwd: nil, canRename: false, canDelete: false,
            parentConversationId: nil, nativeReadState: nil)
        let history = DeliveredMessage(id: "history-user", role: .user, blocks: [.text(text: "已有原生用户消息，始终保留。")])
        func snapshot(_ sequence: UInt64, queue: [QueuedMessage] = [], sent: [DeliveredMessage] = [],
                      runState: RunState = .running) -> DeliveredSnapshot {
            let messages = [history] + sent
            var followUp = FollowUp()
            followUp.queue = queue
            return DeliveredSnapshot(cursor: .init(generation: 701, sequence: sequence), base: nil,
                messageIds: messages.map(\.id), messages: messages, revision: sequence, conversation: summary,
                contextRuntimeId: "sample-instance", resourceId: nil, modelRecordKey: nil, runState: runState,
                transcriptTurns: [], usage: .init(), transcriptItems: messages.map { .message(id: "message:" + $0.id, messageId: $0.id) },
                transcriptOutline: [], transcriptMessageIdentities: messages.map { .init(id: "message:" + $0.id, messageId: $0.id) },
                messageIdentityConfirmations: [], pendingInteractions: [],
                actions: .init(inputAccess: .editable, canSend: runState == .idle, canCancel: runState == .running, canSwitch: false),
                followUp: followUp, subAgents: [], collaborators: [])
        }
        func apply(_ value: DeliveredSnapshot) { store.checkAccess.apply(BindingMapping.snapshot(value)) }
        func assertTranscript(_ messages: [DeliveredMessage]) {
            precondition(store.transcript.rows.map(\.message.id) == messages.map(\.id),
                         "transcript must contain exactly the projected messages in order")
        }
        func enqueueFixture(_ operation: @escaping () throws -> SnapshotResult, consumeReceipt: @escaping (Result<SnapshotResult, Error>) -> Void) {
            let scope = store.checkAccess.projectionScope()
            store.checkAccess.enqueuePreparedOperation(operation, consumeReceipt: consumeReceipt) { value in
                store.checkAccess.applySnapshotResult(value, scope: scope)
            }
        }
        func receipt(_ id: String, _ value: SnapshotResult, beforeProjection: (() -> Void)? = nil) {
            var consumed = false
            enqueueFixture({ value }, consumeReceipt: { result in
                guard case .success(let incoming) = result else { preconditionFailure("synthetic receipt failed") }
                store.checkAccess.acceptSubmission(id, receipt: incoming)
                beforeProjection?()
                consumed = true
            })
            let end = Date().addingTimeInterval(8)
            while !consumed, Date() < end { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
            precondition(consumed && !store.checkAccess.isLoading, "synthetic business receipt must settle")
        }
        apply(snapshot(1))
        let queueInput = window.composer(for: window.focused)
        queueInput.restore("仅排队：尚未派发，不应进入消息列表。", attachments: [])
        let queueRevision = queueInput.revision
        let queued = store.checkAccess.outgoing.begin(text: queueInput.submission, attachments: [])
        store.checkAccess.composerRequest = .queueing
        precondition(store.pendingQueuedMessages.isEmpty && store.followUp.queue.isEmpty)
        precondition(window.focused.submission(using: queueInput).action == .queueing && !window.focused.submission(using: queueInput).enabled)
        assertTranscript([history])
        inspect(window, "pending-queue-input-retained")
        let canonical = QueuedMessage(id: "queue-A", submissionId: queued, text: "仅排队：尚未派发，不应进入消息列表。", attachments: [])
        receipt(queued, SnapshotResult(snapshot: snapshot(2, queue: [canonical]), submissionReceipt: .queued(queueId: canonical.id))) {
            store.checkAccess.composerRequest = nil
            if queueInput.revision == queueRevision { queueInput.clear(submitted: canonical.text, attachments: []) }
            precondition(queueInput.isEmpty, "successful queue admission consumes the unchanged captured input")
            precondition(store.followUp.queue.isEmpty, "receipt consumer must precede projection installation")
            precondition(store.pendingQueuedMessages.isEmpty,
                         "queue receipt must use its own projection, not the old Store snapshot")
        }
        precondition(store.followUp.queue.map(\.id) == [canonical.id])
        inspect(window, "accepted-queue")

        apply(snapshot(3))
        let racing = store.checkAccess.outgoing.begin(text: "忙闲竞态：核心将这次发送实际接纳为排队。", attachments: [])
        store.checkAccess.composerRequest = .sending
        assertTranscript([history])
        precondition(window.focused.submission(using: queueInput).action == .sending)
        inspect(window, "sending-without-native-message")
        let raced = QueuedMessage(id: "queue-race", submissionId: racing, text: "忙闲竞态：核心将这次发送实际接纳为排队。", attachments: [])
        apply(snapshot(4, queue: [raced]))
        precondition(store.pendingQueuedMessages.isEmpty && store.followUp.queue.count == 1,
                     "poll-before-ACK must transfer the same submission only to the queue")
        assertTranscript([history])
        inspect(window, "poll-before-ack")
        let dispatched = DeliveredMessage(id: "native-dispatched", role: .user, blocks: [.text(text: raced.text)])
        apply(snapshot(5, sent: [dispatched]))
        receipt(racing, SnapshotResult(snapshot: snapshot(4, queue: [raced]), submissionReceipt: .queued(queueId: raced.id))) {
            store.checkAccess.composerRequest = nil
        }
        precondition(store.checkAccess.snapshot?.cursor?.sequence == 5 && store.followUp.queue.isEmpty && store.pendingQueuedMessages.isEmpty,
                     "late queued ACK must neither overwrite a newer dispatch nor resurrect its placeholder")
        assertTranscript([history, dispatched])
        inspect(window, "dispatched-late-ack")

        let failure = store.checkAccess.outgoing.begin(text: "已由轮询确认受理，迟到错误不得恢复草稿。", attachments: [])
        let admitted = QueuedMessage(id: "queue-admitted", submissionId: failure, text: "已由轮询确认受理，迟到错误不得恢复草稿。", attachments: [])
        let releaseFailure = DispatchSemaphore(value: 0)
        var failureSettled = false, restoredDrafts = 0
        enqueueFixture({ () throws -> SnapshotResult in
            _ = releaseFailure.wait(timeout: .now() + 8)
            throw TransportError.rejected("合成 RPC 投影失败")
        }, consumeReceipt: { result in
            guard case .failure = result else { preconditionFailure("synthetic RPC must fail") }
            if store.checkAccess.outgoing.fail(failure) { restoredDrafts += 1 }
            failureSettled = true
        })
        apply(snapshot(6, queue: [admitted], sent: [dispatched]))
        releaseFailure.signal()
        let failureDeadline = Date().addingTimeInterval(8)
        while !failureSettled, Date() < failureDeadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
        precondition(failureSettled && restoredDrafts == 0 && store.pendingQueuedMessages.isEmpty,
                     "poll-confirmed admission must forbid draft restoration after a real RPC-lane failure")
        inspect(window, "admitted-before-rpc-failure")

        let unavailable = store.checkAccess.outgoing.begin(text: "已受理，投影暂时不可用。", attachments: [])
        receipt(unavailable, SnapshotResult(snapshot: nil, submissionReceipt: .queued(queueId: "queue-no-projection")))
        precondition(store.loadedConversationID == "queue-fixture" && store.checkAccess.snapshot?.cursor?.sequence == 6 && store.transcript.rows.count == 2,
                     "successful admission with unavailable projection must retain the current conversation and native transcript")
        precondition(store.pendingQueuedMessages.map(\.id) == ["queue-no-projection"])
        let recoveredQueue = QueuedMessage(id: "queue-no-projection", submissionId: unavailable, text: "已受理，投影暂时不可用。", attachments: [])
        apply(snapshot(7, queue: [admitted, recoveredQueue], sent: [dispatched]))
        precondition(store.pendingQueuedMessages.isEmpty && store.followUp.queue.count == 2)

        apply(snapshot(8))
        let immediateInput = "空闲即时派发：原生消息先于受理回执到达。"
        queueInput.restore(immediateInput, attachments: [])
        let immediateRevision = queueInput.revision
        let immediateToken = store.checkAccess.outgoing.begin(text: immediateInput, attachments: [])
        store.checkAccess.composerRequest = .queueing
        let immediateNative = DeliveredMessage(id: "native-before-ack", role: .user, blocks: [.text(text: immediateInput)])
        apply(snapshot(9, sent: [immediateNative]))
        precondition(store.followUp.queue.isEmpty && store.pendingQueuedMessages.isEmpty,
                     "a queue intent dispatched while idle must not manufacture a queue row beside the native echo")
        assertTranscript([history, immediateNative])
        precondition(queueInput.submission == immediateInput)
        inspect(window, "immediate-native-before-ack")
        receipt(immediateToken, SnapshotResult(snapshot: snapshot(9, sent: [immediateNative]), submissionReceipt: .sending)) {
            store.checkAccess.composerRequest = nil
            if queueInput.revision == immediateRevision { queueInput.clear(submitted: immediateInput, attachments: []) }
        }
        precondition(store.followUp.queue.isEmpty && queueInput.isEmpty)
        apply(snapshot(10, sent: [immediateNative], runState: .idle))
        let interrupted = store.checkAccess.outgoing.begin(text: "原生端尚未产生消息就停止。", attachments: [])
        store.checkAccess.composerRequest = .sending
        receipt(interrupted, SnapshotResult(snapshot: snapshot(11, sent: [immediateNative]), submissionReceipt: .sending)) {
            store.checkAccess.composerRequest = nil
        }
        precondition(store.checkAccess.snapshot?.runState == .running)
        assertTranscript([history, immediateNative])
        inspect(window, "accepted-without-native-message")
        apply(snapshot(12, sent: [immediateNative], runState: .idle))
        assertTranscript([history, immediateNative])
        precondition(store.followUp.queue.isEmpty && store.pendingQueuedMessages.isEmpty)
        inspect(window, "stopped-without-native-message")

        let ledger = OutgoingSubmissions()
        let failed = ledger.begin(text: "同文请求", attachments: [])
        precondition(ledger.fail(failed), "unadmitted failure must restore its original input")
        let retry = ledger.begin(text: "同文请求", attachments: [])
        let another = ledger.begin(text: "同文请求", attachments: [])
        precondition(failed != retry && retry != another && ledger.queued.isEmpty)
        ledger.reconcile([.init(id: "unrelated", submissionId: "another-client", text: "同文请求", attachments: [])])
        precondition(ledger.fail(retry), "unrelated queue rows do not acknowledge matching text")
        ledger.reconcile([.init(id: "after-close", submissionId: another, text: "同文请求", attachments: [])])
        ledger.reset()
        precondition(!ledger.fail(another) && ledger.queued.isEmpty,
                     "navigation must retain in-flight admission proof without leaking into the new view")
        if case .remove(let id) = OutgoingSubmissions.queuedCancellation(.queued(queueId: "cancel-exact")) {
            precondition(id == "cancel-exact")
        } else { preconditionFailure("queued cancellation must target its exact queue identity") }
        precondition(OutgoingSubmissions.queuedCancellation(.sending) == nil && OutgoingSubmissions.queuedCancellation(nil) == nil)
        let composer = ComposerModel()
        composer.restore("取消前的输入", attachments: [])
        let original = composer.submission, attachments = composer.attachments
        composer.clear(submitted: original, attachments: attachments)
        let revision = composer.revision
        composer.restoreFailed(original, attachments: attachments, revision: revision)
        precondition(composer.submission == original, "confirmed queued cancellation recovers the original input")
        composer.clear(submitted: original, attachments: attachments)
        let previous = composer.revision
        composer.restore("较新的编辑", attachments: [])
        composer.restoreFailed(original, attachments: attachments, revision: previous)
        precondition(composer.submission == "较新的编辑", "queued cancellation must not overwrite a newer input revision")
        precondition(!MacNativePersistenceGate.isEnabled && !store.checkAccess.started && !window.acceptsWorkspacePersistence)
        store.close(); window.list.close()
    }
}
#endif
