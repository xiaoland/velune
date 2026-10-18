import SwiftUI

/// The outline: user messages of the loaded conversation. A click selects,
/// a double click (or Return, in the popover) goes to the message.
struct TranscriptOutlineView: View {
    @ObservedObject var model: TranscriptModel
    /// In a popover it has its own size and takes Return; docked or floating
    /// it fills its place and leaves Return to the composer.
    var inPopover = true
    /// In the sidebar it takes the sidebar's look.
    var inSidebar = false
    /// The window whose outline it is: right-clicked where no message is, it offers the outline's places.
    var layout: ConversationWindowModel?
    /// Edits the user message of this ID, when it may be edited now.
    var editMessage: (String) -> (() -> Void)? = { _ in nil }
    /// The conversation is still running: the last message's answer is
    /// being written below it, so going to it means going to the end.
    var isRunning = false
    var goToBottom: () -> Void = {}
    let navigate: (String) -> Void
    @State private var selectedID: String?
    var body: some View {
        VStack(spacing: 0) {
            if inSidebar { list.listStyle(.sidebar) } else { list.listStyle(.inset) }
            Divider()
            HStack {
                Text("\(model.userRows.count) 条用户消息").font(.callout).foregroundStyle(.secondary)
                Spacer()
                Button("前往") { if let selectedID { go(to: selectedID) } }
                    .keyboardShortcut(inPopover ? .defaultAction : nil).disabled(selectedID == nil)
            }.padding(10)
        }
        .frame(width: inPopover ? 360 : nil, height: inPopover ? 380 : nil)
        .onChange(of: model.contentRevision) { _, _ in
            if let selectedID, !model.userRows.contains(where: { $0.id == selectedID }) { self.selectedID = nil }
        }
    }
    /// Opens at the end, where the conversation is.
    private var list: some View {
        ScrollViewReader { proxy in
            List(selection: $selectedID) {
                ForEach(model.userRows) { row in
                    Text(row.message.text.isEmpty ? "用户消息" : row.message.text)
                        .lineLimit(3).frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 3).tag(row.id)
                }
            }
            .onAppear { scrollToEnd(proxy) }
            .onChange(of: model.userRows.count) { _, _ in scrollToEnd(proxy) }
            .accessibilityLabel("会话大纲")
            .contextMenu(forSelectionType: String.self) { ids in menu(ids) } primaryAction: { ids in if let id = ids.first { go(to: id) } }
        }
    }

    private func scrollToEnd(_ proxy: ScrollViewProxy) {
        guard let last = model.userRows.last?.id else { return }
        DispatchQueue.main.async { proxy.scrollTo(last, anchor: .bottom) }
    }

    @ViewBuilder private func menu(_ ids: Set<String>) -> some View {
        Group {
            if let id = ids.first {
                Button("前往消息") { go(to: id) }
                if let edit = model.userRows.first(where: { $0.id == id }).flatMap({ editMessage($0.message.id) }) { Button("编辑消息…", action: edit) }
            } else if let layout {
                ToolPlaceSection(window: layout, kind: .outline)
            }
        }
    }

    private func go(to id: String) {
        guard model.userRows.contains(where: { $0.id == id }) else { return }
        if isRunning, id == model.userRows.last?.id { goToBottom() } else { navigate(id) }
    }
}
