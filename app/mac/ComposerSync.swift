import Foundation
import Combine
import VeluneBindings

struct SavedComposerBinding: Codable, Equatable { var inputID: UUID; var revision: UInt64? }

struct SavedComposerPromotion: Codable, Equatable { var inputID: UUID; var runtimeID: String }

/// Serial I/O for a shared directory. A window never becomes its storage owner.
private final class ComposerInputWorker: @unchecked Sendable {
    let transport: Transport
    let queue = DispatchQueue(label: "local.velune.composer-inputs", qos: .userInitiated)
    private var state: ComposerInputState?
    private var executionOwnerID: String?
    private var prepared: [UUID: SavedComposerDraft] = [:]
    private var lastWrites: [String: UInt64] = [:]
    private var attachments: [String: ComposerAttachment] = [:]
    private var sourceVersions: [String: (Date?, Int?)] = [:]
    init(transport: Transport) { self.transport = transport }

    private func cacheURL(_ attachment: ComposerAttachment) -> URL {
        transport.stateDirectory.appendingPathComponent("composer-cache", isDirectory: true)
            .appendingPathComponent(attachment.id, isDirectory: true).appendingPathComponent(attachment.name)
    }

    func content(_ draft: SavedComposerDraft) throws -> ComposerInputContent {
        let owner = try executionOwnerID ?? transport.composerOwnerID()
        executionOwnerID = owner
        var seen: Set<String> = []
        let files = try draft.attachments.map { url -> ComposerAttachment in
            if let reference = attachments[url.path], !FileManager.default.isReadableFile(atPath: url.path) { return reference }
            let version = try url.resourceValues(forKeys: [.contentModificationDateKey, .fileSizeKey])
            if let reference = attachments[url.path], let previous = sourceVersions[url.path], previous.0 == version.contentModificationDate, previous.1 == version.fileSize { return reference }
            let bytes = try Data(contentsOf: url)
            let reference = try transport.uploadComposerAttachment(name: url.lastPathComponent, bytes: bytes)
            attachments[url.path] = reference
            sourceVersions[url.path] = (version.contentModificationDate, version.fileSize)
            let cache = cacheURL(reference)
            try FileManager.default.createDirectory(at: cache.deletingLastPathComponent(), withIntermediateDirectories: true)
            try bytes.write(to: cache, options: .atomic)
            attachments[cache.path] = reference
            let cachedVersion = try cache.resourceValues(forKeys: [.contentModificationDateKey, .fileSizeKey])
            sourceVersions[cache.path] = (cachedVersion.contentModificationDate, cachedVersion.fileSize)
            return reference
        }.filter { seen.insert($0.id).inserted }
        return ComposerInputContent(text: draft.text, attachments: files, runtimeId: draft.choices.runtimeID, runtimeOwnerId: draft.choices.runtimeID == nil ? nil : draft.runtimeOwnerID ?? owner,
            choices: (draft.foreignChoices ?? []) + draft.choices.choices.sorted { $0.key < $1.key }.map { ComposerRuntimeChoice(ownerId: owner, runtimeId: $0.key, choice: $0.value) },
            choiceEdited: draft.choiceEdited ?? false)
    }

