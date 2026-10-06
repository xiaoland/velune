import Combine
import Foundation
import MarkdownUI

/// Stable presentation objects derived from the authoritative conversation projection.
/// Markdown is parsed only when that message's content changes.
@MainActor
final class TranscriptRow: ObservableObject, @MainActor Identifiable {
    var id: String { message.id }
    @Published private(set) var message: Message
    private(set) var markdown: [Int: MarkdownContent] = [:]
    private(set) var parseCount = 0
    init(_ message: Message) { self.message = message; parse(message) }
    func update(_ message: Message) {
        guard self.message != message else { return }
        if self.message.blocks != message.blocks { parse(message) }
        self.message = message
    }
    private func parse(_ message: Message) {
        var next: [Int: MarkdownContent] = [:]
        for (index, block) in message.blocks.enumerated() {
            guard case .text(let text) = block, message.role == .assistant else { continue }
            if index < self.message.blocks.count, self.message.blocks[index] == block, let cached = markdown[index] { next[index] = cached }
            else { next[index] = MarkdownContent(text); parseCount += 1 }
        }
        markdown = next
    }
}

@MainActor
final class TranscriptModel: ObservableObject {
    @Published private(set) var rows: [TranscriptRow] = []
    @Published private(set) var turns: [TranscriptTurn] = []
    @Published private(set) var contentRevision: UInt64 = 0
    private(set) var confirmedMessageIDs: [String: String] = [:]
    private(set) var confirmedTurnIDs: [String: String] = [:]
    var userRows: [TranscriptRow] { rows.filter { $0.message.role == .user } }
    enum Item: @MainActor Identifiable {
        case message(TranscriptRow)
        case work(TranscriptTurn, [TranscriptRow])
        @MainActor var id: String { switch self { case .message(let row): return "message:" + row.id; case .work(let turn, _): return "work:" + turn.id } }
    }
    func items(for visibleRows: [TranscriptRow]) -> [Item] {
        let byID = Dictionary(uniqueKeysWithValues: visibleRows.map { ($0.id, $0) })
        let workMembership = Dictionary(uniqueKeysWithValues: turns.flatMap { turn in turn.workMessageIDs.map { ($0, turn) } })
        var emitted: Set<String> = []
        return visibleRows.compactMap { row in
            guard let turn = workMembership[row.id] else { return .message(row) }
            guard emitted.insert(turn.id).inserted else { return nil }
            return .work(turn, turn.workMessageIDs.compactMap { byID[$0] })
        }
    }
    func retainedExpandedTurnIDs(_ ids: Set<String>) -> Set<String> {
        Set(ids.map { confirmedTurnIDs[$0] ?? $0 }).intersection(turns.map(\.id))
    }
    func reset() { rows = []; turns = []; confirmedMessageIDs = [:]; confirmedTurnIDs = [:]; contentRevision &+= 1 }
    func apply(_ messages: [Message], turns: [TranscriptTurn] = [], confirmations: [MessageIdentityConfirmation] = []) {
        confirmedMessageIDs = Dictionary(uniqueKeysWithValues: confirmations.map { ($0.previousID, $0.currentID) })
        confirmedTurnIDs = confirmedTurnIDs.filter { _, currentID in turns.contains { $0.id == currentID } }
        let newTurnConfirmations = self.turns.compactMap { previous -> (String, String)? in
            guard let currentUser = confirmedMessageIDs[previous.userMessageID],
                  let current = turns.first(where: { $0.userMessageID == currentUser }), current.id != previous.id else { return nil }
            return (previous.id, current.id)
        }
        for (previous, current) in newTurnConfirmations { confirmedTurnIDs[previous] = current }
        var existing = Dictionary(uniqueKeysWithValues: rows.map { ($0.id, $0) })
        for confirmation in confirmations {
            if let row = existing.removeValue(forKey: confirmation.previousID) { existing[confirmation.currentID] = row }
        }
        var next: [TranscriptRow] = []
        var changed = self.turns != turns
        if changed { self.turns = turns }
        for message in messages {
            if let row = existing[message.id] {
                if row.message != message { row.update(message); changed = true }
                next.append(row)
            } else { next.append(TranscriptRow(message)); changed = true }
        }
        if rows.count != next.count || zip(rows, next).contains(where: { $0 !== $1 }) { rows = next; changed = true }
        if changed { contentRevision &+= 1 }
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
    mutating func confirmIdentities(_ confirmations: [String: String]) {
        if let id = expandedFromUserID { expandedFromUserID = confirmations[id] ?? id }
        if let id = sentAfterUserID { sentAfterUserID = confirmations[id] ?? id }
    }
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
    @MainActor func visibleRows(_ rows: [TranscriptRow]) -> [TranscriptRow] {
        guard let id = expandedFromUserID, let index = rows.firstIndex(where: { $0.id == id && $0.message.role == .user }) else { return rows.filter { $0.message.role == .user } }
        return rows.enumerated().compactMap { offset, row in offset >= index || row.message.role == .user ? row : nil }
    }
}
