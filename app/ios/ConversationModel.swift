import SwiftUI

struct LegacyComposerEntry: Identifiable {
    var id: String
    var content: ComposerInputContent
    var target: ComposerTargetChoices? = nil
    var title: String { String(content.text.split(whereSeparator: \.isNewline).first?.prefix(40) ?? "本机输入".prefix(40)) }
}

struct ConversationTarget: Hashable, Codable {
    var key: String { conversation.map { runtime + ":" + $0 } ?? "new-conversation" }
    static func == (lhs: Self, rhs: Self) -> Bool { lhs.key == rhs.key }
    func hash(into hasher: inout Hasher) { hasher.combine(key) }
    var runtime: String
    var conversation: String?
    var title: String
    var cwd: String?
    // Multiple new tabs resolve the same shared new-conversation input.
    var draftID = UUID()
    init(_ listed: ListedConversation) {
        self.init(listed.conversation)
    }
    init(_ summary: ConversationSummary) {
        runtime = summary.runtimeId
        conversation = summary.id
        title = summary.title.displayText()
        cwd = summary.cwd
        draftID = UUID(uuidString: "00000000-0000-0000-0000-000000000000")!
    }
    init(runtime: String, cwd: String?) {
        self.runtime = runtime; self.cwd = cwd; title = "新会话"
    }
}

@MainActor
final class ConversationModel: ObservableObject {
    let client: RemoteClient
    let peer: String
    let target: ConversationTarget
    @Published var snapshot: DeliveredSnapshot?
    @Published var messages: [String: DeliveredMessage] = [:]
    @Published var models: [RuntimeModel] = []
    @Published var selection: ModelSelection? {
        didSet {
            guard !restoringSelection else { return }
            inputRuntimeOwner = composerOwner; composerChoiceEdited = true
            var kept = choices[runtimeID] ?? .init(model: nil, effort: nil, permissionMode: nil)
            kept.model = selection; choices[runtimeID] = kept
            targetChanged()
        }
    }
    @Published private(set) var destination: DraftDestination
    @Published private(set) var loadingModels = false
    private var choices: [String: TurnChoice] = [:]
    private var restoringSelection = false
    @Published var busy = false
    @Published var loading = true
    @Published var failure: String?
    @Published var draft = "" { didSet { if draft != oldValue { composerChanged() } } }
    @Published private(set) var inputAttachments: [ComposerAttachment] = []
    @Published private(set) var composerFailure: String?
    private var sharedInput: ComposerInput?
    private var composerOwner: String?
    private var composerDirectoryRevision: UInt64?
    private var composerLoaded = false
    private var composerPolling = false
    @Published private(set) var legacyInput: ComposerInputContent?
    var legacyInputs: [LegacyComposerEntry] = []
    private var composerDirty = false
    private var contentGeneration: UInt64 = 0
    private var sharedTarget: ComposerTarget?
    private var targetDirty = false
    private var targetBaseline: UInt64?
    private var observedTargetRevision: UInt64?
    private var targetWriteFailed = false
    private var legacyTarget: ComposerTargetChoices?
    private var composerUnbound = false
    private var composerRestored = false
    private var composerBindingRevision: UInt64?
    private var composerBindingBaseline: UInt64?
    private var composerSaving = false
    private var composerBaseline: UInt64?
    private var composerSaveTask: Task<Void, Never>?
    private var applyingComposer = false
    private var inputRuntime: String?
    private var inputRuntimeOwner: String?
    private var composerChoiceEdited = false
    private var pendingCreation: PendingComposerCreation?
    private var runtimeAvailable = false
    @Published var sending: String?
    var onComposerChange: ((ConversationModel) -> Void)?
    var onIdentityChange: ((ConversationModel) -> Void)?
    var problemContext = ""
    var following = true
    var readingOffset: CGFloat = 0
    private var heldUnreadCursor: TranscriptCursor?
    private var markingRead = false
    private var payloads: [String: DeliveredMessage] = [:]

    init(client: RemoteClient, peer: String, target: ConversationTarget) {
        self.client = client; self.peer = peer; self.target = target
        destination = .init(runtime: target.runtime, cwd: target.cwd)
    }

