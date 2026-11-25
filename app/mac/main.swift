import AppKit
import Combine
import SwiftUI
import VeluneBindings
import OSLog

let windowRecoveryLogger = Logger(subsystem: Bundle.main.bundleIdentifier ?? "Velune", category: "WindowRecovery")

/// The focused pane of the key window, and its window.
@MainActor struct ConversationCommandTarget {
    let window: ConversationWindowModel
    var store: ConversationStore { window.commandTab.store }
    func send() { window.send() }
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
    /// The key window's model, followed through AppKit: the windows are no scenes.
    @ObservedObject private var windows = WorkspaceWindows.shared
    @ObservedObject private var pinning = WindowPinning.shared
    @ObservedObject private var shortcuts = KeyboardShortcutSettings.shared
    /// The key workspace or conversation window, or the one whose tool window is key.
    private var target: ConversationCommandTarget? {
        // A message editor owns its input; conversation commands must not
        // silently act on the source tab behind its floating panel.
        guard MessageEditingSession.owning(window: NSApp.keyWindow) == nil else { return nil }
        return windows.commandTarget.map { ConversationCommandTarget(window: $0) }
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
            // Also with no window open: windows are opened from here, not by SwiftUI.
            Button("新建窗口") { WorkspaceWindows.shared.open(.workspace) }
                .keyboardShortcut(shortcuts[.newWindow])
            item(.reopenClosedTab)
        }
        // Not in place of the save items: an app without documents shows none of that group.
        // Above the system's Close, which keeps ⌘W too, so the menu takes these first.
        CommandGroup(after: .newItem) {
            Divider()
            // A conversation window closes its shown tab; the last one closes the window. Any other window closes.
            Button("关闭标签页") {
                let keyWindow = NSApp.keyWindow
                if let edit = MessageEditingSession.owning(window: keyWindow) { edit.cancel() }
                else if let window = ConversationWindowModel.owning(window: keyWindow?.sheetParent ?? keyWindow) { window.perform(.closeTab) }
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
                if let target, !target.store.isSideChat, let id = target.store.loadedConversationID,
                   let conversation = target.store.conversation(id: id),
                   let share = HapiShareTarget.local(conversation, device: target.store.device) {
                    Button("分享到 HAPI…") { target.window.hapiShareTarget = share }
                }
                Divider()
                item(.chooseRuntime)
                item(.chooseModel)
                item(.chooseEffort)
                item(.choosePermission)
                item(.chooseWorkingDirectory)
                Divider()
                item(.nextAttention)
                item(.reload)
                Divider()
                Button("停止生成") { target?.store.cancel() }
                    .keyboardShortcut(shortcuts[.stop])
                    .disabled(target?.store.canCancel != true)
                Button("停止全部后台工作") { target?.store.stopSubAgents() }
                    .keyboardShortcut(shortcuts[.stopSubAgents])
                    .disabled(target?.store.followUp.canStopSubAgents != true)
                if let store = target?.store {
                    switch store.controlHandoff {
                    case .canRelease:
                        Button("释放会话控制权") { store.setConversationHandoff(release: true) }.disabled(store.isLoading)
                    case .releasing:
                        Button("取消交接") { store.setConversationHandoff(release: false) }.disabled(store.isLoading)
                    case .released:
                        Button("恢复 Velune 执行") { store.setConversationHandoff(release: false) }.disabled(store.isLoading)
                    case nil:
                        EmptyView()
                    }
                }
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
                ForEach(ShortcutCommand.numberedTabs) { command in
                    Button(command.title) { target?.window.perform(command) }
                        .keyboardShortcut(shortcuts[command])
                        .disabled(target?.window.canPerform(command) != true)
                }
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
        CommandGroup(before: .sidebar) {
            following(AppearancePreferences.shared) {
                ForEach(ShortcutCommand.zoomCommands) { command in
                    Button(command.title) { command.performZoom() }
                        .keyboardShortcut(shortcuts[command])
                        .disabled(!command.canPerformZoom)
                }
                Divider()
            }
        }
        CommandGroup(before: .sidebar) { followingFocus {
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
        case .backgroundTasks where window.tools[.backgroundTasks].place == .builtIn: window.backgroundTasksPopover
        default: window.tools.isOpen(kind)
        }
        return (shown ? "隐藏" : "显示") + kind.title
    }
}

/// Shown while a window waits for what it restores.
struct WindowRestorationPendingView: View {
    var body: some View {
        ProgressView("正在恢复窗口…")
            .uiFont(.callout)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(nsColor: .windowBackgroundColor))
    }
}

/// A workspace or conversation window's content, hosted by its AppKit
/// window controller. It starts the window's model once it appears; the
/// window's journal is its only recovery point.
@MainActor
struct ConversationWindowContent: View {
    @ObservedObject var window: ConversationWindowModel
    let seed: WindowSeed?
    let persistenceID: String
    /// Reopened by the system: only its own recovery point applies.
    let restoring: Bool
    let previewEmpty: Bool
    let previewSettings: Bool

