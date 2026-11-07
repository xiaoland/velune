import AppKit
import Combine
import SwiftUI
import VeluneBindings

/// The focused pane of the key window, and its window.
@MainActor struct ConversationCommandTarget {
    let window: ConversationWindowModel
    var store: ConversationStore { window.commandTab.store }
    func send() { window.send() }
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

/// Menu items that follow `object`. A Commands body observes no object, so
/// an item it disabled stays as it was when the body last ran, shortcut too.
private struct Observing<Object: ObservableObject, Content: View>: View {
    @ObservedObject var object: Object
    @ViewBuilder let content: () -> Content
    var body: some View { content() }
}

@MainActor
struct ConversationCommands: Commands {
    @Environment(\.openWindow) private var openWindow
    @FocusedValue(\.conversationCommands) private var focusedTarget
    @ObservedObject private var pinning = WindowPinning.shared
    @ObservedObject private var shortcuts = KeyboardShortcutSettings.shared
    /// A floating panel or tool window is no scene: while one is key, its window is the target.
    private var target: ConversationCommandTarget? {
        focusedTarget ?? ConversationWindowModel.owning(detached: NSApp.keyWindow).map { window in
            ConversationCommandTarget(window: window)
        }
    }
    /// Items as `object` stands, or as they are without it.
    @ViewBuilder private func following<Object: ObservableObject, Content: View>(_ object: Object?, @ViewBuilder _ content: @escaping () -> Content) -> some View {
        if let object { Observing(object: object, content: content) } else { content() }
    }
    /// Items as the key window, its focused pane and that pane's tab stand.
    @ViewBuilder private func followingFocus<Content: View>(@ViewBuilder _ content: @escaping () -> Content) -> some View {
        following(target?.window) { following(target?.window.focusedPane) { following(target?.window.commandTab, content) } }
    }
    /// Items as the focused tab's conversation stands as well.
    @ViewBuilder private func followingConversation<Content: View>(@ViewBuilder _ content: @escaping () -> Content) -> some View {
        followingFocus { following(target?.window.commandTab.store) { following(target?.window.composerContext().model, content) } }
    }

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            followingConversation {
                Button("新建会话") { target?.window.createConversation() }
                    .keyboardShortcut(shortcuts[.newConversation])
                    .disabled(target == nil || target?.window.canPerform(.newConversation) != true)
            }
            item(.openConversation)
            Button("新建标签页") { target?.window.openInNewTab() }
                .keyboardShortcut(shortcuts[.newTab])
                .disabled(target == nil)
            Button("新建窗口") { target?.window.openWindow?(.workspace, WindowSeed()) }
                .keyboardShortcut(shortcuts[.newWindow])
                .disabled(target?.window.openWindow == nil)
            item(.reopenClosedTab)
        }
        // Not in place of the save items: an app without documents shows none of that group.
        // Above the system's Close, which keeps ⌘W too, so the menu takes these first.
        CommandGroup(after: .newItem) {
            Divider()
            // A conversation window closes its shown tab; the last one closes the window. Any other window closes.
            Button("关闭标签页") {
                let keyWindow = NSApp.keyWindow
                if let window = ConversationWindowModel.owning(window: keyWindow?.sheetParent ?? keyWindow) { window.perform(.closeTab) }
                else { NSApp.keyWindow?.performClose(nil) }
            }
            .keyboardShortcut(shortcuts[.closeTab])
            Button("关闭窗口") { NSApp.keyWindow?.performClose(nil) }
                .keyboardShortcut(shortcuts[.closeWindow])
        }
        // The system's Find items stay below: nothing here is a text view with a find bar.
        CommandGroup(before: .textEditing) {
            followingFocus {
                Button("查找…") { target?.window.find() }
                    .keyboardShortcut(shortcuts[.find])
                    .disabled(target == nil)
            }
            Divider()
        }
        CommandMenu("会话") {
            followingConversation {
                Button(target?.window.composerContext().submission().action == .queue ? "排队" : "发送消息") { target?.send() }
                    .keyboardShortcut(shortcuts.sendMode == .commandReturnSends ? KeyboardShortcut(.return, modifiers: .command) : nil)
                    .disabled(target?.window.canTargetConversation != true || target?.window.composerContext().submission().canSubmitDraft != true)
                Button("插入本轮") { target?.window.composerContext().steer() }
                    .keyboardShortcut(.return, modifiers: shortcuts.sendMode == .returnSends ? .command : [.command, .option])
                    .disabled(target?.window.canTargetConversation != true || target?.window.composerContext().canSteer != true)
                Button("继续") { target?.window.resume() }
                    .keyboardShortcut(shortcuts[.continueTurn])
                    .disabled(target?.window.canTargetConversation != true || target?.window.composerContext().submission().canResume != true)
                item(.compactContext)
                item(.sideChat)
                item(.attach)
                if let window = target?.window {
                    PreservedInputMenu(window: window, tab: window.commandTab, model: window.composerContext().model)
                }
                Divider()
                item(.chooseRuntime)
                item(.chooseModel)
                item(.chooseEffort)
                item(.choosePermission)
                item(.chooseWorkingDirectory)
                Divider()
                item(.nextAttention)
                Divider()
                Button("停止生成") { target?.store.cancel() }
                    .keyboardShortcut(shortcuts[.stop])
                    .disabled(target?.store.canCancel != true)
                Button("停止子 Agent") { target?.store.stopSubAgents() }
                    .keyboardShortcut(shortcuts[.stopSubAgents])
                    .disabled(target?.store.followUp.canStopSubAgents != true)
            }
        }
        CommandGroup(before: .windowArrangement) {
            following(target?.window) {
                Button("向右拆分") { target?.window.split(.horizontal) }
                    .keyboardShortcut(shortcuts[.splitRight])
                    .disabled(target == nil)
                Button("向下拆分") { target?.window.split(.vertical) }
                    .keyboardShortcut(shortcuts[.splitDown])
                    .disabled(target == nil)
                Button("关闭分屏") { if let window = target?.window { window.closePane(window.focusedPaneID) } }
                    .keyboardShortcut(shortcuts[.closeSplit])
                    .disabled(target?.window.isSplit != true)
            }
            followingConversation {
                Button("后退") { target?.window.focused.goBack() }
                    .keyboardShortcut(shortcuts[.back])
                    .disabled(target?.store.backHistory.isEmpty != false)
                Button("前进") { target?.window.focused.goForward() }
                    .keyboardShortcut(shortcuts[.forward])
                    .disabled(target?.store.forwardHistory.isEmpty != false)
            }
            Divider()
            followingFocus {
                Button("下一个分屏") { target?.window.focusNeighbour(forward: true) }
                    .keyboardShortcut(shortcuts[.nextPane])
                    .disabled(target?.window.isSplit != true)
                Button("上一个分屏") { target?.window.focusNeighbour(forward: false) }
                    .keyboardShortcut(shortcuts[.previousPane])
                    .disabled(target?.window.isSplit != true)
                Divider()
                Button("下一个标签页") { target?.window.selectNeighbourTab(forward: true) }
                    .keyboardShortcut(shortcuts[.nextTab])
                    .disabled((target?.window.focusedPane.tabs.count ?? 0) < 2)
                Button("上一个标签页") { target?.window.selectNeighbourTab(forward: false) }
                    .keyboardShortcut(shortcuts[.previousTab])
                    .disabled((target?.window.focusedPane.tabs.count ?? 0) < 2)
                Button("将标签页移到新窗口") { if let window = target?.window { window.moveToNewWindow(window.focused.id) } }
                    .keyboardShortcut(shortcuts[.moveTabToNewWindow])
                    .disabled(target?.window.canMoveTabToNewWindow != true)
            }
            Divider()
            Toggle("保持在最前面", isOn: Binding(get: { pinning.keyWindowPinned }, set: { _ in pinning.toggle() }))
                .keyboardShortcut(shortcuts[.keepInFront])
                .disabled(!pinning.canPin)
            Divider()
        }
        CommandGroup(before: .sidebar) { followingFocus {
            Button("本机定时任务…") { openWindow(id: "scheduled-tasks") }
            Divider()
            item(.commandPalette)
            Divider()
            item(.focusTranscript)
            ForEach(ToolWindowKind.allCases, id: \.self) { kind in
                Button(toolTitle(kind)) { kind == .composer ? target?.window.focusComposer() : target?.window.toggle(kind) }
                    .keyboardShortcut(shortcuts[ShortcutCommand.tool(kind)])
                    .disabled(target?.window.canToggle(kind) != true)
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
            Button(target?.window.tools.tools(in: .right).isEmpty == false ? "隐藏右侧区域" : "显示右侧区域") { target?.window.tools.toggleArea(.right) }
                .keyboardShortcut(shortcuts[.rightArea])
                .disabled(target == nil)
            Button(target?.window.tools.tools(in: .bottom).isEmpty == false ? "隐藏底部区域" : "显示底部区域") { target?.window.tools.toggleArea(.bottom) }
                .keyboardShortcut(shortcuts[.bottomArea])
                .disabled(target == nil)
            Button(target?.window.tools.isEditorOnly == true ? "恢复停靠区域" : "只显示编辑区") { target?.window.tools.toggleEditorOnly() }
                .keyboardShortcut(shortcuts[.editorOnly])
                .disabled(target == nil)
            Divider()
        } }
        CommandGroup(after: .help) { following(target?.window) { item(.shortcutHelp) } }
    }

