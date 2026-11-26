import AppKit
import SwiftUI
import VeluneBindings

/// A conversation's row identity in the aggregated list: its device and the
/// conversation, so two devices' conversations never share a list row.
struct AggregatedRowID: Hashable {
    let section: String
    let conversation: String
    func belongs(to deviceID: String, conversations: Set<String>) -> Bool {
        section.hasPrefix(deviceID + "‖") && conversations.contains(conversation)
    }
}

/// A window's conversation list, the same wherever it is: its search field,
/// filter and new conversation are its own, never the window toolbar's. The list aggregates every device the window shows:
/// each section belongs to one device, and opening a row targets that
/// device. A conversation selected alone opens in the window's focused pane;
/// several are a selection to manage.
struct ConversationListView: View {
    @ObservedObject var window: ConversationWindowModel
    /// The list's workspace on this Mac (problems, local loading state).
    @ObservedObject var store: ConversationStore
    /// The focused pane's workspace.
    @ObservedObject var focused: ConversationStore
    /// The aggregated list: devices, merged groups, the merged view's choices.
    @ObservedObject private var aggregated: AggregatedListModel
    @State private var searchFocusRequest = 0
    @Environment(\.controlActiveState) private var activeState
    /// The window's, so the title bar's filter menu and the list share it.
    private var browser: ConversationBrowser {
        get { window.listBrowser }
        nonmutating set { window.listBrowser = newValue }
    }
    /// Whether the merged "全部设备" view shows: its folding and ordering are
    /// this Mac's own; a single-device view uses that device's preferences.
    private var mergedChoices: Bool { aggregated.filter == .all }
    /// Wait for the system's double-click decision before a mouse selection opens.
    @State private var pendingOpen: Task<Void, Never>?
    @State private var pendingOpenPress: (row: AggregatedRowID, time: TimeInterval)?
    @StateObject private var clicks = ConversationListClicks()
    @State private var projectTarget: ProjectEditorTarget?
    /// The group whose header was last dragged; the drop reads it after the
    /// button is up, so it stays until the next drag.
    @State private var draggedGroup: String?
    /// The group dragged while the button is down, dimmed in place.
    @State private var draggingGroup: String?
    /// Where the dragged group would land if dropped now.
    @State private var groupLanding: GroupLanding?
    /// Groups moved here, shown in their new places until the read list in
    /// the saved order arrives.
    @State private var pendingGroupMoves: [GroupMove] = []
    /// A group just moved, marked for a moment so the eye finds it.
    @State private var landedGroup: String?
    @State private var groupHeaderHeight: CGFloat = 0
    /// The devices waiting for a connection, shown with their state instead of groups.
    private var pendingDevices: [AggregatedListModel.Device] { aggregated.pendingDevices() }
    private var sections: [AggregatedListModel.Section] {
        pendingGroupMoves.reduce(aggregated.sections) { sections, move in
            guard let from = sections.firstIndex(where: { $0.id == move.key }) else { return sections }
            var moved = sections
            let section = moved.remove(at: from)
            moved.insert(section, at: move.before.flatMap { before in moved.firstIndex { $0.id == before } } ?? moved.count)
            return moved
        }
    }
    @State private var selection: Set<AggregatedRowID> = []

