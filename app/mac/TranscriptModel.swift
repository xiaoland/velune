import Combine
import VeluneBindings
import Foundation

/// Stable presentation identity derived from the authoritative conversation
/// projection. The revision changes only when the message content changes, so
/// native rendering and measurement caches survive unchanged polls.
@MainActor
final class TranscriptRow: @MainActor Identifiable {
    let id: String
    private(set) var message: DeliveredMessage
    private(set) var contentRevision: UInt64 = 0
    /// A caption under a message the runtime has not confirmed yet, such as "发送中".
    let status: String?
    /// Whether a copy button goes under it: the user's messages and a turn's
    /// closing reply, not the messages and text of its work.
    fileprivate(set) var copies: Bool
    /// A turn's closing reply forks the conversation from there: the turn's
    /// user message ID; nil shows no fork button.
    fileprivate(set) var fork: String?
    /// The user message may be edited now; shows an edit button beside copy.
    fileprivate(set) var edit: Bool
    init(_ message: DeliveredMessage, id: String, status: String? = nil, copies: Bool? = nil, fork: String? = nil, edit: Bool = false) {
        self.id = id; self.message = message; self.status = status
        self.copies = copies ?? (message.role == .user)
        self.fork = fork
        self.edit = edit
    }
    func update(_ message: DeliveredMessage) {
        guard self.message != message else { return }
        contentRevision &+= 1
        self.message = message
    }
}

@MainActor
final class TranscriptModel: ObservableObject {
    private struct Presentation {
        var rows: [TranscriptRow] = []
        var turns: [TranscriptTurn] = []
        var items: [TranscriptItem] = []
        var outline: [TranscriptOutlineEntry] = []
        var revision: UInt64 = 0
    }
    @Published private var presentation = Presentation()
    /// The message just sent, shown after the projection until the runtime's
    /// own copy appears in it.
    @Published private var pending: [TranscriptRow] = []
    var rows: [TranscriptRow] { presentation.rows + pending }
    var turns: [TranscriptTurn] { presentation.turns }
    var contentRevision: UInt64 { presentation.revision }
    var userRows: [TranscriptRow] {
        let byID = Dictionary(uniqueKeysWithValues: rows.map { ($0.id, $0) })
        return presentation.outline.compactMap { byID[$0.id] }
    }
    enum Item: @MainActor Identifiable {
        case message(TranscriptRow)
        case work(TranscriptTurn, [TranscriptRow])
        @MainActor var id: String { switch self { case .message(let row): return row.id; case .work(let turn, _): return "work:" + turn.id } }
    }
    /// `turnHeaders` adds a header (with no work under it) after the user
    /// message of each turn that has settings to show but no work of its own.
    func items(for visibleRows: [TranscriptRow], turnHeaders: Bool) -> [Item] {
        let visible = Set(visibleRows.map(\.id))
        let worked = Set(presentation.items.compactMap { item -> String? in if case .work(let id) = item { return id }; return nil })
        let headed = turnHeaders ? Dictionary(turns.filter { ($0.settings != nil || $0.usage != nil) && !worked.contains($0.id) }.map { ($0.userMessageId, $0) }, uniquingKeysWith: { first, _ in first }) : [:]
        let byID = Dictionary(uniqueKeysWithValues: rows.map { ($0.id, $0) })
        // A message split into work and reply has two rows; work members use the first.
        let byMessage = Dictionary(rows.map { ($0.message.id, $0) }, uniquingKeysWith: { first, _ in first })
        let byTurn = Dictionary(uniqueKeysWithValues: turns.map { ($0.id, $0) })
        let pendingItems = pending.filter { visible.contains($0.id) }.map(Item.message)
        return presentation.items.flatMap { item -> [Item] in
            switch item {
            case .message(let id, let messageID):
                guard visible.contains(id), let row = byID[id] else { return [] }
                return [.message(row)] + (headed[messageID].map { [.work($0, [])] } ?? [])
            case .work(let turnID):
                guard let turn = byTurn[turnID] else { return [] }
                let members = turn.workMessageIds.compactMap { byMessage[$0] }.filter { visible.contains($0.id) }
                guard !members.isEmpty else { return [] }
                return [.work(turn, members)]
            }
        } + pendingItems
    }
    func showSubmissions(_ messages: [SubmittedMessage]) {
        guard pending.map({ SubmittedMessage(message: $0.message, status: $0.status) }) != messages else { return }
        pending = messages.map { TranscriptRow($0.message, id: $0.message.id, status: $0.status) }
        presentation.revision &+= 1
    }
    func retainedExpandedTurnIDs(_ ids: Set<String>) -> Set<String> { ids.intersection(turns.map(\.id)) }
    func reset() { pending = []; presentation = Presentation(revision: contentRevision &+ 1) }
    /// Turns carry their own `canFork`: a fork may start from each completed
    /// turn's end, even while a later turn runs; `editable` puts an edit
    /// button under those user messages.
    func apply(_ messages: [DeliveredMessage], turns: [TranscriptTurn], items: [TranscriptItem], outline: [TranscriptOutlineEntry], identities: [TranscriptMessageIdentity], editable: Set<String>) {
        let identitiesByMessage = Dictionary(uniqueKeysWithValues: identities.map { ($0.messageId, $0.id) })
        let existing = Dictionary(uniqueKeysWithValues: rows.map { ($0.id, $0) })
        var changed = presentation.turns != turns || presentation.items != items || presentation.outline != outline
        func row(_ id: String, _ message: DeliveredMessage, copies: Bool, fork: String? = nil) -> TranscriptRow {
            let edit = editable.contains(message.id)
            if let row = existing[id] {
                if row.message != message { row.update(message); changed = true }
                // A running turn's latest message is its reply until more work follows it.
                if row.copies != copies { row.copies = copies; changed = true }
                if row.fork != fork { row.fork = fork; changed = true }
                if row.edit != edit { row.edit = edit; changed = true }
                return row
            }
            changed = true
            return TranscriptRow(message, id: id, copies: copies, fork: fork, edit: edit)
        }
        // The closing message splits into its work blocks and its reply.
        let replyStarts = Dictionary(turns.compactMap { turn in turn.lastMessageId.map { ($0, Int(turn.replyStartBlock)) } }, uniquingKeysWith: { first, _ in first })
        let forkTurns = Dictionary(turns.compactMap { turn in turn.canFork ? turn.lastMessageId.map { ($0, turn.userMessageId) } : nil }, uniquingKeysWith: { first, _ in first })
        let next = messages.flatMap { message -> [TranscriptRow] in
            guard let id = identitiesByMessage[message.id] else { preconditionFailure("Missing transcript presentation identity") }
            let user = message.role == .user || message.role == .client
            let fork = forkTurns[message.id]
            // Only a turn's closing message replies; a reply from its first block is the whole message.
            guard let start = replyStarts[message.id] else { return [row(id, message, copies: user)] }
            guard start > 0, start < message.blocks.count else { return [row(id, message, copies: user || start == 0, fork: start == 0 ? fork : nil)] }
            var work = message, reply = message
            work.blocks = Array(message.blocks[..<start]); reply.blocks = Array(message.blocks[start...])
            return [row(id, work, copies: false), row("reply:" + id, reply, copies: true, fork: fork)]
        }
        if rows.count != next.count || zip(rows, next).contains(where: { $0 !== $1 }) { changed = true }
        guard changed else { return }
        // Publish the complete ordered projection once; native confirmation only
        // changes canonical references, never the identity of an existing row.
        presentation = Presentation(rows: next, turns: turns, items: items, outline: outline, revision: contentRevision &+ 1)
    }

}
