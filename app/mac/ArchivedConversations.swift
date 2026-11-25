import AppKit
import Combine
import SwiftUI
import VeluneBindings

struct ArchivedConversationsMenuItem: View {
    @Environment(\.openWindow) private var openWindow
    var body: some View { Button("已归档会话…") { openWindow(id: "archived-conversations") } }
}

/// This projection belongs to the archive window. The device retains runtime
/// activity and the management queue; archive deliveries never replace its list.
@MainActor final class ArchivedConversationsModel: ObservableObject {
    let source: DeviceStore
    @Published var browser = ConversationBrowser()
    @Published var searchText = ""
    @Published private(set) var conversations: [Conversation] = []
    @Published private(set) var groups: [ConversationGroup] = []
    @Published private(set) var projects: [ConversationProject] = []
    @Published private(set) var isLoading = false
    @Published private(set) var ready = false
    @Published private(set) var loadingGroups: Set<ConversationGroupKey> = []
    @Published private(set) var loadError: String?
    @Published private(set) var operationError: String?
    let completed = PassthroughSubject<Set<String>, Never>()
    private var transport: Transport?
    private var lifecycle = UUID()
    private var generation = 0
    private var active = false
    private var savedPreferences: ConversationBrowserPreferences?
    private var listing: ConfigurationSnapshot?
    private var watches = Set<AnyCancellable>()
    private lazy var capabilities = ConversationStore(device: source)

    init(source: DeviceStore) {
        self.source = source
        source.managedConversations.sink { [weak self] ids in
            self?.completed.send(ids)
            self?.refresh()
        }.store(in: &watches)
    }

    func runtimeName(_ id: String) -> String { source.runtimeInstances.first { $0.id == id }?.name ?? "未知运行时" }
    var pendingIDs: Set<String> { source.conversationManagement.reduce(into: []) { $0.formUnion($1.conversations) } }
    func canUnarchive(_ value: Conversation) -> Bool { value.archived && !pendingIDs.contains(value.id) && !source.isShuttingDown }
    func canDelete(_ value: Conversation) -> Bool { !pendingIDs.contains(value.id) && capabilities.canDeleteConversation(value) && !source.isShuttingDown }

    func start() {
        active = true
        guard transport == nil, !source.isPreview else { return }
        guard let original = source.transport else { loadError = "暂时无法连接设备。"; return }
        lifecycle = UUID()
        transport = original.clientView("archived-conversations:\(lifecycle.uuidString)")
        read { try $0.showArchivedConversations(true) }
    }

    func stop() {
        active = false
        guard let transport else { return }
        self.transport = nil
        lifecycle = UUID()
        ready = false
        isLoading = false
        loadingGroups = []
        source.performOnQueue { try? transport.close() }
    }

    /// Refresh includes the runtime's native archive, not just loaded rows.
    func refresh() {
        guard active else { return }
        guard transport != nil else { start(); return }
        read { [ready] in ready ? try $0.list() : try $0.showArchivedConversations(true) }
    }
    func refreshOnActivation() {
        // Opening the window may also activate it before its initial read finishes.
        guard ready || !isLoading else { return }
        refresh()
    }
    func search() {
        guard ready else { return }
        // Resolve against the last completed queue operation, not a snapshot
        // captured before an in-flight filter or pagination change.
        read { _ in nil }
    }
    func setPreferences() {
        let preferences = browser.preferences
        guard ready, preferences != savedPreferences else { return }
        read { try $0.setConversationBrowserPreferences(preferences) }
    }
    func loadMore(_ group: ConversationGroupKey) {
        guard ready, loadingGroups.insert(group).inserted else { return }
        read(group: group) { try $0.loadMoreConversations(group: group) }
    }

    private func read(group: ConversationGroupKey? = nil, _ operation: @escaping (Transport) throws -> ConfigurationSnapshot?) {
        guard let transport else { return }
        generation += 1
        let request = generation, token = lifecycle, query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        isLoading = true
        source.performOnQueue { [weak self] in
            let result = Result {
                let snapshot = try operation(transport)
                let search = query.isEmpty ? nil : try transport.searchConversations(query)
                return (snapshot, search)
            }
            DispatchQueue.main.async {
                guard let self, token == self.lifecycle else { return }
                if let group { self.loadingGroups.remove(group) }
                // Device queue completions arrive in order. Retain a mutation's
                // delivery even if a newer search supersedes its visible result.
                if case .success(let (snapshot?, _)) = result { self.listing = snapshot }
                guard request == self.generation else { return }
                self.isLoading = false
                switch result {
                case .success(let (_, search)):
                    if let listing = self.listing { self.apply(listing) }
                    if let search {
                        self.conversations = search.conversations.map(BindingMapping.listed).filter { $0.archived && $0.parentConversationID == nil }
                        self.groups = search.groups
                    }
                    self.loadError = self.listing?.historyFailures.isEmpty != false ? nil : "部分运行时的归档会话未能读取，请重试。"
                    if !self.ready {
                        self.ready = true
                        if !self.searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && query.isEmpty { self.search() }
                    }
                case .failure(let error): self.loadError = AppProblem.failure(error, source: "读取已归档会话").detail
                }
            }
        }
    }

