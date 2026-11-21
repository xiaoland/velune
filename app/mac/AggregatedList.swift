import AppKit
import Combine
import Foundation
import VeluneBindings

/// A stable, device-qualified identity for a conversation-group key, for the
/// aggregated list's own ordering and folding preferences.
extension ConversationGroupKey {
    var stableID: String {
        switch self {
        case .pinned: "pinned"
        case .all: "all"
        case .runtime(let id): "runtime:\(id)"
        case .project(let path?): "project:\(path)"
        case .project(nil): "project:"
        }
    }
}

/// The aggregated conversation list of one window: this Mac and every paired
/// device it controls, in one list. Each device keeps its own core-computed
/// groups and its own browsing preferences; this model only merges what the
/// devices return, qualifies group identity with the device, and holds the
/// merged view's own choices (device filter, cross-device folding and
/// ordering) on this Mac. Grouping, sorting and within-device folding stay
/// on the device that owns the conversations, as before aggregation.
@MainActor final class AggregatedListModel: ObservableObject {
    /// Which device's conversations the list shows.
    enum DeviceFilter: Hashable {
        case all, local, peer(String)
        var peerID: String? { if case .peer(let id) = self { id } else { nil } }
    }

    /// One device in the aggregated list.
    struct Device: Equatable, Identifiable {
        /// nil is this Mac.
        let peerID: String?
        let store: DeviceStore
        /// The contact's name, or this Mac's name.
        let name: String
        var id: String { peerID.map { "peer:\($0)" } ?? "local" }
        static func == (lhs: Self, rhs: Self) -> Bool { lhs.id == rhs.id }

        @MainActor func needsConnectionStatus(_ status: ControllerStatus?) -> Bool {
            guard peerID != nil else { return false }
            let hasList = !store.conversations.isEmpty || store.listRevision > 0
            return !hasList || status?.phase != .connected
        }
    }

    /// A group as the merged list shows it: one of a device's own groups,
    /// with the device its conversations belong to.
    struct Section: Identifiable, Equatable {
        let device: Device
        let key: ConversationGroupKey
        let group: ConversationGroup
        let conversations: [Conversation]
        /// The group's title on its own device, computed while merging (MainActor).
        let title: String
        var crossProjectID: UUID? = nil
        var allowsPagination = true
        var collapseID: String { crossProjectID.map { "cross-project:\($0)" } ?? id }
        var id: String { device.id + "‖" + key.stableID + (crossProjectID.map { "‖" + $0.uuidString } ?? "") }
        /// Local projects retain their familiar names in the merged list.
        /// Other groups identify their device when several devices are shown.
        func displayTitle(multipleDevices: Bool) -> String {
            if crossProjectID != nil { return title }
            if device.peerID == nil, case .project = key { return title }
            return multipleDevices ? "\(device.name) — \(title)" : title
        }
        var hasMore: Bool { allowsPagination && group.loaded < group.total }
        static func == (lhs: Self, rhs: Self) -> Bool { lhs.id == rhs.id }
    }

    let crossProjects: CrossDeviceProjectCatalog
    private let local: DeviceStore
    private let hub: DeviceHub?
    private let persistence: AggregatedListPersistence
    private var storeWatches: [String: AnyCancellable] = [:]
    private var managementWatches: [String: AnyCancellable] = [:]
    let managedConversations = PassthroughSubject<(deviceID: String, conversations: Set<String>), Never>()
    struct Management: Identifiable {
        let device: Device
        let request: DeviceStore.ConversationManagement
        var id: UUID { request.id }
    }
    /// Management stays reachable even after changing the sidebar's device filter.
    var conversationManagement: [Management] {
        devices.flatMap { device in device.store.conversationManagement.map { Management(device: device, request: $0) } }
    }
    private var searchWatches: [String: AnyCancellable] = [:]
    private var searchRequest = 0
    private var searchListRevisions: [String: Int] = [:]
    /// One list-type store per device, for search and organization requests.
    private var listStores: [String: ConversationStore] = [:]

