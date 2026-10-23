import SwiftUI

struct WorkspaceView: View {
    @ObservedObject var devices: DeviceStore
    let peer: String
    @Binding var pendingURL: URL?
    @Environment(\.scenePhase) private var phase
    @State private var listing: BindingConfigurationSnapshot?
    @State private var active: [BindingActiveConversation] = []
    @State private var selected: ConversationTarget?
    @State private var compactColumn: NavigationSplitViewColumn = .sidebar
    @State private var query = ""
    @State private var failure: String?
    @State private var creating = false
    @State private var matches: [BindingListedConversation] = []
    @State private var showingMatches = false

    var body: some View {
        NavigationSplitView(preferredCompactColumn: $compactColumn) {
            List(selection: $selected) {
                if let failure { FailureView(text: failure) }
                if listing == nil { ProgressView() }
                ForEach(listing?.conversationGroups ?? [], id: \.key) { group in
                    Section {
                        if !group.collapsed || !query.isEmpty {
                            ForEach(conversations(group.key), id: \.conversation.id) { listed in
                                NavigationLink(value: ConversationTarget(listed)) { row(listed) }
                            }
                            if group.loaded < group.total {
                                Button("加载更多") { Task {
                                    do { listing = try await devices.client.application(peer) { try $0.loadMoreConversations(group: group.key) } }
                                    catch { failure = mobileError(error) }
                                } }
                            }
                        }
                    } header: {
                        Button { toggle(group) } label: {
                            HStack { Text(groupTitle(group)); Spacer(); Image(systemName: group.collapsed ? "chevron.right" : "chevron.down") }
                        }.buttonStyle(.plain)
                    }
                }
                if listing?.conversations.isEmpty == true { ContentUnavailableView("没有会话", systemImage: "bubble.left.and.bubble.right") }
            }
            .navigationTitle(devices.state?.contacts.first { $0.peerId == peer }?.name ?? "会话")
            .searchable(text: $query, prompt: "搜索会话或项目")
            .refreshable { await refresh() }
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Menu {
                        ForEach(devices.state?.contacts ?? [], id: \.peerId) { contact in
                            Button(contact.name) { devices.peer = contact.peerId }
                        }
                        Divider()
                        Button("管理设备") { devices.peer = nil }
                    } label: { Label("设备", systemImage: "desktopcomputer") }
                }
                ToolbarItem(placement: .topBarTrailing) { Button { creating = true } label: { Label("新会话", systemImage: "square.and.pencil") }.disabled(listing == nil) }
                ToolbarItem(placement: .bottomBar) { Text(devices.status(peer)).font(.caption).foregroundStyle(.secondary) }
            }
        } detail: {
            if let selected { ConversationView(client: devices.client, peer: peer, target: selected).id(selected) }
            else { ContentUnavailableView("选择会话", systemImage: "bubble.left.and.bubble.right") }
        }
        .onChange(of: selected) { _, value in if value != nil { compactColumn = .detail } }
        .sheet(isPresented: $creating) {
            NewConversationView(runtimes: listing?.runtimeInstances.filter(\.enabled) ?? [], projects: listing?.conversationProjects ?? []) {
                selected = $0; creating = false
            }
        }
        .sheet(isPresented: $showingMatches) {
            NavigationStack {
                List {
                    if let failure { FailureView(text: failure) }
                    ForEach(matches, id: \.conversation.id) { listed in Button(listed.conversation.title) { selected = ConversationTarget(listed); showingMatches = false } }
                    if matches.isEmpty { Text("没有找到匹配会话") }
                }.navigationTitle("链接中的会话").toolbar { Button("关闭") { showingMatches = false } }
            }
        }
        .task(id: phase) {
            guard phase == .active else { return }
            while !Task.isCancelled {
                await refresh()
                do { try await Task.sleep(for: .seconds(5)) } catch { return }
            }
        }
        .task(id: pendingURL) {
            guard let url = pendingURL else { return }
            pendingURL = nil
            do {
                let result = try await devices.client.application(peer) { try $0.resolveAgentUri(uri: url.absoluteString, runtimeId: nil) }
                if result.conversations.count == 1, result.failures.isEmpty { selected = ConversationTarget(result.conversations[0]) }
                else {
                    matches = result.conversations
                    failure = result.failures.isEmpty ? nil : result.failures.map(\.detail).joined(separator: "\n")
                    showingMatches = true
                }
            } catch { failure = mobileError(error) }
        }
        .environment(\.openURL, OpenURLAction { url in
            guard url.scheme?.lowercased() == "agents" else { return .systemAction }
            pendingURL = url; return .handled
        })
    }

    private func toggle(_ group: BindingConversationGroup) {
        guard var preferences = listing?.conversationBrowserPreferences else { return }
        if group.collapsed { preferences.collapsed.removeAll { $0 == group.key } }
        else { preferences.collapsed.append(group.key) }
        let next = preferences
        Task {
            do { listing = try await devices.client.application(peer) { try $0.setConversationBrowserPreferences(preferences: next) } }
            catch { failure = mobileError(error) }
        }
    }
    private func refresh() async {
        do {
            let result = try await devices.client.application(peer) { (try $0.list(), try $0.conversationActivity(since: 0)) }
            listing = result.0; active = result.1.active
            failure = result.0.historyFailures.isEmpty ? nil : result.0.historyFailures.map(\.detail).joined(separator: "\n")
        } catch { failure = mobileError(error) }
    }
    private func conversations(_ key: BindingConversationGroupKey) -> [BindingListedConversation] {
        (listing?.conversations ?? []).filter { listed in
            listed.group == key && !listed.archived && listed.conversation.parentConversationId == nil &&
            (query.isEmpty || listed.conversation.title.localizedCaseInsensitiveContains(query) || (listed.project ?? "").localizedCaseInsensitiveContains(query))
        }
    }
    private func groupTitle(_ group: BindingConversationGroup) -> String {
        if let name = group.name { return name }
        switch group.key {
        case .all: return "会话"
        case .pinned: return "置顶"
        case .project(let path): return path.map { URL(fileURLWithPath: $0).lastPathComponent } ?? "未指定项目"
        case .runtime(let id): return listing?.runtimeInstances.first { $0.id == id }?.name ?? id
        }
    }
    private func row(_ listed: BindingListedConversation) -> some View {
        HStack(spacing: 10) {
            Image(systemName: listed.pinned ? "pin.fill" : "bubble.left")
                .foregroundStyle(listed.failed ? .red : .secondary)
            VStack(alignment: .leading, spacing: 4) {
                Text(listed.conversation.title).lineLimit(2).fontWeight(listed.unread ? .semibold : .regular)
                HStack {
                    Text(listing?.runtimeInstances.first { $0.id == listed.conversation.runtimeId }?.name ?? "")
                    if let date = listed.conversation.updatedAtUnixMs { Text(Date(timeIntervalSince1970: Double(date) / 1000), style: .relative) }
                }.font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            if active.contains(where: { $0.conversationId == listed.conversation.id }) { ProgressView().controlSize(.small) }
            else if listed.unread { Circle().fill(.tint).frame(width: 7, height: 7) }
        }
    }
}

private struct NewConversationView: View {
    let runtimes: [BindingRuntimeInstance]
    let projects: [BindingConversationProject]
    let create: (ConversationTarget) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var runtime = ""
    @State private var project = ""
    var body: some View {
        NavigationStack {
            Form {
                Picker("运行时", selection: $runtime) { ForEach(runtimes, id: \.id) { Text($0.name).tag($0.id) } }
                Picker("项目", selection: $project) {
                    Text("运行时默认").tag("")
                    ForEach(projects, id: \.path) { Text($0.name ?? URL(fileURLWithPath: $0.path).lastPathComponent).tag($0.path) }
                }
                if runtimes.isEmpty { Text("请先在被控设备上配置运行时") }
            }.navigationTitle("新会话").toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("取消") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) { Button("创建") { create(.init(runtime: runtime, cwd: project.isEmpty ? nil : project)) }.disabled(runtime.isEmpty) }
            }.onAppear { runtime = runtimes.first?.id ?? "" }
        }.presentationDetents([.medium, .large])
    }
}