    func apply(_ data: ConfigurationSnapshot) {
        listing = data
        conversations = data.conversations.map(BindingMapping.listed).filter { $0.archived && $0.parentConversationID == nil }
        groups = data.conversationGroups
        projects = data.conversationProjects
        savedPreferences = data.conversationBrowserPreferences
        browser.restore(data.conversationBrowserPreferences)
    }

    func unarchive(_ values: [Conversation]) { manage(values, deleting: false) }
    func delete(_ values: [Conversation]) { manage(values, deleting: true) }
    func dismissOperationError() { operationError = nil }

    private func manage(_ values: [Conversation], deleting: Bool) {
        guard let transport, !values.isEmpty else { return }
        guard values.allSatisfy(deleting ? canDelete : canUnarchive) else {
            operationError = deleting ? "所选会话当前无法删除，请先停止仍在执行的会话。" : "所选会话仍有待处理操作，请稍后重试。"
            return
        }
        let source = source
        operationError = nil
        source.manageConversations(source: deleting ? "删除已归档会话" : "取消归档", ids: Set(values.map(\.id)), deleting: deleting,
            operation: {
                var succeeded = Set<String>(), failures: [String] = []
                for value in values {
                    do {
                        if deleting { _ = try transport.deleteConversation(runtimeID: value.runtimeID, conversationID: value.id) }
                        else { _ = try transport.organizeConversations([ConversationReference(runtimeId: value.runtimeID,
                            conversationId: value.id, nativeReadState: value.nativeReadState)], .unarchive) }
                        succeeded.insert(value.id)
                    } catch { failures.append("\(value.title)：\(AppProblem.failure(error, source: deleting ? "删除会话" : "取消归档").detail)") }
                }
                return ArchiveManagementResult(succeeded: succeeded, failures: failures)
            }, completedIDs: { $0.succeeded }) { [weak self, weak source] result in
                if deleting { source?.noteDeleted(result.succeeded) }
                else { source?.rememberOrganization(values.filter { result.succeeded.contains($0.id) }, .unarchive) }
                if !result.succeeded.isEmpty, let original = source?.transport {
                    source?.browse({ try original.list() }, source: "刷新会话列表")
                }
                self?.operationError = result.failures.isEmpty ? nil : result.failures.joined(separator: "\n")
                // No success notification is sent for a wholly failed batch.
                if result.succeeded.isEmpty { self?.refresh() }
            }
    }
}

private struct ArchiveManagementResult: Sendable {
    let succeeded: Set<String>
    let failures: [String]
}

struct ArchivedConversationsView: View {
    @StateObject private var devices: DeviceWindowSelectionModel
    init(local: DeviceStore, hub: DeviceHub?) {
        _devices = StateObject(wrappedValue: DeviceWindowSelectionModel(local: local, hub: hub))
    }
    var body: some View {
        ArchivedConversationsContent(source: devices.activeStore)
            .id(devices.selectedPeerID ?? "local")
            .toolbar {
                if !devices.peerDevices.isEmpty {
                    ToolbarItem(placement: .navigation) {
                        Picker("设备", selection: $devices.selectedPeerID) {
                            Text(devices.localDeviceName).tag(nil as String?)
                            ForEach(devices.peerDevices, id: \.id) { Text($0.name).tag($0.id as String?) }
                        }.labelsHidden().help("切换设备")
                    }
                }
            }
    }
}

