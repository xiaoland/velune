import Foundation

// A workspace window's editor arrangement as plain values: the grid of tab
// groups, each group's tabs and the one it shows, and the focused group.
// Views render it and send operations; they do not keep layout of their own.
// Every operation leaves the layout whole (see `WorkspaceLayout.isWhole`) and
// its grid normalized, so callers never repair it afterwards.
//
// Foundation only: it builds and is checked on any platform
// (`swift run velune-workspace-checks`).

/// How a window's groups are arranged: a group, or groups side by side
/// (horizontal) or stacked (vertical), each taking a fraction of the room.
/// The case keeps the name `pane` that saved layouts know.
public indirect enum PaneLayout: Codable, Equatable, Sendable {
    case pane(UUID)
    case split(PaneSplit)

    /// Groups in reading order: left to right, top to bottom.
    public var paneIDs: [UUID] {
        switch self {
        case .pane(let id): [id]
        case .split(let split): split.children.flatMap(\.paneIDs)
        }
    }

    /// `added` beside `pane`'s `edge`, taking half its room.
    public func inserting(_ added: UUID, beside pane: UUID, edge: PaneEdge) -> PaneLayout {
        switch self {
        case .pane(let id):
            guard id == pane else { return self }
            let children: [PaneLayout] = edge.after ? [.pane(id), .pane(added)] : [.pane(added), .pane(id)]
            return .split(PaneSplit(axis: edge.axis, children: children, fractions: [0.5, 0.5]))
        case .split(var split):
            // A group in a split along the same axis shares its own room with the new one.
            if split.axis == edge.axis, let index = split.children.firstIndex(of: .pane(pane)) {
                let half = split.fractions[index] / 2
                split.fractions[index] = half
                split.children.insert(.pane(added), at: edge.after ? index + 1 : index)
                split.fractions.insert(half, at: edge.after ? index + 1 : index)
                return .split(split)
            }
            split.children = split.children.map { $0.inserting(added, beside: pane, edge: edge) }
            return .split(split)
        }
    }

    /// Without `pane`; a split left with one child becomes that child. Nil
    /// when `pane` was all there was.
    public func removing(_ pane: UUID) -> PaneLayout? {
        switch self {
        case .pane(let id): return id == pane ? nil : self
        case .split(var split):
            var children: [PaneLayout] = []
            var fractions: [Double] = []
            for (child, fraction) in zip(split.children, split.fractions) {
                if let kept = child.removing(pane) { children.append(kept); fractions.append(fraction) }
            }
            guard let first = children.first else { return nil }
            if children.count == 1 { return first }
            split.children = children
            split.fractions = fractions
            return PaneLayout.split(split).normalized()
        }
    }

    /// The split at `path` (child indices from the root) with new fractions.
    /// A path that names no split, or fractions of the wrong count, change nothing.
    public func resizing(at path: [Int], fractions: [Double]) -> PaneLayout {
        guard case .split(var split) = self else { return self }
        guard let index = path.first else {
            guard fractions.count == split.children.count else { return self }
            split.fractions = fractions
            return PaneLayout.split(split).normalized()
        }
        guard split.children.indices.contains(index) else { return self }
        split.children[index] = split.children[index].resizing(at: Array(path.dropFirst()), fractions: fractions)
        return .split(split)
    }

    /// The split at `path`, if there is one.
    public func split(at path: [Int]) -> PaneSplit? {
        guard case .split(let split) = self else { return nil }
        guard let index = path.first else { return split }
        guard split.children.indices.contains(index) else { return nil }
        return split.children[index].split(at: Array(path.dropFirst()))
    }

    /// Child indices from the root to `pane`.
    public func path(to pane: UUID) -> [Int]? {
        switch self {
        case .pane(let id): return id == pane ? [] : nil
        case .split(let split):
            for (index, child) in split.children.enumerated() {
                if let rest = child.path(to: pane) { return [index] + rest }
            }
            return nil
        }
    }

    /// No split with fewer than two children, no split directly inside one
    /// along the same axis (its children join the outer one, keeping their
    /// share of the room), and each split's fractions positive and summing
    /// to one. An empty split cannot be represented and is left as is.
    public func normalized() -> PaneLayout {
        guard case .split(var split) = self else { return self }
        var children: [PaneLayout] = []
        var fractions: [Double] = []
        let shares = PaneSplit.normalizedFractions(split.fractions, count: split.children.count)
        for (child, share) in zip(split.children.map { $0.normalized() }, shares) {
            if case .split(let inner) = child, inner.axis == split.axis {
                children += inner.children
                fractions += inner.fractions.map { $0 * share }
            } else {
                children.append(child)
                fractions.append(share)
            }
        }
        if children.count == 1 { return children[0] }
        guard !children.isEmpty else { return self }
        split.children = children
        split.fractions = PaneSplit.normalizedFractions(fractions, count: children.count)
        return .split(split)
    }
}

