import Foundation
import UniformTypeIdentifiers
import VeluneBindings

/// Browsing choices only shape the sidebar. They never select an execution
/// runtime or rewrite the authoritative conversation snapshot. The core
/// groups, orders, and pages the list by them; this mirrors the choices and
/// arranges the rows the core returned into its groups.
struct ConversationBrowser {
    enum Grouping: String, CaseIterable, Identifiable {
        case none, runtime, project
        var id: Self { self }
        var title: String { switch self { case .none: "不分组"; case .runtime: "Agent 运行时"; case .project: "项目" } }
    }
    enum Sort: String, CaseIterable, Identifiable {
        case updated, created
        var id: Self { self }
        var title: String { self == .updated ? "最近更新" : "创建时间" }
        func timestamp(_ conversation: Conversation) -> Int64? { self == .updated ? conversation.updatedAtUnixMs : conversation.createdAtUnixMs }
    }
    enum ProjectFilter: Hashable {
        case all, unspecified, path(String)
    }
    struct Section: Identifiable {
        let group: BindingConversationGroup
        let conversations: [Conversation]
        var id: BindingConversationGroupKey { group.key }
        var hasMore: Bool { group.loaded < group.total }
    }
    var grouping = Grouping.none
    var sort = Sort.updated
    var oldestFirst = false
    var runtimeID: String?
    var project = ProjectFilter.all
    var collapsed: [BindingConversationGroupKey] = []
    /// Groups in the order the user put them, each grouping's own.
    var groupOrder: [BindingConversationGroupKey] = []
    var preferences: BindingConversationBrowserPreferences {
        let value: BindingConversationBrowserProject
        switch project { case .all: value = .all; case .unspecified: value = .unspecified; case .path(let path): value = .path(path: path) }
        return BindingConversationBrowserPreferences(grouping: grouping == .none ? .none : grouping == .runtime ? .runtime : .project, sort: sort == .updated ? .updated : .created, oldestFirst: oldestFirst, runtimeId: runtimeID, project: value, collapsed: collapsed, groupOrder: groupOrder)
    }
    mutating func restore(_ value: BindingConversationBrowserPreferences) {
        grouping = value.grouping == .none ? .none : value.grouping == .runtime ? .runtime : .project
        sort = value.sort == .updated ? .updated : .created
        oldestFirst = value.oldestFirst; runtimeID = value.runtimeId
        switch value.project { case .all: project = .all; case .unspecified: project = .unspecified; case .path(let path): project = .path(path) }
        collapsed = value.collapsed
        groupOrder = value.groupOrder
    }
    var hasFilters: Bool { runtimeID != nil || project != .all }
    func isCollapsed(_ key: BindingConversationGroupKey) -> Bool { collapsed.contains(key) }
    mutating func setCollapsed(_ key: BindingConversationGroupKey, _ value: Bool) {
        if value { if !collapsed.contains(key) { collapsed.append(key) } } else { collapsed.removeAll { $0 == key } }
    }

    /// Groups the user may put in order: the pinned one stays first.
    static func orderable(_ key: BindingConversationGroupKey) -> Bool { key != .pinned && key != .all }
    /// The shown groups follow the order the user gave them, not recent activity.
    func isOrderedByHand(_ shown: [BindingConversationGroupKey]) -> Bool {
        shown.contains { Self.orderable($0) && groupOrder.contains($0) }
    }

    /// Put `key` before `target` (last when nil) among `shown`, the groups as
    /// the list shows them. Groups not shown now, filtered out or of another
    /// grouping, keep their places.
    mutating func move(_ key: BindingConversationGroupKey, before target: BindingConversationGroupKey?, shown: [BindingConversationGroupKey]) {
        let shown = shown.filter(Self.orderable)
        guard shown.contains(key), key != target else { return }
        var moved = shown.filter { $0 != key }
        moved.insert(key, at: target.flatMap { moved.firstIndex(of: $0) } ?? moved.count)
        guard moved != shown else { return }
        // The list shows groups not in the order yet first, as at its head.
        let order = shown.filter { !groupOrder.contains($0) } + groupOrder
        var next = moved.makeIterator()
        groupOrder = order.map { shown.contains($0) ? next.next()! : $0 }
    }

