import SwiftUI

/// A tab's search through its conversation's messages: the bar's state,
/// kept with the tab so switching away and back finds it as left.
@MainActor final class TranscriptFind: ObservableObject {
    @Published var isShown = false
    @Published var query = ""
    /// The match shown, among the matches in the conversation's order.
    @Published var index = 0
    /// The field takes the focus when this changes.
    @Published var focusRequest = 0
}

/// The find bar over the top of the messages. It looks through the
/// messages the list shows as messages (the user's and each turn's reply);
/// what is folded into a turn's work is not searched. Return goes to the
/// next match, Shift-Return to the previous one, Escape closes the bar.
struct TranscriptFindBar: View {
    @ObservedObject var find: TranscriptFind
    @ObservedObject var model: TranscriptModel
    /// Scrolls the list to the message of this ID.
    let jump: (String) -> Void
    @FocusState private var focused: Bool

    private var matches: [String] {
        let query = find.query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return [] }
        return model.items(for: model.rows, turnHeaders: false).compactMap { item in
            guard case .message(let row) = item, row.message.text.localizedStandardContains(query) else { return nil }
            return row.id
        }
    }

    var body: some View {
        let matches = matches
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("在会话中查找", text: $find.query)
                .textFieldStyle(.plain).focused($focused).frame(width: 200)
                .onSubmit { step(NSEvent.modifierFlags.contains(.shift) ? -1 : 1, in: matches) }
            Text(status(matches)).font(.callout).foregroundStyle(.secondary).monospacedDigit()
            Button { step(-1, in: matches) } label: { Image(systemName: "chevron.up") }
                .help("上一个").accessibilityLabel("上一个").disabled(matches.isEmpty)
            Button { step(1, in: matches) } label: { Image(systemName: "chevron.down") }
                .help("下一个").accessibilityLabel("下一个").disabled(matches.isEmpty)
            Button("完成") { find.isShown = false }
        }
        .buttonStyle(.borderless)
        .padding(.horizontal, 10).padding(.vertical, 6)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.separator))
        .onExitCommand { find.isShown = false }
        .task(id: find.focusRequest) { focused = true }
        // A new query starts at its first match.
        .onChange(of: find.query) { _, _ in
            find.index = 0
            if let first = self.matches.first { jump(first) }
        }
    }

    private func status(_ matches: [String]) -> String {
        if find.query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return "" }
        return matches.isEmpty ? "无结果" : "\(min(find.index, matches.count - 1) + 1)/\(matches.count)"
    }

    private func step(_ direction: Int, in matches: [String]) {
        guard !matches.isEmpty else { return }
        find.index = (min(find.index, matches.count - 1) + direction + matches.count) % matches.count
        jump(matches[find.index])
    }
}