    var canEditTarget: Bool { peer == "preview" || composerLoaded && sharedTarget != nil }
    var runtimeID: String { inputRuntime ?? snapshot?.contextRuntimeId ?? destination.runtime }
    var title: String {
        if snapshot?.conversation.parentConversationId != nil { return target.title }
        return snapshot?.conversation.title.displayText() ?? target.title
    }
    var currentTarget: ConversationTarget {
        var result = snapshot.map { ConversationTarget($0.conversation) } ?? target
        result.title = title
        return result
    }
    var cwd: String? { snapshot.map { $0.conversation.cwd } ?? destination.cwd }
    var isNew: Bool { snapshot == nil && target.conversation == nil }
    var composerState: ComposerState {
        var cached = sharedInput; cached?.content = composerContent
        return .init(sharedTarget: sharedTarget.map { .init(context: $0.context, revision: $0.revision, choices: composerTargetContent) }, targetPending: targetDirty, targetBaseline: targetBaseline, legacyTarget: legacyTarget, text: draft, selection: selection, following: following, offset: readingOffset, destination: destination, choices: choices, sharedInput: cached, composerPending: composerDirty, composerBaseline: composerBaseline, legacyInput: legacyInput, composerUnbound: composerUnbound, composerDirectoryRevision: composerDirectoryRevision, pendingCreation: pendingCreation, composerBindingRevision: composerBindingRevision, composerBindingBaseline: composerBindingBaseline)
    }
    func restore(_ state: ComposerState) {
        applyingComposer = true
        defer { applyingComposer = false }
        sharedTarget = state.sharedTarget; observedTargetRevision = state.sharedTarget?.revision; targetDirty = state.targetPending ?? false; targetBaseline = state.targetBaseline; legacyTarget = state.legacyTarget
        composerRestored = true; composerBindingRevision = state.composerBindingRevision; composerBindingBaseline = state.composerBindingBaseline
        composerDirectoryRevision = state.composerDirectoryRevision; pendingCreation = state.pendingCreation
        sharedInput = state.sharedInput; composerDirty = state.composerPending ?? false; composerUnbound = state.composerUnbound ?? false; composerBaseline = state.composerBaseline
        legacyInput = state.legacyInput
        if state.composerSchema != 2, state.sharedInput != nil || !state.text.isEmpty || !(state.choices ?? [:]).isEmpty {
            // Keep old local work behind the existing explicit restore action.
            legacyInput = state.sharedInput?.content ?? .init(text: state.text, attachments: [])
            legacyTarget = .init(runtimeId: state.destination?.runtime, runtimeOwnerId: nil, choices: (state.choices ?? [:]).map { .init(ownerId: "", runtimeId: $0.key, choice: $0.value) }, choiceEdited: true)
            sharedInput = nil; composerDirty = false; composerUnbound = false
        }
        inputAttachments = sharedInput?.content.attachments ?? []
        composerChoiceEdited = state.sharedTarget?.choices.choiceEdited ?? false
        inputRuntime = state.sharedTarget?.choices.runtimeId; inputRuntimeOwner = state.sharedTarget?.choices.runtimeOwnerId
        if let saved = state.destination { destination = saved }
        choices = state.choices ?? [runtimeID: .init(model: state.selection, effort: nil, permissionMode: nil)]
        restoringSelection = true
        draft = state.text; selection = state.selection; following = state.following; readingOffset = state.offset
        restoringSelection = false
    }
    func chooseProject(_ cwd: String?) {
        guard isNew, !busy else { return }
        destination.cwd = cwd
        onComposerChange?(self)
    }
    func chooseRuntime(_ runtime: String) {
        guard isNew, !busy, canEditTarget, runtime != runtimeID else { return }
        // The draft's identity stays fixed while its sending destination changes.
        destination.runtime = runtime; inputRuntime = runtime; inputRuntimeOwner = composerOwner; composerChoiceEdited = true
        restoringSelection = true
        selection = choices[runtime]?.model
        restoringSelection = false
        models = []
        targetChanged()
    }
    func loadModels() async {
        let runtime = runtimeID
        guard !runtime.isEmpty else { return }
        loadingModels = true
        defer { if runtimeID == runtime && !Task.isCancelled { loadingModels = false } }
        let needsDefault = isNew && choices[runtime] == nil
        do {
            let result = try await client.application(peer, operation: .configurationRead) {
                (try $0.runtimeModels(runtimeId: runtime), needsDefault ? try $0.composerChoices() : nil)
            }
            guard runtimeID == runtime, !Task.isCancelled else { return }
            runtimeAvailable = true
            models = result.0
            if needsDefault, choices[runtime] == nil, sharedTarget?.choices.runtimeOwnerId == nil {
                restoringSelection = true
                defer { restoringSelection = false }
                if let defaults = result.1?.runtimes.first(where: { $0.runtimeId == runtime }) {
                    switch defaults.newConversation {
                    case .lastChosen: selection = defaults.lastChosen.model
                    case .chosen(let choice): selection = choice.model
                    }
                } else { selection = nil }
            }
        } catch {
            guard runtimeID == runtime, !Task.isCancelled else { return }
            runtimeAvailable = false
            report(error, source: "读取模型")
        }
    }
    var inputLockMessage: String? {
        if let source = inputRuntimeOwner, source != composerOwner { return "所选运行时位于另一设备，输入已保留。" }
        return switch snapshot?.actions.inputAccess {
        case .none, .editable: nil
        case .checkingControl: "正在检查会话控制权…"
        case .externalController: "另一个客户端正在控制此会话。草稿可继续编辑，暂不能发送。"
        case .controlUnavailable: "暂时无法确认会话控制权，正在重试。"
        case .externalExecution: "此会话正在其他客户端执行。"
        case .readOnly: "此会话当前只可阅读。"
        }
    }
    var canSend: Bool {
        !loading && !loadingModels && !busy && (peer == "preview" || composerLoaded && !composerSaving && pendingCreation == nil && composerFailure == nil) && !runtimeID.isEmpty && (peer == "preview" || (inputRuntimeOwner == nil || inputRuntimeOwner == composerOwner) && runtimeAvailable && (selection == nil || models.contains { $0.selection == selection })) && (target.conversation == nil || snapshot != nil)
            && (!draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !inputAttachments.isEmpty) && snapshot?.actions.canSend != false
    }
    var canEditDraft: Bool { snapshot?.actions.inputAccess != .readOnly }
    func report(_ error: Error, source: String) {
        failure = mobileError(error)
        ProblemStore.shared.record(error, source: source, context: problemContext)
    }
    func holdUnread() { heldUnreadCursor = snapshot?.cursor }
    func resumeReading() { heldUnreadCursor = nil }
    // Native child sessions do not own list organization marks.
    func markSeen() async -> Bool {
        guard !markingRead, let shown = snapshot, shown.cursor != heldUnreadCursor,
              shown.conversation.parentConversationId == nil else { return false }
        markingRead = true
        defer { markingRead = false }
        do {
            let marked = try await client.application(peer, operation: .conversationWrite) { app -> Bool in
                guard !Task.isCancelled else { return false }
                _ = try app.list()
                guard let row = try app.findConversation(conversationId: shown.conversation.id),
                      row.conversation.runtimeId == shown.conversation.runtimeId else { return false }
                guard row.unread else { return false }
                // Core owns the list activity watermark; transcript timestamps are not equivalent.
                _ = try app.organizeConversations(conversations: [.init(runtimeId: shown.conversation.runtimeId, conversationId: shown.conversation.id, nativeReadState: shown.conversation.nativeReadState)], change: .markRead)
                return true
            }
            return marked
        } catch { report(error, source: "标为已读"); return false }
    }

