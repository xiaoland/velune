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
    @ObservedObject var store: ConversationStore
    /// The focused pane's workspace.
    @ObservedObject var focused: ConversationStore
    let placement: Placement
    /// Sharing its area with other tools, the search is a button over a popover.
    var compact = false
    @FocusState private var searchFocused: Bool
    @State private var searchFocusRequest = 0
    @Environment(\.controlActiveState) private var activeState
    @ObservedObject private var toolbarSettings = ToolbarSettings.shared
    /// The window's, so the title bar's filter menu and the list share it.
    private var browser: ConversationBrowser {
        get { window.listBrowser }
        nonmutating set { window.listBrowser = newValue }
    }
    /// Wait for the system's double-click decision before a mouse selection opens.
    @State private var pendingOpen: Task<Void, Never>?
    @StateObject private var clicks = ConversationListClicks()
    @State private var projectTarget: ProjectEditorTarget?
    /// The group whose header was last dragged; the drop reads it after the
    /// button is up, so it stays until the next drag.
    @State private var draggedGroup: ConversationGroupKey?
    /// The group dragged while the button is down, dimmed in place.
    @State private var draggingGroup: ConversationGroupKey?
    /// Where the dragged group would land if dropped now.
    @State private var groupLanding: GroupLanding?
    /// Groups moved here, shown in their new places until the core's list in
    /// the saved order arrives.
    @State private var pendingGroupMoves: [GroupMove] = []
    /// A group just moved, marked for a moment so the eye finds it.
    @State private var landedGroup: ConversationGroupKey?
    @State private var groupHeaderHeight: CGFloat = 0
    private var conversationSections: [ConversationBrowser.Section] {
        pendingGroupMoves.reduce(store.conversationSections) { sections, move in
            guard let from = sections.firstIndex(where: { $0.id == move.key }) else { return sections }
            var moved = sections
            let section = moved.remove(at: from)
            moved.insert(section, at: move.before.flatMap { before in moved.firstIndex { $0.id == before } } ?? moved.count)
            return moved
        }
    }

    var body: some View {
        Group {
            switch placement {
            case .sidebar:
                withSidebarToolbar(list
                    .searchable(text: $window.listSearch, placement: .sidebar, prompt: "搜索会话")
                    .searchFocused($searchFocused)
                    .navigationTitle("会话"))
            case .panel:
                VStack(spacing: 0) {
                    // A block has its search, filter and new conversation in its title bar.
                    if !compact {
                        HStack(spacing: 8) {
                            SearchField(text: $window.listSearch, prompt: "搜索会话", focusRequest: searchFocusRequest)
                            ConversationFilterMenu(window: window, store: store).menuStyle(.borderlessButton).fixedSize()
                            NewConversationButton(window: window).buttonStyle(.borderless)
                        }
                        .labelStyle(.iconOnly)
                        .padding(.horizontal, 10).padding(.vertical, 8)
                        Divider()
                    }
                    list
                }
            }
        }
        .background(PaneAnchor { [window] in window.registerFindArea(.list, anchor: $0) })
        .onAppear { window.listKeyHandler = listKey }
        .onDisappear { cancelPendingOpen() }
        .onChange(of: focused.selectedConversationID) { _, _ in synchronizeFocusedSelection() }
        .onChange(of: ObjectIdentifier(focused)) { _, _ in synchronizeFocusedSelection() }
        .onChange(of: window.listSearchRequest) { _, _ in
            switch placement {
            case .sidebar: searchFocused = true
            case .panel: if !compact { searchFocusRequest += 1 }
            }
        }
        .task(id: window.listSearch) {
            // Searching reads every listed conversation; wait for typing to pause.
            if !window.listSearch.isEmpty { try? await Task.sleep(for: .milliseconds(250)) }
            if !Task.isCancelled { store.setSearchText(window.listSearch) }
        }
        .onReceive(store.device.$conversationBrowserPreferences) { value in
            if let value, value != browser.preferences { pendingGroupMoves = []; browser.restore(value) }
        }
        // The list waits while choices are saved; the first one after is the saved order.
        .onReceive(store.device.$conversationGroups) { _ in pendingGroupMoves = [] }
        .onChange(of: browser.preferences) { _, value in
            if store.device.conversationBrowserPreferences != nil { store.device.setConversationBrowserPreferences(value) }
        }
        .sheet(item: $projectTarget) { ProjectEditorView(store: store, target: $0) }
    }

    /// Customizable where the window's toolbar is (see `VeluneRootView.withToolbar`).
    @ViewBuilder private func withSidebarToolbar(_ content: some View) -> some View {
        if #available(macOS 26, *) {
            content.toolbar(id: ConversationToolbar.id) {
                ToolbarItem(id: "list-filter") { ConversationFilterMenu(window: window, store: store) }
                ToolbarItem(id: "new-conversation") { NewConversationButton(window: window) }
            }
        } else {
            content.toolbar {
                ToolbarItemGroup {
                    ForEach(toolbarSettings.shown(in: .sidebar)) { button in
                        switch button {
                        case .listFilter: ConversationFilterMenu(window: window, store: store)
                        case .newConversation: NewConversationButton(window: window)
                        default: EmptyView()
                        }
                    }
                }
            }
        }
    }

    private var list: some View {
        List(selection: Binding(get: { listSelection }, set: selectList)) {
            if store.device.showsArchived {
                HStack {
                    Label("已归档的会话", systemImage: "archivebox").foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                    Button("返回") { store.device.setShowsArchived(false) }.buttonStyle(.link)
                }.selectionDisabled(true)
            }
            if conversationSections.isEmpty && !store.isLoading && !focused.isLoading {
                if store.searchResults != nil || (browser.hasFilters && !store.conversations.isEmpty) {
                    Text("没有匹配的会话").foregroundStyle(.secondary).selectionDisabled(true)
                    if browser.hasFilters { Button("清除筛选") { browser.runtimeID = nil; browser.project = .all }.selectionDisabled(true) }
                } else if store.device.showsArchived {
                    Text("没有已归档的会话").foregroundStyle(.secondary).selectionDisabled(true)
                }
            }
            ForEach(conversationSections) { section in
                // Flat rows keep group lifetime out of NSOutlineView's
                // disclosure tracking map when an archive removes the last child.
                HStack(spacing: 6) {
                    Button {
                        let expanded = expansion(section.id)
                        expanded.wrappedValue.toggle()
                    } label: {
                        Image(systemName: expansion(section.id).wrappedValue ? "chevron.down" : "chevron.right")
                            .frame(width: 12)
                    }
                    .buttonStyle(.borderless)
                    .accessibilityLabel((expansion(section.id).wrappedValue ? "收起" : "展开") + groupTitle(section.id))
                    sectionHeader(section)
                }
                .uiFont(.caption.weight(.semibold)).foregroundStyle(.secondary)
                .selectionDisabled(true)
                if expansion(section.id).wrappedValue {
                    ForEach(section.conversations) { conversation in
                        conversationRow(conversation)
                            .tag(conversation.id)
                            .background(ConversationRowClickMarker(id: conversation.id, owner: clicks))
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
                                cancelPendingOpen()
                                let id = conversation.id
                                DispatchQueue.main.async { ListDragWatch.shared.begin(id, from: window) }
                                return ConversationLink.itemProvider(runtimeID: conversation.runtimeID, conversationID: id)
                            }
                            .groupDrop(groupDropDelegate(section, .row(conversation.id)), line: groupLines(section, .row(conversation.id)))
                            .opacity(draggingGroup == section.id ? 0.4 : 1)
                    }
                    if section.hasMore {
                        if store.searchResults == nil {
                            Button("加载更多（\(section.group.loaded) / \(section.group.total)）") { store.device.loadMoreConversations(in: section.id) }
                                .buttonStyle(.plain).foregroundStyle(.secondary).selectionDisabled(true)
                                .groupDrop(groupDropDelegate(section, .more), line: groupLines(section, .more))
                                .opacity(draggingGroup == section.id ? 0.4 : 1)
                        } else {
                            Text("只显示前 \(section.group.loaded) 个匹配").foregroundStyle(.secondary).selectionDisabled(true)
                        }
                    }
                }
            }
        }
        .contextMenu(forSelectionType: String.self) { ids in conversationManagementMenu(ids) } primaryAction: { ids in
            cancelPendingOpen()
            guard ids.count == 1, let id = ids.first, let target = store.conversation(id: id), store.canManageConversations, store.canRenameConversation(target) else { return }
            window.renameTarget = target
        }
        .listStyle(.sidebar)
        .background(ListDragLimit())
        .background(ConversationListClickObserver(owner: clicks, single: tappedRow, cancel: cancelPendingOpen))
        .disabled(store.device.authenticationRunning || store.isShuttingDown)
    }

    private var listSelection: Set<String> {
        store.selectedConversationIDs.isEmpty
            ? Set([focused.selectedConversationID].compactMap { $0 }) : store.selectedConversationIDs
    }

    private func cancelPendingOpen() { pendingOpen?.cancel(); pendingOpen = nil }

    private func synchronizeFocusedSelection() {
        cancelPendingOpen()
        if store.selectedConversationIDs.count <= 1 {
            store.setListSelection(Set([focused.selectedConversationID].compactMap { $0 }))
        }
    }

    private func selectList(_ ids: Set<String>) { selectList(ids,event:NSApp.currentEvent) }

    private func selectList(_ ids: Set<String>, event: NSEvent?) {
        cancelPendingOpen()
        store.setListSelection(ids)
        guard ids.count == 1, let id = ids.first, id != focused.selectedConversationID else { return }
        let flags = event?.modifierFlags ?? NSEvent.modifierFlags
        if !flags.intersection([.command, .shift, .control]).isEmpty { return }
        let beside = flags.contains(.option)
        // Keyboard navigation is immediate; a context click or drag only selects.
        if let event, event.modifierFlags.contains(.control) || [.rightMouseDown, .rightMouseUp, .leftMouseDragged].contains(event.type) { return }
        let open = { if beside { window.openInNewTab(id) } else { window.open(id) } }
        guard let event, [.leftMouseDown, .leftMouseUp].contains(event.type) else { open(); return }
        guard event.clickCount == 1 else { return }
        scheduleMouseOpen(id, beside: beside)
    }

    private func tappedRow(_ id: String, flags: NSEvent.ModifierFlags) {
        guard window.renameTarget == nil, listSelection == [id], id != focused.selectedConversationID,
              flags.intersection([.command, .shift, .control]).isEmpty else { return }
        scheduleMouseOpen(id, beside: flags.contains(.option))
    }

    private func scheduleMouseOpen(_ id: String, beside: Bool) {
        cancelPendingOpen()
        pendingOpen = Task { @MainActor in
            do { try await Task.sleep(for: .seconds(NSEvent.doubleClickInterval)) }
            catch { return }
            // A held press may still become a drag; wait for release instead
            // of opening under the pointer while the button remains down.
            while NSEvent.pressedMouseButtons & 1 != 0 {
                do { try await Task.sleep(for: .milliseconds(25)) } catch { return }
            }
            guard !Task.isCancelled, store.selectedConversationIDs == [id],
                  !store.isShuttingDown, !store.device.authenticationRunning else { return }
            pendingOpen = nil
            if beside { window.openInNewTab(id) } else { window.open(id) }
        }
    }

    /// ⌘⌫ deletes the selected conversations (after asking, as the menu does);
    /// a character typed goes on in the search field. Return renames, as a
    /// double click does (the list's primary action).
    private func listKey(_ event: NSEvent) -> Bool {
        let modifiers = event.modifierFlags.intersection([.command, .shift, .option, .control])
        if event.keyCode == 51, modifiers == .command {
            let ids = listSelection
            let targets = ids.compactMap { store.conversation(id: $0) }
            guard !targets.isEmpty, store.canManageConversations, targets.allSatisfy(store.canDeleteConversation) else { NSSound.beep(); return true }
            window.deleteTargets = targets
            return true
        }
        guard modifiers.isSubset(of: .shift), let text = event.characters, text.count == 1, let scalar = text.unicodeScalars.first,
              !CharacterSet.whitespacesAndNewlines.union(.controlCharacters).contains(scalar), !(0xF700...0xF8FF).contains(scalar.value) else { return false }
        window.searchListTyping(text)
        return true
    }

    private func expansion(_ key: ConversationGroupKey) -> Binding<Bool> {
        Binding(get: { store.searchResults != nil || !browser.isCollapsed(key) },
                set: { expanded in if store.searchResults == nil { browser.setCollapsed(key, !expanded) } })
    }

    private func groupTitle(_ key: ConversationGroupKey) -> String {
        switch key {
        case .all: "会话"
        case .pinned: "置顶"
        case .runtime(let id): store.device.runtimeInstances.first { $0.id == id }?.name ?? "Agent 运行时"
        case .project(let path?): ConversationBrowser.projectName(path, projects: store.device.conversationProjects)
        case .project(nil): "无项目"
        }
    }

    private func sectionHeader(_ section: ConversationBrowser.Section) -> some View {
        let title = groupTitle(section.id)
        let path: String? = if case .project(let path?) = section.id { path } else { nil }
        let project = path.flatMap { key in store.device.conversationProjects.first { $0.path == key } }
        let runtimeID: String? = if case .runtime(let id) = section.id { id } else { project?.nativeRuntimeId }
        return HStack(spacing: 6) {
            Text(title).lineLimit(1).truncationMode(.middle)
            Spacer(minLength: 0)
        }
        .help(path.map { key in
            guard let project = store.device.conversationProjects.first(where: { $0.path == key }) else { return key }
            let folders = project.primaryFolder.map { [$0] + project.folders } ?? []
            return folders.isEmpty ? title : folders.map { ($0 as NSString).abbreviatingWithTildeInPath }.joined(separator: "\n")
        } ?? title)
        // Up and down over the spacing around the title in its row, so a group
        // dragged to the top of another lands before it there too.
        .contentShape(VerticalOutset(amount: 16))
        .onTapGesture(count: 2) { if let path { projectTarget = ProjectEditorTarget(path: path) } }
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { groupHeaderHeight = $0 }
        .background {
            if landedGroup == section.id {
                RoundedRectangle(cornerRadius: 5).fill(Color.accentColor.opacity(0.18)).padding(.horizontal, -6).padding(.vertical, -2)
            }
        }
        // Dragged over another group, its header or its rows, a group takes its place.
        .onDrag {
            guard canOrder(section.id) else { draggedGroup = nil; return NSItemProvider() }
            draggedGroup = section.id
            watchGroupDrag(section.id)
            return ConversationLink.groupItemProvider()
        }
        .groupDrop(groupDropDelegate(section, .header), line: groupLines(section, .header))
        .opacity(draggingGroup == section.id ? 0.4 : 1)
        .contextMenu {
            Button("新建会话") {
                cancelPendingOpen()
                window.createConversation(in: section.id)
            }
            .disabled(focused.isSideChat || focused.isLoading || !store.device.enabledRuntimeInstances.contains { runtimeID == nil || $0.id == runtimeID })
            Divider()
            if let path {
                Button("编辑项目…") { projectTarget = ProjectEditorTarget(path: path) }
                if section.group.name != nil && store.device.conversationProjects.first(where: { $0.path == path })?.nativeRuntimeId == nil {
                    Button("恢复文件夹名称") {
                        let folders = store.device.conversationProjects.first { $0.path == path }?.folders ?? []
                        store.device.setProject(path: path, primaryFolder: path, name: nil, folders: folders)
                    }
                }
                if let folder = store.device.conversationProjects.first(where: { $0.path == path })?.primaryFolder {
                    Button("在 Finder 中显示") { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: folder)]) }
                        .disabled(store.device.location(of: section.conversations.first?.runtimeID) != .local)
                }
                Divider()
            }
            if store.searchResults == nil {
                Button(browser.isCollapsed(section.id) ? "展开" : "折叠") { browser.setCollapsed(section.id, !browser.isCollapsed(section.id)) }
            }
            if canOrder(section.id) {
                let shown = orderableGroups
                let index = shown.firstIndex(of: section.id) ?? 0
                Divider()
                Button("上移") { moveGroup(section.id, before: shown[index - 1]) }.disabled(index == 0)
                Button("下移") { moveGroup(section.id, before: index + 2 < shown.count ? shown[index + 2] : nil) }
                    .disabled(index == shown.count - 1)
                Button("移到最前") { moveGroup(section.id, before: shown.first) }.disabled(index == 0)
                Button("移到最后") { moveGroup(section.id, before: nil) }.disabled(index == shown.count - 1)
            }
        }
    }

    /// Put a group before another (nil: last), in its new place at once and
    /// marked for a moment; the core's list in the saved order follows.
    private func moveGroup(_ key: ConversationGroupKey, before: ConversationGroupKey?) {
        let shown = orderableGroups
        withAnimation(.snappy(duration: 0.25)) {
            pendingGroupMoves.append(GroupMove(key: key, before: before))
            browser.move(key, before: before, shown: shown)
        }
        landedGroup = key
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(700))
            guard landedGroup == key else { return }
            withAnimation(.easeOut(duration: 0.35)) { landedGroup = nil }
        }
    }

    /// The dragged group is dimmed in place until the button is released,
    /// dropped or not. Set after the drag has taken its picture of the header.
    private func watchGroupDrag(_ key: ConversationGroupKey) {
        Task { @MainActor in
            draggingGroup = key
            while NSEvent.pressedMouseButtons & 1 != 0 {
                do { try await Task.sleep(for: .milliseconds(50)) } catch { break }
            }
            if draggingGroup == key { draggingGroup = nil; groupLanding = nil }
        }
    }

    /// Where the dragged group lands from `part` of `section`, `y` down from
    /// the part's top: before the group from the upper half of it as shown
    /// (its header and rows), after it from the lower half. The pinned group
    /// leads the list; over it the group goes first. Nil when no group is
    /// dragged that may be put in order.
    private func landing(over section: ConversationBrowser.Section, _ part: GroupDropPart, y: CGFloat) -> GroupLanding? {
        let shown = orderableGroups
        guard store.searchResults == nil, let dragged = draggedGroup, let from = shown.firstIndex(of: dragged), shown.count > 1 else { return nil }
        let before: ConversationGroupKey?
        if let index = shown.firstIndex(of: section.id) {
            let rows = expansion(section.id).wrappedValue ? section.conversations.count + (section.hasMore ? 1 : 0) : 0
            let offset: Int = switch part {
            case .header: 0
            case .row(let id): 1 + (section.conversations.firstIndex { $0.id == id } ?? 0)
            case .more: rows
            }
            let within = groupHeaderHeight > 0 ? min(max(y / groupHeaderHeight, 0), 1) : 0.5
            let upper = (CGFloat(offset) + within) / CGFloat(rows + 1) < 0.5
            before = upper ? section.id : index + 1 < shown.count ? shown[index + 1] : nil
        } else {
            before = shown.first
        }
        // Before itself or the group after it, it stays where it is.
        let moves = before != dragged && before != (from + 1 < shown.count ? shown[from + 1] : nil)
        return GroupLanding(group: section.id, part: part, before: before, moves: moves)
    }

    private func groupDropDelegate(_ section: ConversationBrowser.Section, _ part: GroupDropPart) -> GroupDropDelegate {
        GroupDropDelegate(group: section.id, part: part, landing: $groupLanding,
                          land: { landing(over: section, part, y: $0.location.y) },
                          drop: { landing in
                              groupLanding = nil
                              draggingGroup = nil
                              if landing.moves, let dragged = draggedGroup { moveGroup(dragged, before: landing.before) }
                          })
    }

    /// The insertion line `part` of `section` carries, between groups where
    /// the dragged group would land: over the header of the group it goes in
    /// front of, or under the list's last part when it goes last.
    private func groupLines(_ section: ConversationBrowser.Section, _ part: GroupDropPart) -> Edge? {
        guard let landing = groupLanding, landing.moves else { return nil }
        if case .header = part, landing.before == section.id { return .top }
        guard landing.before == nil, section.id == conversationSections.last?.id else { return nil }
        let expanded = expansion(section.id).wrappedValue
        let last: Bool = switch part {
        case .header: !expanded || (section.conversations.isEmpty && !section.hasMore)
        case .row(let id): !section.hasMore && section.conversations.last?.id == id
        case .more: true
        }
        return last ? .bottom : nil
    }

    /// The shown groups the user may put in order, as the list shows them.
    private var orderableGroups: [ConversationGroupKey] { conversationSections.map(\.id).filter(ConversationBrowser.orderable) }
    /// Groups are put in order outside a search, among at least two.
    private func canOrder(_ key: ConversationGroupKey) -> Bool {
        store.searchResults == nil && ConversationBrowser.orderable(key) && orderableGroups.count > 1
    }

    private func conversationRow(_ conversation: Conversation) -> some View {
        // Who started the turn does not matter; running does, its own or a
        // native sub-agent's that outlives it.
        let ownTurn = store.device.runningConversationIDs.contains(conversation.id) || store.device.elsewhereConversationIDs.contains(conversation.id)
        let running = ownTurn || store.device.subAgentParentIDs.contains(conversation.id)
        // An approval or question, its own or a sub-agent's, waits for the user.
        let waiting = store.device.waitingConversationIDs.contains(conversation.id)
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
            if let request = store.device.conversationManagement.first(where: { $0.conversations.contains(conversation.id) }) {
                Spacer(minLength: 0)
                Image(systemName: request.running ? "arrow.triangle.2.circlepath" : "clock")
                    .foregroundStyle(.secondary).imageScale(.small)
                    .help(request.running ? "正在\(request.source)" : "已排队：\(request.source)")
            } else if waiting {
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

    private func conversationManagementMenu(_ ids: Set<String>) -> some View {
        ConversationManagementMenu(window: window, store: store, ids: ids)
    }

}

/// A view's bounds reaching `amount` beyond its top and bottom edges.
private struct VerticalOutset: Shape {
    let amount: CGFloat
    func path(in rect: CGRect) -> Path { Path(rect.insetBy(dx: 0, dy: -amount)) }
}

/// A part of a group in the list a dragged group may be over.
enum GroupDropPart: Hashable { case header, row(String), more }

/// Where a dragged group would land: before `before` (nil: last), from the
/// part of a group it is over. Not moving it, it shows no line.
struct GroupLanding: Equatable {
    var group: ConversationGroupKey
    var part: GroupDropPart
    var before: ConversationGroupKey?
    var moves: Bool
}

/// A group moved in the list, `key` put before `before` (nil: last).
struct GroupMove: Equatable {
    var key: ConversationGroupKey
    var before: ConversationGroupKey?
}

/// A dragged group over a part of a group. A drop that leaves the order as it
/// is is still taken, so the picture does not fly back as if refused; the
/// cursor shows a move, not a copy.
struct GroupDropDelegate: DropDelegate {
    let group: ConversationGroupKey
    let part: GroupDropPart
    @Binding var landing: GroupLanding?
    let land: (DropInfo) -> GroupLanding?
    let drop: (GroupLanding) -> Void

    func validateDrop(info: DropInfo) -> Bool { land(info) != nil }
    func dropEntered(info: DropInfo) { landing = land(info) }
    func dropUpdated(info: DropInfo) -> DropProposal? {
        let next = land(info)
        if next != landing { landing = next }
        return DropProposal(operation: .move)
    }
    // The next part may be entered before this one is left.
    func dropExited(info: DropInfo) { if landing?.group == group, landing?.part == part { landing = nil } }
    func performDrop(info: DropInfo) -> Bool {
        guard let landing = land(info) else { return false }
        drop(landing)
        return true
    }
}

private extension View {
    /// Takes a dragged group, with the insertion line along `line` when it lands there.
    func groupDrop(_ delegate: GroupDropDelegate, line: Edge?) -> some View {
        onDrop(of: [ConversationLink.groupDragType], delegate: delegate)
            .overlay(alignment: line == .bottom ? .bottom : .top) {
                if line != nil { Capsule().fill(Color.accentColor).frame(height: 2).padding(.horizontal, 2).allowsHitTesting(false) }
            }
    }
}

/// Outside Velune a conversation dragged from the list is taken by nothing,
/// as a tab is: the desktop would keep whatever it carries as a file. The
/// list's drag is its table's, so the table beside this view is told so.
private struct ListDragLimit: NSViewRepresentable {
    final class Finder: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            DispatchQueue.main.async { [weak self] in self?.limit() }
        }
        private weak var table: NSTableView?
        func limit() {
            if table == nil {
                var ancestor = superview
                for _ in 0..<4 {
                    guard let view = ancestor else { break }
                    if let found = Self.table(in: view) { table = found; break }
                    ancestor = view.superview
                }
            }
            table?.setDraggingSourceOperationMask([], forLocal: false)
        }
        private static func table(in view: NSView) -> NSTableView? {
            if let table = view as? NSTableView { return table }
            for child in view.subviews { if let table = table(in: child) { return table } }
            return nil
        }
    }
    func makeNSView(context: Context) -> Finder { Finder() }
    func updateNSView(_ view: Finder, context: Context) { view.limit() }
}

