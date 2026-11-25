import AppKit
import Combine
import SwiftUI

/// The conversation window's toolbar items, by the identifiers AppKit keeps
/// a customized toolbar under.
enum ConversationToolbar {
    static let id = "conversation-window"

    enum Item: String, CaseIterable {
        case back, forward, device, operations
        case reload, outline
        case subAgents = "sub-agents"
        case backgroundTasks = "background-tasks"
        case pin, split, problems

        var identifier: NSToolbarItem.Identifier { .init(rawValue) }

        init?(_ button: ToolbarButton) {
            switch button {
            case .reload: self = .reload
            case .outline: self = .outline
            case .subAgents: self = .subAgents
            case .backgroundTasks: self = .backgroundTasks
            case .pin: self = .pin
            case .split: self = .split
            case .problems: self = .problems
            }
        }
    }
}

/// A conversation window's AppKit shell: the sidebar, the editor column and
/// the inspector in one split view controller, with the window's toolbar.
/// The left and right dock areas are its sidebar and inspector: they
/// collapse and expand with the system's animation and Show/Hide Sidebar,
/// keep their widths as the window resizes, and the widths they are given
/// are saved with the window's tools.
@MainActor final class WorkspaceShell {
    let split: WorkspaceSplitController
    let toolbar: WorkspaceToolbar

    init(window: NSWindow, model: ConversationWindowModel, content: some View, previewEmpty: Bool) {
        split = WorkspaceSplitController(model: model, content: content)
        toolbar = WorkspaceToolbar(model: model, previewEmpty: previewEmpty)
        window.titleVisibility = .hidden
        window.toolbarStyle = .unified
        window.contentViewController = split
        window.toolbar = toolbar.toolbar
        toolbar.attach(to: window)
    }
}

/// See `WorkspaceShell`.
@MainActor final class WorkspaceSplitController: NSSplitViewController {
    let model: ConversationWindowModel
    let sidebarItem: NSSplitViewItem
    let contentItem: NSSplitViewItem
    let inspectorItem: NSSplitViewItem
    private var subscriptions: Set<AnyCancellable> = []
    private var collapseObservations: [NSKeyValueObservation] = []
    private var syncScheduled = false
    /// Areas given their saved width since the window opened.
    private var sized: Set<DockSide> = []

