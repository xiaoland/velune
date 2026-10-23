import SwiftUI

struct ConversationTarget: Hashable {
    var runtime: String
    var conversation: String?
    var title: String
    var cwd: String?
    // New drafts have their own identity even when created in the same project.
    var draftID = UUID()
    init(_ listed: BindingListedConversation) {
        runtime = listed.conversation.runtimeId
        conversation = listed.conversation.id
        title = listed.conversation.title
        cwd = listed.conversation.cwd
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
    @Published var selection: BindingModelSelection?
    @Published var busy = false
    @Published var loading = true
    @Published var failure: String?
    @Published var draft = ""
    @Published var sending: String?
    private var payloads: [String: BindingMessage] = [:]

    init(client: RemoteClient, peer: String, target: ConversationTarget) {
        self.client = client; self.peer = peer; self.target = target
    }

    func open() async {
        do {
            if let id = target.conversation {
                apply(try await client.application(peer) { try $0.readConversation(runtimeId: self.target.runtime, conversationId: id) })
            }
            models = try await client.application(peer) { try $0.runtimeModels(runtimeId: self.target.runtime) }
            failure = nil
        } catch { failure = mobileError(error) }
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
        } catch { failure = mobileError(error) }
    }

    private func apply(_ result: BindingSnapshotResult) { if let next = result.snapshot { merge(next) } }

    func merge(_ next: BindingConversationSnapshot) {
        let fresh = Dictionary(uniqueKeysWithValues: next.messages.map { ($0.id, $0) })
        let ids = Set(next.messageIds)
        messages = next.base == nil ? fresh : messages.merging(fresh) { _, new in new }.filter { ids.contains($0.key) }
        payloads = payloads.filter { id, loaded in fresh[id].map { $0.stamp == loaded.stamp } ?? ids.contains(id) }
        snapshot = next
        if selection == nil, let key = next.modelRecordKey { selection = .managed(modelRecordKey: key) }
    }

    func message(_ id: String) -> BindingMessage? { payloads[id] ?? messages[id] }

    func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        act {
            self.sending = text
            defer { self.sending = nil }
            let selection = self.selection
            if self.snapshot == nil {
                guard self.target.conversation == nil else { throw MobileError.missingConversation }
                self.apply(try await self.client.application(self.peer) {
                    try $0.createConversationWithSelection(runtimeId: self.target.runtime, cwd: self.target.cwd, selection: selection)
                })
            }
            guard let current = self.snapshot else { throw MobileError.missingConversation }
            let turn = BindingComposedTurn(text: text, effort: nil, permissionMode: nil, attachments: [])
            self.apply(try await self.client.application(self.peer) {
                try $0.sendTurnWithSelection(sourceRuntimeId: current.contextRuntimeId, conversationId: current.conversation.id,
                    targetRuntimeId: current.contextRuntimeId, selection: selection, turn: turn)
            })
            self.draft = ""
        }
    }

    func stop() { act {
        guard let current = self.snapshot else { return }
        self.apply(try await self.client.application(self.peer) { try $0.cancel(runtimeId: current.contextRuntimeId, conversationId: current.conversation.id) })
    } }

    func reply(_ id: String, _ reply: BindingRuntimeInteractionReply) { act {
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
        } catch { failure = mobileError(error) }
    }

    private func act(_ body: @escaping () async throws -> Void) {
        guard !busy else { return }
        busy = true; failure = nil
        Task {
            defer { busy = false }
            do { try await body() } catch { failure = mobileError(error) }
        }
    }
}
