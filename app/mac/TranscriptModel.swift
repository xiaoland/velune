import Combine
import Foundation
import MarkdownView

/// Retains the public parser result across native cell reuse, without invoking a
/// parser for off-screen messages. MarkdownReader supplies the first result.
/// Parsing configuration is fixed; future parse-affecting options must invalidate this cache.
@MainActor
final class CachedMarkdownDocument {
    let source: String
    private(set) var result: MarkdownParseResult?
    private let parsed: () -> Void
    init(_ source: String, parsed: @escaping () -> Void) { self.source = source; self.parsed = parsed }
    func retain(_ result: MarkdownParseResult) {
        guard self.result == nil else { return }
        self.result = result
        parsed()
    }
}

/// Stable presentation objects derived from the authoritative conversation projection.
/// Parsed results survive native cell reuse until their source content changes.
@MainActor
final class TranscriptRow: ObservableObject, @MainActor Identifiable {
    let id: String
    @Published private(set) var message: Message
    private(set) var contentRevision: UInt64 = 0
    private(set) var markdown: [Int: CachedMarkdownDocument] = [:]
    private(set) var cacheFillCount = 0
    init(_ message: Message, id: String) { self.id = id; self.message = message; prepareMarkdown(message) }
    func update(_ message: Message) {
        guard self.message != message else { return }
        if self.message.blocks != message.blocks { prepareMarkdown(message) }
        contentRevision &+= 1
        self.message = message
    }
    private func prepareMarkdown(_ message: Message) {
        var next: [Int: CachedMarkdownDocument] = [:]
        for (index, block) in message.blocks.enumerated() {
            guard case .text(let text) = block, message.role == .assistant else { continue }
            if index < self.message.blocks.count, self.message.blocks[index] == block, let cached = markdown[index] { next[index] = cached }
            else { next[index] = CachedMarkdownDocument(text) { [weak self] in self?.cacheFillCount += 1 } }
        }
        markdown = next
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

/// UI visibility is anchored to message identity, never a cached position.
struct TranscriptOutlineState {
    private(set) var expandedFromUserID: String?
    private var followsSentUser = false
    private var sentAfterUserID: String?
    @MainActor mutating func expand(from id: String, in rows: [TranscriptRow]) -> Bool {
        guard rows.contains(where: { $0.id == id && $0.message.role == .user }) else { return false }
        expandedFromUserID = id; followsSentUser = false
        return true
    }
    mutating func collapse() { expandedFromUserID = nil; followsSentUser = false }
    @MainActor mutating func followSentUser(after id: String?, in rows: [TranscriptRow]) {
        followsSentUser = true; sentAfterUserID = id
        revealSentUser(in: rows)
    }
    @MainActor private mutating func revealSentUser(in rows: [TranscriptRow]) {
        guard followsSentUser, let latest = rows.last(where: { $0.message.role == .user }), latest.id != sentAfterUserID else { return }
        expandedFromUserID = latest.id; followsSentUser = false
    }
    @MainActor mutating func reconcile(_ rows: [TranscriptRow]) -> Bool {
        revealSentUser(in: rows)
        guard let id = expandedFromUserID, !rows.contains(where: { $0.id == id && $0.message.role == .user }) else { return false }
        expandedFromUserID = nil
        return true
    }
    @MainActor func visibleRows(_ rows: [TranscriptRow], userIDs: Set<String>) -> [TranscriptRow] {
        guard let id = expandedFromUserID, let index = rows.firstIndex(where: { $0.id == id && $0.message.role == .user }) else { return rows.filter { userIDs.contains($0.id) } }
        return rows.enumerated().compactMap { offset, row in offset >= index || userIDs.contains(row.id) ? row : nil }
    }
}
