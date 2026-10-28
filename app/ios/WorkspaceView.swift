import SwiftUI

struct WorkspaceView: View {
    @ObservedObject var devices: DeviceStore
    let peer: String
    @Binding var pendingURL: URL?
    @Environment(\.scenePhase) private var phase
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.openWindow) private var openWindow
    @Environment(\.supportsMultipleWindows) private var supportsMultipleWindows
    @State private var listing: ConfigurationSnapshot?
    @State private var searchResult: ConversationSearch?
    @State private var active: [ActiveConversation] = []
    @State private var compactColumn: NavigationSplitViewColumn = .sidebar
    @State private var query = ""
    @FocusState private var searchFocused: Bool
    @State private var matches: [ListedConversation] = []
    @State private var showingMatches = false
    @State private var listFailed = false
    private var groups: [ConversationGroup] { query.isEmpty ? listing?.conversationGroups ?? [] : searchResult?.groups ?? [] }

    var body: some View {
        NavigationSplitView(preferredCompactColumn: $compactColumn) {
            List(selection: Binding(get: { devices.selected }, set: { devices.select($0) })) {
                if listing == nil && !listFailed { ProgressView() }
                if !devices.pendingDrafts.isEmpty {
                    Section("草稿") {
                        ForEach(devices.pendingDrafts, id: \.key) { target in
                            NavigationLink(value: target) {
                                Label(devices.draftTitle(target), systemImage: "square.and.pencil").lineLimit(2)
                            }.contextMenu {
                                Button("删除草稿", role: .destructive) { devices.discardDraft(target) }
                            }
                        }
                    }
                }
                ForEach(groups, id: \.key) { group in
                    Section {
                        if !group.collapsed || !query.isEmpty {
                            ForEach(conversations(group.key), id: \.conversation.id) { listed in
                                NavigationLink(value: ConversationTarget(listed)) {
                                    ConversationListRow(listed: listed,
                                        runtimeName: listing?.runtimeInstances.first { $0.id == listed.conversation.runtimeId }?.name ?? "",
                                        active: active)
                                }
                                    .contextMenu {
                                        Button(listed.unread ? "标为已读" : "标为未读", systemImage: listed.unread ? "envelope.open" : "envelope.badge") {
                                            organize(listed, listed.unread ? .markRead : .markUnread)
                                        }
                                        if supportsMultipleWindows {
                                            Button("在新窗口中打开", systemImage: "macwindow.badge.plus") {
                                                openWindow(id: "workspace", value: WindowRoute(peer: peer, target: ConversationTarget(listed)))
                                            }
                                        }
                                    }
                            }
                            if group.loaded < group.total && query.isEmpty {
                                Button("加载更多") { Task {
                                    do { listing = try await devices.client.application(peer, operation: .conversationRead) { try $0.loadMoreConversations(group: group.key) } }
                                    catch { report(error, "加载会话") }
                                } }
                            }
                        }
                    } header: {
                        Button { toggle(group) } label: {
                            HStack { Text(groupTitle(group)); Spacer(); Text("\(group.total)").monospacedDigit(); Image(systemName: group.collapsed ? "chevron.right" : "chevron.down") }
                        }.buttonStyle(.plain)
                    }
                }
                if listing == nil && listFailed {
                    Button("重新读取会话", systemImage: "arrow.clockwise") { Task { await refresh() } }
                } else if !query.isEmpty && searchResult?.conversations.isEmpty == true {
                    ContentUnavailableView.search(text: query)
                } else if listing?.conversations.isEmpty == true {
                    ContentUnavailableView("没有会话", systemImage: "bubble.left.and.bubble.right")
                }
            }
            .navigationSplitViewColumnWidth(min: 240, ideal: 280, max: 340)
            .navigationTitle(devices.deviceName)
            .navigationBarTitleDisplayMode(.inline)
            .searchable(text: $query, prompt: "搜索会话或项目")
            .searchFocused($searchFocused)
            .refreshable { await refresh() }
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Menu {
                        ForEach(devices.state?.contacts ?? [], id: \.peerId) { contact in
                            Button(contact.name) { devices.peer = contact.peerId }
                        }
                        Divider()
                        Button("搜索会话", systemImage: "magnifyingglass") { focusSearch() }
                        NewWindowButton(peer: peer, target: nil)
                        Button("管理设备") { devices.peer = nil }
                    } label: { Label("设备与窗口", systemImage: "desktopcomputer") }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    if devices.selected == nil || sizeClass == .compact {
                        Button("新会话", systemImage: "square.and.pencil", action: create).disabled(listing == nil)
                    }
                }
                ToolbarItem(placement: .bottomBar) { Text(devices.status(peer)).font(.caption).foregroundStyle(.secondary) }
                ToolbarItem(placement: .bottomBar) { ProblemsButton() }
            }
        } detail: {
            if devices.selected != nil, let model = devices.currentModel {
                ConversationView(model: model, active: active, openLinked: openLinked,
                    readChanged: { Task { await refresh() } }, runtimes: listing?.runtimeInstances ?? [],
                    projects: listing?.conversationProjects ?? [], deviceName: devices.deviceName)
                    .id(model.target.key)
                    .toolbar {
                        ToolbarItem(placement: .primaryAction) {
                            Button("新会话", systemImage: "square.and.pencil", action: create).disabled(listing == nil)
                        }
                        ToolbarItem(placement: .topBarLeading) {
                            if !devices.history.isEmpty {
                                Button { devices.back() } label: { Label("返回上个会话", systemImage: "arrow.uturn.backward") }
                            }
                        }
                    }
            } else {
                ContentUnavailableView {
                    Label("选择会话", systemImage: "bubble.left.and.bubble.right")
                } actions: {
                    Button("新会话", systemImage: "square.and.pencil", action: create)
                        .buttonStyle(.borderedProminent).disabled(listing == nil)
                }
            }
        }
        .focusedSceneValue(\.workspaceActions, WorkspaceKeyboardActions(
            create: listing != nil ? create : nil,
            search: { focusSearch() }, back: devices.history.isEmpty ? nil : { devices.back() }))
        .onChange(of: devices.selected) { _, value in if value != nil { compactColumn = .detail } }
        .sheet(isPresented: $showingMatches) {
            NavigationStack {
                List(matches, id: \.conversation.id) { listed in
                    Button(listed.conversation.title.displayText()) { devices.select(ConversationTarget(listed)); showingMatches = false }
                }
                .overlay { if matches.isEmpty { ContentUnavailableView("没有找到匹配会话", systemImage: "magnifyingglass") } }
                .navigationTitle("链接中的会话").toolbar { Button("关闭") { showingMatches = false } }
            }
        }
        .task(id: phase) {
            guard phase == .active else { return }
            while !Task.isCancelled {
                await refresh()
                do { try await Task.sleep(for: .seconds(5)) } catch { return }
            }
        }
        .task(id: query) {
            do { try await Task.sleep(for: .milliseconds(250)) } catch { return }
            await search()
        }
        .task(id: pendingURL) {
            guard let url = pendingURL else { return }
            pendingURL = nil
            do {
                let result = try await devices.client.application(peer, operation: .conversationRead) { try $0.resolveAgentUri(uri: url.absoluteString, runtimeId: nil) }
                if result.conversations.count == 1, result.failures.isEmpty { devices.select(ConversationTarget(result.conversations[0])) }
                else {
                    matches = result.conversations
                    for failure in result.failures { ProblemStore.shared.record(failure.detail, source: "解析会话链接", context: devices.deviceName) }
                    showingMatches = true
                }
            } catch { report(error, "打开会话链接") }
        }
        .environment(\.openURL, OpenURLAction { url in
            guard url.scheme?.lowercased() == "agents" else { return .systemAction }
            pendingURL = url; return .handled
        })
    }

    private func create() {
        let runtimes = listing?.runtimeInstances.filter(\.enabled) ?? []
        let previous = devices.currentModel
        let runtime = runtimes.first { $0.id == previous?.runtimeID } ?? runtimes.first
        devices.select(.init(runtime: runtime?.id ?? "", cwd: previous?.cwd), remembering: true)
    }
    private func openLinked(_ id: String, _ newWindow: Bool) {
        let title = devices.currentModel?.snapshot?.subAgents.first { $0.conversationId == id }?.title
        Task {
            do {
                guard let listed = try await devices.client.application(peer, operation: .conversationRead, { try $0.findConversation(conversationId: id) }) else { throw MobileError.missingConversation }
                var target = ConversationTarget(listed)
                if let title, !title.isEmpty { target.title = title }
                if newWindow { openWindow(id: "workspace", value: WindowRoute(peer: peer, target: target)) }
                else { devices.select(target, remembering: true) }
            } catch { report(error, "打开关联 Agent") }
        }
    }
    private func report(_ error: Error, _ source: String) { ProblemStore.shared.record(error, source: source, context: devices.deviceName) }
    private func focusSearch() {
        compactColumn = .sidebar
        searchFocused = true
    }
    private func organize(_ row: ListedConversation, _ change: ConversationOrganizationChange) {
        if change == .markUnread, devices.selected?.conversation == row.conversation.id { devices.currentModel?.holdUnread() }
        Task {
            do {
                listing = try await devices.client.application(peer, operation: .conversationWrite) {
                    try $0.organizeConversations(conversations: [.init(runtimeId: row.conversation.runtimeId, conversationId: row.conversation.id)], change: change)
                }
                await search()
            } catch { report(error, "标记会话") }
        }
    }
    private func toggle(_ group: ConversationGroup) {
        guard var preferences = listing?.conversationBrowserPreferences else { return }
        if group.collapsed { preferences.collapsed.removeAll { $0 == group.key } }
        else { preferences.collapsed.append(group.key) }
        let next = preferences
        Task {
            do { listing = try await devices.client.application(peer, operation: .configurationWrite) { try $0.setConversationBrowserPreferences(preferences: next) } }
            catch { report(error, "折叠会话分组") }
        }
    }
    private func refresh() async {
        do {
            let result = try await devices.client.application(peer, operation: .configurationRead) { (try $0.list(), try $0.conversationActivity(since: 0)) }
            guard !Task.isCancelled else { return }
            listing = result.0; active = result.1.active; listFailed = false
            for failure in result.0.historyFailures {
                ProblemStore.shared.record(failure.detail, source: "读取会话历史", context: devices.deviceName + " · " + (result.0.runtimeInstances.first { $0.id == failure.runtimeId }?.name ?? failure.runtimeId))
            }
            await search()
        } catch { listFailed = true; report(error, "刷新会话列表") }
    }
    private func search() async {
        let requested = query
        guard !requested.isEmpty else { searchResult = nil; return }
        do {
            let result = try await devices.client.application(peer, operation: .conversationRead) { try $0.searchConversations(text: requested) }
            guard !Task.isCancelled, query == requested else { return }
            searchResult = result
        } catch { report(error, "搜索会话") }
    }
    private func conversations(_ key: ConversationGroupKey) -> [ListedConversation] {
        let rows = query.isEmpty ? listing?.conversations ?? [] : searchResult?.conversations ?? []
        return rows.filter { $0.group == key && !$0.archived && $0.conversation.parentConversationId == nil }
    }
    private func groupTitle(_ group: ConversationGroup) -> String {
        if let name = group.name { return name }
        switch group.key {
        case .all: return "会话"
        case .pinned: return "置顶"
        case .project(let path): return path.map { URL(fileURLWithPath: $0).lastPathComponent } ?? "未指定项目"
        case .runtime(let id): return listing?.runtimeInstances.first { $0.id == id }?.name ?? id
        }
    }
}

