import Foundation
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
    var preferences: BindingConversationBrowserPreferences {
        let value: BindingConversationBrowserProject
        switch project { case .all: value = .all; case .unspecified: value = .unspecified; case .path(let path): value = .path(path: path) }
        return BindingConversationBrowserPreferences(grouping: grouping == .none ? .none : grouping == .runtime ? .runtime : .project, sort: sort == .updated ? .updated : .created, oldestFirst: oldestFirst, runtimeId: runtimeID, project: value, collapsed: collapsed)
    }
    mutating func restore(_ value: BindingConversationBrowserPreferences) {
        grouping = value.grouping == .none ? .none : value.grouping == .runtime ? .runtime : .project
        sort = value.sort == .updated ? .updated : .created
        oldestFirst = value.oldestFirst; runtimeID = value.runtimeId
        switch value.project { case .all: project = .all; case .unspecified: project = .unspecified; case .path(let path): project = .path(path) }
        collapsed = value.collapsed
    }
    var hasFilters: Bool { runtimeID != nil || project != .all }
    func isCollapsed(_ key: BindingConversationGroupKey) -> Bool { collapsed.contains(key) }
    mutating func setCollapsed(_ key: BindingConversationGroupKey, _ value: Bool) {
        if value { if !collapsed.contains(key) { collapsed.append(key) } } else { collapsed.removeAll { $0 == key } }
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
}
