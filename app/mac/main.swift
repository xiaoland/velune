import AppKit
import SwiftUI
import VeluneBindings

/// The focused pane of the key window, and its window.
struct ConversationCommandTarget {
    let store: AppStore
    let send: () -> Void
    let window: ConversationWindowModel
}
private struct ConversationCommandTargetKey: FocusedValueKey {
    typealias Value = ConversationCommandTarget
}
extension FocusedValues {
    var conversationCommands: ConversationCommandTarget? {
        get { self[ConversationCommandTargetKey.self] }
        set { self[ConversationCommandTargetKey.self] = newValue }
    }
}

@MainActor
struct ConversationCommands: Commands {
    @FocusedValue(\.conversationCommands) private var focusedTarget
    /// A floating panel or tool window is no scene: while one is key, its window is the target.
    private var target: ConversationCommandTarget? {
        focusedTarget ?? ConversationWindowModel.owning(detached: NSApp.keyWindow).map { window in
            ConversationCommandTarget(store: window.focused.store, send: window.focused.send, window: window)
        }
    }
    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("新建会话") { target?.store.createConversation() }
                .keyboardShortcut("n")
                .disabled(target == nil || target?.store.isLoading == true)
            Button("新建标签页") { target?.window.openInNewTab() }
                .keyboardShortcut("t")
                .disabled(target == nil)
            Button("新建窗口") { target?.window.openWindow?(WindowSeed()) }
                .keyboardShortcut("n", modifiers: [.command, .option])
                .disabled(target?.window.openWindow == nil)
        }
        CommandGroup(replacing: .saveItem) {
            // A conversation window closes its shown tab; the last one closes the window. Any other window closes.
            Button("关闭标签页") {
                if let window = ConversationWindowModel.owning(window: NSApp.keyWindow) { window.closeTab(window.focused.id) }
                else { NSApp.keyWindow?.performClose(nil) }
            }
            .keyboardShortcut("w")
            Button("关闭窗口") { NSApp.keyWindow?.performClose(nil) }
                .keyboardShortcut("w", modifiers: [.command, .shift])
        }
        CommandMenu("会话") {
            Button("发送消息") { target?.send() }
                .keyboardShortcut(.return, modifiers: .command)
                .disabled(target?.store.canSend != true)
            Button("停止生成") { target?.store.cancel() }
                .keyboardShortcut(.escape, modifiers: [])
                .disabled(target?.store.canCancel != true)
        }
        CommandGroup(before: .windowArrangement) {
            Button("向右拆分") { target?.window.split(.horizontal) }
                .keyboardShortcut("d")
                .disabled(target == nil)
            Button("向下拆分") { target?.window.split(.vertical) }
                .keyboardShortcut("d", modifiers: [.command, .shift])
                .disabled(target == nil)
            Button("关闭分屏") { if let window = target?.window { window.closePane(window.focusedPaneID) } }
                .disabled(target?.window.isSplit != true)
            Button("下一个分屏") { target?.window.focusNeighbour(forward: true) }
                .keyboardShortcut("]")
                .disabled(target?.window.isSplit != true)
            Button("上一个分屏") { target?.window.focusNeighbour(forward: false) }
                .keyboardShortcut("[")
                .disabled(target?.window.isSplit != true)
            Divider()
            Button("下一个标签页") { target?.window.selectNeighbourTab(forward: true) }
                .keyboardShortcut("]", modifiers: [.command, .shift])
                .disabled((target?.window.focusedPane.tabs.count ?? 0) < 2)
            Button("上一个标签页") { target?.window.selectNeighbourTab(forward: false) }
                .keyboardShortcut("[", modifiers: [.command, .shift])
                .disabled((target?.window.focusedPane.tabs.count ?? 0) < 2)
            Button("将标签页移到新窗口") { if let window = target?.window { window.moveToNewWindow(window.focused.id) } }
                .disabled((target?.window.tabCount ?? 0) < 2)
            Divider()
        }
        CommandGroup(before: .sidebar) {
            ForEach(Array(ToolWindowKind.allCases.enumerated()), id: \.element) { index, kind in
                Button(toolTitle(kind)) { kind == .composer ? target?.window.focusComposer() : target?.window.toggle(kind) }
                    .keyboardShortcut(KeyEquivalent(Character("\(index + 1)")))
                    .disabled(target == nil)
            }
            Divider()
            ForEach(ToolWindowKind.allCases, id: \.self) { kind in
                Picker("\(kind.title)位置", selection: Binding(get: { target?.window.choice(for: kind) ?? .place(.builtIn) },
                                                           set: { target?.window.choose($0, for: kind) })) {
                    ForEach(ToolChoice.all(for: kind), id: \.choice) { Text($0.title).tag($0.choice) }
                }
                .disabled(target == nil)
            }
            Divider()
            Button(target?.window.tools.tools(in: .bottom).isEmpty == false ? "隐藏底部区域" : "显示底部区域") { target?.window.tools.toggleArea(.bottom) }
                .keyboardShortcut("y", modifiers: [.command, .shift])
                .disabled(target == nil)
            Button(target?.window.tools.isEditorOnly == true ? "恢复停靠区域" : "只显示编辑区") { target?.window.tools.toggleEditorOnly() }
                .keyboardShortcut(.return, modifiers: [.command, .shift])
                .disabled(target == nil)
            Divider()
        }
    }

    /// The list and outline show and hide; the composer, always somewhere, takes the keyboard.
    private func toolTitle(_ kind: ToolWindowKind) -> String {
        if kind == .composer { return "转到输入区" }
        guard let window = target?.window else { return "显示\(kind.title)" }
        let shown = kind == .outline && window.tools[.outline].place == .builtIn
            ? (window.application.outlinePresentation == .popover ? window.outlinePopover : window.focused.outlineCollapsed)
            : window.tools.isOpen(kind)
        return (shown ? "隐藏" : "显示") + kind.title
    }
}

