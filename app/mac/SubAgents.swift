import SwiftUI

/// The native sub-agents of the focused conversation, with whether each
/// runs. A double click (or Return) opens one's conversation.
struct SubAgentsView: View {
    @ObservedObject var store: AppStore
    /// In a popover it has its own size and takes Return.
    var inPopover = true
    var inSidebar = false
    /// The window whose tool it is: right-clicked where no sub-agent is, it offers the tool's places.
    var layout: ConversationWindowModel?
    let open: (String) -> Void
    @State private var selectedID: String?

    var body: some View {
        let agents = store.subAgents
        VStack(spacing: 0) {
            if agents.isEmpty {
                ContentUnavailableView("没有子 Agent", systemImage: "person.2", description: Text("会话的 Agent 启动原生子 Agent 后，在这里列出并可打开。"))
            } else {
                if inSidebar { list(agents).listStyle(.sidebar) } else { list(agents).listStyle(.inset) }
                Divider()
                HStack {
                    let running = agents.filter { store.isSubAgentRunning($0.id) }.count
                    Text(running > 0 ? "\(agents.count) 个子 Agent，\(running) 个正在运行" : "\(agents.count) 个子 Agent")
                        .font(.callout).foregroundStyle(.secondary)
                    Spacer()
                    Button("打开") { if let selectedID { open(selectedID) } }
                        .keyboardShortcut(inPopover ? .defaultAction : nil).disabled(selectedID == nil)
                }.padding(10)
            }
        }
        .frame(width: inPopover ? 360 : nil, height: inPopover ? 320 : nil)
        .onChange(of: agents.map(\.id)) { _, ids in
            if let selectedID, !ids.contains(selectedID) { self.selectedID = nil }
        }
    }

    private func list(_ agents: [SubAgent]) -> some View {
        List(selection: $selectedID) {
            ForEach(agents) { agent in
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    state(of: agent).frame(width: 14)
                    Text(agent.title).lineLimit(2).frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(.vertical, 2).tag(agent.id)
            }
        }
        .accessibilityLabel("子 Agent")
        .contextMenu(forSelectionType: String.self) { ids in
            if let id = ids.first {
                Button("打开会话") { open(id) }
            } else if let layout {
                ToolPlaceSection(window: layout, kind: .subAgents)
            }
        } primaryAction: { ids in if let id = ids.first { open(id) } }
    }

    @ViewBuilder private func state(of agent: SubAgent) -> some View {
        if store.isSubAgentRunning(agent.id) {
            ProgressView().controlSize(.mini).help("正在运行").accessibilityLabel("正在运行")
        } else if agent.failed {
            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.red).imageScale(.small)
                .help("上一轮失败").accessibilityLabel("上一轮失败")
        } else {
            Image(systemName: "checkmark.circle").foregroundStyle(.secondary).imageScale(.small)
                .help("已结束").accessibilityLabel("已结束")
        }
    }
}
