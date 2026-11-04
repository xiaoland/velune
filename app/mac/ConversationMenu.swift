import AppKit
import SwiftUI
import VeluneBindings

/// The same conversation actions in lists and editor tabs. Presentation of
/// confirmations belongs to the window, so closing a menu cannot lose it.
struct ConversationManagementMenu: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var store: ConversationStore
    let ids: Set<String>

    @ViewBuilder var body: some View {
        let targets = ids.compactMap { store.conversation(id: $0) }
        if runMenu(targets) {
            if targets.count == 1, let target = targets.first {
                Button("发送消息…") { window.openComposerSheet(target.id) }
                if store.device.hasSideChats(target.runtimeID) {
                    Button("侧聊") { window.openSideChat(on: target.id) }
                }
                if store.device.resumableConversations[target.id] != nil {
                    Button("继续") { window.resumeFromList(target.id) }
                }
            }
            let stoppable = targets.filter { store.device.isStoppable($0.id) }
            if !stoppable.isEmpty {
                Button(targets.count > 1 ? "停止所选会话" : "停止") { store.device.stop(stoppable.map(\.id)) }
            } else if targets.count == 1, let target = targets.first, store.device.subAgentRuntimeIDs[target.id] != nil {
                Button("停止子 Agent") { store.device.stopSubAgents(of: target.id) }
            }
            Divider()
        }
        if targets.count == 1, let target = targets.first {
            Button("重命名…") { window.renameTarget = target }.disabled(!store.canManageConversations || !store.canRenameConversation(target))
        }
        if !targets.isEmpty {
            if !targets.contains(where: \.archived) {
                if targets.allSatisfy(\.pinned) { Button("取消置顶") { store.organize(targets, .unpin) } }
                else { Button("置顶") { store.organize(targets, .pin) } }
            }
            if targets.contains(where: \.unread) { Button("标为已读") { store.organize(targets, .markRead) } }
            else { Button("标为未读") { store.organize(targets, .markUnread) }.disabled(targets.contains(where: \.archived)) }
            moveMenu(targets)
            if targets.allSatisfy(\.archived) {
                Button("取消归档") { store.organize(targets, .unarchive) }.disabled(!store.canManageConversations)
            } else {
                Button("归档") { store.organize(targets.filter { !$0.archived }, .archive) }
                    .disabled(!store.canManageConversations || targets.contains(where: store.isRunning))
            }
        }
        if targets.count == 1, let target = targets.first {
            Divider()
            Button("在新标签页中打开") { window.openInNewTab(target.id) }
            Button("在右侧拆分打开") { window.split(.horizontal, opening: target.id) }
            Button("在下方拆分打开") { window.split(.vertical, opening: target.id) }
            Button("在新窗口中打开") { window.openInNewWindow(target.id) }
            Divider()
            Menu("拷贝") {
                Button("会话 ID") { copyToPasteboard(target.nativeID) }.disabled(target.nativeID.isEmpty)
                Button("Velune 链接") { copyToPasteboard(ConversationLink.url(runtimeID: target.runtimeID, conversationID: target.id).absoluteString) }
                if let uri = target.agentsURI {
                    Button("xURL") { copyToPasteboard(uri) }
                }
            }
        }
        Divider()
        Button(targets.count > 1 ? "删除所选会话…" : "删除…", role: .destructive) { window.deleteTargets = targets }
            .disabled(targets.isEmpty || !store.canManageConversations || !targets.allSatisfy(store.canDeleteConversation))
        if targets.contains(where: { !store.canDeleteConversation($0) }) { Text("部分关联原生会话当前无法删除") }
        if targets.count == 1, let target = targets.first, !store.canRenameConversation(target) { Text("原生会话标题当前无法修改") }
    }

    /// Whether the menu offers running the conversations: sending, continuing
    /// or stopping. Native sub-agents are continued by their parent's agent.
    private func runMenu(_ targets: [Conversation]) -> Bool {
        guard !targets.isEmpty, !store.device.isShuttingDown else { return false }
        if targets.count > 1 { return targets.contains { store.device.isStoppable($0.id) } }
        return targets[0].parentConversationID == nil
    }

    /// Moving files a conversation under another project in Velune's list;
    /// where it runs stays its own working directory.
    private func moveMenu(_ targets: [Conversation]) -> some View {
        let current = Set(targets.map(\.project))
        return Menu("移到项目") {
            ForEach(store.device.conversationProjects.filter { current != [$0.path] }, id: \.path) { project in
                Button(ConversationBrowser.projectName(project.path, projects: store.device.conversationProjects)) { store.organize(targets, .moveToProject(path: project.path)) }
            }
            Divider()
            Button("其他文件夹…") {
                let panel = NSOpenPanel()
                panel.canChooseFiles = false; panel.canChooseDirectories = true; panel.allowsMultipleSelection = false
                panel.prompt = "移到此项目"
                if panel.runModal() == .OK, let url = panel.url { store.organize(targets, .moveToProject(path: url.path)) }
            }
            if targets.contains(where: \.moved) {
                Divider()
                Button("移回工作目录所在项目") { store.organize(targets, .moveToProject(path: nil)) }
            }
        }
    }

    private func copyToPasteboard(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }
}
