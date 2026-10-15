import SwiftUI

/// The outline: user messages of the loaded conversation. A click selects,
/// a double click (or Return, in the popover) goes to the message.
struct TranscriptOutlineView: View {
    @ObservedObject var model: TranscriptModel
    /// In a popover it has its own size and takes Return; docked or floating
    /// it fills its place and leaves Return to the composer.
    var inPopover = true
    let navigate: (String) -> Void
    @State private var selectedID: String?
    var body: some View {
        VStack(spacing: 0) {
            List(selection: $selectedID) {
                ForEach(model.userRows) { row in
                    Text(row.message.text.isEmpty ? "用户消息" : row.message.text)
                        .lineLimit(3).frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 3).tag(row.id)
                }
            }
            .listStyle(.inset)
            .accessibilityLabel("会话大纲")
            .contextMenu(forSelectionType: String.self) { ids in
                if let id = ids.first { Button("前往消息") { go(to: id) } }
            } primaryAction: { ids in if let id = ids.first { go(to: id) } }
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
    private func go(to id: String) {
        guard model.userRows.contains(where: { $0.id == id }) else { return }
        navigate(id)
    }
}
