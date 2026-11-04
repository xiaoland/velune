import SwiftUI
import AppKit
import VeluneBindings

/// The same text and matching rules drive result navigation and native highlights.
enum TranscriptSearch {
    static func ranges(in text: String, query: String) -> [NSRange] {
        guard !query.isEmpty else { return [] }
        let source = text as NSString
        var result: [NSRange] = [], offset = 0
        while offset < source.length {
            let range = source.range(of: query, options: [.caseInsensitive, .diacriticInsensitive, .widthInsensitive], range: NSRange(location: offset, length: source.length - offset), locale: .current)
            guard range.location != NSNotFound, range.length > 0 else { break }
            result.append(range)
            offset = NSMaxRange(range)
        }
        return result
    }

    static func text(of block: DeliveredBlock) -> [String] {
        switch block {
        case .text(let text), .notice(let text), .contextCompaction(let text): return [text]
        case .runtimeFailure(let failure, let detail, _): return [failure.title, detail]
        case .reasoning(let summary, let payload):
            if case .inline(let detail) = payload { return [summary, detail] }
            return [summary]
        case .tool(_, let title, _, let output, _):
            if case .inline(let detail)? = output { return [title, detail] }
            return [title]
        case .agentSender(_, let runtime, let title): return [runtime, title]
        case .file, .image: return []
        }
    }

    static func matches(_ message: DeliveredMessage, query: String) -> Bool {
        message.blocks.contains { text(of: $0).contains { !ranges(in: $0, query: query).isEmpty } }
    }

    static func disclosureKeys(in message: DeliveredMessage, query: String) -> Set<String> {
        Set(message.blocks.enumerated().compactMap { index, block in
            guard text(of: block).contains(where: { !ranges(in: $0, query: query).isEmpty }) else { return nil }
            return RenderedMessage.disclosureKey(block, at: index)
        })
    }
}

/// A tab's search through its conversation's messages: the bar's state,
/// kept with the tab so switching away and back finds it as left.
@MainActor final class TranscriptFind: ObservableObject {
    @Published var isShown = false
    @Published var query = ""
    /// The match shown, among the matches in the conversation's order.
    @Published var index = 0
    /// The field takes the focus when this changes.
    @Published var focusRequest = 0

    func matches(in model: TranscriptModel) -> [String] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return [] }
        return model.items(for: model.rows, turnHeaders: false).flatMap { item -> [TranscriptRow] in
            switch item { case .message(let row): return [row]; case .work(_, let rows): return rows }
        }.compactMap { TranscriptSearch.matches($0.message, query: query) ? $0.id : nil }
    }
}

/// Shows the bar while the tab is searching; its own view, since the tab
/// does not publish the search's state.
struct TranscriptFindHost: View {
    @ObservedObject var find: TranscriptFind
    @ObservedObject var model: TranscriptModel
    let jump: (String) -> Void

    var body: some View {
        if find.isShown { TranscriptFindBar(find: find, model: model, jump: jump).padding(8) }
    }
}

/// The find bar searches messages and work, including disclosure details.
/// Return goes to the next matching message, Shift-Return to the previous
/// one, Escape closes the bar. The native list reveals each chosen result.
struct TranscriptFindBar: View {
    @ObservedObject var find: TranscriptFind
    @ObservedObject var model: TranscriptModel
    /// Scrolls the list to the message of this ID.
    let jump: (String) -> Void
    @FocusState private var focused: Bool

    private var matches: [String] { find.matches(in: model) }

    var body: some View {
        let matches = matches
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("在会话中查找", text: $find.query)
                .textFieldStyle(.plain).focused($focused).frame(width: 200)
                .onSubmit { step(NSEvent.modifierFlags.contains(.shift) ? -1 : 1, in: matches) }
            Text(status(matches)).uiFont(.callout).foregroundStyle(.secondary).monospacedDigit()
            Button { step(-1, in: matches) } label: { Image(systemName: "chevron.up") }
                .help("上一个").accessibilityLabel("上一个").disabled(matches.isEmpty)
            Button { step(1, in: matches) } label: { Image(systemName: "chevron.down") }
                .help("下一个").accessibilityLabel("下一个").disabled(matches.isEmpty)
            Button("完成") { find.isShown = false }
        }
        .buttonStyle(.borderless)
        .padding(.horizontal, 10).padding(.vertical, 6)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
        .background(ArrowCursorArea())
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.separator))
        .onExitCommand { find.isShown = false }
        .task(id: find.focusRequest) { focused = true }
        .onAppear { if !matches.isEmpty { jump(matches[min(find.index, matches.count - 1)]) } }
        // Deferred work can add results after the query has already changed.
        .onChange(of: matches) { previous, values in
            let selected = previous.indices.contains(find.index) ? previous[find.index] : nil
            find.index = selected.flatMap { values.firstIndex(of: $0) } ?? 0
            if !values.isEmpty { jump(values[find.index]) }
        }
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

/// The arrow over its bounds. A bar laid over the list would otherwise keep
/// the text cursor of the messages beneath it until clicked.
private struct ArrowCursorArea: NSViewRepresentable {
    final class CursorView: NSView {
        override func resetCursorRects() { addCursorRect(bounds, cursor: .arrow) }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        // The messages' text views take the cursor with tracking areas of their own, not only cursor rects.
        override func updateTrackingAreas() {
            super.updateTrackingAreas()
            for area in trackingAreas { removeTrackingArea(area) }
            addTrackingArea(NSTrackingArea(rect: .zero, options: [.cursorUpdate, .activeAlways, .inVisibleRect], owner: self))
        }
        override func cursorUpdate(with event: NSEvent) { NSCursor.arrow.set() }
    }
    func makeNSView(context: Context) -> CursorView { CursorView() }
    func updateNSView(_ view: CursorView, context: Context) { view.window?.invalidateCursorRects(for: view) }
}