struct NewWindowButton: View {
    let peer: String?
    let target: ConversationTarget?
    @Environment(\.openWindow) private var openWindow
    @Environment(\.supportsMultipleWindows) private var supported
    var body: some View {
        if supported {
            Button("新窗口", systemImage: "macwindow.badge.plus") {
                openWindow(id: "workspace", value: WindowRoute(peer: peer, target: target))
            }
        }
    }
}

struct ConversationListRow: View {
    let listed: ListedConversation
    let runtimeName: String
    var active: [ActiveConversation] = []
    private var activity: [ActiveConversation] {
        active.filter { $0.conversationId == listed.conversation.id || $0.parentConversationId == listed.conversation.id }
    }
    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Group {
                if activity.contains(where: { $0.pendingInteractions > 0 }) {
                    Image(systemName: "hand.raised.fill").foregroundStyle(.orange).accessibilityLabel("等待回复")
                } else if activity.contains(where: { $0.runState == .running || $0.runState == .stopping }) {
                    ProgressView().controlSize(.mini).accessibilityLabel("正在处理")
                } else if listed.unread {
                    if listed.failed { Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.red).accessibilityLabel("未读，执行失败") }
                    else { Circle().fill(.tint).frame(width: 7, height: 7).accessibilityLabel("未读") }
                } else { Color.clear }
            }.frame(width: 12, height: 20)
            VStack(alignment: .leading, spacing: 5) {
                Text(listed.conversation.title.displayText()).lineLimit(2).fontWeight(listed.unread ? .semibold : .regular)
                    .frame(maxWidth: .infinity, alignment: .leading)
                HStack(spacing: 5) {
                    if listed.pinned { Image(systemName: "pin.fill").accessibilityLabel("已置顶") }
                    Text(runtimeName).lineLimit(1)
                    Spacer(minLength: 4)
                    if let timestamp = listed.conversation.updatedAtUnixMs {
                        let date = Date(timeIntervalSince1970: Double(timestamp) / 1000)
                        Text(date, format: Calendar.current.isDateInToday(date) ? .dateTime.hour().minute() : .dateTime.month(.abbreviated).day())
                            .monospacedDigit().lineLimit(1)
                    }
                }.font(.caption).foregroundStyle(.secondary)
            }
        }.padding(.vertical, 4).accessibilityElement(children: .combine)
    }
}