    func perform(_ operation: ComposerInputOperation) throws -> (ComposerInputState, [UUID: SavedComposerDraft], String) {
        if executionOwnerID == nil { executionOwnerID = try transport.composerOwnerID() }
        let previous = state
        let update = try transport.composerInputs(operation)
        if update.state == nil, let state { return (state, prepared, executionOwnerID!) }
        guard let result = update.state else { throw TransportError.rejected("共享输入返回了无效修订") }
        state = result
        if case .ensure(_, let id, let seed) = operation,
           let input = result.inputs.first(where: { $0.id.caseInsensitiveCompare(id) == .orderedSame }) {
            let old = previous?.inputs.first { $0.id.caseInsensitiveCompare(id) == .orderedSame }
            if old == nil && input.revision == 1 || old?.revision == 1 && old?.content == ComposerInputContent(text: "", attachments: [], runtimeId: nil, runtimeOwnerId: nil, choices: [], choiceEdited: false) && input.revision == 2 && input.content == seed { lastWrites[id] = input.revision }
        }
        var contents: [UUID: SavedComposerDraft] = [:]
        for input in result.inputs {
            guard let id = UUID(uuidString: input.id) else { throw TransportError.rejected("输入记录的身份无效") }
            let urls = input.content.attachments.map { attachment -> URL in
                let url = cacheURL(attachment)
                attachments[url.path] = attachment
                if !FileManager.default.fileExists(atPath: url.path) {
                    // A missing blob must not prevent recovering the text and choices.
                    // Keep the reference visible so it can be retried or removed.
                    _ = try? { () throws -> Void in
                        let bytes = try transport.readComposerAttachment(attachment.id)
                        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
                        try bytes.write(to: url, options: .atomic)
                    }()
                }
                if let version = try? url.resourceValues(forKeys: [.contentModificationDateKey, .fileSizeKey]) {
                    sourceVersions[url.path] = (version.contentModificationDate, version.fileSize)
                }
                return url
            }
            contents[id] = SavedComposerDraft(text: input.content.text, attachments: urls,
                choices: ConversationChoices(runtimeID: input.content.runtimeId,
                    choices: Dictionary(uniqueKeysWithValues: input.content.choices.filter { $0.ownerId == executionOwnerID }.map { ($0.runtimeId, $0.choice) })), choiceEdited: input.content.choiceEdited, runtimeOwnerID: input.content.runtimeOwnerId, foreignChoices: input.content.choices.filter { $0.ownerId != executionOwnerID })
        }
        prepared = contents
        return (result, contents, executionOwnerID!)
    }
    func poll(force: Bool) throws -> (ComposerInputState, [UUID: SavedComposerDraft], String) { try perform(.read(sinceRevision: force ? nil : state?.revision)) }

    func acknowledgedRevision(of id: UUID) -> UInt64? {
        let key = id.uuidString.lowercased()
        let revision = state?.inputs.first { $0.id == key }?.revision
        return lastWrites[key] == revision ? revision : nil
    }
    func save(id: UUID, observed: UInt64?, draft: SavedComposerDraft) throws -> (ComposerInputState, [UUID: SavedComposerDraft], String) {
        let key = id.uuidString.lowercased()
        let current = state?.inputs.first { $0.id.caseInsensitiveCompare(key) == .orderedSame }?.revision
        // Consecutive local keystrokes may have been queued before the prior acknowledgement.
        let expected = current != nil && lastWrites[key] == current ? current : observed
        let result = try perform(.save(inputId: key, expectedRevision: expected, content: content(draft)))
        lastWrites[key] = result.0.inputs.first { $0.id.caseInsensitiveCompare(key) == .orderedSame }?.revision
        return result
    }
}

/// Content and context bindings synchronize; each window keeps its own navigation cursor.
@MainActor final class ComposerInputClient {
    private let worker: ComposerInputWorker?
    private weak var device: DeviceStore?
    private(set) var ownerID: String?
    private(set) var state: ComposerInputState?
    private var contents: [UUID: SavedComposerDraft] = [:]
    private var dirty: [UUID: SavedComposerDraft] = [:]
    private var baselines: [UUID: UInt64] = [:]
    private var saveFailures: Set<UUID> = []
    private var available = false
    private var scheduled: [UUID: DispatchWorkItem] = [:]
    private var optimisticBindings: [ComposerContextKey: SavedComposerBinding] = [:]
    private var promotions: [String: SavedComposerPromotion] = [:]
    private var failedBindings: Set<ComposerContextKey> = []
    private var runningBindings: Set<ComposerContextKey> = []
    private var runningPromotions: Set<String> = []
    private var previewBindings: [ComposerContextKey: UUID] = [:]
    private var deferred: [() -> Void] = []
    private var pollPending = false
    private var lastPoll = Date.distantPast
    let changes = PassthroughSubject<Void, Never>()

    init(device: DeviceStore, transport: Transport?) {
        self.device = device
        worker = transport.map(ComposerInputWorker.init)
        if transport == nil { ownerID = "preview" }
    }