    init(model: ConversationWindowModel, content: some View) {
        self.model = model
        let sidebar = NSHostingController(rootView: DockAreaView(window: model, side: .left).applicationAppearance())
        sidebar.sizingOptions = []
        sidebar.view.identifier = NSUserInterfaceItemIdentifier("conversation-sidebar-column")
        sidebarItem = NSSplitViewItem(sidebarWithViewController: sidebar)
        sidebarItem.minimumThickness = 190
        sidebarItem.maximumThickness = 600
        sidebarItem.canCollapse = true
        let editor = NSHostingController(rootView: content)
        editor.sizingOptions = []
        contentItem = NSSplitViewItem(viewController: editor)
        contentItem.minimumThickness = 320
        let inspector = NSHostingController(rootView: DockAreaView(window: model, side: .right).applicationAppearance())
        inspector.sizingOptions = []
        inspectorItem = NSSplitViewItem(inspectorWithViewController: inspector)
        inspectorItem.minimumThickness = 200
        inspectorItem.maximumThickness = 720
        inspectorItem.canCollapse = true
        super.init(nibName: nil, bundle: nil)
        let splitView = WindowSplitView()
        splitView.isVertical = true
        splitView.dividerStyle = .thin
        splitView.dragEnded = { [weak self] in self?.recordWidths() }
        self.splitView = splitView
        // Before the window has a layout of its own: closed areas start closed.
        sidebarItem.isCollapsed = model.tools.tools(in: .left).isEmpty
        inspectorItem.isCollapsed = model.tools.tools(in: .right).isEmpty
        addSplitViewItem(sidebarItem)
        addSplitViewItem(contentItem)
        addSplitViewItem(inspectorItem)
        model.$tools.sink { [weak self] _ in self?.scheduleSync() }.store(in: &subscriptions)
        // The system's Show/Hide Sidebar, a drag past the minimum, or the
        // toolbar's sidebar button: the window's tools follow.
        for (item, side) in [(sidebarItem, DockSide.left), (inspectorItem, .right)] {
            collapseObservations.append(item.observe(\.isCollapsed, options: [.new]) { [weak self] item, _ in
                MainActor.assumeIsolated { self?.collapsed(item.isCollapsed, side) }
            })
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private func collapsed(_ collapsed: Bool, _ side: DockSide) {
        guard collapsed != model.tools.tools(in: side).isEmpty else { return }
        if collapsed { model.tools.collapse(side) } else { model.tools.showArea(side) }
    }

    private func scheduleSync() {
        guard !syncScheduled else { return }
        syncScheduled = true
        // `$tools` publishes before the change; the areas follow what it became.
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.syncScheduled = false
            self.sync(animated: self.model.acceptsWorkspacePersistence && self.view.window?.isVisible == true)
        }
    }

    /// The areas as the window's tools have them.
    func sync(animated: Bool) {
        let tools = model.tools
        for (item, side, width) in [(sidebarItem, DockSide.left, tools.leftWidth), (inspectorItem, .right, tools.rightWidth)] {
            let collapsed = tools.tools(in: side).isEmpty
            // A closed area opens at its saved width; the split keeps it after that.
            if !collapsed, item.isCollapsed, !sized.contains(side), let width {
                let view = item.viewController.view
                view.setFrameSize(NSSize(width: CGFloat(width), height: view.frame.height))
                sized.insert(side)
            }
            if item.isCollapsed != collapsed {
                if animated { item.animator().isCollapsed = collapsed } else { item.isCollapsed = collapsed }
            }
        }
        view.needsLayout = true
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        // An open area takes its saved width once the split has a width to
        // divide, after this layout pass: a position set during it does not hold.
        guard splitView.bounds.width > 0, !sizingScheduled, needsSavedWidths else { return }
        sizingScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.sizingScheduled = false
            self.applySavedWidths()
        }
    }

    private var sizingScheduled = false

    private var needsSavedWidths: Bool {
        (!sized.contains(.left) && !sidebarItem.isCollapsed && model.tools.leftWidth != nil)
            || (!sized.contains(.right) && !inspectorItem.isCollapsed && model.tools.rightWidth != nil)
    }

    private func applySavedWidths() {
        let tools = model.tools
        if !sized.contains(.left), !sidebarItem.isCollapsed, let width = tools.leftWidth {
            sized.insert(.left)
            splitView.setPosition(CGFloat(width), ofDividerAt: 0)
        }
        if !sized.contains(.right), !inspectorItem.isCollapsed, let width = tools.rightWidth {
            sized.insert(.right)
            splitView.setPosition(splitView.bounds.width - CGFloat(width) - splitView.dividerThickness, ofDividerAt: 1)
        }
        splitView.layoutSubtreeIfNeeded()
    }

    /// The widths a divider was let go at.
    private func recordWidths() {
        if !sidebarItem.isCollapsed { model.tools.leftWidth = Double(sidebarItem.viewController.view.frame.width) }
        if !inspectorItem.isCollapsed { model.tools.rightWidth = Double(inspectorItem.viewController.view.frame.width) }
        sized.formUnion([.left, .right])
    }
}

/// The window's split, which tells when a divider it tracked is let go.
@MainActor final class WindowSplitView: NSSplitView {
    var dragEnded: (() -> Void)?

    override func mouseDown(with event: NSEvent) {
        super.mouseDown(with: event)
        dragEnded?()
    }
}

/// A toolbar button drawn as AppKit draws its own: an image, a toggle's on
/// state, and the action as a closure.
@MainActor final class ToolbarButtonView: NSButton {
    var perform: (() -> Void)?

    init(symbol: String, label: String, identifier: String) {
        super.init(frame: .zero)
        bezelStyle = .toolbar
        setButtonType(.pushOnPushOff)
        image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)
        imagePosition = .imageOnly
        setAccessibilityLabel(label)
        setAccessibilityIdentifier(identifier)
        target = self
        action = #selector(clicked)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    @objc private func clicked() { perform?() }

    func set(symbol: String, label: String) {
        guard symbol != symbolName else { return }
        symbolName = symbol
        image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)
    }

    private var symbolName: String?
}

