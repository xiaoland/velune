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
    /// A floating panel is no scene: while one is key, its window is the target.
    private var target: ConversationCommandTarget? {
        focusedTarget ?? ConversationWindowModel.owning(panel: NSApp.keyWindow).map { window in
            ConversationCommandTarget(store: window.focused.store, send: window.focused.send, window: window)
        }
    }
    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("新建会话") { target?.store.createConversation() }
                .keyboardShortcut("n")
                .disabled(target == nil || target?.store.isLoading == true)
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
            Button("关闭面板") { if let window = target?.window { window.close(window.focusedPaneID) } }
                .keyboardShortcut("w", modifiers: [.command, .shift])
                .disabled(target?.window.isSplit != true)
            Button("下一个面板") { target?.window.focusNeighbour(forward: true) }
                .keyboardShortcut("]")
                .disabled(target?.window.isSplit != true)
            Button("上一个面板") { target?.window.focusNeighbour(forward: false) }
                .keyboardShortcut("[")
                .disabled(target?.window.isSplit != true)
            Divider()
        }
        CommandGroup(before: .sidebar) {
            Picker("会话列表", selection: toolBinding(\.list, default: .docked)) {
                Text("停靠在侧栏").tag(ToolWindowLayout.ListPlacement.docked)
                Text("浮动").tag(ToolWindowLayout.ListPlacement.floating)
                Text("隐藏").tag(ToolWindowLayout.ListPlacement.hidden)
            }
            .disabled(target == nil)
            Picker("会话大纲", selection: outlineChoice) {
                Text("弹出").tag(OutlineChoice.popover)
                Text("就地折叠").tag(OutlineChoice.inPlace)
                Text("停靠在右侧").tag(OutlineChoice.docked)
                Text("浮动").tag(OutlineChoice.floating)
            }
            .disabled(target == nil)
            Picker("输入区", selection: toolBinding(\.composer, default: .docked)) {
                Text("停靠在面板底部").tag(ToolWindowLayout.ComposerPlacement.docked)
                Text("浮动").tag(ToolWindowLayout.ComposerPlacement.floating)
            }
            .disabled(target == nil)
            Divider()
        }
    }

    /// Popover and in place are how the toolbar button shows the outline, an
    /// application setting; docked and floating are this window's.
    private var outlineChoice: Binding<OutlineChoice> {
        Binding(get: {
            guard let window = target?.window else { return .popover }
            switch window.tools.outline {
            case .docked: return .docked
            case .floating: return .floating
            case .toolbar: return window.application.outlinePresentation == .inPlace ? .inPlace : .popover
            }
        }, set: { choice in
            guard let window = target?.window else { return }
            switch choice {
            case .popover, .inPlace:
                window.tools.outline = .toolbar
                let presentation: BindingOutlinePresentation = choice == .inPlace ? .inPlace : .popover
                if window.application.outlinePresentation != presentation { window.application.setOutlinePresentation(presentation) }
            case .docked: window.tools.outline = .docked
            case .floating: window.tools.outline = .floating
            }
            window.tools.outlineShown = true
        })
    }

    /// A placement of the target window's tools.
    private func toolBinding<Value>(_ path: WritableKeyPath<ToolWindowLayout, Value>, default value: Value) -> Binding<Value> {
        Binding(get: { target?.window.tools[keyPath: path] ?? value },
                set: { new in
                    guard let window = target?.window else { return }
                    window.tools[keyPath: path] = new
                    window.tools.outlineShown = true
                })
    }
}

@MainActor
struct ConversationWindow: View {
    @StateObject private var window: ConversationWindowModel
    /// The window's panes as last arranged, restored with the window.
    @SceneStorage("paneLayout") private var savedLayout = ""
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
        VeluneRootView(window: window, store: window.list, pane: window.focused, focused: window.focused.store,
                       previewEmpty: previewEmpty, previewSettings: previewSettings)
            .onAppear {
                window.openWindow = { [openWindow] in openWindow(id: WindowSeed.sceneID, value: $0) }
                let saved = savedLayout.data(using: .utf8).flatMap { try? JSONDecoder().decode(SavedWindowLayout.self, from: $0) }
                window.start(restoring: saved, seed: seed)
                // Development check of split panes without driving the interface.
                if CommandLine.arguments.contains("--check-split-panes"), !window.isSplit { window.split(.horizontal); window.split(.vertical) }
                if CommandLine.arguments.contains("--check-tool-windows") { window.tools.float(.list); window.tools.float(.outline); window.tools.float(.composer) }
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