    private func traceRecovery(_ phase: String) {
        let seedID = seed?.id.uuidString ?? "none"
        let tabID = seed?.tab?.uuidString ?? "none"
        let kindID = window.kind.sceneID
        let modelID = window.restorationLifecycleID.uuidString
        let hasConversation = seed?.conversationID != nil
        let activeTabID = window.focused.id.uuidString
        windowRecoveryLogger.debug("phase=\(phase, privacy: .public) model=\(modelID, privacy: .public) kind=\(kindID, privacy: .public) seed=\(seedID, privacy: .public) tab=\(tabID, privacy: .public) hasConversation=\(hasConversation) window=\(persistenceID, privacy: .public) restoring=\(restoring) activeTab=\(activeTabID, privacy: .public)")
    }

    var body: some View {
        VeluneRootView(window: window, store: window.list, tab: window.focused, focused: window.focused.store,
                       previewEmpty: previewEmpty, previewSettings: previewSettings)
            .applicationAppearance()
            .onAppear {
                traceRecovery("appear")
                window.start(restoring: nil, seed: seed, persistenceID: persistenceID, reopened: restoring)
                traceRecovery("started")
                // Development check of split panes without driving the interface.
                if CommandLine.arguments.contains("--check-split-panes"), !window.isSplit { window.split(.horizontal); window.openInNewTab(); window.split(.vertical) }
                if CommandLine.arguments.contains("--check-tool-windows") { window.tools.move(.list, to: .window); window.tools.move(.outline, to: .window); window.tools.move(.composer, to: .window) }
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
            .sheet(isPresented: $window.showsAgentURI) { AgentURIView(window: window) }
    }
}

@MainActor
final class VeluneApplicationDelegate: NSObject, NSApplicationDelegate {
    weak var store: DeviceStore?
    private var terminationPending = false

