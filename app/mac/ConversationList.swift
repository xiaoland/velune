import AppKit
import SwiftUI
import VeluneBindings

/// A window's conversation list, in the sidebar or in a floating panel with
/// its own search field. A conversation selected alone opens in the
/// window's focused pane; several are a selection to manage.
struct ConversationListView: View {
    enum Placement { case sidebar, panel }
    @ObservedObject var window: ConversationWindowModel
    /// The list's workspace.
    @ObservedObject var store: AppStore
    /// The focused pane's workspace.
    @ObservedObject var focused: AppStore
    let placement: Placement
    @Environment(\.controlActiveState) private var activeState
    @State private var browser = ConversationBrowser()
    @State private var renameTarget: Conversation?
    @State private var deleteTargets: [Conversation] = []
    @State private var projectTarget: ProjectEditorTarget?
    private var conversationSections: [ConversationBrowser.Section] {
        if let results = store.searchResults { return ConversationBrowser.sections(conversations: results.conversations, groups: results.groups) }
        return ConversationBrowser.sections(conversations: store.conversations, groups: store.conversationGroups)
    }

    var body: some View {
        Group {
            switch placement {
            case .sidebar:
                withSidebarToolbar(list
                    .searchable(text: $window.listSearch, placement: .sidebar, prompt: "搜索会话")
                    .navigationTitle("会话"))
            case .panel:
                VStack(spacing: 0) {
                    HStack(spacing: 8) {
                        SearchField(text: $window.listSearch, prompt: "搜索会话")
                        browserMenu.menuStyle(.borderlessButton).fixedSize()
                        newConversationButton.buttonStyle(.borderless)
                    }
                    .labelStyle(.iconOnly)
                    .padding(.horizontal, 10).padding(.vertical, 8)
                    Divider()
                    list
                }
            }
        }
        .task(id: window.listSearch) {
            // Searching reads every listed conversation; wait for typing to pause.
            if !window.listSearch.isEmpty { try? await Task.sleep(for: .milliseconds(250)) }
            if !Task.isCancelled { store.setSearchText(window.listSearch) }
        }
        .onReceive(store.$conversationBrowserPreferences) { value in
            if let value, value != browser.preferences { browser.restore(value) }
        }
        .onChange(of: browser.preferences) { _, value in
            if store.conversationBrowserPreferences != nil { store.setConversationBrowserPreferences(value) }
        }
        .sheet(item: $renameTarget) { ConversationRenameView(store: store, conversation: $0) }
        .sheet(item: $projectTarget) { ProjectEditorView(store: store, target: $0) }        .alert(deleteTargets.count == 1 ? "删除会话？" : "删除 \(deleteTargets.count) 个会话？", isPresented: Binding(get: { !deleteTargets.isEmpty }, set: { if !$0 { deleteTargets = [] } })) {
            Button("取消", role: .cancel) { deleteTargets = [] }
            Button("删除", role: .destructive) { store.deleteConversations(deleteTargets); deleteTargets = [] }
        } message: {
            Text("将永久删除所选 \(deleteTargets.count) 个会话，以及它们在各 Agent 运行时中的关联会话数据。此操作无法撤销。")
        }
    }

