import SwiftUI

/// The window toolbar's buttons the user can hide and reorder in the
/// settings. From macOS 26 the system's own Customize Toolbar… does this
/// (see `WorkspaceToolbar`); before it Velune offers the same choices
/// itself over a fixed toolbar.
enum ToolbarButton: String, CaseIterable, Identifiable {
    case reload, outline, subAgents, backgroundTasks, pin, split, problems

    var id: String { rawValue }

    var title: String {
        switch self {
        case .reload: "重新读取"
        case .outline: "会话大纲"
        case .subAgents: "关联 Agent"
        case .backgroundTasks: "后台任务"
        case .pin: "保持在最前面"
        case .split: "拆分"
        case .problems: "问题"
        }
    }

    var symbol: String {
        switch self {
        case .reload: "arrow.clockwise"
        case .outline: "list.bullet"
        case .subAgents: "person.2"
        case .backgroundTasks: "terminal"
        case .pin: "pin"
        case .split: "rectangle.split.2x1"
        case .problems: "exclamationmark.triangle"
        }
    }
}

/// Which buttons the plain toolbars show, and in what order; kept in user
/// defaults. A button the app adds later appears at the end; one it no
/// longer has (the list's filter and new conversation, now the list's own)
/// is left out.
@MainActor final class ToolbarSettings: ObservableObject {
    static let shared = ToolbarSettings()

    private static let orderKey = "toolbarButtonOrder"
    private static let hiddenKey = "toolbarButtonsHidden"

    /// Every button in the user's order.
    @Published private(set) var order: [ToolbarButton]
    @Published private(set) var hidden: Set<ToolbarButton>

    private init() {
        let defaults = UserDefaults.standard
        let saved = (defaults.stringArray(forKey: Self.orderKey) ?? []).compactMap(ToolbarButton.init(rawValue:))
        order = saved + ToolbarButton.allCases.filter { !saved.contains($0) }
        hidden = Set((defaults.stringArray(forKey: Self.hiddenKey) ?? []).compactMap(ToolbarButton.init(rawValue:)))
    }

    var isDefault: Bool { hidden.isEmpty && order == ToolbarButton.allCases }

    /// The ones the toolbar shows, in order.
    var shown: [ToolbarButton] { order.filter { !hidden.contains($0) } }

    func isShown(_ button: ToolbarButton) -> Bool { !hidden.contains(button) }

    func setShown(_ button: ToolbarButton, _ shown: Bool) {
        if shown { hidden.remove(button) } else { hidden.insert(button) }
        save()
    }

    func move(from source: IndexSet, to destination: Int) {
        order.move(fromOffsets: source, toOffset: destination)
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

/// The settings' toolbar section, for macOS before 26.
struct ToolbarSettingsSection: View {
    @ObservedObject private var settings = ToolbarSettings.shared

    var body: some View {
        Section("工具栏") {
            List {
                ForEach(settings.order) { button in
                    Toggle(isOn: Binding(get: { settings.isShown(button) }, set: { settings.setShown(button, $0) })) {
                        Label(button.title, systemImage: button.symbol)
                    }
                }
                .onMove { settings.move(from: $0, to: $1) }
            }
            Text("勾选决定按钮是否显示，拖动调整顺序。有些按钮只在适用时才出现（关联 Agent、后台任务、保持在最前面、问题）。").uiFont(.caption).foregroundStyle(.secondary)
            Button("恢复默认") { settings.reset() }.disabled(settings.isDefault)
        }
    }
}