@MainActor
struct ConversationWindow: View {
    @StateObject private var window: ConversationWindowModel
    /// The window's panes as last arranged, restored with the window.
    @SceneStorage("windowLayout") private var savedLayout = ""
    let seed: WindowSeed?
    @Environment(\.openWindow) private var openWindow
    let previewEmpty: Bool
    let previewSettings: Bool
    init(application: AppStore, seed: WindowSeed?, previewEmpty: Bool, previewSettings: Bool) {
        _window = StateObject(wrappedValue: ConversationWindowModel(application: application))
        self.seed = seed
        self.previewEmpty = previewEmpty
        self.previewSettings = previewSettings
    }
    var body: some View {
        VeluneRootView(window: window, store: window.list, tab: window.focused, focused: window.focused.store,
                       previewEmpty: previewEmpty, previewSettings: previewSettings)
            .onAppear {
                window.openWindow = { [openWindow] in openWindow(id: WindowSeed.sceneID, value: $0) }
                let saved = savedLayout.data(using: .utf8).flatMap { try? JSONDecoder().decode(SavedWindowLayout.self, from: $0) }
                window.start(restoring: saved, seed: seed)
                // Development check of split panes without driving the interface.
                if CommandLine.arguments.contains("--check-split-panes"), !window.isSplit { window.split(.horizontal); window.openInNewTab(); window.split(.vertical) }
                if CommandLine.arguments.contains("--check-tool-windows") { window.tools.move(.list, to: .floating); window.tools.move(.outline, to: .window); window.tools.move(.composer, to: .floating) }
                if CommandLine.arguments.contains("--check-dock-areas") { window.tools.move(.outline, to: .docked(.left)); window.tools.move(.list, to: .docked(.right)); window.tools.move(.composer, to: .docked(.bottom)) }
            }
            .onChange(of: window.savedLayout) { _, layout in
                if let data = try? JSONEncoder().encode(layout), let text = String(data: data, encoding: .utf8) { savedLayout = text }
            }
            // A conversation link opens in a window already showing Velune.
            .handlesExternalEvents(preferring: [ConversationLink.scheme], allowing: [ConversationLink.scheme])
            .onOpenURL { url in
                guard let link = ConversationLink.conversation(in: url) else { return }
                window.open(link.conversationID)
            }
    }
}

