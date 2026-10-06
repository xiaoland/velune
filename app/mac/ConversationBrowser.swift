import Foundation

/// Browsing choices only derive sidebar rows. They never select an execution
/// runtime or rewrite the authoritative conversation snapshot.
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
    enum SectionID: Hashable {
        case all, runtime(String), project(String?)
    }
    struct Section: Identifiable {
        let id: SectionID
        let title: String
        let conversations: [Conversation]
        let totalCount: Int
        var hasMore: Bool { conversations.count < totalCount }
    }
    var initialLimit = 20
    private var additionalPages: [SectionID: Int] = [:]
    mutating func loadMore(_ sectionID: SectionID) { additionalPages[sectionID, default: 0] += 1 }
    mutating func resetPagination() { additionalPages.removeAll() }
    var grouping = Grouping.none
    var sort = Sort.updated
    var oldestFirst = false
    var runtimeID: String?
    var project = ProjectFilter.all
    var search = ""
    struct Query: Equatable {
        let grouping: Grouping
        let sort: Sort
        let oldestFirst: Bool
        let runtimeID: String?
        let project: ProjectFilter
        let search: String
        let initialLimit: Int
    }
    var query: Query { Query(grouping: grouping, sort: sort, oldestFirst: oldestFirst, runtimeID: runtimeID, project: project, search: search, initialLimit: initialLimit) }
    var hasFilters: Bool { runtimeID != nil || project != .all }

    func sections(conversations: [Conversation], runtimes: [RuntimeInstance]) -> [Section] {
        let enabled = Dictionary(uniqueKeysWithValues: runtimes.filter(\.enabled).map { ($0.id, $0.name) })
        let filtered = conversations.filter { conversation in
            guard enabled[conversation.runtimeID] != nil,
                  runtimeID == nil || conversation.runtimeID == runtimeID else { return false }
            switch project {
            case .all: break
            case .unspecified: if conversation.cwd != nil { return false }
            case .path(let path): if conversation.cwd != path { return false }
            }
            return search.isEmpty || conversation.title.localizedCaseInsensitiveContains(search)
                || conversation.cwd?.localizedCaseInsensitiveContains(search) == true
                || enabled[conversation.runtimeID]?.localizedCaseInsensitiveContains(search) == true
        }.sorted { lhs, rhs in
            let left = sort.timestamp(lhs), right = sort.timestamp(rhs)
            if left != right {
                guard let left else { return false } // Unknown native dates stay last in either order.
                guard let right else { return true }
                return oldestFirst ? left < right : left > right
            }
            let titles = lhs.title.localizedStandardCompare(rhs.title)
            return titles == .orderedSame ? lhs.id < rhs.id : titles == .orderedAscending
        }
        let grouped = Dictionary(grouping: filtered) { conversation in
            switch grouping {
            case .none: SectionID.all
            case .runtime: SectionID.runtime(conversation.runtimeID)
            case .project: SectionID.project(conversation.cwd)
            }
        }
        return grouped.map { key, rows in
            let title: String
            switch key {
            case .all: title = "会话"
            case .runtime(let id): title = enabled[id] ?? "Agent 运行时"
            case .project(let path): title = path ?? "未指定项目"
            }
            return Section(id: key, title: title, conversations: Array(rows.prefix(max(1, initialLimit) * (1 + additionalPages[key, default: 0]))), totalCount: rows.count)
        }.sorted { lhs, rhs in
            if lhs.id == .project(nil) { return false }
            if rhs.id == .project(nil) { return true }
            let titles = lhs.title.localizedStandardCompare(rhs.title)
            if titles != .orderedSame { return titles == .orderedAscending }
            return lhs.conversations.first!.id < rhs.conversations.first!.id
        }
    }
}
