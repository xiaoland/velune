import SwiftUI

/// The window toolbar's buttons the user can hide and reorder in the
/// settings. From macOS 26 the system's own Customize Toolbar… does this
/// (see `VeluneRootView.withToolbar`); before it a customizable toolbar
/// costs scrolling speed, so Velune offers the same choices itself over a
/// plain toolbar.
enum ToolbarButton: String, CaseIterable, Identifiable {
    case reload, outline, subAgents, pin, split, problems
    case listFilter, newConversation

    var id: String { rawValue }

    /// The toolbar a button belongs to: the detail's, at the window's trailing
    /// end, or the sidebar's.
    enum Area: CaseIterable {
        case detail, sidebar
        var title: String { self == .detail ? "详情" : "侧栏" }
    }

    var area: Area {
        switch self {
        case .listFilter, .newConversation: .sidebar
        default: .detail
        }
    }

    var title: String {
        switch self {
        case .reload: "重新读取"
        case .outline: "会话大纲"
        case .subAgents: "关联 Agent"
        case .pin: "保持在最前面"
        case .split: "拆分"
        case .problems: "问题"
        case .listFilter: "筛选与分组"
        case .newConversation: "新建会话"
        }
    }

    var symbol: String {
        switch self {
        case .reload: "arrow.clockwise"
        case .outline: "list.bullet"
        case .subAgents: "person.2"
        case .pin: "pin"
        case .split: "rectangle.split.2x1"
        case .problems: "exclamationmark.triangle"
        case .listFilter: "line.3.horizontal.decrease.circle"
        case .newConversation: "square.and.pencil"
        }
    }
}

/// Which buttons the plain toolbars show, and in what order; kept in user
/// defaults. A button the app adds later appears at the end of its toolbar.
@MainActor final class ToolbarSettings: ObservableObject {
    static let shared = ToolbarSettings()

    private static let orderKey = "toolbarButtonOrder"
    private static let hiddenKey = "toolbarButtonsHidden"

    /// Every button in the user's order; each toolbar takes its own in turn.
    @Published private(set) var order: [ToolbarButton]
    @Published private(set) var hidden: Set<ToolbarButton>

    private init() {
        let defaults = UserDefaults.standard
        let saved = (defaults.stringArray(forKey: Self.orderKey) ?? []).compactMap(ToolbarButton.init(rawValue:))
        order = saved + ToolbarButton.allCases.filter { !saved.contains($0) }
        hidden = Set((defaults.stringArray(forKey: Self.hiddenKey) ?? []).compactMap(ToolbarButton.init(rawValue:)))
    }

    var isDefault: Bool { hidden.isEmpty && order == ToolbarButton.allCases }

    /// All of a toolbar's buttons in order, for the settings.
    func buttons(in area: ToolbarButton.Area) -> [ToolbarButton] { order.filter { $0.area == area } }

    /// The ones it shows, in order.
    func shown(in area: ToolbarButton.Area) -> [ToolbarButton] { buttons(in: area).filter { !hidden.contains($0) } }

    func isShown(_ button: ToolbarButton) -> Bool { !hidden.contains(button) }

    func setShown(_ button: ToolbarButton, _ shown: Bool) {
        if shown { hidden.remove(button) } else { hidden.insert(button) }
        save()
    }

    func move(in area: ToolbarButton.Area, from source: IndexSet, to destination: Int) {
        var buttons = buttons(in: area)
        buttons.move(fromOffsets: source, toOffset: destination)
        var next = buttons.makeIterator()
        order = order.map { $0.area == area ? next.next() ?? $0 : $0 }
        save()
    }

    func reset() {
        order = ToolbarButton.allCases
        hidden = []
        save()
    }

    private func save() {
        UserDefaults.standard.set(order.map(\.rawValue), forKey: Self.orderKey)
        UserDefaults.standard.set(hidden.map(\.rawValue).sorted(), forKey: Self.hiddenKey)
    }
}

/// The settings' toolbar sections, for macOS before 26.
struct ToolbarSettingsSection: View {
    @ObservedObject private var settings = ToolbarSettings.shared

    var body: some View {
        ForEach(ToolbarButton.Area.allCases, id: \.self) { area in
            Section("\(area.title)工具栏") {
                List {
                    ForEach(settings.buttons(in: area)) { button in
                        Toggle(isOn: Binding(get: { settings.isShown(button) }, set: { settings.setShown(button, $0) })) {
                            Label(button.title, systemImage: button.symbol)
                        }
                    }
                    .onMove { settings.move(in: area, from: $0, to: $1) }
                }
                if area == .sidebar {
                    Text("勾选决定按钮是否显示，拖动调整顺序。有些按钮只在适用时才出现（关联 Agent、保持在最前面、问题）。").font(.caption).foregroundStyle(.secondary)
                    Button("恢复默认") { settings.reset() }.disabled(settings.isDefault)
                }
            }
        }
    }
}