    func open() async {
        do {
            if let id = target.conversation {
                apply(try await client.application(peer, operation: .conversationRead) { try $0.readConversation(runtimeId: self.target.runtime, conversationId: id) })
            }
            failure = nil
        } catch { report(error, source: "读取会话") }
        loading = false
        await pollComposer()
    }

    func poll() async {
        await pollComposer()
        guard !busy else { return }
        guard let current = snapshot else {
            if target.conversation != nil { await open() }
            return
        }
        do {
            let update = try await client.application(peer, operation: .conversationRead) {
                try $0.snapshotUpdate(runtimeId: current.contextRuntimeId, conversationId: current.conversation.id, since: current.cursor)
            }
            switch update {
            case .changed(let next): merge(next)
            case .absent:
                apply(try await client.application(peer, operation: .conversationRead) { try $0.readConversation(runtimeId: current.contextRuntimeId, conversationId: current.conversation.id) })
            case .unchanged: break
            }
            failure = nil
        } catch { report(error, source: "更新会话") }
    }

    private func apply(_ result: SnapshotResult) { if let next = result.snapshot { merge(next) } }

    func merge(_ next: DeliveredSnapshot) {
        let fresh = Dictionary(uniqueKeysWithValues: next.messages.map { ($0.id, $0) })
        let ids = Set(next.messageIds)
        messages = next.base == nil ? fresh : messages.merging(fresh) { _, new in new }.filter { ids.contains($0.key) }
        payloads = payloads.filter { id, loaded in fresh[id].map { $0.stamp == loaded.stamp } ?? ids.contains(id) }
        let identityChanged = snapshot?.conversation.id != next.conversation.id
        snapshot = next
        if identityChanged { onIdentityChange?(self) }
        if choices[runtimeID] == nil, !composerLoaded {
            restoringSelection = true
            selection = next.modelRecordKey.map { .managed(modelRecordKey: $0) }
            restoringSelection = false
        }
    }