struct ArchivedConversationsContent: View {
    @ObservedObject var source: DeviceStore
    @StateObject var model: ArchivedConversationsModel
    @State private var selection = Set<String>()
    @State private var deleting: [Conversation] = []
    @State private var showsErrorDetails = false
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.openWindow) private var openWindow

    init(source: DeviceStore) {
        self.source = source
        _model = StateObject(wrappedValue: ArchivedConversationsModel(source: source))
    }
    init(model: ArchivedConversationsModel) {
        self.source = model.source
        _model = StateObject(wrappedValue: model)
    }
    private var selected: [Conversation] { model.conversations.filter { selection.contains($0.id) } }
    private var canUnarchive: Bool { !selected.isEmpty && selected.allSatisfy(model.canUnarchive) }
    private var canDelete: Bool { !selected.isEmpty && selected.allSatisfy(model.canDelete) }

    var body: some View {
        VStack(spacing: 0) {
            if let error = model.operationError {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Label("会话操作未完成", systemImage: "exclamationmark.triangle")
                        Spacer(minLength: 12)
                        Button(showsErrorDetails ? "收起详情" : "详情") { showsErrorDetails.toggle() }
                        Button("关闭提示") { model.dismissOperationError() }
                    }
                    if showsErrorDetails {
                        ScrollView { Text(error).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }.frame(height: 100)
                    } else { Text(error).lineLimit(2).textSelection(.enabled) }
                }.padding(12)
                Divider()
            }
            if let error = model.loadError {
                ReadFailureNotice(title: "归档会话未能完整更新", detail: error, retryDisabled: model.isLoading) { model.refresh() }
                Divider()
            }
            if model.conversations.isEmpty && model.groups.isEmpty {
                if model.isLoading { ProgressView("正在读取已归档会话…").frame(maxWidth: .infinity, maxHeight: .infinity) }
                else if model.loadError != nil { Spacer(minLength: 0) }
                else {
                    ContentUnavailableView {
                        Label(model.searchText.isEmpty && !model.browser.hasFilters ? "没有已归档的会话" : "没有匹配的会话", systemImage: "archivebox")
                    } actions: {
                        if !model.searchText.isEmpty || model.browser.hasFilters {
                            Button("清除搜索与筛选") { model.searchText = ""; model.browser.runtimeID = nil; model.browser.project = .all }
                        }
                    }.frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            } else { list }
            Divider()
            HStack {
                if model.isLoading { ProgressView().controlSize(.small); Text("正在读取…") }
                else { Text(selection.isEmpty ? "已显示 \(model.conversations.count) 个会话" : "已选择 \(selection.count) 个会话") }
                Spacer()
                managementMenu
            }.font(.callout).foregroundStyle(.secondary).padding(.horizontal, 12).padding(.vertical, 8)
        }
        .searchable(text: $model.searchText, prompt: "搜索归档会话")
        .toolbar {
            ToolbarItem { filterMenu }
            ToolbarItem { Button("打开会话", systemImage: "arrow.up.right.square") { if let value = selected.first { open(value) } }.disabled(selected.count != 1) }
            ToolbarItem { Button("取消归档", systemImage: "tray.and.arrow.up") { model.unarchive(selected) }.disabled(!canUnarchive) }
            ToolbarItem { Button("删除…", systemImage: "trash") { deleting = selected }.disabled(!canDelete) }
            ToolbarItem { ReadRefreshButton(label: "刷新归档会话") { model.refresh() }.disabled(model.isLoading) }
        }
        .onAppear { source.start(); model.start() }
        .onDisappear { model.stop() }
        .onChange(of: model.browser.preferences) { _, _ in model.setPreferences() }
        .task(id: model.searchText) {
            if !model.searchText.isEmpty { try? await Task.sleep(for: .milliseconds(250)) }
            if !Task.isCancelled { model.search() }
        }
        .onReceive(model.completed) { selection.subtract($0) }
        .onChange(of: model.conversations.map(\.id)) { _, ids in selection.formIntersection(ids) }
        .onChange(of: model.operationError ?? model.loadError) { _, _ in showsErrorDetails = false }
        .onChange(of: scenePhase) { _, phase in if phase == .active { model.refreshOnActivation() } }
        .alert(deleting.count == 1 ? "删除会话？" : "删除 \(deleting.count) 个会话？", isPresented: Binding(get: { !deleting.isEmpty }, set: { if !$0 { deleting = [] } })) {
            Button("取消", role: .cancel) { deleting = [] }
            Button("删除", role: .destructive) { model.delete(deleting); deleting = [] }
        } message: {
            Text("将永久删除所选 \(deleting.count) 个会话，以及它们在各 Agent 运行时中的关联会话数据。此操作无法撤销。")
        }
    }

    private var list: some View {
        List(selection: $selection) {
            ForEach(model.groups, id: \.key) { group in
                if group.key != .all {
                    Button {
                        model.browser.setCollapsed(group.key, !model.browser.isCollapsed(group.key))
                    } label: {
                        Label(groupName(group.key), systemImage: model.browser.isCollapsed(group.key) ? "chevron.right" : "chevron.down")
                            .font(.callout).foregroundStyle(.secondary)
                    }.buttonStyle(.plain).selectionDisabled(true)
                }
                if !model.browser.isCollapsed(group.key) {
                    ForEach(model.conversations.filter { $0.group == group.key }) { value in
                        row(value).tag(value.id)
                    }
                    if group.loaded < group.total && model.searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        ConversationPaginationButton(loaded: group.loaded, total: group.total,
                            isLoading: model.loadingGroups.contains(group.key)) { model.loadMore(group.key) }.selectionDisabled(true)
                    }
                }
            }
        }
    }

    private func row(_ value: Conversation) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 20) {
            VStack(alignment: .leading, spacing: 3) {
                Text(value.title.isEmpty ? "未命名会话" : value.title).lineLimit(1).help(value.title)
                if let path = value.project ?? value.cwd, value.group != .project(path: path) {
                    Text(value.project.map { ConversationBrowser.projectName($0, projects: model.projects) } ?? path)
                        .font(.callout).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle).help(path)
                }
            }
            Spacer(minLength: 10)
            if model.pendingIDs.contains(value.id) { Text("待处理").font(.callout).foregroundStyle(.secondary) }
            Text(model.runtimeName(value.runtimeID)).font(.callout).foregroundStyle(.secondary).lineLimit(1)
            if let time = model.browser.sort.timestamp(value) {
                Text(Date(timeIntervalSince1970: Double(time) / 1000), format: .dateTime.year().month().day())
                    .font(.callout).foregroundStyle(.secondary)
            }
        }.padding(.vertical, 4)
        .contextMenu {
            Button("打开会话") { open(value) }
            Divider()
            Button("取消归档") { model.unarchive(selection.contains(value.id) ? selected : [value]) }
                .disabled(!(selection.contains(value.id) ? canUnarchive : model.canUnarchive(value)))
            Button("删除…", role: .destructive) { deleting = selection.contains(value.id) ? selected : [value] }
                .disabled(!(selection.contains(value.id) ? canDelete : model.canDelete(value)))
        }
    }

    private func open(_ value: Conversation) {
        let seed = WindowSeed(conversationID: value.id, device: source.peerID)
        WindowPresentation.windowed.insert(seed.id)
        openWindow(id: ConversationWindowKind.single.sceneID, value: seed)
    }

    private func groupName(_ key: ConversationGroupKey) -> String {
        switch key {
        case .all: "所有会话"
        case .pinned: "置顶"
        case .runtime(let id): model.runtimeName(id)
        case .project(let path?): ConversationBrowser.projectName(path, projects: model.projects)
        case .project(nil): "无项目"
        }
    }

    private var filterMenu: some View {
        Menu {
            Picker("分组", selection: $model.browser.grouping) { ForEach(ConversationBrowser.Grouping.allCases) { Text($0.title).tag($0) } }
            Picker("会话排序", selection: $model.browser.sort) { ForEach(ConversationBrowser.Sort.allCases) { Text($0.title).tag($0) } }
            Toggle("从旧到新", isOn: $model.browser.oldestFirst)
            Divider()
            Picker("Agent 运行时", selection: $model.browser.runtimeID) {
                Text("所有运行时").tag(nil as String?)
                ForEach(source.runtimeInstances) { Text($0.name).tag($0.id as String?) }
            }
            Picker("项目", selection: $model.browser.project) {
                Text("所有项目").tag(ConversationBrowser.ProjectFilter.all)
                Text("无项目").tag(ConversationBrowser.ProjectFilter.unspecified)
                ForEach(model.projects, id: \.path) { project in
                    Text(ConversationBrowser.projectName(project.path, projects: model.projects)).tag(ConversationBrowser.ProjectFilter.path(project.path))
                }
            }
            if model.browser.hasFilters {
                Divider()
                Button("清除筛选") { model.browser.runtimeID = nil; model.browser.project = .all }
            }
        } label: { Label("筛选", systemImage: model.browser.hasFilters ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease") }
        .disabled(!model.ready).help("运行时、项目、分组与排序")
    }

    @ViewBuilder private var managementMenu: some View {
        if !source.conversationManagement.isEmpty {
            Menu {
                ForEach(source.conversationManagement) { request in
                    Text("\(request.source)：\(request.running ? "执行中" : "排队中")")
                    if !request.running { Button("取消“\(request.source)”") { source.cancelConversationManagement(request.id) } }
                }
            } label: { Label("\(source.conversationManagement.count) 项操作", systemImage: "clock") }
            .menuStyle(.borderlessButton).fixedSize()
        }
    }
}
