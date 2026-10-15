import SwiftUI

/// The conversation's send queue, above the composer: messages that go out
/// one by one after the running turn. A system list: click selects, double
/// click edits, drag reorders, Delete removes; the context menu has the rest.
struct SendQueueView: View {
    @ObservedObject var store: AppStore
    @State private var selection = Set<String>()
    @State private var editing: QueuedMessage?

    private var queue: [QueuedMessage] { store.followUp.queue }
    /// Rows shown before the list scrolls.
    private static let visibleRows = 4
    private static let rowHeight: CGFloat = 24

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            header
            if !queue.isEmpty { list }
        }
        .sheet(item: $editing) { message in
            QueuedMessageEditor(message: message) { text in
                if text.isEmpty && message.attachments.isEmpty { store.removeQueued(message.id) } else if text != message.text { store.editQueued(message.id, text: text) }
            }
        }
        .onChange(of: queue.map(\.id)) { _, ids in selection.formIntersection(ids) }
    }

    private var header: some View {
        HStack(spacing: 6) {
            if let pause = store.followUp.paused {
                Label(pause.explanation, systemImage: "pause.circle")
                Spacer(minLength: 8)
                if !queue.isEmpty { Button("继续发送", action: store.resumeQueue).controlSize(.small).disabled(store.isLoading) }
            } else {
                Text(store.isGenerating ? "排队 \(queue.count) 条，本轮结束后依次发送" : "排队 \(queue.count) 条，依次发送")
                Spacer(minLength: 0)
            }
        }
        .font(.callout).foregroundStyle(.secondary)
    }

    private var list: some View {
        List(selection: $selection) {
            ForEach(queue) { message in
                HStack(spacing: 6) {
                    Text(message.text.isEmpty ? "（仅附件）" : message.text).lineLimit(1).truncationMode(.tail)
                    Spacer(minLength: 4)
                    if !message.attachments.isEmpty {
                        Label("\(message.attachments.count)", systemImage: "paperclip").labelStyle(.titleAndIcon).foregroundStyle(.secondary)
                    }
                }
                .help(message.text)
                .tag(message.id)
            }
            .onMove { from, to in
                guard let index = from.first, from.count == 1 else { return }
                store.moveQueued(queue[index].id, to: to > index ? to - 1 : to)
            }
        }
        .listStyle(.bordered(alternatesRowBackgrounds: false))
        .environment(\.defaultMinListRowHeight, Self.rowHeight)
        .frame(height: CGFloat(min(queue.count, Self.visibleRows)) * Self.rowHeight + 2)
        .accessibilityLabel("发送队列")
        .onDeleteCommand { remove(selection) }
        .contextMenu(forSelectionType: String.self) { ids in
            if ids.count == 1, let message = queue.first(where: { ids.contains($0.id) }), let index = queue.firstIndex(of: message) {
                Button("编辑…") { editing = message }
                Divider()
                Button("移到最前") { store.moveQueued(message.id, to: 0) }.disabled(index == 0)
                Button("上移") { store.moveQueued(message.id, to: index - 1) }.disabled(index == 0)
                Button("下移") { store.moveQueued(message.id, to: index + 1) }.disabled(index == queue.count - 1)
                Divider()
            }
            if !ids.isEmpty { Button(ids.count > 1 ? "删除 \(ids.count) 条" : "删除") { remove(ids) } }
        } primaryAction: { ids in
            if ids.count == 1, let message = queue.first(where: { ids.contains($0.id) }) { editing = message }
        }
    }

    private func remove(_ ids: Set<String>) {
        for message in queue where ids.contains(message.id) { store.removeQueued(message.id) }
    }
}

/// Edits a queued message's text; its attachments and next-turn choices stay.
private struct QueuedMessageEditor: View {
    let message: QueuedMessage
    let save: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("编辑排队消息").font(.headline)
            TextEditor(text: $text).font(.body).frame(minWidth: 420, minHeight: 140)
                .overlay(RoundedRectangle(cornerRadius: 5).strokeBorder(Color(nsColor: .separatorColor)))
            if !message.attachments.isEmpty {
                Text("附件 \(message.attachments.count) 个保持不变。").font(.callout).foregroundStyle(.secondary)
            }
            HStack {
                Spacer()
                Button("取消", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button("存储") { save(text.trimmingCharacters(in: .whitespacesAndNewlines)); dismiss() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .onAppear { text = message.text }
    }
}
