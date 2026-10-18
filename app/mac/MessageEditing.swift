import SwiftUI
import VeluneBindings

/// A user message opened for editing.
struct MessageEditRequest: Identifiable {
    let message: Message
    /// Its conversation can take the replacement in place.
    let inPlace: Bool
    /// The conversation's last user message: in place needs no question.
    let isLast: Bool
    var id: String { message.id }
}

/// Edits a user message and sends it as a new turn in its place. The last
/// message continues in place; for an earlier one the user chooses between
/// a new conversation and replacing what followed it.
struct MessageEditorSheet: View {
    @ObservedObject var store: AppStore
    let request: MessageEditRequest
    /// A fork's new conversation, to show in a new tab.
    let forked: (BindingSnapshotResult) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @State private var attachments: [String] = []
    @State private var asking = false

    /// Embedded images have no file to attach again.
    private var embeddedImages: Int {
        request.message.blocks.filter { if case .image(_, .data) = $0 { return true }; return false }.count
    }
    private var isEmpty: Bool { text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && attachments.isEmpty }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("编辑消息").font(.headline)
            TextEditor(text: $text).font(.body).frame(minWidth: 460, minHeight: 160)
                .overlay(RoundedRectangle(cornerRadius: 5).strokeBorder(Color(nsColor: .separatorColor)))
            if !attachments.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(attachments, id: \.self) { path in
                        HStack(spacing: 6) {
                            Label(URL(fileURLWithPath: path).lastPathComponent, systemImage: "paperclip").help(path)
                            Spacer(minLength: 8)
                            Button("移除") { attachments.removeAll { $0 == path } }.controlSize(.small)
                        }
                    }
                }
            }
            if embeddedImages > 0 {
                Text("原消息中的 \(embeddedImages) 张内嵌图片没有文件，编辑后的消息不包含它们。").font(.callout).foregroundStyle(.secondary)
            }
            if !store.canSend {
                Text("当前不能发送。").font(.callout).foregroundStyle(.secondary)
            }
            HStack {
                Spacer()
                Button("取消", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button("发送") { if request.isLast && request.inPlace { send(.inPlace) } else { asking = true } }
                    .keyboardShortcut(.defaultAction).disabled(isEmpty || store.editability(of: request.message.id) == nil)
            }
        }
        .padding(20)
        .onAppear {
            text = request.message.blocks.compactMap { if case .text(let text) = $0 { return text }; return nil }.joined(separator: "\n")
            attachments = request.message.blocks.compactMap { block in
                switch block {
                case .file(let path), .image(_, .path(let path)): return path
                default: return nil
                }
            }
        }
        .confirmationDialog("从这条消息继续", isPresented: $asking) {
            Button("在新会话中继续") { send(.fork) }
            if request.inPlace { Button("在原会话中继续", role: .destructive) { send(.inPlace) } }
            Button("取消", role: .cancel) {}
        } message: {
            Text(request.inPlace
                 ? "在原会话中继续会替换这条消息及其后的全部内容；在新会话中继续则保留当前会话，新会话包含这条消息之前的内容。"
                 : "此运行时不能在原会话中替换这条消息；新会话包含这条消息之前的内容，当前会话保持不变。")
        }
    }

    private func send(_ placement: BindingEditPlacement) {
        store.editMessage(request.message.id, placement: placement, text: text.trimmingCharacters(in: .whitespacesAndNewlines), attachments: attachments, forked: forked)
        dismiss()
    }
}