    /// A command's menu item as the focused window performs it.
    private func item(_ command: ShortcutCommand) -> some View {
        followingConversation {
            Button(command.title) { target?.window.perform(command) }
                .keyboardShortcut(shortcuts[command])
                .disabled(target?.window.canPerform(command) != true)
        }
    }

    /// The list and outline show and hide; the composer, always somewhere, takes the keyboard.
    private func toolTitle(_ kind: ToolWindowKind) -> String {
        if kind == .composer { return "转到输入区" }
        guard let window = target?.window else { return "显示\(kind.title)" }
        let shown = switch kind {
        case .outline where window.tools[.outline].place == .builtIn:
            window.application.outlinePresentation == .popover ? window.outlinePopover : window.focused.outlineCollapsed
        case .agents where window.tools[.agents].place == .builtIn: window.agentsPopover
        default: window.tools.isOpen(kind)
        }
        return (shown ? "隐藏" : "显示") + kind.title
    }
}

@MainActor
struct ConversationWindow: View {
    @StateObject private var window: ConversationWindowModel
    /// The window's panes as last arranged, restored with the window.
    @SceneStorage("windowLayout") private var savedLayout = ""
    @SceneStorage("windowIdentity") private var windowIdentity = UUID().uuidString
    @State private var savedLayoutWritable = true
    @State private var startsWindowed: Bool
    let seed: WindowSeed?
    @Environment(\.openWindow) private var openWindow
    let previewEmpty: Bool
    let previewSettings: Bool
    init(application: DeviceStore, kind: ConversationWindowKind, seed: WindowSeed?, previewEmpty: Bool, previewSettings: Bool) {
        // A paired device's window shows that device's core through the same interface.
        let target = seed?.device.flatMap { DeviceHub.shared?.store(for: $0) } ?? application
        _window = StateObject(wrappedValue: ConversationWindowModel(application: target, kind: kind, device: seed?.device))
        _startsWindowed = State(initialValue: seed.map { WindowPresentation.windowed.contains($0.id) } ?? false)
        self.seed = seed
        self.previewEmpty = previewEmpty
        self.previewSettings = previewSettings
    }
    var body: some View {
        VeluneRootView(window: window, store: window.list, tab: window.focused, focused: window.focused.store,
                       previewEmpty: previewEmpty, previewSettings: previewSettings)
            .applicationAppearance()
            // Explicitly opened windows start as ordinary windows even when
            // the source is full screen. Re-enable full screen after presentation.
            .windowFullScreenBehavior(startsWindowed ? .disabled : .automatic)
            .onAppear {
                if !window.application.isPreview && !MacNativePersistenceGate.isEnabled {
                    savedLayoutWritable = false
                }
                if let seed { WindowPresentation.windowed.remove(seed.id) }
                window.openWindow = { [openWindow] kind, seed in
                    WindowPresentation.windowed.insert(seed.id)
                    openWindow(id: kind.sceneID, value: seed)
                }
                window.application.openConversationWindow = { [openWindow, device = window.device] seed in
                    var seed = seed; seed.device = device
                    WindowPresentation.windowed.insert(seed.id)
                    openWindow(id: ConversationWindowKind.workspace.sceneID, value: seed)
                }
                DispatchQueue.main.async {
                    if startsWindowed, window.window?.isKeyWindow == true { startsWindowed = false }
                }
                var saved: SavedWindowLayout?
                if !savedLayout.isEmpty && savedLayoutWritable {
                    do {
                        var decoded = try SavedWindowLayout.decodeKnown(Data(savedLayout.utf8))
                        if decoded.schemaVersion == 0 {
                            let backupKey = "velune.before-beta.windowLayout.\(windowIdentity)"
                            if let previous = UserDefaults.standard.string(forKey: backupKey), previous != savedLayout {
                                throw NSError(domain: "VelunePersistence", code: 22, userInfo: [NSLocalizedDescriptionKey: "窗口 SceneStorage 恢复副本与旧布局不一致"])
                            }
                            UserDefaults.standard.set(savedLayout, forKey: backupKey)
                            decoded.schemaVersion = 1
                            let converted = try JSONEncoder().encode(decoded)
                            guard let text = String(data: converted, encoding: .utf8) else { throw CocoaError(.fileWriteInapplicableStringEncoding) }
                            savedLayout = text
                        }
                        saved = decoded
                    } catch {
                        savedLayoutWritable = false
                        window.application.recordProblem(error, source: "窗口恢复")
                    }
                }
                window.start(restoring: saved, seed: seed, persistenceID: windowIdentity)
                // Development check of split panes without driving the interface.
                if CommandLine.arguments.contains("--check-split-panes"), !window.isSplit { window.split(.horizontal); window.openInNewTab(); window.split(.vertical) }
                if CommandLine.arguments.contains("--check-tool-windows") { window.tools.move(.list, to: .floating); window.tools.move(.outline, to: .window); window.tools.move(.composer, to: .floating) }
                if CommandLine.arguments.contains("--check-send-queue") { window.focused.checkSendQueue() }
                if CommandLine.arguments.contains("--check-edit-message") { window.focused.checkEditMessage() }
                if CommandLine.arguments.contains("--check-list-actions") { window.checkListActions() }
                if CommandLine.arguments.contains("--check-side-chat") { window.checkSideChat() }
                if CommandLine.arguments.contains("--check-group-creation") { window.checkGroupCreation() }
                if CommandLine.arguments.contains("--check-dock-areas") { window.tools.move(.outline, to: .docked(.left)); window.tools.move(.list, to: .docked(.right)); window.tools.move(.composer, to: .docked(.bottom)) }
                if window.application.isPreview, CommandLine.arguments.contains("--check-composer-minimum") {
                    window.tools.bottomHeight = Double(ComposerTextView.minimumHeight + 54)
                    UserDefaults.standard.set(true, forKey: "NSQuitAlwaysKeepsWindows")
                }
                if window.application.isPreview, CommandLine.arguments.contains("--check-composer-sharing") { window.checkComposerSharing() }
            }
            .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { note in
                guard startsWindowed, let shown = note.object as? NSWindow, shown === window.window else { return }
                DispatchQueue.main.async { startsWindowed = false }
            }
            .onChange(of: window.savedLayout) { _, layout in
                guard savedLayoutWritable else { return }
                if let data = try? JSONEncoder().encode(layout), let text = String(data: data, encoding: .utf8) { savedLayout = text }
                window.persistWorkspace()
            }
            .onReceive(window.composerDrafts.changes.debounce(for: .milliseconds(350), scheduler: RunLoop.main)) { _ in
                guard savedLayoutWritable else { return }
                if let data = try? JSONEncoder().encode(window.savedLayout), let text = String(data: data, encoding: .utf8) { savedLayout = text }
            }
            .onReceive(NotificationCenter.default.publisher(for: NSWindow.willCloseNotification)) { note in
                if let closed = note.object as? NSWindow, closed === window.window {
                    guard savedLayoutWritable else { window.noteWindowClosed(); return }
                    if let data = try? JSONEncoder().encode(window.savedLayout), let text = String(data: data, encoding: .utf8) { savedLayout = text; window.persistWorkspace() }
                    window.noteWindowClosed()
                }
            }
            .onReceive(NotificationCenter.default.publisher(for: NSApplication.willTerminateNotification)) { _ in
                guard savedLayoutWritable else { return }
                if let data = try? JSONEncoder().encode(window.savedLayout), let text = String(data: data, encoding: .utf8) { savedLayout = text }
            }
            // A conversation link opens in a window already showing Velune.
            .handlesExternalEvents(preferring: MacExecutionContext.admitted.externalEventSchemes, allowing: MacExecutionContext.admitted.externalEventSchemes)
            .sheet(isPresented: $window.showsAgentURI) { AgentURIView(window: window) }
            .onOpenURL { url in
                if !MacExecutionContext.admitted.isDevelopment, url.scheme == "agents" { window.openAgentURI(url.absoluteString); return }
                guard let link = ConversationLink.conversation(in: url) else { return }
                window.open(link.conversationID)
            }
    }
}

