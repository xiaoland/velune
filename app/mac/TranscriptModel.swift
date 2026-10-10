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
    init(_ message: Message, id: String) { self.id = id; self.message = message }
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
    var rows: [TranscriptRow] { presentation.rows }
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
    func items(for visibleRows: [TranscriptRow]) -> [Item] {
        let visible = Set(visibleRows.map(\.id))
        let byMessage = Dictionary(uniqueKeysWithValues: rows.map { ($0.message.id, $0) })
        let byTurn = Dictionary(uniqueKeysWithValues: turns.map { ($0.id, $0) })
        return presentation.items.compactMap { item in
            switch item {
            case .message(let id, let messageID):
                guard visible.contains(id), let row = byMessage[messageID] else { return nil }
                return .message(row)
            case .work(let turnID):
                guard let turn = byTurn[turnID] else { return nil }
                let members = turn.workMessageIDs.compactMap { byMessage[$0] }.filter { visible.contains($0.id) }
                guard !members.isEmpty else { return nil }
                return .work(turn, members)
            }
        }
    }
    func retainedExpandedTurnIDs(_ ids: Set<String>) -> Set<String> { ids.intersection(turns.map(\.id)) }
    func reset() { presentation = Presentation(revision: contentRevision &+ 1) }
    func apply(_ messages: [Message], turns: [TranscriptTurn], items: [TranscriptItem], outline: [TranscriptOutlineEntry], identities: [TranscriptMessageIdentity]) {
        let identitiesByMessage = Dictionary(uniqueKeysWithValues: identities.map { ($0.messageID, $0.id) })
        let existing = Dictionary(uniqueKeysWithValues: rows.map { ($0.id, $0) })
        var changed = presentation.turns != turns || presentation.items != items || presentation.outline != outline
        let next = messages.map { message -> TranscriptRow in
            guard let id = identitiesByMessage[message.id] else { preconditionFailure("Missing transcript presentation identity") }
            if let row = existing[id] {
                if row.message != message { row.update(message); changed = true }
                return row
            }
            changed = true
            return TranscriptRow(message, id: id)
        }
        if rows.count != next.count || zip(rows, next).contains(where: { $0 !== $1 }) { changed = true }
        guard changed else { return }
        // Publish the complete ordered projection once; native confirmation only
        // changes canonical references, never the identity of an existing row.
        presentation = Presentation(rows: next, turns: turns, items: items, outline: outline, revision: contentRevision &+ 1)
    }

}
