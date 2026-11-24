import Foundation
import VeluneWorkspaceModel

// Checks of the workspace layout model: `swift run velune-workspace-checks`.
// Exits non-zero on the first failed group, naming the failed expectations.

var failures: [String] = []
var passed = 0

func expect(_ condition: @autoclosure () -> Bool, _ message: String, line: Int = #line) {
    if condition() { passed += 1 } else { failures.append("line \(line): \(message)") }
}

func check(_ name: String, _ body: () throws -> Void) {
    let before = failures.count
    do { try body() } catch { failures.append("\(name): threw \(error)") }
    FileHandle.standardError.write(Data(((failures.count == before ? "PASS " : "FAIL ") + name + "\n").utf8))
}

func ids(_ count: Int) -> [UUID] { (0..<count).map { _ in UUID() } }

/// Whole after every step, and every split normalized.
func expectSound(_ layout: WorkspaceLayout, _ step: String, line: Int = #line) {
    expect(layout.isWhole, "\(step): layout whole", line: line)
    expect(layout.grid == layout.grid.normalized(), "\(step): grid normalized", line: line)
    func sums(_ grid: PaneLayout) -> Bool {
        guard case .split(let split) = grid else { return true }
        return abs(split.fractions.reduce(0, +) - 1) < 1e-9 && split.fractions.allSatisfy { $0 > 0 } && split.children.allSatisfy(sums)
    }
    expect(sums(layout.grid), "\(step): fractions sum to one", line: line)
}

check("selection follows the tab bar's history") {
    let t = ids(4)
    var layout = WorkspaceLayout(tab: t[0])
    layout.insert(t[1]); layout.insert(t[2]); layout.insert(t[3])
    expect(layout.focusedGroup.tabs == t, "tabs appended in order")
    expect(layout.focusedTab == t[3], "inserted tab shown")
    layout.select(t[1])
    layout.remove(t[1])
    expect(layout.focusedTab == t[3], "closing the shown tab returns to the last shown one")
    layout.remove(t[3])
    expect(layout.focusedTab == t[0] || layout.focusedTab == t[2], "falls back through history")
    var fresh = WorkspaceLayout(grid: .pane(UUID()), groups: [:], focused: UUID())
    expect(fresh == nil, "a grid naming a missing group is not whole")
    let group = UUID()
    fresh = WorkspaceLayout(grid: .pane(group), groups: [group: TabGroup(tabs: [t[0], t[1], t[2]], selected: t[2])!], focused: group)
    fresh!.remove(t[2])
    expect(fresh!.focusedTab == t[1], "without history the left neighbour is shown")
    expect(TabGroup(tabs: []) == nil && TabGroup(tabs: [t[0], t[0]]) == nil && TabGroup(tabs: [t[0]], selected: t[1]) == nil,
           "empty, duplicate or foreign selection groups are refused")
}

check("the last tab of the window stays") {
    let t = ids(2)
    var layout = WorkspaceLayout(tab: t[0])
    expect(layout.remove(t[0]) == nil, "the window's only tab is not removed")
    expect(layout.tabIDs == [t[0]], "still there")
    layout.replaceLastTab(with: t[1])
    expect(layout.tabIDs == [t[1]], "replaced in place")
    expectSound(layout, "after replacement")
}

check("reordering along the bar") {
    let t = ids(4)
    var layout = WorkspaceLayout(tab: t[0])
    for tab in t.dropFirst() { layout.insert(tab) }
    layout.moveTab(t[0], toPlace: 2)
    expect(layout.focusedGroup.tabs == [t[1], t[2], t[0], t[3]], "place counts the tabs as they will be")
    layout.moveTab(t[3], toPlace: 0)
    expect(layout.focusedGroup.tabs == [t[3], t[1], t[2], t[0]], "to the front")
    layout.moveTab(t[3], toPlace: 99)
    expect(layout.focusedGroup.tabs == [t[1], t[2], t[0], t[3]], "clamped to the end")
    let group = layout.focused
    layout.move(t[1], to: group, at: 3)
    expect(layout.focusedGroup.tabs == [t[2], t[0], t[1], t[3]], "insertion point within the same group")
}