    func applicationWillFinishLaunching(_ notification: Notification) {
        // Links come here rather than to a scene: no scene shows conversations.
        NSAppleEventManager.shared().setEventHandler(self, andSelector: #selector(handleURLEvent(_:reply:)),
                                                     forEventClass: AEEventClass(kInternetEventClass), andEventID: AEEventID(kAEGetURL))
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        // After the windows the system restores have been made.
        DispatchQueue.main.async { WorkspaceWindows.shared.finishLaunching() }
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool {
        if !hasVisibleWindows { WorkspaceWindows.shared.reopen() }
        return false
    }

    @objc private func handleURLEvent(_ event: NSAppleEventDescriptor, reply: NSAppleEventDescriptor) {
        guard let text = event.paramDescriptor(forKeyword: AEKeyword(keyDirectObject))?.stringValue, let url = URL(string: text) else { return }
        WorkspaceWindows.shared.handle(url)
    }

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
enum VeluneEntry {
    static func main() {
        if CommandLine.arguments.contains("--check-conversation-requests") {
            admitMacExecution()
            conversationRequestChecks()
        }
        if CommandLine.arguments.contains("--check-notification-navigation") {
            admitMacExecution()
            notificationNavigationChecks()
        }
        if CommandLine.arguments.contains("--check-outgoing-submissions") {
            admitMacExecution()
            outgoingSubmissionChecks()
        }
        if CommandLine.arguments.contains("--check-sidebar-toolbar") {
            admitMacExecution()
            sidebarToolbarChecks()
        }
        if CommandLine.arguments.contains("--check-shell-motion") {
            admitMacExecution()
            shellMotionChecks()
        }
        if CommandLine.arguments.contains("--check-background-notifications") {
            admitMacExecution()
            backgroundNotificationChecks()
        }
        if CommandLine.arguments.contains("--check-provider-analytics") {
            admitMacExecution()
            providerAnalyticsOffscreenChecks()
        }
        if CommandLine.arguments.contains("--check-archived-conversations") {
            admitMacExecution()
            archivedConversationsChecks()
        }
        if CommandLine.arguments.contains("--check-palette-ime") {
            admitMacExecution()
            commandPaletteChecks()
        }
        if CommandLine.arguments.contains("--check-list-pagination") {
            admitMacExecution()
            conversationPaginationChecks()
        }
        if CommandLine.arguments.contains("--check-background-tasks") {
            admitMacExecution()
            backgroundTaskChecks()
        }
        if CommandLine.arguments.contains("--check-continuation-preparation") {
            admitMacExecution()
            continuationPreparationChecks()
        }
        if CommandLine.arguments.contains("--check-queued-message-editor") {
            admitMacExecution()
            queuedMessageEditorChecks()
        }
        if CommandLine.arguments.contains("--check-composer-ownership") {
            admitMacExecution()
            composerOwnershipCheck()
        }
        if CommandLine.arguments.contains("--check-composer-controls"),
           CommandLine.arguments.contains("--check-editor-visual") {
            // Programmatic AppKit checks own their lifecycle independently;
            // ordinary SwiftUI app, device and scene construction stays untouched.
            admitMacExecution()
            let directory = URL(fileURLWithPath: ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"] ?? "/tmp/velune-next-ux/composer", isDirectory: true)
            try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            messageEditorVisualCheck(directory: directory)
            exit(0)
        }
        if CommandLine.arguments.contains("--check-message-experience"),
           CommandLine.arguments.contains("--experience-message-editor-offscreen")
            || CommandLine.arguments.contains("--experience-fork-send-only")
            || CommandLine.arguments.contains("--experience-claude-compact-edit-only")
            || CommandLine.arguments.contains("--experience-appearance-zoom-only")
            || CommandLine.arguments.contains("--experience-runtime-editor-only")
            || CommandLine.arguments.contains("--experience-compaction-only")
            || CommandLine.arguments.contains("--experience-sidebar-only")
            || CommandLine.arguments.contains("--experience-runtime-recovery-only")
            || CommandLine.arguments.contains("--experience-workspace-only") {
            // Enter before SwiftUI initializes its App/delegate/scene machinery.
            admitMacExecution()
            messageExperienceChecks()
        }
        VeluneApp.main()
    }
}

@MainActor private func admitMacExecution() {
    do {
        let context = try MacExecutionContext.application()
        try context.activateDevelopmentEnvironment()
        try context.validateDevelopmentChecks(CommandLine.arguments)
        try context.validateCheckDirectories(environment: ProcessInfo.processInfo.environment)
    } catch { MacExecutionContext.failAndExit(error) }
}

@MainActor
struct VeluneApp: App {
    @NSApplicationDelegateAdaptor(VeluneApplicationDelegate.self) private var appDelegate
    @StateObject private var store: DeviceStore
    private let previewEmpty: Bool
    private let previewSettings: Bool

    init() {
        // Admit purpose and home before checks, telemetry, defaults-backed
        // preferences or any core/device object can be initialized.
        admitMacExecution()
        let retiredComposerChecks: Set<String> = ["--check-composer-state", "--check-composer-recovery-write", "--check-composer-recovery-read"]
        if !retiredComposerChecks.isDisjoint(with: CommandLine.arguments) {
            FileHandle.standardError.write(Data("旧 Composer 双所有权检查已退役；请使用 --check-composer-ownership --composer-check-content-target 与 --check-composer-controls。\n".utf8))
            exit(2)
        }
        if CommandLine.arguments.contains("--check-composer-ownership") { composerOwnershipCheck() }
        if CommandLine.arguments.contains("--check-new-tab") { composerOwnershipCheck() }
        if CommandLine.arguments.contains("--check-composer-controls") { composerControlsCheck() }
        if CommandLine.arguments.contains("--check-runtime-interactions") { runtimeInteractionChecks() }
        if CommandLine.arguments.contains("--check-aggregated-list") { aggregatedListChecks() }
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
            let hub = DeviceHub(local: transport)
            hub.localStore = applicationStore
            hub.start()
        }
        KeyboardShortcutSettings.shared.load(home: nativeHome) { [weak applicationStore] in
            applicationStore?.recordProblem($0, source: "保存快捷键")
        }
        // Tabs live in each pane; the system's window tabs would stack a second, look-alike bar above them.
        NSWindow.allowsAutomaticWindowTabbing = false
        KeyRouter.install()
        GlobalHotKeys.shared.start()
        // Before AppKit restores windows, which it does as the application launches.
        WorkspaceWindows.shared.start(.init(store: applicationStore, previewEmpty: previewEmpty, previewSettings: previewSettings))
    }

    var body: some Scene {
        // Workspace and conversation windows are AppKit's (`WorkspaceWindows`);
        // the scenes here are the auxiliary windows, opened only when asked
        // for, and none takes a link: the application's handler does.
        Window("问题", id: "problems") {
            ProblemsView(local: store, hub: DeviceHub.shared).frame(minWidth: 600, minHeight: 320)
        }
        .defaultSize(width: 760, height: 480)
        .defaultLaunchBehavior(.suppressed)
        .handlesExternalEvents(matching: [])
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
                ScheduledTasksMenuItem()
                ArchivedConversationsMenuItem()
            }

        }
        Window("定时任务", id: "scheduled-tasks") {
            ScheduledTasksView(local: store, hub: DeviceHub.shared).frame(minWidth: 860, minHeight: 560)
        }.defaultSize(width: 1040, height: 700)
        .defaultLaunchBehavior(.suppressed)
        .handlesExternalEvents(matching: [])
        Window("已归档会话", id: "archived-conversations") {
            ArchivedConversationsView(local: store, hub: DeviceHub.shared).frame(minWidth: 680, minHeight: 420)
        }.defaultSize(width: 920, height: 640)
        .defaultLaunchBehavior(.suppressed)
        .handlesExternalEvents(matching: [])
        Window("资源库", id: "resource-library") {
            ResourceLibraryView(store: store).frame(minWidth: 820, minHeight: 500)
        }.defaultSize(width: 1040, height: 680)
        .defaultLaunchBehavior(.suppressed)
        .handlesExternalEvents(matching: [])
        Window("分析", id: "analytics") {
            AnalyticsView(store: store).frame(minWidth: 680, minHeight: 460)
        }.defaultSize(width: 860, height: 640)
        .defaultLaunchBehavior(.suppressed)
        .handlesExternalEvents(matching: [])
        Settings {
            SettingsView(store: store)
                .applicationAppearance()
        }
    }
}