    init(window: ConversationWindowModel, store: ConversationStore, focused: ConversationStore) {
        self.window = window
        self.store = store
        self.focused = focused
        self.aggregated = window.aggregated
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                SearchField(text: $window.listSearch, prompt: "搜索会话", focusRequest: searchFocusRequest)
                    .frame(maxWidth: .infinity)
                ConversationFilterMenu(window: window, store: store).menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
                    .background(PaneAnchor { anchor in anchor()?.identifier = NSUserInterfaceItemIdentifier("conversation-list-filter") })
                NewConversationButton(window: window).buttonStyle(.borderless)
                    .background(PaneAnchor { anchor in anchor()?.identifier = NSUserInterfaceItemIdentifier("conversation-list-new") })
            }
            .labelStyle(.iconOnly)
            .padding(.horizontal, 10).padding(.top, 6).padding(.bottom, 4)
            list
        }
        .background(PaneAnchor { [window] in window.registerFindArea(.list, anchor: $0) })
        .onAppear { window.listKeyHandler = listKey; synchronizeFocusedSelection() }
        .onDisappear { cancelPendingOpen() }
        .onChange(of: focused.selectedConversationID) { _, _ in synchronizeFocusedSelection() }
        .onChange(of: ObjectIdentifier(focused)) { _, _ in synchronizeFocusedSelection() }
        .onChange(of: window.listSearchRequest) { _, _ in searchFocusRequest += 1 }
        .task(id: window.listSearch) {
            // Searching reads every listed conversation; wait for typing to pause.
            if !window.listSearch.isEmpty { try? await Task.sleep(for: .milliseconds(250)) }
            if !Task.isCancelled { aggregated.setSearchText(window.listSearch) }
        }
        // The filter menu edits the filtered device's own preferences, as a
        // single-device window always did; the merged view only merges them.
        .onReceive(filteredDevice.store.$conversationBrowserPreferences) { value in
            if let value, value != browser.preferences { pendingGroupMoves = []; browser.restore(value) }
        }
        .onReceive(aggregated.$filter) { _ in
            pendingGroupMoves = []
            if let value = filteredDevice.store.conversationBrowserPreferences, value != browser.preferences { browser.restore(value) }
        }
        // The list waits while choices are saved; the first one after is the saved order.
        .onReceive(aggregated.$sections) { _ in if !mergedChoices { pendingGroupMoves = [] } }
        .onReceive(aggregated.managedConversations) { result in
            cancelPendingOpen()
            selection = selection.filter { !$0.belongs(to: result.deviceID, conversations: result.conversations) }
            let grouped = selectedByDevice(selection)
            for device in aggregated.visibleDevices {
                aggregated.listStore(for: device).setListSelection(grouped[device.id] ?? [])
            }
        }
        .onChange(of: browser.preferences) { _, value in
            if filteredDevice.store.conversationBrowserPreferences != nil { filteredDevice.store.setConversationBrowserPreferences(value) }
        }
        .sheet(item: $projectTarget) { target in
            ProjectEditorView(store: target.store, target: target)
        }
    }

    /// The device the filter menu edits: the filtered one, or this Mac in the merged view.
    private var filteredDevice: AggregatedListModel.Device {
        switch aggregated.filter {
        case .peer(let id): aggregated.devices.first { $0.peerID == id } ?? aggregated.devices[0]
        default: aggregated.devices[0]
        }
    }

    private var list: some View {
        let failures = aggregated.searchFailures
        let pending = pendingDevices.filter { device in !failures.contains { $0.device.id == device.id } }
        return List(selection: Binding(get: { selection }, set: selectList)) {
            if store.device.showsArchived {
                HStack {
                    Label("已归档的会话", systemImage: "archivebox").foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                    Button("返回") { store.device.setShowsArchived(false) }.buttonStyle(.link)
                }.selectionDisabled(true)
            }
            ForEach(failures) { failure in
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 6) {
                        Label(aggregated.showsMultipleDevices ? "无法搜索\(failure.device.name)" : "无法搜索会话",
                              systemImage: "exclamationmark.circle").lineLimit(1)
                        Spacer(minLength: 0)
                        if failure.retrying { ProgressView().controlSize(.mini).accessibilityLabel("正在重试搜索") }
                        Button("重试") { aggregated.retrySearch(on: failure.device) }
                            .buttonStyle(.borderless).disabled(failure.retrying)
                            .accessibilityLabel("重试搜索\(failure.device.name)")
                    }
                    Text(failure.detail).lineLimit(2).help(failure.detail)
                }.uiFont(.caption).foregroundStyle(.secondary).selectionDisabled(true)
            }
            if !searchActive {
                ForEach(aggregated.visibleDevices) { device in
                    if let error = device.store.conversationListReadError {
                        ReadFailureNotice(title: aggregated.showsMultipleDevices ? "无法更新\(device.name)的会话" : "无法更新会话列表", detail: error) {
                            device.store.refreshConversations()
                        }.selectionDisabled(true)
                    }
                }
            }
            if sections.isEmpty && pending.isEmpty && !store.isLoading && !focused.isLoading {
                if searchActive && aggregated.searchLoading && failures.isEmpty {
                    ProgressView("正在搜索会话…").controlSize(.small).selectionDisabled(true)
                } else if (searchActive && failures.isEmpty) || (!searchActive && browser.hasFilters && !filteredDevice.store.conversations.isEmpty) {
                    Text("没有匹配的会话").foregroundStyle(.secondary).selectionDisabled(true)
                    if browser.hasFilters { Button("清除筛选") { browser.runtimeID = nil; browser.project = .all }.selectionDisabled(true) }
                } else if !searchActive && store.device.showsArchived {
                    Text("没有已归档的会话").foregroundStyle(.secondary).selectionDisabled(true)
                }
            }
            if aggregated.showsMultipleDevices || aggregated.filter != .all {
                ForEach(pending) { device in
                    DeviceStatusRow(name: device.name, status: DeviceHub.shared?.statusText(for: device.peerID ?? "") ?? "未连接") {
                        aggregated.retry(device)
                    }
                    .selectionDisabled(true)
                }
            }
            ForEach(sections) { section in
                // Flat rows keep group lifetime out of NSOutlineView's
                // disclosure tracking map when an archive removes the last child.
                if aggregated.showsHeader(section) {
                HStack(spacing: 6) {
                    Button {
                        let expanded = expansion(section)
                        expanded.wrappedValue.toggle()
                    } label: {
                        Image(systemName: expansion(section).wrappedValue ? "chevron.down" : "chevron.right")
                            .frame(width: 12)
                    }
                    .buttonStyle(.borderless)
                    .accessibilityLabel((expansion(section).wrappedValue ? "收起" : "展开") + sectionTitle(section))
                    sectionHeader(section)
                }
                .uiFont(.caption.weight(.semibold)).foregroundStyle(.secondary)
                .selectionDisabled(true)
                }
                if expansion(section).wrappedValue {
                    ForEach(section.conversations) { conversation in
                        let row = AggregatedRowID(section: section.id, conversation: conversation.id)
                        conversationRow(section: section, conversation: conversation)
                            .help(section.crossProjectID == nil ? conversation.title : "\(conversation.title)\n执行设备：\(section.device.name)")
                            .tag(row)
                            .background(ConversationRowClickMarker(id: row, owner: clicks))
                            // Always selectable, also while the conversation pressed on
                            // opens: the rows do not change under the press.
                            .contextMenu { conversationManagementMenu(selection.isEmpty ? [] : selection) }
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
                        if !searchActive {
                            ConversationPaginationButton(loaded: section.group.loaded, total: section.group.total,
                                isLoading: section.device.store.loadingConversationGroups.contains(section.key),
                                sourceName: section.crossProjectID == nil ? nil : section.device.name,
                                error: section.device.store.conversationGroupReadErrors[section.key],
                                load: { section.device.store.loadMoreConversations(in: section.key) })
                                .selectionDisabled(true)
                                .groupDrop(groupDropDelegate(section, .more), line: groupLines(section, .more))
                                .opacity(draggingGroup == section.id ? 0.4 : 1)
                        } else {
                            Text("只显示前 \(section.group.loaded) 个匹配").foregroundStyle(.secondary).selectionDisabled(true)
                        }
                    }
                }
            }
        }
        .contextMenu(forSelectionType: AggregatedRowID.self) { ids in conversationManagementMenu(ids) } primaryAction: { ids in
            cancelPendingOpen()
            guard ids.count == 1, let row = ids.first, let section = section(for: row),
                  let target = section.conversations.first(where: { $0.id == row.conversation }),
                  facadeStore(for: section).canManageConversations, facadeStore(for: section).canRenameConversation(target) else { return }
            window.renameTarget = .init(conversation: target, device: section.device.store)
        }
        .listStyle(.sidebar)
        .background(ListDragLimit())
        .background(ConversationListClickObserver(owner: clicks, single: tappedRow, cancel: cancelPendingOpen))
        .disabled(filteredDevice.store.authenticationRunning || store.isShuttingDown)
    }

    private var searchActive: Bool { !aggregated.searchText.isEmpty }

    private func cancelPendingOpen() { pendingOpen?.cancel(); pendingOpen = nil; pendingOpenPress = nil }

    private func synchronizeFocusedSelection() {
        cancelPendingOpen()
        guard selection.count <= 1, let id = focused.selectedConversationID else { return }
        guard let section = sections.first(where: { $0.device.store === focused.device && $0.conversations.contains(where: { $0.id == id }) }) else { return }
        selection = [AggregatedRowID(section: section.id, conversation: id)]
    }

    private func section(for row: AggregatedRowID) -> AggregatedListModel.Section? {
        sections.first { $0.id == row.section && $0.conversations.contains { $0.id == row.conversation } }
    }

    private func selectList(_ ids: Set<AggregatedRowID>) {
        cancelPendingOpen()
        selection = ids
        let grouped = selectedByDevice(ids)
        // Keep each device's own list selection in step, for its management requests.
        for device in aggregated.visibleDevices {
            aggregated.listStore(for: device).setListSelection(grouped[device.id] ?? [])
        }
        guard ids.count == 1, let row = ids.first, let section = section(for: row),
              focused.device !== section.device.store || row.conversation != focused.selectedConversationID else { return }
        let event = NSApp.currentEvent
        let flags = event?.modifierFlags ?? NSEvent.modifierFlags
        if !flags.intersection([.command, .shift, .control]).isEmpty { return }
        let beside = flags.contains(.option)
        // Keyboard navigation is immediate; a context click or drag only selects.
        if let event, event.modifierFlags.contains(.control) || [.rightMouseDown, .rightMouseUp, .leftMouseDragged].contains(event.type) { return }
        let device = section.device.store
        let open = { if beside { window.openInNewTab(row.conversation, device: device) } else { window.open(row.conversation, device: device) } }
        guard let event, [.leftMouseDown, .leftMouseUp].contains(event.type) else { open(); return }
        guard event.clickCount == 1 else { return }
        scheduleMouseOpen(row, beside: beside, pressedAt: event.timestamp)
    }

    private func tappedRow(_ row: AggregatedRowID, flags: NSEvent.ModifierFlags, pressedAt: TimeInterval) {
        guard window.renameTarget == nil, selection == [row], let section = section(for: row),
              focused.device !== section.device.store || row.conversation != focused.selectedConversationID,
              flags.intersection([.command, .shift, .control]).isEmpty else { return }
        scheduleMouseOpen(row, beside: flags.contains(.option), pressedAt: pressedAt)
    }

    private func scheduleMouseOpen(_ row: AggregatedRowID, beside: Bool, pressedAt: TimeInterval) {
        let pressedAt = pendingOpenPress?.row == row ? min(pendingOpenPress!.time, pressedAt) : pressedAt
        cancelPendingOpen()
        pendingOpenPress = (row, pressedAt)
        pendingOpen = Task { @MainActor in
            // NSTableView's tracking can hold the main run loop. Count the
            // double-click interval from the actual press, never from its
            // delayed SwiftUI selection callback or duplicate native observer.
            let remaining = ConversationListClicks.openDelay(pressedAt: pressedAt)
            do { if remaining > 0 { try await Task.sleep(for: .seconds(remaining)) } }
            catch { return }
            // A held press may still become a drag; wait for release instead
            // of opening under the pointer while the button remains down.
            while NSEvent.pressedMouseButtons & 1 != 0 {
                do { try await Task.sleep(for: .milliseconds(25)) } catch { return }
            }
            guard !Task.isCancelled, selection == [row], let device = section(for: row)?.device.store,
                  !device.isShuttingDown, !device.authenticationRunning else { return }
            pendingOpen = nil
            pendingOpenPress = nil
            if beside { window.openInNewTab(row.conversation, device: device) } else { window.open(row.conversation, device: device) }
        }
    }

    /// ⌘⌫ deletes the selected conversations (after asking, as the menu does);
    /// a character typed goes on in the search field. Return renames, as a
    /// double click does (the list's primary action).
    private func listKey(_ event: NSEvent) -> Bool {
        let modifiers = event.modifierFlags.intersection([.command, .shift, .option, .control])
        if event.keyCode == 51, modifiers == .command {
            window.deleteTargets = selection.compactMap { row -> ConversationWindowModel.ConversationActionTarget? in
                guard let section = section(for: row),
                      let conversation = section.conversations.first(where: { $0.id == row.conversation }),
                      facadeStore(for: section).canDeleteConversation(conversation) else { return nil }
                return .init(conversation: conversation, device: section.device.store)
            }
            return !window.deleteTargets.isEmpty
        }
        guard modifiers.isSubset(of: .shift), let text = event.characters, text.count == 1, let scalar = text.unicodeScalars.first,
              !CharacterSet.whitespacesAndNewlines.union(.controlCharacters).contains(scalar), !(0xF700...0xF8FF).contains(scalar.value) else { return false }
        window.searchListTyping(text)
        return true
    }

    private func expansion(_ section: AggregatedListModel.Section) -> Binding<Bool> {
        if mergedChoices || section.crossProjectID != nil {
            return Binding(get: { searchActive || !aggregated.isCollapsed(section) },
                           set: { expanded in if !searchActive { aggregated.setCollapsed(section, !expanded) } })
        }
        return Binding(get: { searchActive || !browser.isCollapsed(section.key) },
                       set: { expanded in if !searchActive { browser.setCollapsed(section.key, !expanded) } })
    }

    private func sectionTitle(_ section: AggregatedListModel.Section) -> String {
        section.displayTitle(multipleDevices: aggregated.showsMultipleDevices)
    }

    @ViewBuilder private func sectionHeader(_ section: AggregatedListModel.Section) -> some View {
        if let id = section.crossProjectID, let project = aggregated.crossProjects.projects.first(where: { $0.id == id }) {
            Text(project.name).lineLimit(1).help(project.name)
                .contextMenu {
                    ForEach(aggregated.devices) { device in
                        if let folder = project.folders.first(where: { $0.peerID == device.peerID }) {
                            Button("在 \(device.name) 中新建会话") {
                                _ = window.openNewConversationTab(device: device.store, runtimeID: nil, cwd: folder.path, project: nil)
                            }
                        }
                    }
                }
        } else { sourceSectionHeader(section) }
    }

    private func sourceSectionHeader(_ section: AggregatedListModel.Section) -> some View {
        let title = sectionTitle(section)
        let path: String? = if case .project(let path?) = section.key { path } else { nil }
        let projects = section.device.store.conversationProjects
        let project = path.flatMap { key in projects.first { $0.path == key } }
        let runtimeID: String? = if case .runtime(let id) = section.key { id } else { project?.nativeRuntimeId }
        let device = section.device.store
        return HStack(spacing: 6) {
            Text(title).lineLimit(1).truncationMode(.middle)
            Spacer(minLength: 0)
        }
        .help(path.map { key in
            guard let project = projects.first(where: { $0.path == key }) else { return key }
            let folders = project.primaryFolder.map { [$0] + project.folders } ?? []
            return folders.isEmpty ? title : folders.map { ($0 as NSString).abbreviatingWithTildeInPath }.joined(separator: "\n")
        } ?? title)
        // Up and down over the spacing around the title in its row, so a group
        // dragged to the top of another lands before it there too.
        .contentShape(VerticalOutset(amount: 16))
        .onTapGesture(count: 2) {
            if let path {
                projectTarget = ProjectEditorTarget(path: path, store: window.listStore(for: device))
            }
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { groupHeaderHeight = $0 }
        .background {
            if landedGroup == section.id {
                RoundedRectangle(cornerRadius: 5).fill(Color.accentColor.opacity(0.18)).padding(.horizontal, -6).padding(.vertical, -2)
            }
        }
        // Dragged over another group, its header or its rows, a group takes its place.
        .onDrag {
            guard canOrder(section) else { draggedGroup = nil; return NSItemProvider() }
            draggedGroup = section.id
            watchGroupDrag(section.id)
            return ConversationLink.groupItemProvider()
        }
        .groupDrop(groupDropDelegate(section, .header), line: groupLines(section, .header))
        .opacity(draggingGroup == section.id ? 0.4 : 1)
        .contextMenu {
            Button("新建会话") {
                cancelPendingOpen()
                window.createConversation(in: section.key, device: device)
            }
            .disabled(focused.isSideChat || focused.isLoading || !device.enabledRuntimeInstances.contains { runtimeID == nil || $0.id == runtimeID })
            Divider()
            if let path {
                Button("编辑项目…") {
                    projectTarget = ProjectEditorTarget(path: path, store: window.listStore(for: device))
                }
                if section.group.name != nil && projects.first(where: { $0.path == path })?.nativeRuntimeId == nil {
                    Button("恢复文件夹名称") {
                        let folders = projects.first { $0.path == path }?.folders ?? []
                        device.setProject(path: path, primaryFolder: path, name: nil, folders: folders)
                    }
                }
                if let folder = projects.first(where: { $0.path == path })?.primaryFolder {
                    Button("在 Finder 中显示") { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: folder)]) }
                        .disabled(device.peerID != nil || device.location(of: section.conversations.first?.runtimeID) != .local)
                }
                Divider()
            }
            if !searchActive {
                let collapsedNow = mergedChoices ? aggregated.isCollapsed(section) : browser.isCollapsed(section.key)
                Button(collapsedNow ? "展开" : "折叠") {
                    if mergedChoices { aggregated.setCollapsed(section, !collapsedNow) }
                    else { browser.setCollapsed(section.key, !collapsedNow) }
                }
            }
            if canOrder(section) {
                let shown = orderableGroups
                let index = shown.firstIndex(of: section.id) ?? 0
                Divider()
                Button("上移") { moveGroup(section, before: shown[index - 1]) }.disabled(index == 0)
                Button("下移") { moveGroup(section, before: index + 2 < shown.count ? shown[index + 2] : nil) }
                    .disabled(index == shown.count - 1)
                Button("移到最前") { moveGroup(section, before: shown.first) }.disabled(index == 0)
                Button("移到最后") { moveGroup(section, before: nil) }.disabled(index == shown.count - 1)
            }
        }
    }

    /// Put a group before another (nil: last), in its new place at once and
    /// marked for a moment; the saved order's list follows.
    private func moveGroup(_ section: AggregatedListModel.Section, before target: String?) {
        withAnimation(.snappy(duration: 0.25)) {
            if mergedChoices {
                // The merged model reorders its sections at once and persists the order.
                aggregated.move(section, before: target.flatMap { id in sections.first(where: { $0.id == id }) })
            } else {
                pendingGroupMoves.append(GroupMove(key: section.id, before: target))
                let shownKeys = orderableGroups.compactMap { id in sections.first(where: { $0.id == id })?.key }
                let targetKey = target.flatMap { id in sections.first(where: { $0.id == id })?.key }
                browser.move(section.key, before: targetKey, shown: shownKeys)
            }
        }
        landedGroup = section.id
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(700))
            guard landedGroup == section.id else { return }
            withAnimation(.easeOut(duration: 0.35)) { landedGroup = nil }
        }
    }

    /// The dragged group is dimmed in place until the button is released,
    /// dropped or not. Set after the drag has taken its picture of the header.
    private func watchGroupDrag(_ key: String) {
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
    private func landing(over section: AggregatedListModel.Section, _ part: GroupDropPart, y: CGFloat) -> GroupLanding? {
        let shown = orderableGroups
        guard !searchActive, let dragged = draggedGroup, let from = shown.firstIndex(of: dragged), shown.count > 1 else { return nil }
        let before: String?
        if let index = shown.firstIndex(of: section.id) {
            let rows = expansion(section).wrappedValue ? section.conversations.count + (section.hasMore ? 1 : 0) : 0
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

    private func groupDropDelegate(_ section: AggregatedListModel.Section, _ part: GroupDropPart) -> GroupDropDelegate {
        GroupDropDelegate(group: section.id, part: part, landing: $groupLanding,
                          land: { landing(over: section, part, y: $0.location.y) },
                          drop: { landing in
                              groupLanding = nil
                              draggingGroup = nil
                              if landing.moves, let dragged = draggedGroup, let source = sections.first(where: { $0.id == dragged }) {
                                  moveGroup(source, before: landing.before)
                              }
                          })
    }

    /// The insertion line `part` of `section` carries, between groups where
    /// the dragged group would land: over the header of the group it goes in
    /// front of, or under the list's last part when it goes last.
    private func groupLines(_ section: AggregatedListModel.Section, _ part: GroupDropPart) -> Edge? {
        guard let landing = groupLanding, landing.moves else { return nil }
        if case .header = part, landing.before == section.id { return .top }
        guard landing.before == nil, section.id == sections.last?.id else { return nil }
        let expanded = expansion(section).wrappedValue
        let last: Bool = switch part {
        case .header: !expanded || (section.conversations.isEmpty && !section.hasMore)
        case .row(let id): !section.hasMore && section.conversations.last?.id == id
        case .more: true
        }
        return last ? .bottom : nil
    }

    /// Candidate groups in display order. Whether reordering is enabled also depends on search and count.
    private var orderableGroups: [String] {
        sections.filter { ConversationBrowser.orderable($0.key) }.map(\.id)
    }
    /// Groups are put in order outside a search, among at least two.
    private func canOrder(_ section: AggregatedListModel.Section) -> Bool {
        !searchActive && ConversationBrowser.orderable(section.key) && orderableGroups.count > 1
    }

    private func facadeStore(for section: AggregatedListModel.Section) -> ConversationStore {
        aggregated.listStore(for: section.device)
    }

    private func conversationRow(section: AggregatedListModel.Section, conversation: Conversation) -> some View {
        let device = section.device.store
        // Who started the turn does not matter; running does, its own or a
        // native sub-agent's that outlives it.
        let ownTurn = device.runningConversationIDs.contains(conversation.id) || device.elsewhereConversationIDs.contains(conversation.id)
        let running = ownTurn || device.subAgentParentIDs.contains(conversation.id)
        // An approval or question, its own or a sub-agent's, waits for the user.
        let waiting = device.waitingConversationIDs.contains(conversation.id)
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
            Text(conversation.title).uiFont(.body.weight(unread ? .semibold : .regular)).lineLimit(1).help(conversation.title)
            if let request = device.conversationManagement.first(where: { $0.conversations.contains(conversation.id) }) {
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

    /// Selections spanning devices manage each device's conversations with
    /// its own core; the menu names the device when several take part.
    private func selectedByDevice(_ rows: Set<AggregatedRowID>) -> [String: Set<String>] {
        var result: [String: Set<String>] = [:]
        for row in rows {
            if let section = section(for: row) {
                result[section.device.id, default: []].insert(row.conversation)
            }
        }
        return result
    }

    private func conversationManagementMenu(_ rows: Set<AggregatedRowID>) -> some View {
        let grouped = selectedByDevice(rows)
        let groups = aggregated.visibleDevices.compactMap { device -> (device: AggregatedListModel.Device, ids: Set<String>)? in
            grouped[device.id].map { (device, $0) }
        }
        return Group {
            ForEach(Array(groups.enumerated()), id: \.offset) { index, group in
                if groups.count > 1 {
                    Text(group.device.name).uiFont(.headline).foregroundStyle(.secondary)
                }
                ConversationManagementMenu(window: window, store: aggregated.listStore(for: group.device), ids: group.ids)
                if index < groups.count - 1 { Divider() }
            }
        }
    }
}

/// A paired device that has no conversation list yet: its connection state
/// and a retry, in place of its groups.
private struct DeviceStatusRow: View {
    let name: String
    let status: String
    let retry: () -> Void
    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "desktopcomputer").frame(width: 12).accessibilityHidden(true)
            Text(name).lineLimit(1)
            Spacer(minLength: 0)
            Text(status).foregroundStyle(.secondary)
            Button("重试", action: retry).buttonStyle(.borderless).foregroundStyle(Color.accentColor)
        }
        .uiFont(.caption.weight(.semibold)).foregroundStyle(.secondary)
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
    var group: String
    var part: GroupDropPart
    var before: String?
    var moves: Bool
}

/// A group moved in the list, `key` put before `before` (nil: last).
struct GroupMove: Equatable {
    var key: String
    var before: String?
}

/// A dragged group over a part of a group. A drop that leaves the order as it
/// is is still taken, so the picture does not fly back as if refused; the
/// cursor shows a move, not a copy.
struct GroupDropDelegate: DropDelegate {
    let group: String
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
/// toolbar or the list's title bar. The device picker chooses which device's
/// conversations fill the list; the rest of the menu edits the filtered
/// device's own preferences, as a single-device window always did.
struct ConversationFilterMenu: View {
    @ObservedObject var window: ConversationWindowModel
    /// The list's workspace.
    @ObservedObject var store: ConversationStore
    @State private var projectTarget: ProjectEditorTarget?
    @State private var crossProjectsPresented = false
    @ObservedObject private var aggregated: AggregatedListModel

    init(window: ConversationWindowModel, store: ConversationStore) {
        self.window = window
        self.store = store
        self.aggregated = window.aggregated
    }

    /// The device the grouping menu edits: the filtered one, or this Mac.
    private var device: AggregatedListModel.Device {
        switch aggregated.filter {
        case .peer(let id): aggregated.devices.first { $0.peerID == id } ?? aggregated.devices[0]
        default: aggregated.devices[0]
        }
    }

    var body: some View {
        Menu {
            Picker("设备", selection: deviceFilterBinding) {
                ForEach(aggregated.filterChoices, id: \.self) { choice in
                    Text(aggregated.name(for: choice)).tag(choice)
                }
            }
            Divider()
            Picker("分组", selection: $window.listBrowser.grouping) { ForEach(ConversationBrowser.Grouping.allCases) { Text($0.title).tag($0) } }
            if window.listBrowser.grouping != .none {
                Picker("组排序", selection: Binding(get: { window.listBrowser.groupSort }, set: { window.listBrowser.setGroupSort($0, shown: aggregated.sections.map(\.key).filter(ConversationBrowser.orderable)) })) {
                    ForEach(ConversationBrowser.GroupSort.allCases) { Text($0.title).tag($0) }
                }
            }
            Picker("会话排序", selection: $window.listBrowser.sort) { ForEach(ConversationBrowser.Sort.allCases) { Text($0.title).tag($0) } }
            Toggle("从旧到新", isOn: $window.listBrowser.oldestFirst)
            Divider()
            Picker("Agent 运行时", selection: $window.listBrowser.runtimeID) {
                Text("所有运行时").tag(Optional<String>.none)
                ForEach(device.store.enabledRuntimeInstances) { Text($0.name).tag(Optional($0.id)) }
                if let id = window.listBrowser.runtimeID, !device.store.enabledRuntimeInstances.contains(where: { $0.id == id }) {
                    Text("当前筛选的运行时不可用").tag(Optional(id))
                }
            }
            Picker("项目", selection: $window.listBrowser.project) {
                Text("所有项目").tag(ConversationBrowser.ProjectFilter.all)
                Text("无项目").tag(ConversationBrowser.ProjectFilter.unspecified)
                ForEach(device.store.conversationProjects, id: \.path) { project in
                    Text(ConversationBrowser.projectName(project.path, projects: device.store.conversationProjects)).tag(ConversationBrowser.ProjectFilter.path(project.path))
                }
                if case .path(let path) = window.listBrowser.project, !device.store.conversationProjects.contains(where: { $0.path == path }) {
                    Text(ConversationBrowser.projectName(path, projects: device.store.conversationProjects)).tag(ConversationBrowser.ProjectFilter.path(path))
                }
            }
            Button("跨设备项目…") { crossProjectsPresented = true }
            if !device.store.conversationProjects.isEmpty {
                Menu("编辑项目") {
                    ForEach(device.store.conversationProjects, id: \.path) { project in
                        Button(ConversationBrowser.projectName(project.path, projects: device.store.conversationProjects)) {
                            projectTarget = ProjectEditorTarget(path: project.path, store: window.listStore(for: device.store))
                        }
                    }
                }
            }
            if window.listBrowser.hasFilters { Divider(); Button("清除筛选") { window.listBrowser.runtimeID = nil; window.listBrowser.project = .all } }
            Divider()
            ArchivedConversationsMenuItem()
            Divider()
            // Where the list itself is; the rest of the window's layout is in a tab's menu.
            Menu("布局") { ToolPlaceMenu(window: window, kind: .list) }
        } label: { Label("显示", systemImage: window.listBrowser.hasFilters ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease") }
        .help("设备、会话分组、筛选、排序、已归档会话与列表的位置")
        .sheet(item: $projectTarget) { target in
            ProjectEditorView(store: target.store, target: target)
        }
        .sheet(isPresented: $crossProjectsPresented) { CrossDeviceProjectsEditor(catalog: aggregated.crossProjects, devices: aggregated.devices) }
        .accessibilityLabel("显示")
        .accessibilityIdentifier("conversation-list-filter")
        .disabled(device.store.conversationBrowserPreferences == nil)
    }

    private var deviceFilterBinding: Binding<AggregatedListModel.DeviceFilter> {
        Binding(get: { aggregated.filter }, set: { aggregated.setFilter($0) })
    }
}

struct NewConversationButton: View {
    @ObservedObject var window: ConversationWindowModel

    var body: some View {
        Button { window.createConversation() } label: { Label("新建会话", systemImage: "square.and.pencil") }
            .help(KeyboardShortcutSettings.shared.help("新建会话", .newConversation)).disabled(window.application.enabledRuntimeInstances.isEmpty)
            .accessibilityIdentifier("conversation-new-conversation")
    }
}
