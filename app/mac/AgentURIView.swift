import SwiftUI

struct AgentURIView: View {
    @ObservedObject var window: ConversationWindowModel
    @State private var selection: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("打开会话").font(.headline)
            if let failure = window.agentURIFailure { Text(failure).foregroundStyle(.secondary).textSelection(.enabled) }
            if !window.agentURIMatches.isEmpty {
                List(window.agentURIMatches, selection: $selection) { conversation in
                    VStack(alignment: .leading) {
                        Text(conversation.title)
                        Text(window.list.runtimeInstances.first { $0.id == conversation.runtimeID }?.name ?? conversation.runtimeID).foregroundStyle(.secondary)
                        if let cwd = conversation.cwd { Text(cwd).font(.caption).foregroundStyle(.secondary) }
                    }
                    .tag(conversation.id)
                }
                .frame(minHeight: 160, maxHeight: 280)
                .contextMenu(forSelectionType: String.self) { _ in } primaryAction: { ids in
                    if let id = ids.first { open(id) }
                }
            }
            HStack {
                Spacer()
                Button("关闭") { window.showsAgentURI = false }.keyboardShortcut(.cancelAction)
                if !window.agentURIMatches.isEmpty {
                    Button("打开") { if let selection { open(selection) } }
                        .disabled(selection == nil).keyboardShortcut(.defaultAction)
                }
            }
        }
        .padding(20).frame(width: 520)
    }

    private func open(_ id: String) {
        window.showsAgentURI = false
        window.open(id)
    }
}