    func context(_ conversationID: String) -> ComposerContextKey? {
        if conversationID.hasPrefix(ConversationStore.draftConversationPrefix) { return .newConversation }
        guard !conversationID.hasPrefix("tab:"), !conversationID.hasPrefix("side-chat:"), let ownerID else { return nil }
        return .conversation(ownerId: ownerID, conversationId: conversationID)
    }
    func inputID(for conversationID: String) -> UUID? {
        guard let context = context(conversationID) else { return nil }
        if worker == nil { return previewBindings[context] }
        if let pending = optimisticBindings[context] { return pending.inputID }
        guard let key = state?.bindings.first(where: { $0.context == context })?.inputId else { return nil }
        return UUID(uuidString: key)
    }
    func draft(_ id: UUID) -> SavedComposerDraft? { dirty[id] ?? contents[id] }
    func isDirty(_ id: UUID) -> Bool { dirty[id] != nil }
    func pendingBinding(_ conversationID: String) -> SavedComposerBinding? { context(conversationID).flatMap { optimisticBindings[$0] } }
    var pendingPromotions: [String: SavedComposerPromotion] { promotions }
    func restoreBinding(_ conversationID: String, value: SavedComposerBinding) {
        guard let context = context(conversationID) else { return }
        optimisticBindings[context] = value
    }
    func restorePromotion(_ conversationID: String, value: SavedComposerPromotion) { promotions[conversationID] = value }
    func revision(_ id: UUID) -> UInt64? { isDirty(id) ? baselines[id] : state?.inputs.first { UUID(uuidString: $0.id) == id }?.revision }
    func issue(for id: UUID) -> String? {
        guard worker != nil else { return nil }
        guard state != nil else { return "正在恢复输入" }
        guard available else { return "输入尚未同步，可继续编辑" }
        guard !failedBindings.contains(where: { optimisticBindings[$0]?.inputID == id }) else { return "输入关联已更新，当前输入已保留" }
        guard !saveFailures.contains(id) else { return "输入同步失败，未发送内容已保留" }
        guard state?.inputs.contains(where: { UUID(uuidString: $0.id) == id }) == true else { return "正在同步输入" }
        return nil
    }
    func restorePending(_ id: UUID, draft: SavedComposerDraft, revision: UInt64?) {
        guard dirty[id] == nil else { return }
        dirty[id] = draft; baselines[id] = revision
        saveNow(id, draft: draft)
    }
    func needsNewSeed(_ conversationID: String) -> Bool {
        guard conversationID.hasPrefix(ConversationStore.draftConversationPrefix) else { return false }
        guard let id = inputID(for: conversationID) else { return true }
        if worker == nil { return contents[id] == nil }
        guard let input = state?.inputs.first(where: { UUID(uuidString: $0.id) == id }) else { return true }
        return input.revision == 1 && input.content == ComposerInputContent(text: "", attachments: [], runtimeId: nil, runtimeOwnerId: nil, choices: [], choiceEdited: false)
    }

    func poll(force: Bool = false) {
        guard let worker, !pollPending, force || Date().timeIntervalSince(lastPoll) > 0.6 else { return }
        pollPending = true; lastPoll = Date()
        worker.queue.async { [weak self] in
            let result = Result { try worker.poll(force: force) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.pollPending = false
                switch result {
                case .success(let result): self.apply(result); if force { self.changes.send() }
                case .failure(let error):
                    self.available = false; self.changes.send()
                    if force { self.device?.recordProblem(error, source: "读取共享输入") }
                }
            }
        }
    }

    private func apply(_ result: (ComposerInputState, [UUID: SavedComposerDraft], String)) {
        let previous = state
        let wasAvailable = available
        available = true
        state = result.0; contents = result.1
        if ownerID == nil { ownerID = result.2 }
        if previous != state || !wasAvailable { changes.send() }
        // Reconnect can acknowledge a write whose response was lost, or retry
        // the same baseline. A newer, different revision is never overwritten.
        for id in Array(saveFailures) {
            guard let draft = dirty[id] else { continue }
            if result.1[id] == draft {
                dirty[id] = nil; baselines[id] = nil; saveFailures.remove(id); changes.send()
            } else if result.0.inputs.first(where: { UUID(uuidString: $0.id) == id })?.revision == baselines[id] {
                saveFailures.remove(id); saveNow(id, draft: draft)
            }
        }
        if !deferred.isEmpty { let waiting = deferred; deferred.removeAll(); waiting.forEach { $0() } }
        retryAssociations()
    }