/// The conversation window's toolbar: the sidebar button over the sidebar,
/// navigation, and the editor's buttons at the trailing end. The list's
/// own actions stay with the list wherever it is. Its
/// buttons follow the window: the focused tab, its conversation, where the
/// tools are. From macOS 26 the system's Customize Toolbar… arranges it;
/// before that the settings' choices do (`ToolbarSettings`).
@MainActor final class WorkspaceToolbar: NSObject, NSToolbarDelegate, NSPopoverDelegate {
    typealias Item = ConversationToolbar.Item
    let toolbar = NSToolbar(identifier: ConversationToolbar.id)
    private unowned let model: ConversationWindowModel
    private let previewEmpty: Bool
    private let problems: MergedProblemsModel
    private var buttons: [Item: ToolbarButtonView] = [:]
    private var items: [Item: NSToolbarItem] = [:]
    private var subscriptions: Set<AnyCancellable> = []
    private var focusSubscriptions: Set<AnyCancellable> = []
    private weak var observedTab: ConversationTab?
    private weak var observedStore: ConversationStore?
    private var refreshScheduled = false
    private var popovers: [ToolWindowKind: NSPopover] = [:]
    private weak var window: NSWindow?

    init(model: ConversationWindowModel, previewEmpty: Bool) {
        self.model = model
        self.previewEmpty = previewEmpty
        problems = MergedProblemsModel(local: model.list.device, hub: DeviceHub.shared)
        super.init()
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        if #available(macOS 26, *) {
            toolbar.allowsUserCustomization = true
            toolbar.autosavesConfiguration = true
        }
        let changes: [AnyPublisher<Void, Never>] = [
            model.objectWillChange.map { _ in () }.eraseToAnyPublisher(),
            model.aggregated.objectWillChange.map { _ in () }.eraseToAnyPublisher(),
            model.list.device.objectWillChange.map { _ in () }.eraseToAnyPublisher(),
            problems.objectWillChange.map { _ in () }.eraseToAnyPublisher(),
            KeyboardShortcutSettings.shared.objectWillChange.map { _ in () }.eraseToAnyPublisher(),
        ]
        for change in changes { change.sink { [weak self] in self?.scheduleRefresh() }.store(in: &subscriptions) }
        if #unavailable(macOS 26) {
            ToolbarSettings.shared.objectWillChange.sink { [weak self] in
                DispatchQueue.main.async { self?.arrange() }
            }.store(in: &subscriptions)
        }
    }

    func attach(to window: NSWindow) {
        self.window = window
        refresh()
    }

    // MARK: Arrangement

    /// The items in their places: the sidebar button before the sidebar's
    /// tracking separator, the editor's at the trailing end.
    private var arrangement: [NSToolbarItem.Identifier] {
        let editor: [Item]
        if #available(macOS 26, *) {
            editor = [.reload, .outline, .subAgents, .backgroundTasks, .pin, .split, .problems]
        } else {
            editor = ToolbarSettings.shared.shown.compactMap(Item.init)
        }
        let navigation: [Item] = [.back, .forward, .device, .operations]
        var identifiers: [NSToolbarItem.Identifier] = [.toggleSidebar, .sidebarTrackingSeparator]
        identifiers += navigation.map(\.identifier)
        identifiers.append(.flexibleSpace)
        identifiers += editor.map(\.identifier)
        identifiers.append(.inspectorTrackingSeparator)
        return identifiers
    }

    /// The settings changed what the toolbar shows (before macOS 26).
    private func arrange() {
        let wanted = arrangement
        guard toolbar.itemIdentifiers != wanted else { return }
        toolbar.itemIdentifiers = wanted
        refresh()
    }

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] { arrangement }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [.toggleSidebar, .sidebarTrackingSeparator, .inspectorTrackingSeparator, .flexibleSpace, .space]
            + Item.allCases.map(\.identifier)
    }

    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier,
                 willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        guard let kind = Item(rawValue: identifier.rawValue) else { return nil }
        let item = NSToolbarItem(itemIdentifier: identifier)
        item.label = title(kind)
        item.paletteLabel = title(kind)
        switch kind {
        case .device:
            let name = model.device.map { DeviceHub.shared?.name(of: $0) ?? "远程设备" } ?? ""
            item.view = hosting(Label(name, systemImage: "desktopcomputer").labelStyle(.titleAndIcon).foregroundStyle(.secondary).fixedSize())
            item.toolTip = "此窗口中的会话运行在“\(name)”上"
        case .operations:
            item.view = hosting(ConversationOperationStatus(aggregated: model.aggregated).fixedSize())
        default:
            let button = ToolbarButtonView(symbol: symbol(kind), label: title(kind), identifier: accessibilityIdentifier(kind))
            button.perform = { [weak self] in self?.perform(kind) }
            buttons[kind] = button
            item.view = button
            switch kind {
            case .reload: model.registerRefreshToolbarButton { [weak button] in button }
            case .outline: model.registerToolbarButton(.outline) { [weak button] in button }
            case .subAgents: model.registerToolbarButton(.agents) { [weak button] in button }
            case .backgroundTasks: model.registerToolbarButton(.backgroundTasks) { [weak button] in button }
            default: break
            }
        }
        items[kind] = item
        if flag { scheduleRefresh() }
        return item
    }

    private func hosting(_ view: some View) -> NSView {
        let host = NSHostingView(rootView: view.applicationAppearance())
        host.sizingOptions = [.intrinsicContentSize]
        return host
    }

    private func title(_ item: Item) -> String {
        switch item {
        case .back: "后退"
        case .forward: "前进"
        case .device: "设备"
        case .operations: "会话操作"
        case .reload: "刷新当前会话"
        case .outline: "会话大纲"
        case .subAgents: "关联 Agent"
        case .backgroundTasks: "后台任务"
        case .pin: "保持在最前面"
        case .split: "拆分"
        case .problems: "问题"
        }
    }

    private func symbol(_ item: Item) -> String {
        switch item {
        case .back: "chevron.backward"
        case .forward: "chevron.forward"
        case .reload: "arrow.clockwise"
        case .outline: "list.bullet"
        case .subAgents: "person.2"
        case .backgroundTasks: "terminal"
        case .pin: "pin"
        case .split: "rectangle.split.2x1"
        case .problems: "exclamationmark.triangle"
        case .device, .operations: ""
        }
    }

    private func accessibilityIdentifier(_ item: Item) -> String {
        switch item {
        case .back: "conversation-back"
        case .forward: "conversation-forward"
        default: "conversation-toolbar-\(item.rawValue)"
        }
    }

    // MARK: Actions

    private func perform(_ item: Item) {
        let tab = model.focused
        switch item {
        case .back: tab.goBack()
        case .forward: tab.goForward()
        case .reload: tab.store.reloadFromRuntimes()
        case .outline:
            if model.tools[.outline].place != .builtIn { model.toggle(.outline) }
            else if tab.store.device.outlinePresentation == .inPlace { tab.outlineCollapsed.toggle() }
            else { model.outlinePopover.toggle() }
        case .subAgents:
            if model.tools[.agents].place != .builtIn { model.toggle(.agents) } else { model.agentsPopover.toggle() }
        case .backgroundTasks:
            if model.tools[.backgroundTasks].place != .builtIn { model.toggle(.backgroundTasks) } else { model.backgroundTasksPopover.toggle() }
        case .pin: model.pinned.toggle()
        // As in Xcode: Option-click divides downward.
        case .split: model.split(NSEvent.modifierFlags.contains(.option) ? .vertical : .horizontal)
        case .problems: model.openScene?("problems")
        case .device, .operations: break
        }
        refresh()
    }

    // MARK: State

    private func scheduleRefresh() {
        guard !refreshScheduled else { return }
        refreshScheduled = true
        // Sources publish before they change; the toolbar follows what they became.
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.refreshScheduled = false
            self.refresh()
        }
    }

    /// Watches the focused tab and its conversation, whichever they are now.
    private func follow(_ tab: ConversationTab) {
        guard tab !== observedTab || tab.store !== observedStore else { return }
        observedTab = tab
        observedStore = tab.store
        focusSubscriptions = [
            tab.objectWillChange.sink { [weak self] _ in self?.scheduleRefresh() },
            tab.store.objectWillChange.sink { [weak self] _ in self?.scheduleRefresh() },
            tab.store.transcript.objectWillChange.sink { [weak self] _ in self?.scheduleRefresh() },
        ]
    }

    func refresh() {
        // The window keeps the title for the Window menu and Mission Control; the tab bar shows it.
        let device = model.device.map { DeviceHub.shared?.name(of: $0) ?? "远程设备" }
        let windowTitle = [previewEmpty ? "新会话" : model.focusedEditor.title, device].compactMap { $0 }.joined(separator: " — ")
        if let window, window.title != windowTitle { window.title = windowTitle }
        let tab = model.focused
        follow(tab)
        let store = tab.store
        let shortcuts = KeyboardShortcutSettings.shared
        let tools = model.tools
        let focus = model.hasConversationFocus
        let outlineUnavailable = !focus || store.pendingConversationID != nil || store.transcript.userRows.isEmpty

        setHidden(.device, model.device == nil)
        setHidden(.operations, model.aggregated.conversationManagement.isEmpty)
        setHidden(.subAgents, !(focus && (tools[.agents].place != .builtIn || store.hasLinkedAgents)))
        setHidden(.backgroundTasks, !(focus && (tools[.backgroundTasks].place != .builtIn || store.canShowBackgroundTasks)))
        // A window of the editor area alone is often one to keep in view.
        setHidden(.pin, !(!model.hasToolWindows || model.pinned))
        setHidden(.problems, model.list.device.problems.isEmpty)

        update(.back, enabled: focus && !store.backHistory.isEmpty, help: shortcuts.help("回到之前查看的会话", .back))
        update(.forward, enabled: focus && !store.forwardHistory.isEmpty, help: shortcuts.help("前往后退前的会话", .forward))
        update(.reload, enabled: store.canReloadFromRuntimes, help: shortcuts.help("重新读取会话、控制状态和输入执行配置；右键可刷新设备状态", .reload))

        if tools[.outline].place != .builtIn {
            update(.outline, on: tools.isOpen(.outline), help: shortcuts.help(tools.isOpen(.outline) ? "隐藏会话大纲" : "显示会话大纲", .showOutline))
        } else if store.device.outlinePresentation == .inPlace {
            update(.outline, on: tab.outlineCollapsed, enabled: !outlineUnavailable || tab.outlineCollapsed,
                   help: shortcuts.help(tab.outlineCollapsed ? "展开完整会话" : "折叠为用户消息大纲", .showOutline))
        } else {
            update(.outline, on: model.outlinePopover && !outlineUnavailable, enabled: !outlineUnavailable, help: shortcuts.help("会话大纲", .showOutline))
        }

        let running = LinkedAgent.all(of: store).contains { store.device.isConversationRunning($0.id) }
        if tools[.agents].place != .builtIn {
            update(.subAgents, on: tools.isOpen(.agents), help: shortcuts.help(tools.isOpen(.agents) ? "隐藏关联 Agent" : "显示关联 Agent", .showAgents))
        } else {
            update(.subAgents, on: model.agentsPopover, help: shortcuts.help(LinkedAgent.summary(of: store), .showAgents))
        }
        buttons[.subAgents]?.set(symbol: running ? "person.2.fill" : "person.2", label: title(.subAgents))
        if tools[.backgroundTasks].place != .builtIn {
            update(.backgroundTasks, on: tools.isOpen(.backgroundTasks),
                   help: shortcuts.help(tools.isOpen(.backgroundTasks) ? "隐藏后台任务" : "显示后台任务", .showBackgroundTasks))
        } else {
            update(.backgroundTasks, on: model.backgroundTasksPopover, help: shortcuts.help("查看后台任务", .showBackgroundTasks))
        }

        update(.pin, on: model.pinned, help: model.pinned ? "不再保持在其他应用的窗口前面" : "保持在其他应用的窗口前面")
        buttons[.pin]?.set(symbol: model.pinned ? "pin.fill" : "pin", label: title(.pin))
        update(.split, help: shortcuts.help("在右侧拆分出新面板", .splitRight) + "；" + shortcuts.help("按住 Option 点按则向下拆分", .splitDown))

        let active = problems.entries.filter { $0.problem.resolvedAt == nil }.count
        if let button = buttons[.problems] {
            let title = active > 0 ? "\(active)" : ""
            if button.title != title {
                button.title = title
                button.imagePosition = active > 0 ? .imageLeading : .imageOnly
            }
            button.setAccessibilityLabel("问题，\(active) 项未恢复")
        }
        update(.problems, help: "查看问题与历史诊断")

        present(.outline, item: .outline, shown: model.outlinePopover && !outlineUnavailable
                && tools[.outline].place == .builtIn && store.device.outlinePresentation == .popover)
        present(.agents, item: .subAgents, shown: model.agentsPopover && tools[.agents].place == .builtIn)
        present(.backgroundTasks, item: .backgroundTasks, shown: model.backgroundTasksPopover && tools[.backgroundTasks].place == .builtIn)
    }

    private func setHidden(_ kind: Item, _ hidden: Bool) {
        guard let item = items[kind], item.isHidden != hidden else { return }
        item.isHidden = hidden
    }

    private func update(_ kind: Item, on: Bool = false, enabled: Bool = true, help: String) {
        guard let button = buttons[kind] else { return }
        let state: NSControl.StateValue = on ? .on : .off
        if button.state != state { button.state = state }
        if button.isEnabled != enabled { button.isEnabled = enabled }
        if button.toolTip != help { button.toolTip = help }
        items[kind]?.toolTip = help
    }

    // MARK: Popovers

    private func present(_ kind: ToolWindowKind, item: Item, shown: Bool) {
        let popover = popovers[kind]
        if !shown {
            if let popover, popover.isShown { popover.performClose(nil) }
            return
        }
        guard popover?.isShown != true, let button = buttons[item], button.window != nil, items[item]?.isHidden != true else { return }
        let next = NSPopover()
        next.behavior = .transient
        next.delegate = self
        let content = NSHostingController(rootView: ToolbarPopoverContent(window: model, kind: kind).applicationAppearance())
        content.sizingOptions = [.preferredContentSize]
        next.contentViewController = content
        popovers[kind] = next
        next.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
    }

    func popoverDidClose(_ notification: Notification) {
        guard let popover = notification.object as? NSPopover,
              let kind = popovers.first(where: { $0.value === popover })?.key else { return }
        popovers[kind] = nil
        switch kind {
        case .outline: if model.outlinePopover { model.outlinePopover = false }
        case .agents: if model.agentsPopover { model.agentsPopover = false }
        case .backgroundTasks: if model.backgroundTasksPopover { model.backgroundTasksPopover = false }
        default: break
        }
    }
}

