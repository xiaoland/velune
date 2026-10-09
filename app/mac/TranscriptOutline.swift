import SwiftUI

struct TranscriptOutlineView: View {
    @ObservedObject var model: TranscriptModel
    let navigate: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var selectedID: String?
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("会话大纲").font(.headline); Spacer() }.padding(20)
            List(selection: $selectedID) {
                ForEach(model.userRows) { row in
                    Text(row.message.text.isEmpty ? "用户消息" : row.message.text)
                        .lineLimit(3).frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 4).tag(row.id)
                }
            }.listStyle(.bordered).padding(.horizontal, 20)
            .contextMenu(forSelectionType: String.self) { ids in
                if let id = ids.first { Button("前往消息") { go(to: id) } }
            } primaryAction: { ids in if let id = ids.first { go(to: id) } }
            HStack {
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("前往消息") { if let selectedID { go(to: selectedID) } }
                    .keyboardShortcut(.defaultAction).disabled(selectedID == nil)
            }.padding(16)
        }.frame(width: 440, height: 400)
        .onChange(of: model.contentRevision) { _, _ in
            if let selectedID, !model.userRows.contains(where: { $0.id == selectedID }) { self.selectedID = nil }
        }
    }
    private func go(to id: String) {
        guard model.userRows.contains(where: { $0.id == id }) else { return }
        navigate(id)
        dismiss()
    }
}