public struct PaneSplit: Codable, Equatable, Sendable {
    public enum Axis: String, Codable, Sendable { case horizontal, vertical }
    public var axis: Axis
    public var children: [PaneLayout]
    public var fractions: [Double]

    public init(axis: Axis, children: [PaneLayout], fractions: [Double]) {
        self.axis = axis
        self.children = children
        self.fractions = fractions
    }

    /// `count` shares summing to one: missing, non-finite or non-positive
    /// ones become an equal share before the sum is scaled.
    static func normalizedFractions(_ fractions: [Double], count: Int) -> [Double] {
        guard count > 0 else { return [] }
        let equal = 1 / Double(count)
        let valid = (0..<count).map { index -> Double in
            guard index < fractions.count, fractions[index].isFinite, fractions[index] > 0 else { return equal }
            return fractions[index]
        }
        let total = valid.reduce(0, +)
        // Shares already summing to one stay as they are, so normalizing twice changes nothing.
        if abs(total - 1) < 1e-9 { return valid }
        return valid.map { $0 / total }
    }
}

/// Which side of a group something goes to.
public enum PaneEdge: Equatable, Sendable {
    case leading, trailing, top, bottom
    public var axis: PaneSplit.Axis { self == .leading || self == .trailing ? .horizontal : .vertical }
    public var after: Bool { self == .trailing || self == .bottom }
}

/// Where a tab dropped on a group goes: among its tabs, or beside an edge.
public enum PaneDropZone: Equatable, Sendable {
    case center
    case edge(PaneEdge)
}

/// A group's tabs in order and the one it shows.
public struct TabGroup: Codable, Equatable, Sendable {
    public private(set) var tabs: [UUID]
    public private(set) var selected: UUID
    /// Shown tabs, most recent last, for where closing the shown one returns.
    /// Not saved: restored tabs that were never shown have no history.
    public private(set) var history: [UUID]

    private enum CodingKeys: String, CodingKey { case tabs, selected }

    /// Nil without tabs, with a tab named twice, or selecting none of them.
    public init?(tabs: [UUID], selected: UUID? = nil) {
        guard let first = tabs.first, Set(tabs).count == tabs.count else { return nil }
        let selected = selected ?? first
        guard tabs.contains(selected) else { return nil }
        self.tabs = tabs
        self.selected = selected
        history = [selected]
    }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let tabs = try values.decode([UUID].self, forKey: .tabs)
        let selected = try values.decode(UUID.self, forKey: .selected)
        guard let group = TabGroup(tabs: tabs, selected: selected) else {
            throw DecodingError.dataCorruptedError(forKey: .tabs, in: values, debugDescription: "标签组为空、重复或未选中其中的标签")
        }
        self = group
    }

    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(tabs, forKey: .tabs)
        try values.encode(selected, forKey: .selected)
    }

    mutating func select(_ tab: UUID) {
        guard tabs.contains(tab) else { return }
        history.removeAll { $0 == tab }
        history.append(tab)
        selected = tab
    }

    /// `tab` at `index` (clamped; the end when nil), shown.
    mutating func insert(_ tab: UUID, at index: Int?) {
        tabs.insert(tab, at: max(0, min(index ?? tabs.count, tabs.count)))
        select(tab)
    }

    /// Closing the shown tab returns to the group's last shown one,
    /// otherwise its left neighbour. False for the only tab or one not here.
    mutating func remove(_ tab: UUID) -> Bool {
        guard tabs.count > 1, let index = tabs.firstIndex(of: tab) else { return false }
        tabs.remove(at: index)
        history.removeAll { $0 == tab }
        if selected == tab { select(history.last ?? tabs[max(0, index - 1)]) }
        return true
    }

    /// `tab` to the place `index` names among the tabs as they are now,
    /// as a tab bar's insertion point does: before the tab at `index`.
    mutating func move(_ tab: UUID, to index: Int) {
        guard let from = tabs.firstIndex(of: tab) else { return }
        tabs.remove(at: from)
        tabs.insert(tab, at: max(0, min(from < index ? index - 1 : index, tabs.count)))
        select(tab)
    }
}