    /// Customizable where the window's toolbar is (see `VeluneRootView.withToolbar`).
    @ViewBuilder private func withSidebarToolbar(_ content: some View) -> some View {
        if #available(macOS 26, *) {
            content.toolbar(id: ConversationToolbar.id) {
                ToolbarItem(id: "list-filter") { browserMenu }
                ToolbarItem(id: "new-conversation") { newConversationButton }
            }
        } else {
            content.toolbar {
                ToolbarItemGroup {
                    browserMenu
                    newConversationButton
                }
            }
        }
    }

    private var list: some View {
        List(selection: Binding(get: { store.selectedConversationIDs.count > 1 ? store.selectedConversationIDs : Set([focused.selectedConversationID].compactMap { $0 }) },
                                set: { ids in
                                    store.setListSelection(ids)
                                    if ids.count == 1, let id = ids.first, id != focused.selectedConversationID {
                                        // Option-click opens beside the shown tab rather than in its place.
                                        if NSEvent.modifierFlags.contains(.option) { window.openInNewTab(id) } else { window.open(id) }
                                    }
                                })) {
            if store.showsArchived {
                HStack {
                    Label("已归档的会话", systemImage: "archivebox").foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                    Button("返回") { store.setShowsArchived(false) }.buttonStyle(.link)
                }.selectionDisabled(true)
            }
            if conversationSections.isEmpty && !store.isLoading && !focused.isLoading {
                if store.searchResults != nil || (browser.hasFilters && !store.conversations.isEmpty) {
                    Text("没有匹配的会话").foregroundStyle(.secondary).selectionDisabled(true)
                    if browser.hasFilters { Button("清除筛选") { browser.runtimeID = nil; browser.project = .all }.selectionDisabled(true) }
                } else if store.showsArchived {
                    Text("没有已归档的会话").foregroundStyle(.secondary).selectionDisabled(true)
                }
            }
            ForEach(conversationSections) { section in
                Section(isExpanded: expansion(section.id)) {
                    ForEach(section.conversations) { conversation in
                        conversationRow(conversation)
                            .tag(conversation.id)
                            // Always selectable, also while the conversation pressed on
                            // opens: the rows do not change under the press.
                            .contextMenu { conversationManagementMenu(store.selectedConversationIDs.contains(conversation.id) ? store.selectedConversationIDs : [conversation.id]) }
                            // Dropped on a pane, it opens there or beside it; outside
                            // every window, in a window of its own. The list's own drag
                            // (`itemProvider`), not a gesture on the row (`onDrag`): the
                            // table then starts the drag on any motion, where a gesture
                            // lost the press to the table's selection dragging across
                            // rows. Only Velune takes it, as a tab (`ConversationLink.dragType`).
                            .itemProvider { [window] in
                                let id = conversation.id
                                DispatchQueue.main.async { ListDragWatch.shared.begin(id, from: window) }
                                return ConversationLink.itemProvider(runtimeID: conversation.runtimeID, conversationID: id)
                            }
                    }
                    if section.hasMore {
                        if store.searchResults == nil {
                            Button("加载更多（\(section.group.loaded) / \(section.group.total)）") { store.loadMoreConversations(in: section.id) }
                                .buttonStyle(.plain).foregroundStyle(.secondary).selectionDisabled(true)
                        } else {
                            Text("只显示前 \(section.group.loaded) 个匹配").foregroundStyle(.secondary).selectionDisabled(true)
                        }
                    }
                } header: { sectionHeader(section) }
            }
        }
        .contextMenu(forSelectionType: String.self) { ids in conversationManagementMenu(ids) } primaryAction: { ids in
            guard ids.count == 1, let id = ids.first, let target = store.conversation(id: id), store.canManageConversations, store.canRenameConversation(target) else { return }
            renameTarget = target
        }
        .listStyle(.sidebar)
        .disabled(store.authenticationRunning || store.isShuttingDown)
    }

    private func expansion(_ key: BindingConversationGroupKey) -> Binding<Bool> {
        Binding(get: { store.searchResults != nil || !browser.isCollapsed(key) },
                set: { expanded in if store.searchResults == nil { browser.setCollapsed(key, !expanded) } })
    }

    private func groupTitle(_ key: BindingConversationGroupKey) -> String {
        switch key {
        case .all: "会话"
        case .pinned: "置顶"
        case .runtime(let id): store.runtimeInstances.first { $0.id == id }?.name ?? "Agent 运行时"
        case .project(let path?): ConversationBrowser.projectName(path, projects: store.conversationProjects)
        case .project(nil): "无项目"
        }
    }

    private func sectionHeader(_ section: ConversationBrowser.Section) -> some View {
        let title = groupTitle(section.id)
        let path: String? = if case .project(let path?) = section.id { path } else { nil }
        return HStack(spacing: 6) {
            Text(title).lineLimit(1).truncationMode(.middle)
            Spacer(minLength: 0)
        }
        .help(path.map { (($0 as NSString).abbreviatingWithTildeInPath) } ?? title)
        .contentShape(Rectangle())
        .onTapGesture(count: 2) { if let path { projectTarget = ProjectEditorTarget(path: path) } }
        .contextMenu {
            if let path {
                Button("编辑项目…") { projectTarget = ProjectEditorTarget(path: path) }
                if section.group.name != nil {
                    Button("恢复文件夹名称") {
                        let folders = store.conversationProjects.first { $0.path == path }?.folders ?? []
                        store.setProject(path: path, primaryFolder: path, name: nil, folders: folders)
                    }
                }
                Button("在 Finder 中显示") { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)]) }
                    .disabled(store.location(of: section.conversations.first?.runtimeID) != .local)
                Divider()
            }
            if store.searchResults == nil {
                Button(browser.isCollapsed(section.id) ? "展开" : "折叠") { browser.setCollapsed(section.id, !browser.isCollapsed(section.id)) }
            }
        }
    }

    private func conversationRow(_ conversation: Conversation) -> some View {
        // Who started the turn does not matter; running does, its own or a
        // native sub-agent's that outlives it.
        let ownTurn = store.runningConversationIDs.contains(conversation.id) || store.elsewhereConversationIDs.contains(conversation.id)
        let running = ownTurn || store.subAgentParentIDs.contains(conversation.id)
        // An approval or question, its own or a sub-agent's, waits for the user.
        let waiting = store.waitingConversationIDs.contains(conversation.id)
        // A turn's end is news; its progress is not. The conversation the
        // focused window shows is being read.
        let unread = conversation.unread && !running && !(conversation.id == focused.loadedConversationID && activeState == .key)
        // Its title and state only: unread, failed, running or waiting.
        return HStack(alignment: .firstTextBaseline, spacing: 5) {
            // A turn that ended in failure is news of its own.
            Group {
                if unread && conversation.failed {
                    Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.red).imageScale(.small).help("上一轮失败")
                } else {
                    Circle().fill(.tint).frame(width: 7, height: 7).opacity(unread ? 1 : 0)
                }
            }.frame(width: 12).accessibilityHidden(true)
            Text(conversation.title).lineLimit(1).fontWeight(unread ? .semibold : nil).help(conversation.title)
            if waiting {
                Spacer(minLength: 0)
                Image(systemName: "hand.raised.fill").foregroundStyle(.orange).imageScale(.small)
                    .help("等待你处理").accessibilityLabel("等待你处理")
            } else if running {
                Spacer(minLength: 0)
                let label = ownTurn ? "正在运行" : "子 Agent 正在运行"
                ProgressView().controlSize(.mini).help(label).accessibilityLabel(label)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityValue(unread ? (conversation.failed ? "未读，上一轮失败" : "未读") : "")
    }

    @ViewBuilder private func conversationManagementMenu(_ ids: Set<String>) -> some View {
        let targets = ids.compactMap { store.conversation(id: $0) }
        if targets.count == 1, let target = targets.first {
            Button("重命名…") { renameTarget = target }.disabled(!store.canManageConversations || !store.canRenameConversation(target))
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
            Button("拷贝会话 ID") { copyToPasteboard(target.nativeID) }.disabled(target.nativeID.isEmpty)
            Button("拷贝链接") { copyToPasteboard(ConversationLink.url(runtimeID: target.runtimeID, conversationID: target.id).absoluteString) }
        }
        Divider()
        Button(targets.count > 1 ? "删除所选会话…" : "删除…", role: .destructive) { deleteTargets = targets }
            .disabled(targets.isEmpty || !store.canManageConversations || !targets.allSatisfy(store.canDeleteConversation))
        if targets.contains(where: { !store.canDeleteConversation($0) }) { Text("部分关联原生会话当前无法删除") }
        if targets.count == 1, let target = targets.first, !store.canRenameConversation(target) { Text("原生会话标题当前无法修改") }
    }

    /// Moving files a conversation under another project in Velune's list;
    /// where it runs stays its own working directory.
    private func moveMenu(_ targets: [Conversation]) -> some View {
        let current = Set(targets.map(\.project))
        return Menu("移到项目") {
            ForEach(store.conversationProjects.filter { current != [$0.path] }, id: \.path) { project in
                Button(ConversationBrowser.projectName(project.path, projects: store.conversationProjects)) { store.organize(targets, .moveToProject(path: project.path)) }
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

    private var newConversationButton: some View {
        Button { focused.createConversation() } label: { Label("新建会话", systemImage: "square.and.pencil") }
            .help("新建会话（⌘ N）").disabled(focused.isLoading || focused.enabledRuntimeInstances.isEmpty)
    }

    private var browserMenu: some View {
        Menu {
            Picker("分组", selection: $browser.grouping) { ForEach(ConversationBrowser.Grouping.allCases) { Text($0.title).tag($0) } }
            Picker("排序", selection: $browser.sort) { ForEach(ConversationBrowser.Sort.allCases) { Text($0.title).tag($0) } }
            Toggle("从旧到新", isOn: $browser.oldestFirst)
            Divider()
            Picker("Agent 运行时", selection: $browser.runtimeID) {
                Text("所有运行时").tag(Optional<String>.none)
                ForEach(store.enabledRuntimeInstances) { Text($0.name).tag(Optional($0.id)) }
                if let id = browser.runtimeID, !store.enabledRuntimeInstances.contains(where: { $0.id == id }) {
                    Text("当前筛选的运行时不可用").tag(Optional(id))
                }
            }
            Picker("项目", selection: $browser.project) {
                Text("所有项目").tag(ConversationBrowser.ProjectFilter.all)
                Text("无项目").tag(ConversationBrowser.ProjectFilter.unspecified)
                ForEach(store.conversationProjects, id: \.path) { project in
                    Text(ConversationBrowser.projectName(project.path, projects: store.conversationProjects)).tag(ConversationBrowser.ProjectFilter.path(project.path))
                }
                if case .path(let path) = browser.project, !store.conversationProjects.contains(where: { $0.path == path }) {
                    Text(ConversationBrowser.projectName(path, projects: store.conversationProjects)).tag(ConversationBrowser.ProjectFilter.path(path))
                }
            }
            if browser.hasFilters { Divider(); Button("清除筛选") { browser.runtimeID = nil; browser.project = .all } }
            Divider()
            Toggle("显示已归档的会话", isOn: Binding(get: { store.showsArchived }, set: { store.setShowsArchived($0) }))
            Divider()
            // Where the list itself is; the rest of the window's layout is in a tab's menu.
            Menu("布局") { ToolPlaceMenu(window: window, kind: .list) }
        } label: { Label("显示", systemImage: browser.hasFilters || store.showsArchived ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease") }
        .help("会话分组、筛选、排序、已归档会话与列表的位置")
        .accessibilityLabel("显示")
        .disabled(store.conversationBrowserPreferences == nil)
    }
}

/// The system search field, for a list outside a window's toolbar.
struct SearchField: NSViewRepresentable {
    @Binding var text: String
    let prompt: String

    func makeNSView(context: Context) -> NSSearchField {
        let field = NSSearchField()
        field.placeholderString = prompt
        field.delegate = context.coordinator
        field.controlSize = .regular
        return field
    }

    func updateNSView(_ field: NSSearchField, context: Context) {
        context.coordinator.text = $text
        if field.stringValue != text { field.stringValue = text }
    }

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    final class Coordinator: NSObject, NSSearchFieldDelegate {
        var text: Binding<String>
        init(text: Binding<String>) { self.text = text }
        func controlTextDidChange(_ notification: Notification) {
            if let field = notification.object as? NSSearchField { text.wrappedValue = field.stringValue }
        }
    }
}