    func ensure(_ conversationID: String, model: ComposerModel, completion: @escaping (UUID) -> Void) {
        guard let worker else {
            guard let context = context(conversationID) else { completion(model.id); return }
            let id = previewBindings[context] ?? model.id
            previewBindings[context] = id
            if contents[id] == nil { contents[id] = model.savedDraft }
            completion(id); return
        }
        guard let context = context(conversationID) else {
            if ownerID == nil {
                deferred.append { [weak self, weak model] in if let self, let model { self.ensure(conversationID, model: model, completion: completion) } }
                poll(force: true)
            }
            return
        }
        let seed = model.savedDraft ?? SavedComposerDraft(text: "", attachments: [], choices: ConversationChoices(), choiceEdited: false)
        let id = model.id
        worker.queue.async { [weak self] in
            let result = Result {
                let updated = try worker.perform(.ensure(context: context, inputId: id.uuidString.lowercased(), seed: worker.content(seed)))
                return (updated, worker.acknowledgedRevision(of: id))
            }
            DispatchQueue.main.async {
                guard let self else { return }
                switch result {
                case .success(let acknowledged):
                    let result = acknowledged.0
                    if result.0.bindings.contains(where: { $0.context == context && $0.inputId == id.uuidString.lowercased() }), self.dirty[id] != nil, self.baselines[id] == nil { self.baselines[id] = acknowledged.1 }
                    self.apply(result)
                    if let key = result.0.bindings.first(where: { $0.context == context })?.inputId, let resolved = UUID(uuidString: key) { completion(resolved) }
                case .failure(let error): self.device?.recordProblem(error, source: "恢复共享输入")
                }
            }
        }
    }