/// A window's grid, groups and focus, kept whole by its operations.
public struct WorkspaceLayout: Codable, Equatable, Sendable {
    public private(set) var grid: PaneLayout
    public private(set) var groups: [UUID: TabGroup]
    public private(set) var focused: UUID

    /// One group with one tab.
    public init(tab: UUID, group: UUID = UUID()) {
        grid = .pane(group)
        groups = [group: TabGroup(tabs: [tab])!]
        focused = group
    }

    /// Nil unless whole: see `isWhole`. The grid is normalized.
    public init?(grid: PaneLayout, groups: [UUID: TabGroup], focused: UUID) {
        self.grid = grid.normalized()
        self.groups = groups
        self.focused = focused
        guard isWhole else { return nil }
    }

    private enum CodingKeys: String, CodingKey { case grid, groups, focused }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        guard let layout = WorkspaceLayout(grid: try values.decode(PaneLayout.self, forKey: .grid),
                                           groups: try values.decode([UUID: TabGroup].self, forKey: .groups),
                                           focused: try values.decode(UUID.self, forKey: .focused)) else {
            throw DecodingError.dataCorruptedError(forKey: .grid, in: values, debugDescription: "工作区布局不完整")
        }
        self = layout
    }

    /// Every group in the grid exactly once and none outside it, each tab in
    /// exactly one group, the focused group among them.
    public var isWhole: Bool {
        let ids = grid.paneIDs
        let tabs = ids.flatMap { groups[$0]?.tabs ?? [] }
        return Set(ids).count == ids.count && Set(ids) == Set(groups.keys) && groups[focused] != nil
            && Set(tabs).count == tabs.count && Self.splitsAreWhole(grid)
    }

    private static func splitsAreWhole(_ layout: PaneLayout) -> Bool {
        guard case .split(let split) = layout else { return true }
        return split.children.count >= 2 && split.children.count == split.fractions.count
            && split.children.allSatisfy(splitsAreWhole)
    }

    // MARK: Reading

    /// Groups in reading order.
    public var groupIDs: [UUID] { grid.paneIDs }
    public var isSplit: Bool { groups.count > 1 }
    /// Every tab, group by group in reading order.
    public var tabIDs: [UUID] { groupIDs.flatMap { groups[$0]!.tabs } }
    public var tabCount: Int { groups.values.reduce(0) { $0 + $1.tabs.count } }
    public var focusedGroup: TabGroup { groups[focused]! }
    /// The focused group's shown tab.
    public var focusedTab: UUID { focusedGroup.selected }

    public func group(holding tab: UUID) -> UUID? {
        groups.first { $0.value.tabs.contains(tab) }?.key
    }

    public func contains(tab: UUID) -> Bool { group(holding: tab) != nil }

    // MARK: Focus and selection

    public mutating func focus(_ group: UUID) {
        guard groups[group] != nil else { return }
        focused = group
    }

    /// Show `tab` in its group and focus that group.
    public mutating func select(_ tab: UUID) {
        guard let id = group(holding: tab) else { return }
        groups[id]!.select(tab)
        focused = id
    }

    /// The tab after (or before) the focused group's shown one, wrapping.
    public func neighbour(forward: Bool) -> UUID? {
        let group = focusedGroup
        guard group.tabs.count > 1, let index = group.tabs.firstIndex(of: group.selected) else { return nil }
        return group.tabs[(index + (forward ? 1 : group.tabs.count - 1)) % group.tabs.count]
    }

    /// The group after (or before) the focused one in reading order, wrapping.
    public func neighbourGroup(forward: Bool) -> UUID? {
        let ids = groupIDs
        guard ids.count > 1, let index = ids.firstIndex(of: focused) else { return nil }
        return ids[(index + (forward ? 1 : ids.count - 1)) % ids.count]
    }

    // MARK: Adding

    /// A new tab in `group` (the focused one when nil) at `index`, shown and focused.
    public mutating func insert(_ tab: UUID, in group: UUID? = nil, at index: Int? = nil) {
        let id = group.flatMap { groups[$0] == nil ? nil : $0 } ?? focused
        guard !contains(tab: tab) else { return }
        groups[id]!.insert(tab, at: index)
        focused = id
    }

    /// A new group holding a new tab beside `group` (the focused one when nil), focused.
    public mutating func addGroup(_ id: UUID = UUID(), with tab: UUID, beside group: UUID? = nil, edge: PaneEdge) {
        let beside = group.flatMap { groups[$0] == nil ? nil : $0 } ?? focused
        guard groups[id] == nil, !contains(tab: tab) else { return }
        grid = grid.inserting(id, beside: beside, edge: edge).normalized()
        groups[id] = TabGroup(tabs: [tab])!
        focused = id
    }

    // MARK: Removing

    public enum Removal: Equatable, Sendable {
        /// Taken from its group, which shows another tab.
        case removed(group: UUID)
        /// Its group had no other tab and closed.
        case closedGroup(UUID)
    }

    /// Take `tab` out. Its group closes with its last tab; the window's last
    /// tab is not taken (nil), since what replaces it is the window's choice.
    @discardableResult public mutating func remove(_ tab: UUID) -> Removal? {
        guard let id = group(holding: tab) else { return nil }
        if groups[id]!.remove(tab) { return .removed(group: id) }
        guard isSplit else { return nil }
        removeGroup(id)
        return .closedGroup(id)
    }

    /// Close a group with every tab in it; returns those tabs. The only group stays.
    @discardableResult public mutating func closeGroup(_ id: UUID) -> [UUID] {
        guard isSplit, let group = groups[id] else { return [] }
        removeGroup(id)
        return group.tabs
    }

    /// The window's only tab replaced by `replacement`, as a window does to
    /// keep a tab when its last one leaves.
    public mutating func replaceLastTab(with replacement: UUID) {
        guard tabCount == 1, !contains(tab: replacement) else { return }
        groups[focused] = TabGroup(tabs: [replacement])!
    }

    private mutating func removeGroup(_ id: UUID) {
        let order = groupIDs
        guard let removed = grid.removing(id) else { return }
        grid = removed
        groups[id] = nil
        if focused == id {
            // Focus goes to the group that took its room: the one before it in reading order, else after.
            let index = order.firstIndex(of: id)!
            focused = order[..<index].last(where: { groups[$0] != nil }) ?? order[(index + 1)...].first(where: { groups[$0] != nil })!
        }
    }

    // MARK: Moving

    /// `tab` to place `index` of its own group, as dragging along its bar does.
    public mutating func moveTab(_ tab: UUID, toPlace index: Int) {
        guard let id = group(holding: tab) else { return }
        let from = groups[id]!.tabs.firstIndex(of: tab)!
        // A place is where the tab ends up; the group's move takes an insertion point.
        groups[id]!.move(tab, to: index > from ? index + 1 : index)
        focused = id
    }

    /// `tab` into `target` before its tab at `index` (the end when nil),
    /// shown and focused. Its own group closes when left empty.
    public mutating func move(_ tab: UUID, to target: UUID, at index: Int? = nil) {
        guard let source = group(holding: tab), groups[target] != nil else { return }
        if source == target {
            groups[target]!.move(tab, to: index ?? groups[target]!.tabs.count)
            focused = target
            return
        }
        if !groups[source]!.remove(tab) { removeGroup(source) }
        groups[target]!.insert(tab, at: index)
        focused = target
    }

    /// `tab` into a new group beside `target`'s `edge` (its own group when
    /// nil), shown and focused. A group's only tab moved beside that same
    /// group changes nothing; moved beside another group, its group closes.
    public mutating func split(_ tab: UUID, beside target: UUID? = nil, edge: PaneEdge, group id: UUID = UUID()) {
        guard let source = group(holding: tab), groups[id] == nil else { return }
        let beside = target ?? source
        guard groups[beside] != nil else { return }
        if source == beside && groups[source]!.tabs.count == 1 { return }
        if !groups[source]!.remove(tab) { removeGroup(source) }
        grid = grid.inserting(id, beside: beside, edge: edge).normalized()
        groups[id] = TabGroup(tabs: [tab])!
        focused = id
    }

    // MARK: Sizes

    /// New fractions for the split at `path`; they are normalized.
    public mutating func resize(at path: [Int], fractions: [Double]) {
        grid = grid.resizing(at: path, fractions: fractions)
    }

    /// Equal shares again for the split at `path`.
    public mutating func equalize(at path: [Int]) {
        guard let split = grid.split(at: path) else { return }
        resize(at: path, fractions: Array(repeating: 1, count: split.children.count))
    }

    // MARK: Saving

    /// Without `tabs` (temporary editors such as side chats); a group left
    /// empty is dropped. Nil when nothing would remain.
    public func excluding(_ tabs: Set<UUID>) -> WorkspaceLayout? {
        guard !tabs.isEmpty else { return self }
        var kept = self
        for id in groupIDs {
            for tab in groups[id]!.tabs where tabs.contains(tab) {
                if !kept.groups[id]!.remove(tab) {
                    guard kept.isSplit else { return nil }
                    kept.removeGroup(id)
                }
            }
        }
        return kept
    }
}
