import SwiftUI

struct ConversationTarget: Hashable, Codable {
    var key: String { runtime + ":" + (conversation ?? draftID.uuidString) }
    static func == (lhs: Self, rhs: Self) -> Bool { lhs.key == rhs.key }
    func hash(into hasher: inout Hasher) { hasher.combine(key) }
    var runtime: String
    var conversation: String?
    var title: String
    var cwd: String?
    // New drafts have their own identity even when created in the same project.
    var draftID = UUID()
    init(_ listed: BindingListedConversation) {
        self.init(listed.conversation)
    }
    init(_ summary: BindingConversationSummary) {
        runtime = summary.runtimeId
        conversation = summary.id
        title = summary.title
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
    @Published var snapshot: BindingConversationSnapshot?
    @Published var messages: [String: BindingMessage] = [:]
    @Published var models: [BindingRuntimeModel] = []
    @Published var selection: BindingModelSelection? {
        didSet {
            guard !restoringSelection else { return }
            choices[runtimeID] = .init(model: selection, effort: nil, permissionMode: nil)
            onComposerChange?(self)
        }
    }
    @Published private(set) var destination: DraftDestination
    @Published private(set) var loadingModels = false
    private var choices: [String: BindingTurnChoice] = [:]
    private var restoringSelection = false
    @Published var busy = false
    @Published var loading = true
    @Published var failure: String?
    @Published var draft = "" { didSet { onComposerChange?(self) } }
    @Published var sending: String?
    var onComposerChange: ((ConversationModel) -> Void)?
    var onIdentityChange: ((ConversationModel) -> Void)?
    var problemContext = ""
    var following = true
    var readingOffset: CGFloat = 0
    private var heldUnreadCursor: BindingTranscriptCursor?
    private var markingRead = false
    private var payloads: [String: BindingMessage] = [:]

    init(client: RemoteClient, peer: String, target: ConversationTarget) {
        self.client = client; self.peer = peer; self.target = target
        destination = .init(runtime: target.runtime, cwd: target.cwd)
    }

    var runtimeID: String { snapshot?.contextRuntimeId ?? destination.runtime }
    var title: String {
        if snapshot?.conversation.parentConversationId != nil { return target.title }
        return snapshot?.conversation.title ?? target.title
    }
    var currentTarget: ConversationTarget {
        var result = snapshot.map { ConversationTarget($0.conversation) } ?? target
        result.title = title
        return result
    }
    var cwd: String? { snapshot.map { $0.conversation.cwd } ?? destination.cwd }
    var isNew: Bool { snapshot == nil && target.conversation == nil }
    var composerState: ComposerState {
        .init(text: draft, selection: selection, following: following, offset: readingOffset, destination: destination, choices: choices)
    }
    func restore(_ state: ComposerState) {
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
        guard isNew, !busy, runtime != runtimeID else { return }
        // The draft's identity stays fixed while its sending destination changes.
        destination.runtime = runtime
        restoringSelection = true
        selection = choices[runtime]?.model
        restoringSelection = false
        models = []
        onComposerChange?(self)
    }
    func loadModels() async {
        let runtime = runtimeID
        guard !runtime.isEmpty else { return }
        loadingModels = true
        defer { if runtimeID == runtime && !Task.isCancelled { loadingModels = false } }
        let needsDefault = isNew && choices[runtime] == nil
        do {
            let result = try await client.application(peer) {
                (try $0.runtimeModels(runtimeId: runtime), needsDefault ? try $0.composerChoices() : nil)
            }
            guard runtimeID == runtime, !Task.isCancelled else { return }
            models = result.0
            if needsDefault, choices[runtime] == nil {
                if let defaults = result.1?.runtimes.first(where: { $0.runtimeId == runtime }) {
                    switch defaults.newConversation {
                    case .lastChosen: selection = defaults.lastChosen.model
                    case .chosen(let choice): selection = choice.model
                    }
                } else { selection = nil }
            }
        } catch {
            guard runtimeID == runtime, !Task.isCancelled else { return }
            report(error, source: "读取模型")
        }
    }
    var canSend: Bool {
        !loading && !loadingModels && !busy && !runtimeID.isEmpty && (target.conversation == nil || snapshot != nil)
            && !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && snapshot?.actions.canSend != false
    }
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
            let marked = try await client.application(peer) { app -> Bool in
                guard !Task.isCancelled else { return false }
                _ = try app.list()
                guard let row = try app.findConversation(conversationId: shown.conversation.id),
                      row.conversation.runtimeId == shown.conversation.runtimeId else { return false }
                guard row.unread else { return false }
                // Core owns the list activity watermark; transcript timestamps are not equivalent.
                _ = try app.organizeConversations(conversations: [.init(runtimeId: shown.conversation.runtimeId, conversationId: shown.conversation.id)], change: .markRead)
                return true
            }
            return marked
        } catch { report(error, source: "标为已读"); return false }
    }

    func open() async {
        do {
            if let id = target.conversation {
                apply(try await client.application(peer) { try $0.readConversation(runtimeId: self.target.runtime, conversationId: id) })
            }
            failure = nil
        } catch { report(error, source: "读取会话") }
        loading = false
    }

    func poll() async {
        guard !busy else { return }
        guard let current = snapshot else {
            if target.conversation != nil { await open() }
            return
        }
        do {
            let update = try await client.application(peer) {
                try $0.snapshotUpdate(runtimeId: current.contextRuntimeId, conversationId: current.conversation.id, since: current.cursor)
            }
            switch update {
            case .changed(let next): merge(next)
            case .absent:
                apply(try await client.application(peer) { try $0.readConversation(runtimeId: current.contextRuntimeId, conversationId: current.conversation.id) })
            case .unchanged: break
            }
            failure = nil
        } catch { report(error, source: "更新会话") }
    }

    private func apply(_ result: BindingSnapshotResult) { if let next = result.snapshot { merge(next) } }

    func merge(_ next: BindingConversationSnapshot) {
        let fresh = Dictionary(uniqueKeysWithValues: next.messages.map { ($0.id, $0) })
        let ids = Set(next.messageIds)
        messages = next.base == nil ? fresh : messages.merging(fresh) { _, new in new }.filter { ids.contains($0.key) }
        payloads = payloads.filter { id, loaded in fresh[id].map { $0.stamp == loaded.stamp } ?? ids.contains(id) }
        let identityChanged = snapshot?.conversation.id != next.conversation.id
        snapshot = next
        if identityChanged { onIdentityChange?(self) }
        if choices[runtimeID] == nil {
            selection = next.modelRecordKey.map { .managed(modelRecordKey: $0) }
        }
    }

    func message(_ id: String) -> BindingMessage? { payloads[id] ?? messages[id] }

    func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard canSend else { return }
        let runtime = runtimeID, cwd = cwd, selection = selection
        act("发送消息") {
            self.sending = text
            defer { self.sending = nil }
            if self.snapshot == nil {
                guard self.target.conversation == nil else { throw MobileError.missingConversation }
                self.apply(try await self.client.application(self.peer) {
                    try $0.createConversationWithSelection(runtimeId: runtime, cwd: cwd, selection: selection)
                })
            }
            guard let current = self.snapshot else { throw MobileError.missingConversation }
            let turn = BindingComposedTurn(text: text, effort: nil, permissionMode: nil, attachments: [])
            self.apply(try await self.client.application(self.peer) {
                try $0.sendTurnWithSelection(sourceRuntimeId: current.contextRuntimeId, conversationId: current.conversation.id,
                    targetRuntimeId: runtime, selection: selection, turn: turn)
            })
            if self.draft.trimmingCharacters(in: .whitespacesAndNewlines) == text { self.draft = "" }
        }
    }

    func stop() { act("停止生成") {
        guard let current = self.snapshot else { return }
        self.apply(try await self.client.application(self.peer) { try $0.cancel(runtimeId: current.contextRuntimeId, conversationId: current.conversation.id) })
    } }

    func reply(_ id: String, _ reply: BindingRuntimeInteractionReply) { act("回复运行时请求") {
        guard let current = self.snapshot else { return }
        self.apply(try await self.client.application(self.peer) {
            try $0.replyRuntimeInteraction(runtimeId: current.contextRuntimeId, conversationId: current.conversation.id, interactionId: id, reply: reply)
        })
    } }

    func loadPayload(_ id: String) async {
        guard let current = snapshot else { return }
        do {
            let loaded = try await client.application(peer) {
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