    func save(_ id: UUID, draft: SavedComposerDraft) {
        guard worker != nil else { contents[id] = draft; changes.send(); return }
        if dirty[id] == nil { baselines[id] = state?.inputs.first { UUID(uuidString: $0.id) == id }?.revision }
        dirty[id] = draft
        scheduled[id]?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.saveNow(id, draft: draft) }
        scheduled[id] = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25, execute: work)
    }
    private func saveNow(_ id: UUID, draft: SavedComposerDraft) {
        scheduled.removeValue(forKey: id)?.cancel()
        guard let worker else { contents[id] = draft; return }
        if dirty[id] == nil { baselines[id] = state?.inputs.first { UUID(uuidString: $0.id) == id }?.revision }
        dirty[id] = draft
        let observed = baselines[id]
        worker.queue.async { [weak self] in
            let result = Result { try worker.save(id: id, observed: observed, draft: draft) }
            DispatchQueue.main.async {
                guard let self else { return }
                switch result {
                case .success(let result):
                    self.saveFailures.remove(id)
                    if self.dirty[id] == draft { self.dirty[id] = nil; self.baselines[id] = nil }
                    self.apply(result)
                    self.changes.send()
                case .failure(let error): self.saveFailures.insert(id); self.available = false; self.changes.send(); self.device?.recordProblem(error, source: "同步输入")
                }
            }
        }
    }

    func beginNavigation(_ conversationID: String) {
        guard let context = context(conversationID), failedBindings.remove(context) != nil else { return }
        optimisticBindings[context] = nil
    }
    func bind(_ conversationID: String, input: ComposerModel) {
        guard let context = context(conversationID) else { return }
        guard worker != nil else {
            previewBindings[context] = input.id; contents[input.id] = input.savedDraft; changes.send(); return
        }
        failedBindings.remove(context)
        optimisticBindings[context] = .init(inputID: input.id, revision: state?.bindings.first(where: { $0.context == context })?.revision)
        if let draft = input.savedDraft { saveNow(input.id, draft: draft) }
        else { retryAssociations() }
    }
    /// A sent first message consumes the global new-conversation input: the
    /// entry immediately takes the next empty input (choices kept), so a
    /// draft tab met while the creation is in flight resolves the fresh one
    /// and cannot follow the sent input's conversation when creation lands.
    /// Nothing happens when the entry does not point at `sent` (a borrowed
    /// input leaves the global new entry alone).
    func consumeNewConversationInput(_ sent: ComposerModel) {
        let draftID = ConversationStore.draftConversationPrefix + "new-conversation"
        guard inputID(for: draftID) == sent.id else { return }
        let kept = sent.savedDraft
        let fresh = ComposerModel(followsConversation: false)
        fresh.restoreSavedDraft(SavedComposerDraft(text: "", attachments: [], choices: kept?.choices ?? ConversationChoices(), choiceEdited: kept?.choiceEdited ?? false, runtimeOwnerID: kept?.runtimeOwnerID, foreignChoices: kept?.foreignChoices))
        bind(draftID, input: fresh)
    }
    /// A failed first send hands the consumed input back to the global new
    /// entry — only while the entry's fresh input is still untouched; with
    /// newer content in it, the consumed input stays kept (保留的输入)
    /// rather than replacing that work.
    func restoreConsumedNewConversationInput(_ consumed: ComposerModel) {
        let draftID = ConversationStore.draftConversationPrefix + "new-conversation"
        guard let current = inputID(for: draftID), current != consumed.id else { return }
        if let currentDraft = draft(current), !currentDraft.text.isEmpty || !currentDraft.attachments.isEmpty { return }
        bind(draftID, input: consumed)
    }
    func promote(_ model: ComposerModel, to conversationID: String, runtimeID: String) {
        guard worker != nil, ownerID != nil else { return }
        promotions[conversationID] = .init(inputID: model.id, runtimeID: runtimeID)
        if let draft = model.savedDraft { saveNow(model.id, draft: draft) }
        retryAssociations()
    }
    private func retryAssociations() {
        guard available, let worker, let ownerID else { return }
        var changedBindingStatus = false
        for (context, pending) in optimisticBindings where dirty[pending.inputID] == nil && !saveFailures.contains(pending.inputID) {
            let binding = state?.bindings.first { $0.context == context }
            if binding?.inputId == pending.inputID.uuidString.lowercased() {
                optimisticBindings[context] = nil; failedBindings.remove(context); changedBindingStatus = true
            } else if binding?.revision == pending.revision, state?.inputs.contains(where: { UUID(uuidString: $0.id) == pending.inputID }) == true {
                run(worker, .bind(context: context, inputId: pending.inputID.uuidString.lowercased(), expectedRevision: pending.revision))
            } else { if failedBindings.insert(context).inserted { changedBindingStatus = true } }
        }
        if changedBindingStatus { changes.send() }
        for (conversation, promotion) in promotions where dirty[promotion.inputID] == nil {
            if state?.inputs.contains(where: { UUID(uuidString: $0.id) == promotion.inputID }) == true { run(worker, .promote(inputId: promotion.inputID.uuidString.lowercased(), ownerId: ownerID, conversationId: conversation, runtimeId: promotion.runtimeID)) }
        }
    }
    private func run(_ worker: ComposerInputWorker, _ operation: ComposerInputOperation) {
        if case .bind(let context, _, _) = operation { guard runningBindings.insert(context).inserted else { return } }
        if case .promote(_, _, let conversation, _) = operation { guard runningPromotions.insert(conversation).inserted else { return } }
        worker.queue.async { [weak self] in
            let result = Result { try worker.perform(operation) }
            DispatchQueue.main.async {
                guard let self else { return }
                if case .bind(let context, _, _) = operation { self.runningBindings.remove(context) }
                if case .promote(_, _, let conversation, _) = operation { self.runningPromotions.remove(conversation) }
                switch result {
                case .success(let result):
                    if case .bind(let context, let id, let expected) = operation {
                        if self.optimisticBindings[context]?.inputID.uuidString.lowercased() == id { self.optimisticBindings[context] = nil; self.failedBindings.remove(context) }
                        else if self.optimisticBindings[context]?.revision == expected { self.optimisticBindings[context]?.revision = result.0.bindings.first { $0.context == context }?.revision }
                    }
                    if case .promote(let id, _, let conversation, _) = operation, self.promotions[conversation]?.inputID.uuidString.lowercased() == id { self.promotions[conversation] = nil }
                    self.apply(result)
                    self.changes.send()
                case .failure(let error):
                    if case .bind(let context, _, _) = operation { self.failedBindings.insert(context) }
                    self.available = false; self.changes.send(); self.device?.recordProblem(error, source: "更新输入关联")
                }
            }
        }
    }
    func flush(_ completion: @escaping () -> Void) {
        guard let worker else { completion(); return }
        for (id, draft) in dirty { saveNow(id, draft: draft) }
        drain(worker, passes: 3, completion: completion)
    }
    private func drain(_ worker: ComposerInputWorker, passes: Int, completion: @escaping () -> Void) {
        worker.queue.async { [weak self] in
            DispatchQueue.main.async {
                guard let self else { completion(); return }
                self.retryAssociations()
                if passes > 0 && (!self.runningBindings.isEmpty || !self.runningPromotions.isEmpty) {
                    self.drain(worker, passes: passes - 1, completion: completion)
                } else { completion() }
            }
        }
    }
}
