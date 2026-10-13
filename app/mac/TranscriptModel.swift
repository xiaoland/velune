import Combine
import Foundation

/// Stable presentation identity derived from the authoritative conversation
/// projection. The revision changes only when the message content changes, so
/// native rendering and measurement caches survive unchanged polls.
@MainActor
final class TranscriptRow: @MainActor Identifiable {
    let id: String
    private(set) var message: Message
    private(set) var contentRevision: UInt64 = 0
    /// A caption under a message the runtime has not confirmed yet, such as "发送中".
    let status: String?
    init(_ message: Message, id: String, status: String? = nil) { self.id = id; self.message = message; self.status = status }
    func update(_ message: Message) {
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
    @Published private var pending: TranscriptRow?
    var rows: [TranscriptRow] { presentation.rows + (pending.map { [$0] } ?? []) }
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
        let headed = turnHeaders ? Dictionary(turns.filter { $0.settings != nil && !worked.contains($0.id) }.map { ($0.userMessageID, $0) }, uniquingKeysWith: { first, _ in first }) : [:]
        let byID = Dictionary(uniqueKeysWithValues: rows.map { ($0.id, $0) })
        // A message split into work and reply has two rows; work members use the first.
        let byMessage = Dictionary(rows.map { ($0.message.id, $0) }, uniquingKeysWith: { first, _ in first })
        let byTurn = Dictionary(uniqueKeysWithValues: turns.map { ($0.id, $0) })
        let pendingItem = pending.flatMap { visible.contains($0.id) ? Item.message($0) : nil }
        return presentation.items.flatMap { item -> [Item] in
            switch item {
            case .message(let id, let messageID):
                guard visible.contains(id), let row = byID[id] else { return [] }
                return [.message(row)] + (headed[messageID].map { [.work($0, [])] } ?? [])
            case .work(let turnID):
                guard let turn = byTurn[turnID] else { return [] }
                let members = turn.workMessageIDs.compactMap { byMessage[$0] }.filter { visible.contains($0.id) }
                guard !members.isEmpty else { return [] }
                return [.work(turn, members)]
            }
        } + (pendingItem.map { [$0] } ?? [])
    }
    /// Shows `message` at the end of the list with `status` under it; nil removes it.
    func showPending(_ message: Message?, status: String?) {
        guard let message else {
            if pending != nil { pending = nil; presentation.revision &+= 1 }
            return
        }
        guard pending?.message != message || pending?.status != status else { return }
        pending = TranscriptRow(message, id: message.id, status: status)
        presentation.revision &+= 1
    }
    func retainedExpandedTurnIDs(_ ids: Set<String>) -> Set<String> { ids.intersection(turns.map(\.id)) }
    func reset() { presentation = Presentation(revision: contentRevision &+ 1) }
    func apply(_ messages: [Message], turns: [TranscriptTurn], items: [TranscriptItem], outline: [TranscriptOutlineEntry], identities: [TranscriptMessageIdentity]) {
        let identitiesByMessage = Dictionary(uniqueKeysWithValues: identities.map { ($0.messageID, $0.id) })
        let existing = Dictionary(uniqueKeysWithValues: rows.map { ($0.id, $0) })
        var changed = presentation.turns != turns || presentation.items != items || presentation.outline != outline
        func row(_ id: String, _ message: Message) -> TranscriptRow {
            if let row = existing[id] {
                if row.message != message { row.update(message); changed = true }
                return row
            }
            changed = true
            return TranscriptRow(message, id: id)
        }
        // The closing message splits into its work blocks and its reply.
        let replyStarts = Dictionary(turns.compactMap { turn in turn.lastMessageID.map { ($0, turn.replyStartBlock) } }, uniquingKeysWith: { first, _ in first })
        let next = messages.flatMap { message -> [TranscriptRow] in
            guard let id = identitiesByMessage[message.id] else { preconditionFailure("Missing transcript presentation identity") }
            guard let start = replyStarts[message.id], start > 0, start < message.blocks.count else { return [row(id, message)] }
            var work = message, reply = message
            work.blocks = Array(message.blocks[..<start]); reply.blocks = Array(message.blocks[start...])
            return [row(id, work), row("reply:" + id, reply)]
        }
        if rows.count != next.count || zip(rows, next).contains(where: { $0 !== $1 }) { changed = true }
        guard changed else { return }
        // Publish the complete ordered projection once; native confirmation only
        // changes canonical references, never the identity of an existing row.
        presentation = Presentation(rows: next, turns: turns, items: items, outline: outline, revision: contentRevision &+ 1)
    }

}