@MainActor
final class VeluneApplicationDelegate: NSObject, NSApplicationDelegate {
    weak var store: AppStore?
    private var terminationPending = false

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard !terminationPending else { return .terminateLater }
        if let unsaved = ResourceLibraryModel.shared?.unsavedNames, !unsaved.isEmpty {
            let alert = NSAlert()
            alert.messageText = "资源库中有未保存的修改"
            alert.informativeText = "退出将放弃对“\(unsaved.joined(separator: "”“"))”的修改。"
            alert.addButton(withTitle: "取消")
            alert.addButton(withTitle: "放弃并退出").hasDestructiveAction = true
            guard alert.runModal() == .alertSecondButtonReturn else { return .terminateCancel }
        }
        guard let store else { return .terminateNow }
        terminationPending = true
        store.shutdown { [weak self] closed in
            // Preview and unopened cores can complete synchronously. Reply only
            // after AppKit has entered its terminateLater state.
            DispatchQueue.main.async {
                self?.terminationPending = false
                sender.reply(toApplicationShouldTerminate: closed)
            }
        }
        return .terminateLater
    }
}

@main
@MainActor
struct VeluneApp: App {
    @NSApplicationDelegateAdaptor(VeluneApplicationDelegate.self) private var appDelegate
    @StateObject private var store: AppStore
    private let previewEmpty: Bool
    private let previewSettings: Bool

    init() {
        let preview = Bundle.main.bundleIdentifier == "local.velune.visual-preview" || CommandLine.arguments.contains { $0.hasPrefix("--preview") }
        let applicationStore = AppStore(preview: preview)
        _store = StateObject(wrappedValue: applicationStore)
        previewEmpty = CommandLine.arguments.contains("--preview-empty")
        previewSettings = CommandLine.arguments.contains("--preview-settings")
        appDelegate.store = applicationStore
        // Tabs live in each pane; the system's window tabs would stack a second, look-alike bar above them.
        NSWindow.allowsAutomaticWindowTabbing = false
    }

    var body: some Scene {
        WindowGroup("Velune", id: WindowSeed.sceneID, for: WindowSeed.self) { $seed in
            ConversationWindow(application: store, seed: seed, previewEmpty: previewEmpty, previewSettings: previewSettings)
                .frame(minWidth: 780, minHeight: 540)
        }
        .defaultSize(width: 1060, height: 760)
        .commands {
            CommandGroup(replacing: .appInfo) {
                Button("关于 Velune") {
                    var options: [NSApplication.AboutPanelOptionKey: Any] = [:]
                    if let version = Bundle.main.object(forInfoDictionaryKey: "VeluneDisplayVersion") as? String {
                        options[.applicationVersion] = version
                    }
                    NSApplication.shared.orderFrontStandardAboutPanel(options: options)
                }
                Button("开源许可…") {
                    if let url = Bundle.main.url(forResource: "THIRD_PARTY_NOTICES", withExtension: "md", subdirectory: "ThirdParty") { NSWorkspace.shared.open(url) }
                }
            }
            ConversationCommands()
            InspectorCommands()
            CommandGroup(after: .sidebar) {
                AnalyticsMenuItem()
                ResourceLibraryMenuItem()
                ProblemsMenuItem(store: store)
            }

        }
        Window("问题", id: "problems") {
            ProblemsView(store: store).frame(minWidth: 600, minHeight: 320)
        }.defaultSize(width: 760, height: 480)
        Window("资源库", id: "resource-library") {
            ResourceLibraryView(store: store).frame(minWidth: 820, minHeight: 500)
        }.defaultSize(width: 1040, height: 680)
        Window("分析", id: "analytics") {
            AnalyticsView(store: store).frame(minWidth: 680, minHeight: 460)
        }.defaultSize(width: 860, height: 640)
        Settings {
            SettingsView(store: store)
        }
    }
}