    func message(_ id: String) -> DeliveredMessage? { payloads[id] ?? messages[id] }

    func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard canSend else { return }
        let runtime = runtimeID, cwd = cwd, selection = selection
        let submittedContent = composerContent
        let submittedGeneration = contentGeneration
        let submittedTarget = composerTargetContent
        act("发送消息") {
            try await self.flushComposer()
            guard let input = self.sharedInput else { throw MobileError.unavailable }
            let choice = submittedTarget.choices.first { $0.runtimeId == runtime && $0.ownerId == self.composerOwner }?.choice
            self.sending = text
            defer { self.sending = nil }
            if self.snapshot == nil {
                guard self.target.conversation == nil else { throw MobileError.missingConversation }
                let created = try await self.client.application(self.peer, operation: .conversationWrite) {
                    try $0.createConversation(runtimeId: runtime, cwd: cwd, selection: selection)
                }
                guard let native = created.snapshot, let owner = self.composerOwner else { throw MobileError.missingConversation }
                self.pendingCreation = .init(inputID: input.id, ownerID: owner, runtimeID: native.conversation.runtimeId, conversationID: native.conversation.id, choices: submittedTarget)
                self.apply(created)
                self.onComposerChange?(self)
                try await self.completeCreation()
            }
            guard let current = self.snapshot else { throw MobileError.missingConversation }
            let turn = ComposedTurn(text: text, effort: choice?.effort, permissionMode: choice?.permissionMode, attachments: submittedContent.attachments.map { "composer:" + $0.id })
            self.apply(try await self.client.application(self.peer, operation: .send) {
                try $0.sendTurn(sourceRuntimeId: current.contextRuntimeId, conversationId: current.conversation.id,
                    targetRuntimeId: runtime, selection: selection, turn: turn)
            })
            if self.sharedInput?.id == input.id, self.contentGeneration == submittedGeneration, self.composerContent == submittedContent {
                self.applyingComposer = true
                self.draft = ""; self.inputAttachments = []; self.composerChoiceEdited = false
                self.applyingComposer = false
                self.composerChanged()
                try await self.flushComposer()
            }
        }
    }

    func compact() { act("压缩上下文") {
        guard let current = self.snapshot, current.followUp.canCompact else { return }
        let selection = self.selection
        self.apply(try await self.client.application(self.peer, operation: .compact) {
            try $0.compactConversation(runtimeId: current.contextRuntimeId, conversationId: current.conversation.id, selection: selection)
        })
    } }

    func stop() { act("停止生成") {
        guard let current = self.snapshot else { return }
        self.apply(try await self.client.application(self.peer, operation: .cancel) { try $0.cancel(runtimeId: current.contextRuntimeId, conversationId: current.conversation.id) })
    } }

    func reply(_ id: String, _ reply: RuntimeInteractionReply) { act("回复运行时请求") {
        guard let current = self.snapshot else { return }
        self.apply(try await self.client.application(self.peer, operation: .reply) {
            try $0.replyRuntimeInteraction(runtimeId: current.contextRuntimeId, conversationId: current.conversation.id, interactionId: id, reply: reply)
        })
    } }

    func loadPayload(_ id: String) async {
        guard let current = snapshot else { return }
        do {
            let loaded = try await client.application(peer, operation: .conversationRead) {
                try $0.messagePayloads(runtimeId: current.contextRuntimeId, conversationId: current.conversation.id, messageIds: [id])
            }
            for message in loaded where messages[message.id]?.stamp == message.stamp { payloads[message.id] = message }
            objectWillChange.send()
        } catch { report(error, source: "读取完整内容") }
    }

    private func act(_ source: String, _ body: @escaping () async throws -> Void) {
        guard !busy else { return }
        busy = true; failure = nil
        Task {
            defer { busy = false }
            do { try await body(); ProblemStore.shared.recovered(source: source, context: problemContext) } catch { report(error, source: source) }
        }
    }
}