    /// The groups of the grouping shown follow recent activity again.
    mutating func orderByActivity() {
        groupOrder.removeAll { key in
            switch (grouping, key) {
            case (.project, .project), (.runtime, .runtime): true
            default: false
            }
        }
    }

    /// The core's groups in its order, each with the rows it loaded.
    static func sections(conversations: [Conversation], groups: [BindingConversationGroup]) -> [Section] {
        let rows = Dictionary(grouping: conversations.filter { $0.group != nil }) { $0.group! }
        return groups.map { Section(group: $0, conversations: rows[$0.key] ?? []) }
    }

    /// A project's name in the list: the user's, else its primary folder's
    /// name, with the enclosing folder when another project shares the name.
    static func projectName(_ path: String, projects: [BindingConversationProject]) -> String {
        if let name = projects.first(where: { $0.path == path })?.name { return name }
        let url = URL(fileURLWithPath: path)
        let name = url.lastPathComponent
        let shared = projects.contains { $0.path != path && $0.name == nil && URL(fileURLWithPath: $0.path).lastPathComponent == name }
        return shared ? "\(name)（\(url.deletingLastPathComponent().lastPathComponent)）" : name
    }
}

/// Velune's link to a conversation: `velune://conversation?runtime=…&id=…`.
enum ConversationLink {
    static let scheme = "velune"
    static func url(runtimeID: String, conversationID: String) -> URL {
        var components = URLComponents()
        components.scheme = scheme; components.host = "conversation"
        components.queryItems = [URLQueryItem(name: "runtime", value: runtimeID), URLQueryItem(name: "id", value: conversationID)]
        return components.url!
    }
    /// The conversation a link names; nil for any other URL.
    static func conversation(in url: URL) -> (runtimeID: String, conversationID: String)? {
        guard url.scheme == scheme, url.host == "conversation",
              let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems,
              let runtime = items.first(where: { $0.name == "runtime" })?.value, !runtime.isEmpty,
              let id = items.first(where: { $0.name == "id" })?.value, !id.isEmpty else { return nil }
        return (runtime, id)
    }

    /// What a drag from the list carries: Velune's own type (declared in the
    /// app's Info.plist) and no URL, since the desktop would keep a URL as an
    /// internet location file, and another app as a link or text.
    static let dragType = UTType(exportedAs: "local.velune.conversation-link")
    /// A group header dragged within the list, to put the group elsewhere;
    /// only this process takes it (declared in the app's Info.plist).
    static let groupDragType = UTType(exportedAs: "local.velune.conversation-group")
    /// What a pane or tab bar takes: the list's drag, or a tab's link.
    static let dropTypes: [UTType] = [dragType, .url]

    static func itemProvider(runtimeID: String, conversationID: String) -> NSItemProvider {
        let data = url(runtimeID: runtimeID, conversationID: conversationID).dataRepresentation
        let provider = NSItemProvider()
        provider.registerDataRepresentation(forTypeIdentifier: dragType.identifier, visibility: .ownProcess) { done in
            done(data, nil)
            return nil
        }
        return provider
    }

    /// The link a dropped item carries, as `dropTypes` take it.
    static func load(_ provider: NSItemProvider, then handle: @escaping @Sendable (URL) -> Void) {
        if provider.hasItemConformingToTypeIdentifier(dragType.identifier) {
            _ = provider.loadDataRepresentation(forTypeIdentifier: dragType.identifier) { data, _ in
                if let data, let url = URL(dataRepresentation: data, relativeTo: nil) { handle(url) }
            }
        } else {
            _ = provider.loadObject(ofClass: NSURL.self) { object, _ in
                if let url = object as? URL { handle(url) }
            }
        }
    }
}