    /// The devices shown, this Mac first, then paired devices in the hub's order.
    @Published private(set) var devices: [Device] = []
    @Published var filter: DeviceFilter
    @Published private(set) var collapsed: [String]
    /// Cross-device group order for the merged view's manual ordering.
    @Published private(set) var groupOrder: [String]
    /// Searching: the text and each device's own matches.
    @Published var searchText = ""
    @Published private(set) var searchResults: [String: (conversations: [Conversation], groups: [ConversationGroup])] = [:]
    @Published private(set) var searchFailure: String?

    init(local: DeviceStore, hub: DeviceHub?, persistence: AggregatedListPersistence? = nil, crossProjects: CrossDeviceProjectCatalog? = nil) {
        self.crossProjects = crossProjects ?? CrossDeviceProjectCatalog.controller(home: local.isPreview || !MacNativePersistenceGate.isEnabled ? nil : local.transport?.stateDirectory)
        self.local = local
        self.hub = hub
        self.persistence = persistence ?? AggregatedListPersistence(home: local.isPreview || !MacNativePersistenceGate.isEnabled ? nil : local.transport?.stateDirectory) { [weak local] in
            local?.recordProblem($0, source: "保存聚合列表设置")
        }
        filter = self.persistence.filter
        collapsed = self.persistence.collapsed
        groupOrder = self.persistence.groupOrder
        self.crossProjects.objectWillChange.receive(on: RunLoop.main).sink { [weak self] _ in self?.rebuildSections() }.store(in: &cancellables)
        rebuildDevices()
        hub?.$state.receive(on: RunLoop.main).sink { [weak self] _ in self?.rebuildDevices() }.store(in: &cancellables)
        rebuildSections()
    }

    private var cancellables = Set<AnyCancellable>()

    /// The filter menu's choices: every device the user can show.
    var filterChoices: [DeviceFilter] {
        [.all, .local] + devices.filter { $0.peerID != nil }.map { .peer($0.peerID!) }
    }

    func name(for filter: DeviceFilter) -> String {
        switch filter {
        case .all: "全部设备"
        case .local: localDeviceName
        case .peer(let id): devices.first { $0.peerID == id }?.name ?? hub?.name(of: id) ?? "设备"
        }
    }

    var localDeviceName: String { DeviceHub.deviceName }

    private func rebuildDevices() {
        var next: [Device] = [Device(peerID: nil, store: local, name: DeviceHub.deviceName)]
        if let state = hub?.state {
            for contact in state.contacts where contact.controlAuthorized {
                guard case .confirmed = contact.confirmation else { continue }
                let peer = contact.peerId
                guard let store = hub?.store(for: peer) else { continue }
                next.append(Device(peerID: peer, store: store, name: contact.name))
            }
        }
        updateDevices(next)
    }

    /// Replace the device projection and its observations together. Synthetic
    /// checks supply preview stores here without starting a remote endpoint.
    func updateDevices(_ next: [Device]) {
        let old = Set(devices.map(\.id))
        devices = next
        for device in next where !old.contains(device.id) {
            storeWatches[device.id] = device.store.objectWillChange.receive(on: RunLoop.main).sink { [weak self] _ in self?.rebuildSections() }
            managementWatches[device.id] = device.store.managedConversations.sink { [weak self] ids in
                self?.managedConversations.send((device.id, ids))
            }
        }
        storeWatches = storeWatches.filter { id, _ in next.contains(where: { $0.id == id }) }
        managementWatches = managementWatches.filter { id, _ in next.contains(where: { $0.id == id }) }
        for id in Array(listStores.keys) where !next.contains(where: { $0.id == id }) {
            listStores.removeValue(forKey: id)?.setSearchText("")
            searchWatches[id] = nil
            searchListRevisions[id] = nil
            searchResults[id] = nil
        }
        rebuildSections()
    }

    /// The devices the current filter shows.
    var visibleDevices: [Device] {
        switch filter {
        case .all: devices
        case .local: devices.filter { $0.peerID == nil }
        case .peer(let id): devices.filter { $0.peerID == id }
        }
    }

    /// Whether the merged list shows more than one device's conversations:
    /// group titles carry the device name then.
    var showsMultipleDevices: Bool { filter == .all && visibleDevices.count > 1 }

