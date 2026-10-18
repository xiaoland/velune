import SwiftUI
import VeluneBindings

/// A conversation the focused one works with, as the linked agents list it.
struct LinkedAgent: Identifiable, Equatable {
    enum Relation: Equatable {
        /// The conversation whose agent started this native sub-agent's one.
        case parent
        case subAgent(failed: Bool)
        /// A conversation of Velune's agent-to-agent collaboration.
        case collaborator(runtimeName: String?, contacted: Bool, contactedBy: Bool)
    }
    let id: String
    let title: String
    let relation: Relation

    /// The loaded conversation's: the one that started it, its native
    /// sub-agents, then its collaborators, each conversation once.
    @MainActor static func all(of store: AppStore) -> [LinkedAgent] {
        guard store.pendingConversationID == nil else { return [] }
        var found: [LinkedAgent] = []
        if let parent = store.loadedParentConversationID {
            found.append(LinkedAgent(id: parent, title: store.conversation(id: parent)?.title ?? "父会话", relation: .parent))
        }
        found += store.subAgents.map { LinkedAgent(id: $0.id, title: $0.title, relation: .subAgent(failed: $0.failed)) }
        found += store.collaborators.map {
            LinkedAgent(id: $0.id, title: $0.title.isEmpty ? store.conversation(id: $0.id)?.title ?? "未命名会话" : $0.title,
                        relation: .collaborator(runtimeName: $0.runtimeName, contacted: $0.contacted, contactedBy: $0.contactedBy))
        }
        var seen = Set<String>()
        return found.filter { seen.insert($0.id).inserted }
    }

    /// "2 个子 Agent，1 个正在运行；1 个协作会话", for the toolbar and the list's foot.
    @MainActor static func summary(of store: AppStore) -> String {
        let agents = all(of: store)
        var parts: [String] = []
        let subAgents = agents.filter { if case .subAgent = $0.relation { true } else { false } }
        if !subAgents.isEmpty {
            let running = subAgents.filter { store.isConversationRunning($0.id) }.count
            parts.append(running > 0 ? "\(subAgents.count) 个子 Agent，\(running) 个正在运行" : "\(subAgents.count) 个子 Agent")
        }
        let collaborators = agents.filter { if case .collaborator = $0.relation { true } else { false } }
        if !collaborators.isEmpty {
            let running = collaborators.filter { store.isConversationRunning($0.id) }.count
            parts.append(running > 0 ? "\(collaborators.count) 个协作会话，\(running) 个正在运行" : "\(collaborators.count) 个协作会话")
        }
        if agents.contains(where: { $0.relation == .parent }) { parts.insert("由父会话启动", at: 0) }
        return parts.isEmpty ? "没有关联 Agent" : parts.joined(separator: "；")
    }
}

/// The conversations the focused one works with: the one that started it
/// when it is a native sub-agent's, its native sub-agents, and those it took
/// part with in Velune's agent-to-agent collaboration, whichever started.
/// A double click (or Return) opens one where the setting says; its menu
/// offers every place.
struct LinkedAgentsView: View {
    @ObservedObject var store: AppStore
    /// In a popover it has its own size and takes Return.
    var inPopover = true
    var inSidebar = false
    /// The window whose tool it is: right-clicked where no agent is, it offers the tool's places.
    var layout: ConversationWindowModel?
    /// Opens a conversation where it says, or where the setting says when nil.
    let open: (String, BindingLinkedConversationOpening?) -> Void
    @State private var selectedID: String?

    var body: some View {
        let agents = LinkedAgent.all(of: store)
        VStack(spacing: 0) {
            if agents.isEmpty {
                ContentUnavailableView("没有关联 Agent", systemImage: "person.2",
                                       description: Text("会话的 Agent 启动子 Agent，或经 Velune 与其他会话协作后，在这里列出并可打开。"))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                if inSidebar { list(agents).listStyle(.sidebar) } else { list(agents).listStyle(.inset) }
                Divider()
                HStack {
                    Text(LinkedAgent.summary(of: store)).font(.callout).foregroundStyle(.secondary).lineLimit(2)
                    Spacer()
                    Button("打开") { if let selectedID { open(selectedID, nil) } }
                        .keyboardShortcut(inPopover ? .defaultAction : nil).disabled(selectedID == nil)
                }.padding(10)
            }
        }
        .frame(width: inPopover ? 360 : nil, height: inPopover ? 360 : nil)
        .onChange(of: agents.map(\.id)) { _, ids in
            if let selectedID, !ids.contains(selectedID) { self.selectedID = nil }
        }
    }