/// What a toolbar button's popover shows, following the focused tab.
private struct ToolbarPopoverContent: View {
    @ObservedObject var window: ConversationWindowModel
    let kind: ToolWindowKind

    var body: some View {
        Content(window: window, kind: kind, tab: window.focused, store: window.focused.store)
    }

    private struct Content: View {
        let window: ConversationWindowModel
        let kind: ToolWindowKind
        @ObservedObject var tab: ConversationTab
        @ObservedObject var store: ConversationStore

        var body: some View {
            switch kind {
            case .outline:
                TranscriptOutlineView(model: store.transcript, layout: window, editMessage: { [store, tab] id in
                    store.editability(of: id) == nil ? nil : { window.outlinePopover = false; tab.edit(messageID: id) }
                }, deleteMessage: { [store] id in
                    store.canDeleteMessage(id) ? { window.outlinePopover = false; store.confirmDeleteMessage(id) } : nil
                }, isRunning: store.loadedConversationID.map(store.device.isConversationRunning) ?? false,
                goToBottom: { [tab] in window.outlinePopover = false; tab.scrollRequest &+= 1 }) { id in
                    window.outlinePopover = false
                    tab.navigate(to: id)
                }
            case .agents:
                LinkedAgentsView(store: store, layout: window) { id, opening in
                    window.agentsPopover = false
                    window.openLinked(id, from: tab, opening: opening)
                }
            case .backgroundTasks:
                BackgroundTasksView(store: store, layout: window) { id, opening in
                    window.backgroundTasksPopover = false
                    window.openLinked(id, from: tab, opening: opening)
                }
            default:
                EmptyView()
            }
        }
    }
}