check("splitting and closing groups") {
    let t = ids(5)
    var layout = WorkspaceLayout(tab: t[0])
    let first = layout.focused
    layout.insert(t[1])
    layout.split(t[1], edge: .trailing)
    expectSound(layout, "split right")
    expect(layout.groupIDs.count == 2 && layout.groupIDs[0] == first, "new group on the right")
    expect(layout.focusedTab == t[1] && layout.groups[first]!.tabs == [t[0]], "tab moved and focused")
    let right = layout.focused
    layout.split(t[1], edge: .bottom)
    expect(layout.groupIDs.count == 2, "a group's only tab beside itself goes nowhere")
    layout.addGroup(with: t[2], beside: right, edge: .bottom)
    expectSound(layout, "split below")
    layout.addGroup(with: t[3], beside: first, edge: .trailing)
    expectSound(layout, "same-axis split shares its room")
    guard case .split(let root) = layout.grid else { return expect(false, "root is a split") }
    expect(root.axis == .horizontal && root.children.count == 3, "flat horizontal split of three")
    expect(abs(root.fractions[0] - 0.25) < 1e-9 && abs(root.fractions[1] - 0.25) < 1e-9 && abs(root.fractions[2] - 0.5) < 1e-9,
           "the split group shares its half")
    let below = layout.group(holding: t[2])!
    layout.focus(below)
    expect(layout.remove(t[2]) == .closedGroup(below), "a group closes with its last tab")
    expect(layout.focused == right, "focus goes to the group that took its room")
    expectSound(layout, "after closing below")
    guard case .split(let flattened) = layout.grid else { return expect(false, "still split") }
    expect(flattened.axis == .horizontal && flattened.children.count == 3 && flattened.children.allSatisfy { if case .pane = $0 { true } else { false } },
           "a vertical split left with one child joins the horizontal one")
    expect(layout.closeGroup(right) == [t[1]], "closing a group returns its tabs")
    let middle = layout.group(holding: t[3])!
    expect(layout.closeGroup(middle) == [t[3]], "and the next")
    expect(layout.closeGroup(first).isEmpty, "the only group stays")
    expect(layout.grid == .pane(first), "back to one group")
    expectSound(layout, "one group")
    _ = t[4]
}

check("moving across groups") {
    let t = ids(3)
    var layout = WorkspaceLayout(tab: t[0])
    let left = layout.focused
    layout.addGroup(with: t[1], edge: .trailing)
    let right = layout.focused
    layout.insert(t[2], in: right)
    layout.move(t[2], to: left, at: 0)
    expect(layout.groups[left]!.tabs == [t[2], t[0]] && layout.focused == left && layout.focusedTab == t[2], "inserted before the first tab and focused")
    layout.move(t[1], to: left)
    expect(!layout.isSplit && layout.groups[left]!.tabs == [t[2], t[0], t[1]], "the emptied group closes")
    expectSound(layout, "after moving")
    layout.split(t[2], edge: .leading)
    let added = layout.focused
    layout.split(t[2], beside: left, edge: .bottom)
    expect(layout.groups[added] == nil && layout.groupIDs.count == 2, "moving a group's only tab beside another closes it")
    expectSound(layout, "after moving beside another group")
}

check("sizes") {
    let t = ids(3)
    var layout = WorkspaceLayout(tab: t[0])
    layout.addGroup(with: t[1], edge: .trailing)
    layout.addGroup(with: t[2], edge: .bottom)
    layout.resize(at: [], fractions: [3, 1])
    guard case .split(let root) = layout.grid else { return expect(false, "split") }
    expect(abs(root.fractions[0] - 0.75) < 1e-9, "fractions normalized")
    layout.resize(at: [], fractions: [1])
    expect(layout.grid == .split(root), "wrong count changes nothing")
    layout.resize(at: [7], fractions: [1, 1])
    expect(layout.grid == .split(root), "missing path changes nothing")
    layout.resize(at: [1], fractions: [-1, .nan])
    expectSound(layout, "invalid fractions become equal shares")
    expect(layout.grid.split(at: [1])?.fractions == [0.5, 0.5], "equal shares")
    layout.equalize(at: [])
    expect(layout.grid.split(at: [])?.fractions == [0.5, 0.5], "equalized")
    expect(layout.grid.path(to: layout.group(holding: t[2])!) == [1, 1], "path to the lower right group")
}

