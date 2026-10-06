import Combine
import Foundation
import MarkdownUI

/// Stable presentation objects derived from the authoritative conversation projection.
/// Markdown is parsed only when that message's content changes.
@MainActor
final class TranscriptRow: ObservableObject, Identifiable {
    let id: String
    @Published private(set) var message: Message
    private(set) var markdown: [Int: MarkdownContent] = [:]
    private(set) var parseCount = 0
    init(_ message: Message) { id = message.id; self.message = message; parse(message) }
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
    @Published private(set) var contentRevision: UInt64 = 0
    func reset() { rows = []; contentRevision &+= 1 }
    func apply(_ messages: [Message]) {
        let existing = Dictionary(uniqueKeysWithValues: rows.map { ($0.id, $0) })
        var next: [TranscriptRow] = []
        var changed = false
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