@MainActor
final class VeluneApplicationDelegate: NSObject, NSApplicationDelegate {
    weak var store: DeviceStore?
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
        let stoppingWork = store.hasRunningWork
        if stoppingWork {
            let alert = NSAlert()
            alert.messageText = "有会话或子 Agent 正在运行"
            alert.informativeText = "退出会停止它们。"
            alert.addButton(withTitle: "取消")
            alert.addButton(withTitle: "停止并退出").hasDestructiveAction = true
            guard alert.runModal() == .alertSecondButtonReturn else { return .terminateCancel }
        }
        store.rememberedToolLayout.flush()
        for window in ConversationWindowModel.openWindows { window.persistWorkspace() }
        terminationPending = true
        store.shutdown(stoppingWork: stoppingWork) { [weak self] closed in
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
    @StateObject private var store: DeviceStore
    private let previewEmpty: Bool
    private let previewSettings: Bool

    init() {
        // Admit purpose and home before checks, telemetry, defaults-backed
        // preferences or any core/device object can be initialized.
        do {
            let context = try MacExecutionContext.application()
            try context.activateDevelopmentEnvironment()
            try context.validateDevelopmentChecks(CommandLine.arguments)
            try context.validateCheckDirectories(environment: ProcessInfo.processInfo.environment)
        } catch { MacExecutionContext.failAndExit(error) }
        let retiredComposerChecks: Set<String> = ["--check-composer-state", "--check-composer-recovery-write", "--check-composer-recovery-read"]
        if !retiredComposerChecks.isDisjoint(with: CommandLine.arguments) {
            FileHandle.standardError.write(Data("旧 Composer 双所有权检查已退役；请使用 --check-composer-ownership --composer-check-content-target 与 --check-composer-controls。\n".utf8))
            exit(2)
        }
        if CommandLine.arguments.contains("--check-composer-ownership") { composerOwnershipCheck() }
        if CommandLine.arguments.contains("--check-new-tab") { composerOwnershipCheck() }
        if CommandLine.arguments.contains("--check-composer-controls") { composerControlsCheck() }
        if CommandLine.arguments.contains("--check-message-experience") { messageExperienceChecks() }
        let preview = CommandLine.arguments.contains { $0.hasPrefix("--preview") }
        MacNativePersistenceGate.close()
        let transport: Transport?
        do { transport = preview ? nil : try Transport.applicationDefault() }
        catch { MacExecutionContext.failAndExit(error) }
        var startupError: Error?
        if !preview, let transport {
            do {
                try transport.prepareForLaunch()
                MacNativePersistenceGate.open()
            } catch {
                startupError = error
            }
        }
        let applicationStore = DeviceStore(transport: transport, preview: preview)
        if let startupError { applicationStore.recordProblem(startupError, source: "初始化") }
        let nativeHome = MacNativePersistenceGate.isEnabled ? applicationStore.transport?.stateDirectory : nil
        AppearancePreferences.shared.load(home: nativeHome) { [weak applicationStore] detail in
            applicationStore?.recordProblem(detail, source: "外观设置")
        }
        _store = StateObject(wrappedValue: applicationStore)
        previewEmpty = CommandLine.arguments.contains("--preview-empty")
        previewSettings = CommandLine.arguments.contains("--preview-settings")
        appDelegate.store = applicationStore
        if !preview, MacNativePersistenceGate.isEnabled, let transport = applicationStore.transport {
            DeviceHub(local: transport).start()
        }
        KeyboardShortcutSettings.shared.load(home: nativeHome) { [weak applicationStore] in
            applicationStore?.recordProblem($0, source: "保存快捷键")
        }
        // Tabs live in each pane; the system's window tabs would stack a second, look-alike bar above them.
        NSWindow.allowsAutomaticWindowTabbing = false
        KeyRouter.install()
        GlobalHotKeys.shared.start()
    }

    var body: some Scene {
        WindowGroup("Velune", id: ConversationWindowKind.workspace.sceneID, for: WindowSeed.self) { $seed in
            ConversationWindow(application: store, kind: .workspace, seed: seed, previewEmpty: previewEmpty, previewSettings: previewSettings)
                .frame(minWidth: 520, minHeight: 420)
        }
        .defaultSize(width: 1060, height: 760)
        .commands {
            CommandGroup(replacing: .appInfo) {
                Button("关于 Velune") {
                    var options: [NSApplication.AboutPanelOptionKey: Any] = [:]
                    if let version = Bundle.main.object(forInfoDictionaryKey: "VeluneDisplayVersion") as? String {
                        options[.applicationVersion] = version
                    }
                    let projectURL = URL(string: "https://github.com/QuantumNous/new-api")!
                    let sourceTag = "v\(Bundle.main.object(forInfoDictionaryKey: "VeluneDisplayVersion") as? String ?? "")"
                    let sourceURL = URL(string: "https://github.com/xiaoland/velune/releases/download/\(sourceTag)/velune-\(sourceTag)-source.tar.gz")!
                    let projectLink = projectURL.absoluteString
                    let creditText = NSMutableAttributedString(
                        string: "Velune includes New API RelayKit for protocol conversion.\n\nFrontend design and development by New API contributors.\n\n\(projectLink)\n\nCorresponding source archive: \(sourceURL.absoluteString)"
                    )
                    creditText.addAttribute(
                        .link,
                        value: projectURL,
                        range: (creditText.string as NSString).range(of: projectLink)
                    )
                    let sourceLink = sourceURL.absoluteString
                    creditText.addAttribute(
                        .link,
                        value: sourceURL,
                        range: (creditText.string as NSString).range(of: sourceLink)
                    )
                    options[.credits] = creditText
                    NSApplication.shared.orderFrontStandardAboutPanel(options: options)
                }
                Button("开源许可…") {
                    if let url = Bundle.main.url(forResource: "THIRD_PARTY_NOTICES", withExtension: "md", subdirectory: "ThirdParty") { NSWorkspace.shared.open(url) }
                }
                Button("Velune AGPL-3.0-only 许可证…") {
                    if let url = Bundle.main.url(forResource: "LICENSE", withExtension: nil, subdirectory: "ThirdParty") { NSWorkspace.shared.open(url) }
                }
            }
            ConversationCommands()
            CommandGroup(after: .sidebar) {
                AnalyticsMenuItem()
                ResourceLibraryMenuItem()
                ProblemsMenuItem(store: store)
            }

        }
        // Its own scene: its windows' frames are not the next workspace
        // window's, and a window restores as one opened for a conversation.
        WindowGroup("Velune", id: ConversationWindowKind.single.sceneID, for: WindowSeed.self) { $seed in
            ConversationWindow(application: store, kind: .single, seed: seed, previewEmpty: previewEmpty, previewSettings: previewSettings)
                .frame(minWidth: 520, minHeight: 420)
        }
        .defaultSize(width: 720, height: 760)
        .defaultLaunchBehavior(.suppressed)
        Window("问题", id: "problems") {
            ProblemsView(store: store).frame(minWidth: 600, minHeight: 320)
        }.defaultSize(width: 760, height: 480)
        Window("本机定时任务", id: "scheduled-tasks") {
            ScheduledTasksView(store: store).frame(minWidth: 860, minHeight: 560)
        }.defaultSize(width: 1040, height: 700)
        Window("资源库", id: "resource-library") {
            ResourceLibraryView(store: store).frame(minWidth: 820, minHeight: 500)
        }.defaultSize(width: 1040, height: 680)
        Window("分析", id: "analytics") {
            AnalyticsView(store: store).frame(minWidth: 680, minHeight: 460)
        }.defaultSize(width: 860, height: 640)
        Settings {
            SettingsView(store: store)
                .applicationAppearance()
        }
    }
}