check("saving and restoring") {
    let t = ids(4)
    var layout = WorkspaceLayout(tab: t[0])
    layout.insert(t[1])
    layout.addGroup(with: t[2], edge: .trailing)
    layout.insert(t[3])
    layout.select(t[2])
    let data = try JSONEncoder().encode(layout)
    let decoded = try JSONDecoder().decode(WorkspaceLayout.self, from: data)
    expect(decoded.grid == layout.grid && decoded.focused == layout.focused && decoded.groups.mapValues(\.tabs) == layout.groups.mapValues(\.tabs)
           && decoded.groups.mapValues(\.selected) == layout.groups.mapValues(\.selected), "round trip keeps grid, tabs, selection and focus")
    let group = UUID(), other = UUID()
    let foreign = try JSONEncoder().encode(["grid": PaneLayout.pane(group)])
    expect((try? JSONDecoder().decode(WorkspaceLayout.self, from: foreign)) == nil, "incomplete data refused")
    let doubled = #"{"grid":{"pane":{"_0":"\#(group)"}},"groups":["\#(group)",{"tabs":["\#(t[0])"],"selected":"\#(t[0])"},"\#(other)",{"tabs":["\#(t[0])"],"selected":"\#(t[0])"}],"focused":"\#(group)"}"#
    expect((try? JSONDecoder().decode(WorkspaceLayout.self, from: Data(doubled.utf8))) == nil, "a group outside the grid or a tab in two groups is refused")
    let side = layout.excluding([t[2], t[3]])
    expect(side.map { !$0.isSplit && $0.tabIDs == [t[0], t[1]] } == true, "temporary tabs and their emptied group dropped")
    expect(side?.isWhole == true, "still whole")
    expect(WorkspaceLayout(tab: t[0]).excluding([t[0]]) == nil, "nothing left")
}

check("random operations keep the layout whole") {
    var generator = SystemRandomNumberGenerator()
    for _ in 0..<200 {
        var layout = WorkspaceLayout(tab: UUID())
        for step in 0..<60 {
            let tabs = layout.tabIDs, groups = layout.groupIDs
            let tab = tabs.randomElement(using: &generator)!, group = groups.randomElement(using: &generator)!
            let edge = [PaneEdge.leading, .trailing, .top, .bottom].randomElement(using: &generator)!
            switch Int.random(in: 0..<9, using: &generator) {
            case 0: layout.insert(UUID(), in: group, at: Int.random(in: -1...5, using: &generator))
            case 1: layout.addGroup(with: UUID(), beside: group, edge: edge)
            case 2: layout.remove(tab)
            case 3: layout.closeGroup(group)
            case 4: layout.moveTab(tab, toPlace: Int.random(in: -1...6, using: &generator))
            case 5: layout.move(tab, to: group, at: Bool.random(using: &generator) ? nil : Int.random(in: 0...4, using: &generator))
            case 6: layout.split(tab, beside: Bool.random(using: &generator) ? nil : group, edge: edge)
            case 7: layout.resize(at: layout.grid.path(to: group).map { Array($0.dropLast()) } ?? [], fractions: (0..<Int.random(in: 1...4, using: &generator)).map { _ in Double.random(in: -0.5...2, using: &generator) })
            default: layout.select(tab)
            }
            expectSound(layout, "random step \(step)")
            if failures.count > 20 { return }
        }
    }
}

FileHandle.standardError.write(Data("\(passed) expectations passed, \(failures.count) failed\n".utf8))
if !failures.isEmpty {
    for failure in failures.prefix(30) { FileHandle.standardError.write(Data(("  " + failure + "\n").utf8)) }
    exit(1)
}
