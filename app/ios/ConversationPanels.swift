import SwiftUI

enum ConversationPanel: String, CaseIterable {
    case outline = "大纲"
    case agents = "关联 Agent"
}

struct ConversationInspector: View {
    @ObservedObject var model: ConversationModel
    let active: [ActiveConversation]
    @Binding var panel: ConversationPanel
    let jump: (String) -> Void
    let open: (String, Bool) -> Void
    let close: () -> Void
    @Environment(\.supportsMultipleWindows) private var supportsMultipleWindows
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Picker("会话面板", selection: $panel) {
                    ForEach(ConversationPanel.allCases, id: \.self) { Text($0.rawValue).tag($0) }
                }.pickerStyle(.segmented)
                Button(action: close) { Image(systemName: "xmark").frame(width: 44, height: 44) }
                    .buttonStyle(.plain).foregroundStyle(.secondary)
                    .accessibilityLabel("关闭面板").keyboardShortcut(.cancelAction)
            }.padding(.leading, 12).padding(.trailing, 4).padding(.vertical, 4)
            Divider()
            if panel == .outline { outline } else { agents }
        }
        .background(Color(.systemBackground))
    }
    private var outline: some View {
        List {
            ForEach(Array((model.snapshot?.transcriptOutline ?? []).enumerated()), id: \.element.id) { index, item in
                Button { jump(item.id) } label: {
                    HStack(alignment: .top, spacing: 10) {
                        Text("\(index + 1)").font(.caption.monospacedDigit()).foregroundStyle(.secondary).frame(width: 20, alignment: .leading)
                        Text(model.message(item.messageId)?.plainText ?? "消息").font(.callout).lineLimit(3).foregroundStyle(.primary)
                    }.padding(.vertical, 4)
                }
            }
        }
        .listStyle(.plain)
        .overlay { if model.snapshot?.transcriptOutline.isEmpty != false { ContentUnavailableView("没有大纲条目", systemImage: "list.bullet") } }
        .accessibilityIdentifier("conversationOutline")
    }
    private var agents: some View {
        List {
            if let parent = model.snapshot?.conversation.parentConversationId {
                Section("父会话") { agent(parent, "打开父会话", failed: false) }
            }
            if let children = model.snapshot?.subAgents, !children.isEmpty {
                Section("子 Agent") { ForEach(children, id: \.conversationId) { agent($0.conversationId, $0.title, failed: $0.failed) } }
            }
            if let collaborators = model.snapshot?.collaborators, !collaborators.isEmpty {
                Section("协作") {
                    ForEach(collaborators, id: \.conversationId) { other in
                        agent(other.conversationId, other.title, failed: false,
                              detail: [other.runtimeName, other.contacted && other.contactedBy ? "互相联系" : other.contacted ? "本会话联系了它" : "它联系了本会话"].compactMap { $0 }.joined(separator: " · "))
                    }
                }
            }
        }
        .listStyle(.plain)
        .overlay {
            if model.snapshot?.conversation.parentConversationId == nil && model.snapshot?.subAgents.isEmpty != false && model.snapshot?.collaborators.isEmpty != false {
                ContentUnavailableView("没有关联 Agent", systemImage: "person.2")
            }
        }.accessibilityIdentifier("linkedAgents")
    }
    private func agent(_ id: String, _ title: String, failed: Bool, detail: String? = nil) -> some View {
        let activity = active.first { $0.conversationId == id }
        return Button { open(id, false) } label: {
            HStack(alignment: .top, spacing: 10) {
                if let activity, activity.pendingInteractions > 0 { Image(systemName: "hand.raised.fill").foregroundStyle(.orange).accessibilityLabel("等待回复") }
                else if let activity, activity.runState == .running || activity.runState == .stopping { ProgressView().controlSize(.small).accessibilityLabel("正在运行") }
                else { Image(systemName: failed ? "exclamationmark.circle" : "checkmark.circle").foregroundStyle(failed ? .red : .secondary).accessibilityLabel(failed ? "执行失败" : "已结束") }
                VStack(alignment: .leading, spacing: 4) {
                    Text(title.isEmpty ? "未命名会话" : title).font(.callout).foregroundStyle(.primary).lineLimit(2)
                    if let detail { Text(detail).font(.caption).foregroundStyle(.secondary) }
                }
            }.padding(.vertical, 4)
        }.contextMenu {
            Button("打开会话") { open(id, false) }
            if supportsMultipleWindows { Button("在新窗口中打开") { open(id, true) } }
        }
    }
}

extension DeliveredMessage {
    var plainText: String {
        blocks.compactMap { if case .text(let text) = $0 { return text }; return nil }.joined(separator: "\n")
    }
}