extension ConversationModel {
    private var composerContent: ComposerInputContent { .init(text: draft, attachments: inputAttachments) }
    private var composerTargetContent: ComposerTargetChoices {
        let runtime = inputRuntime ?? runtimeID
        return .init(runtimeId: runtime, runtimeOwnerId: inputRuntimeOwner ?? composerOwner,
                     choices: (sharedTarget?.choices.choices.filter { $0.ownerId != composerOwner } ?? []) + choices.sorted { $0.key < $1.key }.map { .init(ownerId: composerOwner ?? "", runtimeId: $0.key, choice: $0.value) }, choiceEdited: composerChoiceEdited)
    }
    private var composerTargetContext: ComposerContextKey? {
        guard let owner = composerOwner else { return nil }
        return (snapshot?.conversation.id ?? target.conversation).map { .conversation(ownerId: owner, conversationId: $0) } ?? .newConversation
    }
    private func composerError(_ error: Error) -> String {
        mobileError(error) + (targetWriteFailed ? "；重新选择以采用当前配置" : "")
    }
    private func targetChanged() {
        guard !applyingComposer else { return }
        // This method is reached only by an explicit selection/import.
        targetBaseline = observedTargetRevision ?? sharedTarget?.revision
        targetDirty = true; onComposerChange?(self)
        composerSaveTask?.cancel()
        composerSaveTask = Task { [weak self] in
            do { try await Task.sleep(for: .milliseconds(250)); try await self?.flushComposer() }
            catch is CancellationError {} catch { self?.composerFailure = self?.composerError(error); self?.report(error, source: "同步目标执行配置") }
        }
    }
    private func receiveTarget(_ target: ComposerTarget) {
        applyingComposer = true
        sharedTarget = target; composerChoiceEdited = target.choices.choiceEdited
        inputRuntime = target.choices.runtimeId; inputRuntimeOwner = target.choices.runtimeOwnerId
        choices = Dictionary(uniqueKeysWithValues: target.choices.choices.filter { $0.ownerId == composerOwner }.map { ($0.runtimeId, $0.choice) })
        restoringSelection = true; selection = choices[runtimeID]?.model; restoringSelection = false
        applyingComposer = false; onComposerChange?(self)
    }
    private func composerChanged() {
        guard !applyingComposer else { return }
        contentGeneration &+= 1
        if !composerDirty { composerBaseline = sharedInput?.revision }
        composerDirty = true
        if sharedInput == nil { sharedInput = .init(id: UUID().uuidString.lowercased(), revision: 0, content: composerContent); composerUnbound = true; composerBindingBaseline = composerBindingRevision }
        onComposerChange?(self)
        composerSaveTask?.cancel()
        composerSaveTask = Task { [weak self] in
            do {
                try await Task.sleep(for: .milliseconds(250))
                try await self?.flushComposer()
            } catch is CancellationError {} catch { self?.composerFailure = self?.composerError(error); self?.report(error, source: "同步输入") }
        }
    }
    private func receiveComposer(_ input: ComposerInput) {
        applyingComposer = true
        sharedInput = input
        draft = input.content.text; inputAttachments = input.content.attachments
        applyingComposer = false
        onComposerChange?(self)
    }
    private func flushComposer() async throws {
        guard peer != "preview", composerLoaded, !composerSaving else { return }
        composerSaving = true
        defer { composerSaving = false }
        while composerDirty, let input = sharedInput {
            let content = composerContent, baseline = composerBaseline
            let update = try await client.application(peer, operation: .conversationWrite) {
                try $0.manageComposerInputs(operation: .save(inputId: input.id, expectedRevision: baseline == 0 ? nil : baseline, content: content))
            }
            guard let saved = update.state?.inputs.first(where: { $0.id == input.id }) else { throw MobileError.unavailable }
            sharedInput = saved; composerBaseline = saved.revision
            if composerContent == content { composerDirty = false; composerBaseline = nil }
            composerFailure = nil; onComposerChange?(self)
        }
        while targetDirty, let context = composerTargetContext {
            let content = composerTargetContent, baseline = targetBaseline
            let update: ComposerInputUpdate
            do { update = try await client.application(peer, operation: .conversationWrite) { try $0.manageComposerInputs(operation: .saveTarget(context: context, expectedRevision: baseline, choices: content)) } }
            catch { targetWriteFailed = true; throw error }
            targetWriteFailed = false
            guard let saved = update.state?.targets.first(where: { $0.context == context }) else { throw MobileError.unavailable }
            sharedTarget = saved; observedTargetRevision = saved.revision; targetBaseline = saved.revision
            if composerTargetContent == content { targetDirty = false; targetBaseline = nil }
            composerFailure = nil; onComposerChange?(self)
        }
        if composerUnbound, let input = sharedInput, let owner = composerOwner {
            let context: ComposerContextKey = (snapshot?.conversation.id ?? target.conversation).map { .conversation(ownerId: owner, conversationId: $0) } ?? .newConversation
            let baseline = composerBindingBaseline
            _ = try await client.application(peer, operation: .conversationWrite) { try $0.manageComposerInputs(operation: .bind(context: context, inputId: input.id, expectedRevision: baseline)) }
            composerUnbound = false; onComposerChange?(self)
        }
    }
    private func completeCreation() async throws {
        guard let creation = pendingCreation else { return }
        guard creation.ownerID == composerOwner else { throw MobileError.unavailable }
        if snapshot == nil {
            let result = try await client.application(peer, operation: .conversationRead) { try $0.readConversation(runtimeId: creation.runtimeID, conversationId: creation.conversationID) }
            apply(result)
        }
        if let choices = creation.choices {
            _ = try await client.application(peer, operation: .conversationWrite) { try $0.manageComposerInputs(operation: .ensureTarget(context: .conversation(ownerId: creation.ownerID, conversationId: creation.conversationID), seed: choices)) }
        }
        _ = try await client.application(peer, operation: .conversationWrite) { try $0.manageComposerInputs(operation: .promote(inputId: creation.inputID, ownerId: creation.ownerID, conversationId: creation.conversationID, runtimeId: creation.runtimeID)) }
        pendingCreation = nil; onComposerChange?(self)
    }
    private func pollComposer() async {
        guard peer != "preview", !busy, !composerSaving, !composerPolling else { return }
        composerPolling = true
        defer { composerPolling = false }
        do {
            if composerOwner == nil { composerOwner = try await client.application(peer, operation: .conversationRead) { try $0.composerOwnerId() } }
            guard let owner = composerOwner else { return }
            try await completeCreation()
            var context: ComposerContextKey = (snapshot?.conversation.id ?? target.conversation).map { .conversation(ownerId: owner, conversationId: $0) } ?? .newConversation
            let baseline = composerDirectoryRevision ?? 0
            let update = try await client.application(peer, operation: .conversationRead) {
                try $0.manageComposerInputs(operation: .read(sinceRevision: self.composerDirectoryRevision))
            }
            if let state = update.state {
                if isNew, let input = sharedInput,
                   let creation = state.creations.first(where: { creation in creation.inputId == input.id && creation.ownerId == owner && creation.revision > baseline && state.bindings.contains { $0.inputId == input.id && $0.context == .conversation(ownerId: owner, conversationId: creation.conversationId) } }) {
                    composerLoaded = false
                    let created = try await client.application(peer, operation: .conversationRead) { try $0.readConversation(runtimeId: creation.runtimeId, conversationId: creation.conversationId) }
                    apply(created)
                    context = .conversation(ownerId: owner, conversationId: creation.conversationId)
                }
                var resolved = state
                if !state.bindings.contains(where: { $0.context == context }) {
                    let seed = ComposerInputContent(text: "", attachments: [])
                    let created = try await client.application(peer, operation: .conversationWrite) {
                        try $0.manageComposerInputs(operation: .ensure(context: context, inputId: UUID().uuidString.lowercased(), seed: seed))
                    }
                    guard let state = created.state else { throw MobileError.unavailable }; resolved = state
                }
                guard isNew || snapshot != nil else { return }
                if !resolved.targets.contains(where: { $0.context == context }) || resolved.targets.first(where: { $0.context == context })?.choices.runtimeId == nil {
                    let seed = ComposerTargetChoices(runtimeId: snapshot?.contextRuntimeId ?? destination.runtime, runtimeOwnerId: owner, choices: [], choiceEdited: false)
                    let created = try await client.application(peer, operation: .conversationWrite) { try $0.manageComposerInputs(operation: .ensureTarget(context: context, seed: seed)) }
                    guard let state = created.state else { throw MobileError.unavailable }; resolved = state
                }
                if let target = resolved.targets.first(where: { $0.context == context }) {
                    observedTargetRevision = target.revision
                    if targetDirty && target.choices == composerTargetContent { targetDirty = false; targetBaseline = nil; targetWriteFailed = false }
                    if !targetDirty { receiveTarget(target) }
                }
                composerDirectoryRevision = resolved.revision
                let binding = resolved.bindings.first { $0.context == context }
                composerBindingRevision = binding?.revision
                if composerUnbound, binding?.inputId == sharedInput?.id { composerUnbound = false; composerBindingBaseline = nil }
                else if composerUnbound, !composerRestored, sharedInput?.revision == 0 { composerBindingBaseline = binding?.revision }
                if !composerDirty, !composerUnbound, let id = resolved.bindings.first(where: { $0.context == context })?.inputId, let input = resolved.inputs.first(where: { $0.id == id }) { receiveComposer(input) }
            }
            composerLoaded = true
            try await flushComposer()
            composerFailure = nil
        } catch { composerFailure = composerError(error); report(error, source: "接续输入") }
    }
    func importLegacyInput() { if let legacyInput { importLegacyInput(legacyInput) } }
    func importLegacyInput(_ content: ComposerInputContent, target importedTarget: ComposerTargetChoices? = nil) {
        guard let owner = composerOwner else { return }
        receiveComposer(.init(id: UUID().uuidString.lowercased(), revision: 0, content: content))
        if var target = importedTarget ?? legacyTarget {
            target.runtimeOwnerId = target.runtimeId == nil ? nil : owner
            target.choices = target.choices.map { .init(ownerId: owner, runtimeId: $0.runtimeId, choice: $0.choice) }
            let context = composerTargetContext ?? .newConversation
            receiveTarget(.init(context: context, revision: sharedTarget?.revision ?? 0, choices: target)); targetChanged()
        }
        self.legacyInput = nil; legacyTarget = nil; composerUnbound = true; composerBindingBaseline = composerBindingRevision
        composerChanged()
        Task { await pollComposer() }
    }
    func removeInputAttachment(_ id: String) { inputAttachments.removeAll { $0.id == id }; composerChanged() }
}