    private func list(_ agents: [LinkedAgent]) -> some View {
        List(selection: $selectedID) {
            section("父会话", agents.filter { $0.relation == .parent })
            section("子 Agent", agents.filter { if case .subAgent = $0.relation { true } else { false } })
            section("协作", agents.filter { if case .collaborator = $0.relation { true } else { false } })
        }
        .accessibilityLabel("关联 Agent")
        .contextMenu(forSelectionType: String.self) { ids in
            if let id = ids.first {
                Button("在当前标签页中打开") { open(id, .currentTab) }
                Button("在新标签页中打开") { open(id, .newTab) }
                Button("在浮层中打开") { open(id, .sheet) }
                Button("在新窗口中打开") { open(id, .newWindow) }
            } else if let layout {
                ToolPlaceSection(window: layout, kind: .agents)
            }
        } primaryAction: { ids in if let id = ids.first { open(id, nil) } }
    }

    @ViewBuilder private func section(_ title: String, _ agents: [LinkedAgent]) -> some View {
        if !agents.isEmpty {
            Section(title) {
                ForEach(agents) { agent in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        state(of: agent).frame(width: 14)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(agent.title).lineLimit(2)
                            if let detail = detail(of: agent) { Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .padding(.vertical, 2).tag(agent.id)
                }
            }
        }
    }

    /// Who contacted whom, and the collaborator's runtime.
    private func detail(of agent: LinkedAgent) -> String? {
        guard case .collaborator(let runtimeName, let contacted, let contactedBy) = agent.relation else { return nil }
        let direction = contacted && contactedBy ? "互相联系" : contacted ? "本会话联系了它" : "它联系了本会话"
        return [runtimeName, direction].compactMap { $0 }.joined(separator: " · ")
    }

    @ViewBuilder private func state(of agent: LinkedAgent) -> some View {
        if store.isConversationRunning(agent.id) {
            ProgressView().controlSize(.mini).help("正在运行").accessibilityLabel("正在运行")
        } else {
            switch agent.relation {
            case .subAgent(failed: true):
                Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.red).imageScale(.small)
                    .help("上一轮失败").accessibilityLabel("上一轮失败")
            case .subAgent:
                Image(systemName: "checkmark.circle").foregroundStyle(.secondary).imageScale(.small)
                    .help("已结束").accessibilityLabel("已结束")
            case .parent:
                Image(systemName: "arrow.turn.left.up").foregroundStyle(.secondary).imageScale(.small)
            case .collaborator(_, let contacted, let contactedBy):
                Image(systemName: contacted && contactedBy ? "arrow.left.arrow.right" : contacted ? "arrow.up.right" : "arrow.down.left")
                    .foregroundStyle(.secondary).imageScale(.small)
            }
        }
    }
}

/// A conversation opened over its window from one linked to it, read and
/// continued there as in a tab; it may move to a tab of the window or to a
/// window of its own, keeping what it has read.
struct LinkedConversationSheet: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var tab: ConversationTab
    @ObservedObject var store: AppStore
    let size: CGSize

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Text(tab.title).font(.headline).lineLimit(1).truncationMode(.middle)
                Spacer(minLength: 12)
                Button("在标签页中打开", action: window.moveSheetToTab)
                Button("在新窗口中打开", action: window.moveSheetToNewWindow)
                Button("完成", action: window.closeSheet).keyboardShortcut(.cancelAction)
            }
            .padding(.horizontal, 16).padding(.vertical, 10)
            Divider()
            TabContentView(window: window, tab: tab, store: store, previewEmpty: false, inSheet: true)
                .frame(minWidth: 0, maxWidth: .infinity, minHeight: 0, maxHeight: .infinity)
        }
        .frame(width: size.width, height: size.height)
    }
}