/// The system search field, for a list outside a window's toolbar.
struct SearchField: NSViewRepresentable {
    @Binding var text: String
    let prompt: String
    /// The field takes the focus when this changes, and when it first appears if not 0.
    var focusRequest = 0

    func makeNSView(context: Context) -> NSSearchField {
        let field = NSSearchField()
        field.placeholderString = prompt
        field.delegate = context.coordinator
        field.controlSize = .regular
        if focusRequest != 0 { DispatchQueue.main.async { field.window?.makeFirstResponder(field) } }
        context.coordinator.focusRequest = focusRequest
        return field
    }

    func updateNSView(_ field: NSSearchField, context: Context) {
        context.coordinator.text = $text
        if field.stringValue != text { field.stringValue = text }
        if context.coordinator.focusRequest != focusRequest {
            context.coordinator.focusRequest = focusRequest
            DispatchQueue.main.async { field.window?.makeFirstResponder(field) }
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    final class Coordinator: NSObject, NSSearchFieldDelegate {
        var text: Binding<String>
        var focusRequest = 0
        init(text: Binding<String>) { self.text = text }
        func controlTextDidChange(_ notification: Notification) {
            if let field = notification.object as? NSSearchField { text.wrappedValue = field.stringValue }
        }
    }
}

/// The conversation list's grouping, filtering and sorting, in the window's
/// toolbar or the list's title bar.
struct ConversationFilterMenu: View {
    @ObservedObject var window: ConversationWindowModel
    /// The list's workspace.
    @ObservedObject var store: ConversationStore
    @State private var projectTarget: ProjectEditorTarget?

    var body: some View {
        Menu {
            Picker("分组", selection: $window.listBrowser.grouping) { ForEach(ConversationBrowser.Grouping.allCases) { Text($0.title).tag($0) } }
            if window.listBrowser.grouping != .none {
                Picker("组排序", selection: Binding(get: { window.listBrowser.groupSort }, set: { window.listBrowser.setGroupSort($0, shown: store.conversationSections.map(\.id).filter(ConversationBrowser.orderable)) })) {
                    ForEach(ConversationBrowser.GroupSort.allCases) { Text($0.title).tag($0) }
                }
            }
            Picker("会话排序", selection: $window.listBrowser.sort) { ForEach(ConversationBrowser.Sort.allCases) { Text($0.title).tag($0) } }
            Toggle("从旧到新", isOn: $window.listBrowser.oldestFirst)
            Divider()
            Picker("Agent 运行时", selection: $window.listBrowser.runtimeID) {
                Text("所有运行时").tag(Optional<String>.none)
                ForEach(store.device.enabledRuntimeInstances) { Text($0.name).tag(Optional($0.id)) }
                if let id = window.listBrowser.runtimeID, !store.device.enabledRuntimeInstances.contains(where: { $0.id == id }) {
                    Text("当前筛选的运行时不可用").tag(Optional(id))
                }
            }
            Picker("项目", selection: $window.listBrowser.project) {
                Text("所有项目").tag(ConversationBrowser.ProjectFilter.all)
                Text("无项目").tag(ConversationBrowser.ProjectFilter.unspecified)
                ForEach(store.device.conversationProjects, id: \.path) { project in
                    Text(ConversationBrowser.projectName(project.path, projects: store.device.conversationProjects)).tag(ConversationBrowser.ProjectFilter.path(project.path))
                }
                if case .path(let path) = window.listBrowser.project, !store.device.conversationProjects.contains(where: { $0.path == path }) {
                    Text(ConversationBrowser.projectName(path, projects: store.device.conversationProjects)).tag(ConversationBrowser.ProjectFilter.path(path))
                }
            }
            if !store.device.conversationProjects.isEmpty {
                Menu("编辑项目") {
                    ForEach(store.device.conversationProjects, id: \.path) { project in
                        Button(ConversationBrowser.projectName(project.path, projects: store.device.conversationProjects)) {
                            projectTarget = ProjectEditorTarget(path: project.path)
                        }
                    }
                }
            }
            if window.listBrowser.hasFilters { Divider(); Button("清除筛选") { window.listBrowser.runtimeID = nil; window.listBrowser.project = .all } }
            Divider()
            Toggle("显示已归档的会话", isOn: Binding(get: { store.device.showsArchived }, set: { store.device.setShowsArchived($0) }))
            Divider()
            // Where the list itself is; the rest of the window's layout is in a tab's menu.
            Menu("布局") { ToolPlaceMenu(window: window, kind: .list) }
        } label: { Label("显示", systemImage: window.listBrowser.hasFilters || store.device.showsArchived ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease") }
        .help("会话分组、筛选、排序、已归档会话与列表的位置")
        .sheet(item: $projectTarget) { ProjectEditorView(store: store, target: $0) }
        .accessibilityLabel("显示")
        .disabled(store.device.conversationBrowserPreferences == nil)
    }
}

struct NewConversationButton: View {
    @ObservedObject var window: ConversationWindowModel

    var body: some View {
        Button { window.createConversation() } label: { Label("新建会话", systemImage: "square.and.pencil") }
            .help(KeyboardShortcutSettings.shared.help("新建会话", .newConversation)).disabled(window.application.enabledRuntimeInstances.isEmpty)
    }
}

/// Search over a popover, for the list's title bar. Marked while a search filters the list.
struct ConversationSearchButton: View {
    @ObservedObject var window: ConversationWindowModel

    var body: some View {
        Button { window.listSearchPopover.toggle() } label: { Image(systemName: window.listSearch.isEmpty ? "magnifyingglass" : "magnifyingglass.circle.fill") }
            .buttonStyle(.borderless).help("搜索会话").accessibilityLabel("搜索会话")
            .popover(isPresented: $window.listSearchPopover, arrowEdge: .bottom) {
                SearchField(text: $window.listSearch, prompt: "搜索会话", focusRequest: 1).frame(width: 240).padding(10)
            }
    }
}

/// What the list adds to its title bar when it is a block among other tools:
/// search and filter, and, where the window's toolbar is not beside it, a new conversation.
struct ConversationListHeaderActions: View {
    @ObservedObject var window: ConversationWindowModel
    let side: DockSide?

    var body: some View {
        ConversationSearchButton(window: window)
        ConversationFilterMenu(window: window, store: window.list).menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().labelStyle(.iconOnly)
        if side != .left { NewConversationButton(window: window).buttonStyle(.borderless).labelStyle(.iconOnly) }
    }
}

extension ConversationStore {
    /// The list's groups as shown, searching or not.
    var conversationSections: [ConversationBrowser.Section] {
        if let results = searchResults { return ConversationBrowser.sections(conversations: results.conversations, groups: results.groups) }
        return ConversationBrowser.sections(conversations: conversations, groups: device.conversationGroups)
    }
}

/// The window toolbar's new-conversation button at the top of a sidebar that
/// holds the list as a block among other tools, where the list's own
/// toolbar is not there to carry it.
struct ListBlockToolbar: ViewModifier {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var focused: ConversationStore
    @ObservedObject private var toolbarSettings = ToolbarSettings.shared

    @ViewBuilder func body(content: Content) -> some View {
        if window.listIsBlock, window.tools[.list].place.side == .left {
            if #available(macOS 26, *) {
                content.toolbar(id: ConversationToolbar.id) { ToolbarItem(id: "new-conversation") { NewConversationButton(window: window) } }
            } else if toolbarSettings.isShown(.newConversation) {
                content.toolbar { ToolbarItem { NewConversationButton(window: window) } }
            } else {
                content
            }
        } else {
            content
        }
    }
}