    /// The merged list's groups in display order. Grouping, order within a
    /// device and row order stay the owning core's; devices follow this
    /// Mac, then the hub's contact order; the merged view's manual group
    /// order applies across devices when set.
    @Published private(set) var sections: [Section] = []

    /// The title a device's own group shows, from that device's configuration.
    private func groupTitle(_ group: ConversationGroup, key: ConversationGroupKey, store: DeviceStore) -> String {
        if let name = group.name { return name }
        switch key {
        case .all: return "会话"
        case .pinned: return "置顶"
        case .runtime(let id): return store.runtimeInstances.first { $0.id == id }?.name ?? "Agent 运行时"
        case .project(let path?): return ConversationBrowser.projectName(path, projects: store.conversationProjects)
        case .project(nil): return "无项目"
        }
    }

    private func rebuildSections() {
        refreshSearches()
        let visible = visibleDevices
        var merged: [Section] = []
        var searchMerged: [Section] = []
        for device in visible {
            if !searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, let results = searchResults[device.id] {
                searchMerged += ConversationBrowser.sections(conversations: results.conversations, groups: results.groups).map {
                    Section(device: device, key: $0.group.key, group: $0.group, conversations: $0.conversations, title: groupTitle($0.group, key: $0.group.key, store: device.store))
                }
                continue
            }
            merged += ConversationBrowser.sections(conversations: device.store.conversations, groups: device.store.conversationGroups).map {
                Section(device: device, key: $0.group.key, group: $0.group, conversations: $0.conversations, title: groupTitle($0.group, key: $0.group.key, store: device.store))
            }
        }
        if !searchMerged.isEmpty || !searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            sections = projectSections(reorder(searchMerged))
        } else {
            sections = projectSections(reorder(merged))
        }
        objectWillChange.send()
    }

    /// Keep owner-qualified shards so selection, pagination and management still
    /// dispatch to the source device; only the visual project group is unified.
    private func projectSections(_ input: [Section]) -> [Section] {
        var result: [Section] = []
        for section in input {
            if case .pinned = section.key { result.append(section); continue }
            let projectGrouping: Bool = if case .project = section.key { true } else { local.conversationBrowserPreferences?.grouping == .project }
            guard projectGrouping else { result.append(section); continue }
            var remaining = section.conversations
            for project in crossProjects.projects {
                let folders = project.folders.filter { $0.peerID == section.device.peerID }
                let matched = remaining.filter { conversation in
                    guard let path = conversation.cwd ?? conversation.project else { return false }
                    return folders.contains { $0.contains(path) }
                }
                if !matched.isEmpty {
                    let shard = Section(device: section.device, key: section.key, group: section.group, conversations: matched, title: project.name, crossProjectID: project.id)
                    result.append(shard)
                    let ids = Set(matched.map(\.id)); remaining.removeAll { ids.contains($0.id) }
                }
            }
            if remaining.count == section.conversations.count || !remaining.isEmpty { result.append(Section(device: section.device, key: section.key, group: section.group, conversations: remaining, title: section.title)) }
        }
        var ordered: [Section] = [], consumed = Set<UUID>()
        for section in result {
            if let id = section.crossProjectID {
                if consumed.insert(id).inserted { ordered += result.filter { $0.crossProjectID == id } }
            } else { ordered.append(section) }
        }
        for index in ordered.indices {
            let source = ordered[index]
            ordered[index].allowsPagination = !ordered.dropFirst(index + 1).contains { $0.device.id == source.device.id && $0.key == source.key }
        }
        return ordered
    }
    func showsHeader(_ section: Section) -> Bool {
        guard let project = section.crossProjectID else { return true }
        return sections.first(where: { $0.crossProjectID == project })?.id == section.id
    }

    /// The merged manual order, applied across devices; groups it names come
    /// first, the rest keep their places after them. Fixed groups keep their
    /// core-provided slots; a single-device view uses only that device's order.
    private func reorder(_ input: [Section]) -> [Section] {
        guard filter == .all, !groupOrder.isEmpty else { return input }
        var rank: [String: Int] = [:]
        for (index, id) in groupOrder.enumerated() { rank[id] = index }
        var ordered = input.filter { ConversationBrowser.orderable($0.key) }.enumerated().sorted { lhs, rhs in
            let l = rank[lhs.element.id] ?? groupOrder.count + lhs.offset
            let r = rank[rhs.element.id] ?? groupOrder.count + rhs.offset
            return l < r
        }.map(\.element).makeIterator()
        return input.map { ConversationBrowser.orderable($0.key) ? ordered.next()! : $0 }
    }

    // MARK: - The merged view's own choices

    func setFilter(_ value: DeviceFilter) {
        guard value != filter else { return }
        filter = value
        resetSearches()
        rebuildSections()
        persist()
    }

    func isCollapsed(_ section: Section) -> Bool { collapsed.contains(section.collapseID) }
    func setCollapsed(_ section: Section, _ value: Bool) {
        if value { if !collapsed.contains(section.collapseID) { collapsed.append(section.collapseID) } }
        else { collapsed.removeAll { $0 == section.collapseID } }
        persist()
    }

    /// Put `section` before `target` (nil: last) in the merged view, keeping
    /// the user's order. Only the merged "全部设备" view orders across devices.
    func move(_ section: Section, before target: Section?) {
        guard filter == .all, section.id != target?.id else { return }
        let shown = sections.filter { ConversationBrowser.orderable($0.key) }
        guard shown.contains(where: { $0.id == section.id }) else { return }
        var moved = shown.filter { $0.id != section.id }
        moved.insert(section, at: target.flatMap { target in moved.firstIndex { $0.id == target.id } } ?? moved.count)
        guard moved.map(\.id) != shown.map(\.id) else { return }
        let shownIDs = Set(shown.map(\.id))
        let order = groupOrder + shown.map(\.id).filter { !groupOrder.contains($0) }
        var next = moved.map(\.id).makeIterator()
        // Replace only visible slots. Filtered or temporarily unavailable
        // groups retain their positions in the previously saved order.
        groupOrder = order.map { shownIDs.contains($0) ? next.next()! : $0 }
        persist()
        rebuildSections()
    }

    private func persist() {
        let value: String = switch filter {
        case .all: "all"
        case .local: "local"
        case .peer(let id): "peer:\(id)"
        }
        persistence.save(filter: value, collapsed: collapsed, groupOrder: groupOrder)
    }

    // MARK: - Search

    func setSearchText(_ text: String) {
        guard text != searchText else { return }
        searchText = text
        resetSearches()
        rebuildSections()
    }

    private func resetSearches() {
        searchRequest += 1
        searchWatches.removeAll()
        searchListRevisions.removeAll()
        searchResults = [:]
        for store in listStores.values { store.setSearchText("") }
    }

    private func refreshSearches() {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return }
        let request = searchRequest
        for device in visibleDevices {
            let store = listStore(for: device)
            let revision = device.store.listRevision
            if searchWatches[device.id] != nil {
                guard searchListRevisions[device.id] != revision else { continue }
                searchListRevisions[device.id] = revision
                // ConversationStore refreshes successful searches on a list
                // revision. A failed initial search has no results to refresh.
                if store.searchResults == nil {
                    store.setSearchText("")
                    store.setSearchText(query)
                }
                continue
            }
            searchListRevisions[device.id] = revision
            store.setSearchText(query)
            searchWatches[device.id] = store.$searchResults.receive(on: RunLoop.main).sink { [weak self] results in
                guard let self, request == self.searchRequest else { return }
                if let results {
                    self.searchResults[device.id] = (results.conversations, results.groups)
                } else {
                    self.searchResults[device.id] = nil
                }
                self.rebuildSections()
            }
        }
    }

    /// A list-type store for a device: search and organization go through
    /// the same ConversationStore machinery a tab uses.
    func listStore(for device: Device) -> ConversationStore {
        if let store = listStores[device.id] { return store }
        let store = ConversationStore(device: device.store)
        listStores[device.id] = store
        return store
    }

    /// The device a conversation id belongs to, as far as the loaded lists say.
    func device(of conversationID: String) -> Device? {
        visibleDevices.first { $0.store.conversations.contains { $0.id == conversationID } }
            ?? devices.first { $0.store.conversations.contains { $0.id == conversationID } }
    }

    // MARK: - Connections

    /// The hub's connection state for a device, nil for this Mac.
    func status(of device: Device) -> ControllerStatus? {
        device.peerID.flatMap { hub?.controller($0) }
    }

    /// Keep a state/retry row throughout first connection and recovery, and
    /// alongside retained groups while offline. Only a connected device with
    /// a loaded list no longer needs the row.
    func pendingDevices() -> [Device] {
        visibleDevices.filter { $0.needsConnectionStatus(status(of: $0)) }
    }

    func retry(_ device: Device) {
        guard let peer = device.peerID else { return }
        hub?.retryConnection(peer)
    }
}

/// The aggregated list choices file: schema 1, kept on this Mac.
private struct AggregatedListFile: Codable {
    static let schema = 1
    var schema = AggregatedListFile.schema
    /// "all", "local" or "peer:<id>".
    var filter: String
    var collapsed: [String]
    var groupOrder: [String]
}

/// The aggregated list's own choices, kept on this Mac. Unknown or
/// unreadable formats use display defaults while preserving the original file.
@MainActor final class AggregatedListPersistence {
    private(set) var filter: AggregatedListModel.DeviceFilter
    private(set) var collapsed: [String]
    private(set) var groupOrder: [String]
    private let url: URL?
    private let failed: (Error) -> Void
    private var pending: DispatchWorkItem?
    private var canWrite = true

    init(home: URL?, failed: @escaping (Error) -> Void) {
        self.failed = failed
        url = home?.appendingPathComponent("mac-aggregated-list.json")
        filter = .all
        collapsed = []
        groupOrder = []
        guard let url, FileManager.default.fileExists(atPath: url.path) else { return }
        let data = try? Data(contentsOf: url)
        let file = data.flatMap { try? JSONDecoder().decode(AggregatedListFile.self, from: $0) }
        let strict = data.flatMap { data -> Bool? in
            guard let file else { return nil }
            guard let source = try? JSONSerialization.jsonObject(with: data),
                  let encodedData = try? JSONEncoder().encode(file),
                  let encoded = try? JSONSerialization.jsonObject(with: encodedData) else { return nil }
            return SavedWindowLayout.hasOnlyEncodedKeys(source, encoded: encoded)
        }
        guard let file, file.schema == AggregatedListFile.schema, strict == true else {
            canWrite = false
            failed(NSError(domain: "VelunePersistence", code: 1, userInfo: [NSLocalizedDescriptionKey: "mac-aggregated-list.json 格式不受支持；保留原文件，聚合列表设置不会写入。请升级到支持该格式的版本后重试。"]))
            return
        }
        switch file.filter {
        case "all": filter = .all
        case "local": filter = .local
        default:
            if file.filter.hasPrefix("peer:") { filter = .peer(String(file.filter.dropFirst("peer:".count))) }
        }
        collapsed = file.collapsed
        groupOrder = file.groupOrder
    }

    func save(filter: String, collapsed: [String], groupOrder: [String]) {
        self.filter = parse(filter)
        self.collapsed = collapsed
        self.groupOrder = groupOrder
        guard canWrite, let url else { return }
        pending?.cancel()
        let file = AggregatedListFile(filter: filter, collapsed: collapsed, groupOrder: groupOrder)
        let work = DispatchWorkItem { [url, failed] in
            do {
                let data = try JSONEncoder().encode(file)
                try data.write(to: url, options: .atomic)
            } catch {
                failed(NSError(domain: "VelunePersistence", code: 2, userInfo: [NSLocalizedDescriptionKey: "聚合列表设置保存失败：\(error.localizedDescription)"]))
            }
        }
        pending = work
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 0.3, execute: work)
    }

    private func parse(_ value: String) -> AggregatedListModel.DeviceFilter {
        switch value {
        case "all": return .all
        case "local": return .local
        default: return value.hasPrefix("peer:") ? .peer(String(value.dropFirst("peer:".count))) : .all
        }
    }
}
