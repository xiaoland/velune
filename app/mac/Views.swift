import SwiftUI
import AppKit
import VeluneBindings

/// The conversation window's toolbar. The list's column and the panes add
/// their items to it, and the user's arrangement is kept under this name.
enum ConversationToolbar {
    static let id = "conversation"
}

/// A conversation window: the panes, with dock areas for tool windows at
/// the left, right and bottom. The list has its own workspace and opens
/// conversations in the focused pane, which the toolbar and commands act on.
struct VeluneRootView: View {
    @ObservedObject var window: ConversationWindowModel
    /// The list's workspace.
    @ObservedObject var store: AppStore
    /// The focused pane's shown tab.
    @ObservedObject var tab: ConversationTab
    /// Its workspace.
    @ObservedObject var focused: AppStore
    let previewEmpty: Bool
    let previewSettings: Bool
    @Environment(\.openSettings) private var openSettings
    @Environment(\.openWindow) private var openWindow
    @Environment(\.controlActiveState) private var activeState
    @State private var showsChanges = false
    /// Change review is implemented but not yet part of the product
    /// (2026-10-11 decision); this keeps its entry closed.
    private static let changeReviewEnabled = false

    var body: some View {
        // The sidebar and the inspector are the left and right dock areas; collapsing one collapses the area.
        NavigationSplitView(columnVisibility: Binding(get: { window.tools.tools(in: .left).isEmpty ? .detailOnly : .all },
                                                      set: { $0 == .detailOnly ? window.tools.collapse(.left) : window.tools.showArea(.left) })) {
            DockAreaView(window: window, side: .left)
                .navigationSplitViewColumnWidth(min: 190, ideal: 240, max: 600)
        } detail: {
            GeometryReader { geometry in
                VStack(spacing: 0) {
                    PaneLayoutView(window: window, layout: window.layout, path: [], previewEmpty: previewEmpty)
                    if !window.tools.tools(in: .bottom).isEmpty { BottomAreaView(window: window, available: geometry.size.height) }
                }
            }
            // The window keeps the title for the Window menu and Mission Control; the tab bar shows it.
            .navigationTitle(previewEmpty ? "新会话" : focused.selectedConversationTitle ?? "Velune")
            .toolbar(removing: .title)
            // Customizable, so the toolbar's own menu offers display modes and Customize Toolbar…
            .toolbar(id: ConversationToolbar.id) {
                ToolbarItem(id: "back", placement: .navigation) {
                    Button { tab.goBack() } label: { Label("后退", systemImage: "chevron.backward") }
                        .help("回到之前查看的会话（⌃⌘←）").disabled(focused.backHistory.isEmpty)
                }
                ToolbarItem(id: "forward", placement: .navigation) {
                    Button { tab.goForward() } label: { Label("前进", systemImage: "chevron.forward") }
                        .help("前往后退前的会话（⌃⌘→）").disabled(focused.forwardHistory.isEmpty)
                }
                // Without the title nothing holds the buttons to the trailing end.
                ToolbarItem(id: "trailing-space") { Spacer() }
                ToolbarItem(id: "reload") {
                    Button { focused.reloadFromRuntimes() } label: { Label("重新读取", systemImage: "arrow.clockwise") }
                        .keyboardShortcut("r").disabled(!focused.canReloadFromRuntimes)
                        .help("从 Agent 运行时重新读取会话列表和当前会话（⌘ R）；用于补上自动跟随遗漏的变化")
                }
                ToolbarItem(id: "outline") { outlineButton }
                if window.tools[.subAgents].place != .builtIn || !focused.subAgents.isEmpty {
                    ToolbarItem(id: "sub-agents") { subAgentsButton }
                }
                // A window of the editor area alone is often one to keep in view.
                if !window.hasToolWindows || window.pinned {
                    ToolbarItem(id: "pin") {
                        Toggle(isOn: $window.pinned) { Label("保持在最前面", systemImage: window.pinned ? "pin.fill" : "pin") }
                            .help(window.pinned ? "不再保持在其他应用的窗口前面" : "保持在其他应用的窗口前面")
                    }
                }
                ToolbarItem(id: "split") {
                    // As in Xcode: Option-click divides downward.
                    Button { window.split(NSEvent.modifierFlags.contains(.option) ? .vertical : .horizontal) } label: { Label("拆分", systemImage: "rectangle.split.2x1") }
                        .help("在右侧拆分出新面板（⌘ D）；按住 Option 点按则向下拆分（⇧⌘ D）")
                }
                if Self.changeReviewEnabled {
                    ToolbarItem(id: "changes") {
                        Toggle(isOn: $showsChanges) { Label("变更", systemImage: "plusminus") }
                            .help("工作目录的 git 变更").keyboardShortcut("d", modifiers: [.command, .option])
                            .disabled(focused.loadedConversationWorkingDirectory == nil && !showsChanges)
                    }
                }
                if !store.problems.isEmpty {
                    ToolbarItem(id: "problems") { ProblemsButton(store: store) }
                }
            }
            // One inspector column: the right dock area, or change review while that is open.
            .inspector(isPresented: Binding(get: { showsChanges || !window.tools.tools(in: .right).isEmpty },
                                            set: { shown in
                                                if shown { window.tools.showArea(.right) }
                                                else if showsChanges { showsChanges = false }
                                                else { window.tools.collapse(.right) }
                                            })) {
                if showsChanges { ChangeReviewView(store: focused).inspectorColumnWidth(min: 300, ideal: 440, max: 900) }
                else { DockAreaView(window: window, side: .right).inspectorColumnWidth(min: 200, ideal: 260, max: 720) }
            }
        }
        .navigationSplitViewStyle(.balanced)
        .background(WindowLevelSetter(pinned: window.pinned))
        .overlay { if let side = window.dockHint { DockZoneHighlight(side: side) } }
        .onChange(of: activeState, initial: true) { _, value in if value == .key { focused.noteFocused() } }
        .onChange(of: tab.id) { _, _ in window.outlinePopover = false; window.subAgentsPopover = false }
        .onChange(of: focused.pendingConversationID) { _, value in if value != nil { window.outlinePopover = false; window.subAgentsPopover = false } }
        .onChange(of: focused.loadedConversationID) { _, _ in window.outlinePopover = false; window.subAgentsPopover = false }
        .onChange(of: store.outlinePresentation) { _, _ in window.outlinePopover = false }
        .focusedSceneValue(\.conversationCommands, ConversationCommandTarget(store: focused, send: tab.send, window: window))
        .task { if previewSettings { openSettings() } }
    }

    /// The outline as a tool window is shown and hidden by the button.
    /// In its own place the setting decides: a popover anchored to the
    /// button, or a toggle that collapses the focused pane's message list in place.
    @ViewBuilder private var outlineButton: some View {
        let unavailable = focused.pendingConversationID != nil || focused.transcript.userRows.isEmpty
        if window.tools[.outline].place != .builtIn {
            Toggle(isOn: Binding(get: { window.tools.isOpen(.outline) }, set: { _ in window.toggle(.outline) })) { Label("会话大纲", systemImage: "list.bullet") }
                .help(window.tools.isOpen(.outline) ? "隐藏会话大纲（⌘ 2）" : "显示会话大纲（⌘ 2）")
        } else {
            settingOutlineButton(unavailable: unavailable)
        }
    }

    /// The sub-agents as a tool window are shown and hidden by the button; in
    /// their own place they pop over from it, offered only when the focused
    /// conversation has any.
    @ViewBuilder private var subAgentsButton: some View {
        let agents = focused.subAgents
        let running = agents.filter { focused.isSubAgentRunning($0.id) }.count
        let summary = running > 0 ? "\(agents.count) 个子 Agent，\(running) 个正在运行" : "\(agents.count) 个子 Agent"
        if window.tools[.subAgents].place != .builtIn {
            Toggle(isOn: Binding(get: { window.tools.isOpen(.subAgents) }, set: { _ in window.toggle(.subAgents) })) { Label("子 Agent", systemImage: "person.2") }
                .help(window.tools.isOpen(.subAgents) ? "隐藏子 Agent（⌘ 4）" : "显示子 Agent（⌘ 4）")
        } else {
            Button { window.subAgentsPopover.toggle() } label: { Label("子 Agent", systemImage: running > 0 ? "person.2.fill" : "person.2") }
                .help("\(summary)（⌘ 4）")
                .popover(isPresented: $window.subAgentsPopover, arrowEdge: .bottom) {
                    SubAgentsView(store: focused, layout: window) { id in
                        window.subAgentsPopover = false
                        focused.openLinkedConversation(id: id)
                    }
                }
        }
    }

    @ViewBuilder private func settingOutlineButton(unavailable: Bool) -> some View {
        switch store.outlinePresentation {
        case .popover:
            Button { window.outlinePopover.toggle() } label: { Label("会话大纲", systemImage: "list.bullet") }
                .help("会话大纲（⌘ 2）").disabled(unavailable)
                .popover(isPresented: Binding(get: { window.outlinePopover && !unavailable }, set: { window.outlinePopover = $0 }), arrowEdge: .bottom) {
                    TranscriptOutlineView(model: focused.transcript, layout: window, editMessage: { [focused, tab] id in
                        focused.editability(of: id) == nil ? nil : { window.outlinePopover = false; tab.edit(messageID: id) }
                    }) { id in
                        window.outlinePopover = false
                        tab.navigate(to: id)
                    }
                }
        case .inPlace:
            Toggle(isOn: $tab.outlineCollapsed) { Label("会话大纲", systemImage: "list.bullet") }
                .help(tab.outlineCollapsed ? "展开完整会话（⌘ 2）" : "折叠为用户消息大纲（⌘ 2）")
                .disabled(unavailable && !tab.outlineCollapsed)
        }
    }
}

/// Where a dragged panel would dock: the sidebar, the inspector, or the strip below the panes.
private struct DockZoneHighlight: View {
    let side: DockSide

    var body: some View {
        GeometryReader { geometry in
            let size = geometry.size
            let rect: CGRect = switch side {
            case .left: CGRect(x: 0, y: 0, width: min(260, size.width / 3), height: size.height)
            case .right: CGRect(x: size.width - min(280, size.width / 3), y: 0, width: min(280, size.width / 3), height: size.height)
            case .bottom: CGRect(x: 0, y: size.height - 180, width: size.width, height: 180)
            }
            RoundedRectangle(cornerRadius: 8)
                .fill(Color.accentColor.opacity(0.12))
                .strokeBorder(Color.accentColor.opacity(0.6), lineWidth: 2)
                .frame(width: max(0, rect.width - 12), height: max(0, rect.height - 12))
                .position(x: rect.midX, y: rect.midY)
        }
        .ignoresSafeArea()
        .allowsHitTesting(false)
    }
}

struct ConversationRenameView: View {
    @ObservedObject var store: AppStore
    let conversation: Conversation
    @Environment(\.dismiss) private var dismiss
    @State private var title: String
    @FocusState private var titleFocused: Bool
    init(store: AppStore, conversation: Conversation) {
        self.store = store; self.conversation = conversation
        _title = State(initialValue: conversation.title)
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("重命名会话").font(.headline)
            TextField("会话名称", text: $title).focused($titleFocused).onSubmit(rename)
                .disabled(store.conversationManagementStatus != nil)
            if let status = store.conversationManagementStatus { Text(status).font(.callout).foregroundStyle(.secondary) }
            HStack {
                if store.conversationManagementStatus != nil { ProgressView().controlSize(.small) }
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction).disabled(store.conversationManagementStatus != nil)
                Button("重命名", action: rename).keyboardShortcut(.defaultAction)
                    .disabled(!store.canManageConversations || title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }.padding(20).frame(width: 360).onAppear { titleFocused = true }
    }
    private func rename() { store.renameConversation(conversation, title: title) { dismiss() } }
}

struct ProjectEditorTarget: Identifiable {
    /// The project's primary folder.
    let path: String
    var id: String { path }
}

/// A project's name and folders in Velune's list. Conversations whose
/// working directory is any of its folders belong to it; the folders and
/// the conversations' working directories are unchanged.
struct ProjectEditorView: View {
    @ObservedObject var store: AppStore
    let target: ProjectEditorTarget
    @Environment(\.dismiss) private var dismiss
    @State private var name: String
    @State private var primary: String
    /// Every folder, the primary one first as it was opened.
    @State private var folders: [String]
    @State private var selection: String?
    @FocusState private var focused: Bool
    private let worktrees: [String]
    init(store: AppStore, target: ProjectEditorTarget) {
        self.store = store; self.target = target
        let project = store.conversationProjects.first { $0.path == target.path }
        _name = State(initialValue: project?.name ?? "")
        _primary = State(initialValue: target.path)
        _folders = State(initialValue: [target.path] + (project?.folders ?? []))
        worktrees = project?.worktrees ?? []
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("编辑项目").font(.headline)
            TextField("项目名称", text: $name, prompt: Text(Self.folderName(primary))).focused($focused)
            Text("名称只用于 Velune 的会话列表；留空则显示主文件夹的名称。").font(.caption).foregroundStyle(.secondary)
            Text("文件夹").font(.subheadline.weight(.semibold)).padding(.top, 4)
            VStack(spacing: 0) {
                List(selection: $selection) {
                    ForEach(folders, id: \.self) { folder in
                        HStack {
                            Text(Self.abbreviated(folder)).lineLimit(1).truncationMode(.middle).help(folder)
                            Spacer(minLength: 8)
                            if folder == primary { Text("主文件夹").foregroundStyle(.secondary) }
                        }
                        .tag(folder)
                    }
                }
                .contextMenu(forSelectionType: String.self) { selected in
                    if let folder = selected.first {
                        Button("设为主文件夹") { primary = folder }.disabled(folder == primary)
                        Button("在 Finder 中显示") { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: folder)]) }
                        Divider()
                        Button("移除") { remove(folder) }.disabled(folder == primary)
                    }
                }
                .listStyle(.bordered(alternatesRowBackgrounds: false))
                .frame(height: 120)
                HStack(spacing: 0) {
                    Button(action: add) { Image(systemName: "plus").frame(width: 22, height: 20) }
                        .help("添加文件夹").accessibilityLabel("添加文件夹")
                    Button { if let selection { remove(selection) } } label: { Image(systemName: "minus").frame(width: 22, height: 20) }
                        .disabled(selection == nil || selection == primary)
                        .help("移除所选文件夹").accessibilityLabel("移除所选文件夹")
                    Spacer()
                    Button("设为主文件夹") { if let selection { primary = selection } }
                        .disabled(selection == nil || selection == primary)
                }
                .buttonStyle(.borderless).controlSize(.small).padding(.top, 4)
            }
            Text("工作目录在任一文件夹中的会话都归入此项目；新会话的工作目录建议使用主文件夹。项目不改变文件夹或会话的工作目录。")
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if !worktrees.isEmpty {
                Text("Git worktree").font(.subheadline.weight(.semibold)).padding(.top, 4)
                ScrollView {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(worktrees, id: \.self) { folder in
                            Text(Self.abbreviated(folder)).lineLimit(1).truncationMode(.middle).help(folder).textSelection(.enabled)
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 72).foregroundStyle(.secondary)
                Text("这些 worktree 中的会话按所属仓库自动归入此项目。").font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("存储", action: save).keyboardShortcut(.defaultAction)
            }.padding(.top, 4)
        }.padding(20).frame(width: 440).onAppear { focused = true }
    }
    private static func folderName(_ path: String) -> String { URL(fileURLWithPath: path).lastPathComponent }
    private static func abbreviated(_ path: String) -> String { (path as NSString).abbreviatingWithTildeInPath }
    private func add() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false; panel.canChooseDirectories = true; panel.allowsMultipleSelection = true
        panel.prompt = "添加到项目"
        guard panel.runModal() == .OK else { return }
        for url in panel.urls where !folders.contains(url.path) { folders.append(url.path) }
    }
    private func remove(_ folder: String) {
        guard folder != primary else { return }
        folders.removeAll { $0 == folder }
        if selection == folder { selection = nil }
    }
    private func save() {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        store.setProject(path: target.path, primaryFolder: primary, name: trimmed.isEmpty || trimmed == Self.folderName(primary) ? nil : trimmed,
                         folders: folders.filter { $0 != primary })
        dismiss()
    }
}

// Limit the workaround to expansion writes; content controls keep their own transactions.
struct ImmediateDisclosureGroup<Label: View, Content: View>: View {
    @State private var expanded = false
    private var externalExpansion: Binding<Bool>?
    private let content: Content
    private let label: Label

    init(isExpanded: Binding<Bool>? = nil, @ViewBuilder content: () -> Content, @ViewBuilder label: () -> Label) {
        externalExpansion = isExpanded
        self.content = content()
        self.label = label()
    }

    private var expansion: Binding<Bool> {
        Binding(
            get: { externalExpansion?.wrappedValue ?? expanded },
            set: { value in
                var transaction = Transaction(animation: nil)
                transaction.disablesAnimations = true
                let target = externalExpansion ?? $expanded
                target.transaction(transaction).wrappedValue = value
            }
        )
    }

    var body: some View {
        DisclosureGroup(isExpanded: expansion) { content } label: { label }
    }
}

extension ImmediateDisclosureGroup where Label == Text {
    init(_ title: String, @ViewBuilder content: () -> Content) {
        self.init(content: content, label: { Text(title) })
    }
}

enum SettingsTab: Hashable { case providers, routes, conversation, runtimes }

/// The settings tab that is showing. Entry points elsewhere in the app set it
/// before opening the window, so settings open where the task needs them.
@MainActor final class SettingsNavigator: ObservableObject {
    static let shared = SettingsNavigator()
    @Published var tab: SettingsTab = .providers
}

/// Opens settings on a specific tab.
struct SettingsTabLink: View {
    let tab: SettingsTab
    let title: String
    @Environment(\.openSettings) private var openSettings
    var body: some View {
        Button(title) { SettingsNavigator.shared.tab = tab; openSettings() }
    }
}

struct SettingsView: View {
    @ObservedObject var store: AppStore
    @ObservedObject private var navigator = SettingsNavigator.shared
    var body: some View {
        TabView(selection: $navigator.tab) {
            ProviderSettingsView(store: store).tabItem { Label("AI提供商", systemImage: "network") }.tag(SettingsTab.providers)
            ModelRouteSettingsView(store: store).tabItem { Label("模型路由", systemImage: "arrow.triangle.branch") }.tag(SettingsTab.routes)
            ConversationSettingsView(store: store).tabItem { Label("会话", systemImage: "bubble.left.and.bubble.right") }.tag(SettingsTab.conversation)
            RuntimeSettingsView(store: store).tabItem { Label("Agent 运行时", systemImage: "terminal") }.tag(SettingsTab.runtimes)
        }
        // Settings are compact forms of one fixed size, so switching tabs never resizes the window.
        .frame(width: 690, height: 560)
    }
}

private struct ConversationSettingsView: View {
    @ObservedObject var store: AppStore
    @State private var limit = 20
    @State private var totalLimit = 200
    private static let statusLineItems: [(item: BindingStatusLineItem, title: String)] = [
        (.contextUsage, "上下文用量"), (.sessionTokens, "本会话累计 token"), (.workingDirectory, "工作目录与 Git 分支"),
        (.rateLimits, "账户用量限额（Codex）"), (.sessionCost, "费用估算"),
    ]
    var body: some View {
        Form {
            Section("大纲") {
                Picker("大纲按钮", selection: Binding(get: { store.outlinePresentation }, set: { store.setOutlinePresentation($0) })) {
                    Text("弹出面板").tag(BindingOutlinePresentation.popover)
                    Text("就地折叠").tag(BindingOutlinePresentation.inPlace)
                }.disabled(store.isLoading || store.isShuttingDown)
                Text("大纲以用户消息为条目。弹出面板在按钮处列出大纲；就地折叠把消息列表缩起为只剩用户消息，点击其中一条即恢复完整会话并停在该处。").font(.caption).foregroundStyle(.secondary)
            }
            Section("状态栏") {
                ForEach(Self.statusLineItems, id: \.item) { entry in
                    Toggle(entry.title, isOn: Binding(
                        get: { store.statusLine.contains(entry.item) },
                        set: { pinned in store.setStatusLine(pinned ? store.statusLine + [entry.item] : store.statusLine.filter { $0 != entry.item }) }))
                }.disabled(store.isLoading || store.isShuttingDown)
                Text("状态栏位于输入框下方、发送按钮左侧，只显示运行时原生记录中已有的项；点按它可查看全部项。费用为运行时按其价目的估算。").font(.caption).foregroundStyle(.secondary)
            }
            Section("会话列表") {
                LabeledContent("每次加载") { TextField("数量", value: $limit, format: .number).frame(width: 70); Text("个会话") }
                LabeledContent("列表总数上限") { TextField("数量", value: $totalLimit, format: .number).frame(width: 70); Text("个会话") }
                Text("列表最多先读取总数上限个会话，按组依次分给各组，每组每次加载若干个；折叠的组不读取。之后可在各组中加载更多。").font(.caption).foregroundStyle(.secondary)
                Button("保存") { store.setConversationBrowserLimits(BindingConversationBrowserLimits(page: UInt32(limit), total: UInt32(totalLimit))) }
                    .disabled(limit < 1 || totalLimit < 1 || (limit == Int(store.conversationBrowserLimits.page) && totalLimit == Int(store.conversationBrowserLimits.total)) || store.isShuttingDown)
            }

        }.formStyle(.grouped).onAppear { limit = Int(store.conversationBrowserLimits.page); totalLimit = Int(store.conversationBrowserLimits.total) }
    }
}

struct ProviderSettingsView: View {
    @ObservedObject var store: AppStore
    @State private var selectedID: String?
    @State private var editor: AIProvider?
    @State private var creating = false
    @State private var importing = false
    @State private var templates = false
    @State private var deleting: AIProvider?
    var body: some View {
        VStack(spacing: 0) {
            List(selection: $selectedID) {
                ForEach(store.providers) { provider in
                    VStack(alignment: .leading, spacing: 3) {
                        Text(provider.name)
                        Text("\(provider.models.count) 个模型 · \(provider.endpoint)").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }.tag(provider.id)
                }
            }.contextMenu(forSelectionType: String.self) { ids in
                if let id = ids.first, let provider = store.providers.first(where: { $0.id == id }) { Button("编辑…") { editor = provider }; Button("删除…") { deleting = provider } }
            } primaryAction: { ids in if let id = ids.first { editor = store.providers.first { $0.id == id } } }
            .listStyle(.bordered).padding(.horizontal, 20).padding(.top, 16)
            HStack {
                Button { creating = true } label: { Image(systemName: "plus") }.help("添加 AI 提供商")
                Button { deleting = store.providers.first { $0.id == selectedID } } label: { Image(systemName: "minus") }.disabled(selectedID == nil)
                Button("从运行时导入…") { importing = true }.disabled(store.providerImportTypes.isEmpty || store.isBusy)
                Menu { Button("模型模板…") { templates = true } } label: { Image(systemName: "ellipsis") }.menuStyle(.borderlessButton).menuIndicator(.hidden).frame(width: 28).accessibilityLabel("更多操作")
                Spacer()
                Button("编辑…") { editor = store.providers.first { $0.id == selectedID } }.disabled(selectedID == nil)
            }.padding(.horizontal, 20).padding(.vertical, 12)

        }
        .sheet(isPresented: $importing) { ProviderImportView(store: store) }
        .sheet(item: $editor) { ProviderEditor(store: store, provider: $0) }
        .sheet(isPresented: $creating) { ProviderEditor(store: store, provider: nil) }
        .sheet(isPresented: $templates) { ModelTemplatesView(store: store) }
        .alert("删除 AI 提供商？", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { provider in
            Button("删除", role: .destructive) { store.deleteProvider(id: provider.id); deleting = nil }; Button("取消", role: .cancel) { deleting = nil }
        } message: { provider in Text("删除「\(provider.name)」及其模型配置。使用这些模型的运行时需要重新选择模型；原运行时文件与会话保留。") }
    }
}

private struct ProviderModelDraft: Identifiable {
    let id = UUID()
    var recordKey = ""
    var providerModelID = ""
    var nickname = ""
    var icon = ""
    var contextWindow = ""
    var maxOutputTokens = ""
    var declaresReasoning = false
    var reasoningLevels = ""
    var adapterMetadataJSON: String?
    init() {}
    init(_ model: ProviderModel) {
        recordKey = model.recordKey; providerModelID = model.providerModelID; nickname = model.nickname; icon = model.icon ?? ""
        contextWindow = model.contextWindow.map(String.init) ?? ""; maxOutputTokens = model.maxOutputTokens.map(String.init) ?? ""
        declaresReasoning = model.reasoningLevels != nil; reasoningLevels = model.reasoningLevels?.joined(separator: ", ") ?? ""
        adapterMetadataJSON = model.adapterMetadataJSON
    }
    var displayName: String { nickname.isEmpty ? (providerModelID.isEmpty ? "新模型" : providerModelID) : nickname }
    var valid: Bool { !providerModelID.isEmpty && validOptionalTokenCount(contextWindow) && validOptionalTokenCount(maxOutputTokens) && !(UInt32(maxOutputTokens).map { output in UInt32(contextWindow).map { output > $0 } ?? false } ?? false) }
    var efforts: [String]? { declaresReasoning ? reasoningLevels.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty } : nil }
    var capabilitySummary: String {
        let effortsSummary = efforts.map { $0.isEmpty ? "不支持推理 effort" : "支持 effort：" + $0.joined(separator: "、") } ?? "推理 effort 未知"
        return effortsSummary + (contextWindow.isEmpty ? "" : " · 上下文 \(contextWindow)") + (maxOutputTokens.isEmpty ? "" : " · 输出 \(maxOutputTokens)")
    }
    var definition: ProviderModel { ProviderModel(recordKey: recordKey, providerModelID: providerModelID, nickname: nickname, icon: icon.isEmpty ? nil : icon, contextWindow: UInt32(contextWindow), maxOutputTokens: UInt32(maxOutputTokens), reasoningLevels: efforts, adapterMetadataJSON: adapterMetadataJSON) }
}
private func validOptionalTokenCount(_ value: String) -> Bool { value.isEmpty || (UInt32(value).map { $0 > 0 } ?? false) }

private struct ProviderModelFields: View {
    @Binding var draft: ProviderModelDraft
    var modelIDFocused: FocusState<Bool>.Binding
    var body: some View {
        Section("模型") {
            TextField("模型 ID", text: $draft.providerModelID, prompt: Text("提供商 API 规定的标识"))
                .focused(modelIDFocused)
            TextField("显示名称", text: $draft.nickname, prompt: Text("可选"))
        }
        Section {
            ImmediateDisclosureGroup {
                VStack(alignment: .leading, spacing: 12) {
                    TextField("上下文窗口", text: $draft.contextWindow, prompt: Text("未知"))
                    TextField("最大输出 Token", text: $draft.maxOutputTokens, prompt: Text("未知"))
                    Toggle("声明支持的 reasoning effort", isOn: $draft.declaresReasoning)
                    if draft.declaresReasoning {
                        TextField("支持的 effort", text: $draft.reasoningLevels, prompt: Text("逗号分隔，空白表示不支持"))
                    }
                    Text("以提供商协议为准；此处声明可用能力，不改变会话当前选择的 effort。").font(.caption).foregroundStyle(.secondary)
                    TextField("图标", text: $draft.icon, prompt: Text("可选的 SF Symbol 名称"))
                }.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 8)
            } label: {
                VStack(alignment: .leading, spacing: 3) { Text("能力与 reasoning effort"); Text(draft.capabilitySummary).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
            }
        }
    }
}

private enum ProviderEditorSelection: Hashable { case connection, model(UUID) }

struct ProviderEditor: View {
    @ObservedObject var store: AppStore
    let provider: AIProvider?
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var protocolID: ProviderProtocol = .chatCompletionsV1
    @State private var endpoint = ""
    @State private var models: [ProviderModelDraft] = []
    @State private var selection: ProviderEditorSelection? = .connection
    @State private var apiKey = ""
    @State private var originalAPIKey: String?
    @State private var attemptedKeyRead = false
    @State private var showKey = false
    @State private var editingAPIKey = false
    @State private var clearAuthentication = false
    @State private var templateDraft: ModelTemplate?
    @State private var choosingTemplates = false
    @FocusState private var providerNameFocused: Bool
    @FocusState private var modelIDFocused: Bool
    private var authentication: ProviderAuthentication { store.providers.first { $0.id == provider?.id }?.authentication ?? provider?.authentication ?? ProviderAuthentication() }
    private var modelIndex: Int? { guard case .model(let id) = selection else { return nil }; return models.firstIndex { $0.id == id } }
    private var valid: Bool { !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && URL(string: endpoint)?.host != nil && models.allSatisfy(\.valid) && (!editingAPIKey || (!apiKey.isEmpty && (originalAPIKey != nil || authentication.method != .apiKey || clearAuthentication))) }
    var body: some View {
        VStack(spacing: 0) {
            Text(provider == nil ? "添加 AI 提供商" : "编辑 AI 提供商").font(.headline).frame(maxWidth: .infinity, alignment: .leading).padding(20)
            Divider()
            HSplitView {
                VStack(spacing: 0) {
                    List(selection: $selection) {
                        Label("连接与认证", systemImage: "network").tag(ProviderEditorSelection.connection)
                        Section("模型") {
                            ForEach(models) { model in VStack(alignment: .leading, spacing: 3) { Text(model.displayName).lineLimit(1); if !model.providerModelID.isEmpty { Text(model.providerModelID).font(.caption).foregroundStyle(.secondary).lineLimit(1) } }.tag(ProviderEditorSelection.model(model.id)) }
                        }
                    }.contextMenu(forSelectionType: ProviderEditorSelection.self) { ids in
                        if ids.count == 1, let target = ids.first { Button("编辑") { editSelection(target) } }
                    } primaryAction: { ids in if ids.count == 1, let target = ids.first { editSelection(target) } }
                    .listStyle(.sidebar)
                    HStack {
                        Menu {
                            Button("添加新模型") { addModel(ProviderModelDraft()) }
                            Button("从模板中选取…") { choosingTemplates = true }
                        } label: { Image(systemName: "plus") }.menuStyle(.borderlessButton)
                        Button { if let index = modelIndex { models.remove(at: index); selection = .connection } } label: { Image(systemName: "minus") }.disabled(modelIndex == nil)
                        Spacer()
                    }.padding(12)
                }.frame(minWidth: 170, idealWidth: 190, maxWidth: 230)
                Group {
                    if let index = modelIndex {
                        Form {
                            ProviderModelFields(draft: $models[index], modelIDFocused: $modelIDFocused)
                            Section { Button("保存为模型模板…") { let model = models[index].definition; templateDraft = ModelTemplate(name: model.displayName, suggestedProviderModelID: model.providerModelID, nickname: model.nickname, icon: model.icon, contextWindow: model.contextWindow, maxOutputTokens: model.maxOutputTokens, reasoningLevels: model.reasoningLevels) } }
                        }
                    } else { connectionForm }
                }.formStyle(.grouped).frame(minWidth: 360, maxWidth: .infinity, maxHeight: .infinity)
            }
            Divider()

            HStack { Spacer(); Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Button("保存") { commit() }.keyboardShortcut(.defaultAction).disabled(!valid || store.isBusy) }.padding(16)
        }.frame(width: 650, height: 510)
        .onAppear { name = provider?.name ?? ""; protocolID = provider?.protocolID ?? .chatCompletionsV1; endpoint = provider?.endpoint ?? ""; models = (provider?.models ?? []).map(ProviderModelDraft.init); editingAPIKey = authentication.method == .unconfigured; readAPIKeyIfNeeded() }
        .onChange(of: selection) { _, value in if value == .connection { readAPIKeyIfNeeded() } }
        .sheet(isPresented: $choosingTemplates) { ModelTemplatePicker(templates: store.modelTemplates) { values in for template in values { addModel(ProviderModelDraft(template.model)) } } }
        .sheet(item: $templateDraft) { value in ModelTemplateEditor(store: store, template: value) }
        .sheet(isPresented: $store.showsAuthentication) { AuthenticationView(store: store) }
    }
    private var connectionForm: some View {
        Form {
            Section("连接") {
                TextField("名称", text: $name)
                    .focused($providerNameFocused)
                Picker("协议", selection: Binding(get: { protocolID }, set: { value in
                    guard value != protocolID else { return }
                    for index in models.indices { models[index].adapterMetadataJSON = nil }
                    protocolID = value
                })) { ForEach(store.protocols.filter(\.supported)) { Text($0.name).tag($0.id) } }
                Text("切换协议后采用默认适配参数；模型 ID 与已填写的能力保留。")
                    .font(.caption).foregroundStyle(.secondary)
                TextField("服务地址", text: $endpoint, prompt: Text(protocolID == .messagesV1 ? "https://…" : "https://…/v1"))
                    .help(protocolID == .messagesV1 ? "Anthropic Messages 协议基础 URL；请求路径为 /v1/messages。" : "提供商的协议服务基础 URL。")
            }
            Section("认证") {
                LabeledContent("方式", value: editingAPIKey ? "API key" : clearAuthentication ? "尚未配置" : authentication.method.label)
                if editingAPIKey {
                    HStack {
                        if showKey { TextField("API key", text: $apiKey) } else { SecureField("API key", text: $apiKey) }
                        Button { showKey.toggle() } label: { Image(systemName: showKey ? "eye.slash" : "eye") }.buttonStyle(.borderless).help(showKey ? "隐藏 API key" : "显示 API key")
                    }.privacySensitive()
                    if authentication.method == .apiKey && originalAPIKey == nil && !clearAuthentication {
                        Button("读取 API key") { attemptedKeyRead = false; readAPIKeyIfNeeded() }.disabled(store.isLoading)
                    }
                } else {
                    if authentication.method == .oauth { Text(authentication.configured ? "已登录" : "需要登录").foregroundStyle(.secondary) }
                    if let provider, !clearAuthentication, !authentication.actions.isEmpty { Button("登录…") { store.startAuthentication(provider.id) }.disabled(store.isBusy) }
                    Button("使用 API key…") { editingAPIKey = true; clearAuthentication = true; apiKey = "" }
                }
                if authentication.configured || editingAPIKey {
                    Button("清除认证", role: .destructive) { clearAuthentication = true; editingAPIKey = false; apiKey = ""; originalAPIKey = nil }
                }
                if let provenance = authentication.provenance {
                    ImmediateDisclosureGroup("来源详情") { Text(provenance).font(.callout).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading).padding(.top, 8) }
                }
            }
        }
    }
    private func editSelection(_ target: ProviderEditorSelection) {
        selection = target
        DispatchQueue.main.async {
            switch target {
            case .connection: modelIDFocused = false; providerNameFocused = true
            case .model: providerNameFocused = false; modelIDFocused = true
            }
        }
    }
    private func addModel(_ draft: ProviderModelDraft) { models.append(draft); selection = .model(draft.id) }
    private func readAPIKeyIfNeeded() {
        guard let provider, authentication.method == .apiKey, !attemptedKeyRead, !clearAuthentication else { return }
        attemptedKeyRead = true
        store.readProviderAPIKey(provider.id) { value in originalAPIKey = value; apiKey = value; editingAPIKey = true }
    }
    private func commit() {
        let edit: AuthenticationEdit = editingAPIKey && apiKey != originalAPIKey ? .setAPIKey(apiKey) : clearAuthentication ? .clear : .keep
        store.saveProvider(AIProvider(id: provider?.id ?? "", name: name.trimmingCharacters(in: .whitespacesAndNewlines), protocolID: protocolID, endpoint: endpoint.trimmingCharacters(in: .whitespacesAndNewlines), authentication: authentication, models: models.map(\.definition), quotaLimits: provider?.quotaLimits ?? []), authenticationEdit: edit) { apiKey = ""; originalAPIKey = nil; dismiss() }
    }
}

private struct ModelTemplatePicker: View {
    let templates: [ModelTemplate]
    let add: ([ModelTemplate]) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var selection: Set<String> = []
    @State private var search = ""
    @State private var addedCount = 0
    private var candidates: [ModelTemplate] { templates.filter { search.isEmpty || "\($0.name) \($0.suggestedProviderModelID)".localizedCaseInsensitiveContains(search) } }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("从模板中选取").font(.headline); Spacer() }.padding(20)
            TextField("搜索模板", text: $search).textFieldStyle(.roundedBorder).padding(.horizontal, 20).padding(.bottom, 8)
            List(selection: $selection) { ForEach(candidates) { value in VStack(alignment: .leading, spacing: 3) { Text(value.name); Text(value.suggestedProviderModelID).font(.caption).foregroundStyle(.secondary) }.tag(value.id) } }.listStyle(.bordered).padding(.horizontal, 20)
            .overlay { if candidates.isEmpty { Text(templates.isEmpty ? "尚无模型模板，可在设置中添加。" : "没有匹配的模板").foregroundStyle(.secondary) } }
            HStack {
                Text(addedCount > 0 ? "已添加 \(addedCount) 个模型" : "按住 ⌘ 或 Shift 选择多个模板").font(.caption).foregroundStyle(.secondary)
                Spacer(); Button("完成") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("添加所选") { let values = templates.filter { selection.contains($0.id) }; add(values); addedCount += values.count; selection.removeAll() }.disabled(selection.isEmpty).keyboardShortcut(.defaultAction)
            }.padding(16)
        }.frame(width: 480, height: 380)
        .onChange(of: search) { _, _ in selection.formIntersection(candidates.map(\.id)) }
    }
}

struct ModelTemplatesView: View {
    @ObservedObject var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @State private var selectedID: String?
    @State private var editor: ModelTemplate?
    @State private var catalog = false
    var body: some View {
        VStack(spacing: 0) {
            Text("模型模板").font(.headline).frame(maxWidth: .infinity, alignment: .leading).padding(20)
            Text("模板用于快速填写模型，修改模板不会改变已配置的模型。").font(.callout).foregroundStyle(.secondary).padding(.horizontal, 20).padding(.bottom, 12)
            List(selection: $selectedID) { ForEach(store.modelTemplates) { template in VStack(alignment: .leading, spacing: 3) { Text(template.name); Text(template.suggestedProviderModelID).font(.caption).foregroundStyle(.secondary) }.tag(template.id) } }
            .contextMenu(forSelectionType: String.self) { ids in
                if ids.count == 1, let id = ids.first, let template = store.modelTemplates.first(where: { $0.id == id }) { Button("编辑…") { editor = template } }
            } primaryAction: { ids in if ids.count == 1, let id = ids.first { editor = store.modelTemplates.first { $0.id == id } } }
            .listStyle(.bordered).padding(.horizontal, 20)
            HStack {
                Button { editor = ModelTemplate(name: "", suggestedProviderModelID: "") } label: { Image(systemName: "plus") }
                Button { if let selectedID { store.deleteTemplate(selectedID) } } label: { Image(systemName: "minus") }.disabled(selectedID == nil || store.isLoading)
                Button("从公开目录添加…") { catalog = true }
                Spacer(); Button("编辑…") { editor = store.modelTemplates.first { $0.id == selectedID } }.disabled(selectedID == nil); Button("完成") { dismiss() }.keyboardShortcut(.cancelAction)
            }.padding(16)

        }.frame(width: 480, height: 380).sheet(item: $editor) { ModelTemplateEditor(store: store, template: $0) }
        .sheet(isPresented: $catalog) { PublicModelCatalogView(store: store) }
    }
}
struct ModelTemplateEditor: View {
    @ObservedObject var store: AppStore
    let template: ModelTemplate
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var draft = ProviderModelDraft()
    @FocusState private var modelIDFocused: Bool
    var body: some View {
        VStack(spacing: 0) {
            Text("编辑模型模板").font(.headline).frame(maxWidth: .infinity, alignment: .leading).padding(16)
            Form { Section { TextField("模板名称", text: $name) }; ProviderModelFields(draft: $draft, modelIDFocused: $modelIDFocused) }.formStyle(.grouped)

            HStack { Spacer(); Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Button("保存") { let model = draft.definition; store.saveTemplate(ModelTemplate(id: template.id, name: name, suggestedProviderModelID: model.providerModelID, nickname: model.nickname, icon: model.icon, contextWindow: model.contextWindow, maxOutputTokens: model.maxOutputTokens, reasoningLevels: model.reasoningLevels)) { dismiss() } }.keyboardShortcut(.defaultAction).disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !draft.valid || store.isLoading) }.padding(16)
        }.frame(width: 440, height: 400).onAppear { name = template.name; draft = ProviderModelDraft(template.model) }
    }
}

struct RuntimeSettingsView: View {
    @ObservedObject var store: AppStore
    @Environment(\.openWindow) private var openWindow
    @State private var selectedID: String?
    @State private var editor: RuntimeInstance?
    @State private var creating = false
    @State private var discovering = false
    @State private var deleting: RuntimeInstance?
    /// Runtime ids before an add sheet opened, to find what it added.
    @State private var knownIDs: Set<String> = []
    /// A just-added runtime and how many library resources it could use.
    @State private var added: (runtime: RuntimeInstance, resources: Int)?
    private var selected: RuntimeInstance? { store.runtimeInstances.first { $0.id == selectedID } }

    var body: some View {
        VStack(spacing: 0) {
            List(selection: $selectedID) {
                ForEach(store.runtimeInstances) { instance in
                    VStack(alignment: .leading, spacing: 3) {
                        HStack { Text(instance.name); if !instance.enabled { Text("已停用").foregroundStyle(.secondary) } }
                        Text((store.runtimeTypes.first { $0.id == instance.typeID }?.name ?? instance.typeID) + (store.location(of: instance.id).host.map { " · SSH \($0)" } ?? "")).font(.caption).foregroundStyle(.secondary)
                    }.tag(instance.id)
                }
            }
            .contextMenu(forSelectionType: String.self) { ids in instanceMenu(ids) } primaryAction: { ids in
                if let id = ids.first { editor = store.runtimeInstances.first { $0.id == id } }
            }
            .listStyle(.bordered).padding(.horizontal, 20).padding(.top, 16)
            HStack {
                Button { rememberIDs(); creating = true } label: { Image(systemName: "plus") }.help("添加 Agent 运行时实例")
                Button("快速导入…") { rememberIDs(); discovering = true }.disabled(store.isBusy)
                Button { deleting = selected } label: { Image(systemName: "minus") }.disabled(selected == nil).help("删除运行时实例")
                Spacer()
                Button("在资源库中查看…") { if let id = selectedID { ResourceLibraryNavigator.shared.show(id, openWindow) } }
                    .disabled(selected == nil).help("管理所选运行时的指令、技能等原生资源")
                Button("编辑…") { editor = selected }.disabled(selected == nil)
            }.padding(.horizontal, 20).padding(.vertical, 12)
        }
        .sheet(isPresented: $discovering, onDismiss: offerResources) { RuntimeDiscoveryView(store: store) }
        .sheet(item: $editor) { instance in RuntimeEditor(store: store, instance: instance) }
        .alert("在 \(added?.runtime.name ?? "") 中启用资源？", isPresented: Binding(get: { added != nil }, set: { if !$0 { added = nil } })) {
            Button("在资源库中查看") { if let id = added?.runtime.id { ResourceLibraryNavigator.shared.show(id, openWindow) } }
            Button("以后", role: .cancel) {}
        } message: {
            Text("资源库中有 \(added?.resources ?? 0) 项资源可用于这个运行时。新运行时不会自动启用它们，可以在资源库中选择要启用的内容。")
        }
        .sheet(isPresented: $creating, onDismiss: offerResources) { RuntimeEditor(store: store, instance: nil) }
        .alert("删除运行时实例？", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { instance in
            Button("删除", role: .destructive) { store.deleteRuntimeInstance(id: instance.id) }
            Button("取消", role: .cancel) {}
        } message: { _ in
            Text("只删除此配置实例；Agent 运行时自己的会话、资源和配置文件不会被删除。")
        }
    }

    private func rememberIDs() { knownIDs = Set(store.runtimeInstances.map(\.id)) }

    /// After an add sheet has closed, offers the library to a new runtime that
    /// can use some of its resources.
    private func offerResources() {
        guard let runtime = store.runtimeInstances.first(where: { !knownIDs.contains($0.id) }) else { return }
        let id = runtime.id
        ResourceLibraryModel.shared(transport: store.resourceTransport).run({ transport in
            try transport.resourceSyncStatus().filter { status in
                guard status.runtimeId == id, case .notEnabled = status.state else { return false }
                return true
            }.count
        }, done: { count in
            if count > 0 { added = (runtime, count) }
        })
    }

    @ViewBuilder private func instanceMenu(_ ids: Set<String>) -> some View {
        if let id = ids.first, let runtime = store.runtimeInstances.first(where: { $0.id == id }) {
            Button("编辑…") { editor = runtime }
            Button("在资源库中查看…") { ResourceLibraryNavigator.shared.show(id, openWindow) }
            Button(runtime.enabled ? "停用" : "启用") { var updated = runtime; updated.enabled.toggle(); store.saveRuntimeInstance(updated) }.disabled(store.isBusy)
            Button("删除…") { deleting = runtime }
        }
    }
}

struct RuntimeEditor: View {
    @ObservedObject var store: AppStore
    let instance: RuntimeInstance?
    private var types: [RuntimeTypeDescriptor] { store.runtimeTypes }
    private var gatewayID: String { store.gateway.id }
    private var isSaving: Bool { store.isLoading }
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openWindow) private var openWindow
    @State private var name = ""
    @State private var enabled = true
    @State private var typeID = ""
    @State private var settings: [String: String] = [:]
    @State private var draftID = UUID().uuidString
    @State private var typeDrafts: [String: [String: String]] = [:]
    @State private var executableDiscoveryRunning = false
    @State private var executableDiscoveryMessage: String?
    @State private var execution: ExecutionLoad = .loading
    @State private var home: HomeChoice = .inherit
    @State private var homePath = ""
    @State private var remoteRoot = ""
    /// Where the saved instance's managed HOME is, for revealing it.
    @State private var managedHome: String?
    @State private var launchArguments: [LaunchArgument] = []
    @State private var environment: [EnvironmentEntry] = []
    @State private var agent: AgentLoad?
    private struct LaunchArgument: Identifiable { let id = UUID(); var value = "" }
    private struct EnvironmentEntry: Identifiable { let id = UUID(); var name = ""; var value = "" }
    private enum AgentLoad { case loading, failed(String), ready(BindingAcpRuntimeInfo) }
    @State private var location: LocationChoice = .local
    @State private var sshBinary = "/usr/bin/ssh"
    @State private var sshDestination = ""
    @State private var sshPort = ""
    @State private var remoteResources = ""
    private enum ExecutionLoad { case loading, failed, ready }
    private enum HomeChoice: Hashable { case inherit, existing, managed }
    private enum LocationChoice: Hashable { case local, ssh }
    private var descriptor: RuntimeTypeDescriptor? { types.first { $0.id == typeID } }
    /// Paths of a remote instance are on its machine: typed, not chosen or discovered here.
    private var remote: Bool { location == .ssh }
    private var sshBinaryField: SettingField {
        SettingField(key: "sshBinary", label: "SSH 程序", kind: .filePath, required: true, value: "", options: [],
                     help: "本机 OpenSSH 客户端的绝对路径。")
    }
    private var sshDestinationField: SettingField {
        SettingField(key: "sshDestination", label: "主机", kind: .text, required: true, value: "", options: [],
                     help: "OpenSSH 主机名或 ~/.ssh/config 中的别名。连接不交互：请预先完成登录认证与主机信任；不转发认证 agent。")
    }
    private var sshPortField: SettingField {
        SettingField(key: "sshPort", label: "端口", kind: .text, required: false, value: "", options: [], help: "留空使用 SSH 配置中的端口。")
    }
    private var remoteResourcesField: SettingField {
        SettingField(key: "remoteResources", label: "远端辅助目录", kind: .text, required: descriptor?.remoteResources == true, value: "", options: [],
                     help: "执行机器上的绝对目录，Velune 首次使用时在其中放入读取会话等所需的辅助程序；应独立于运行时目录。")
    }
    private var transportExecution: BindingRuntimeTransport? {
        guard remote else { return .local }
        let binary = MacPath.normalized(sshBinary, field: sshBinaryField)
        let destination = sshDestination.trimmingCharacters(in: .whitespacesAndNewlines)
        let portText = sshPort.trimmingCharacters(in: .whitespacesAndNewlines)
        let port = UInt16(portText)
        guard binary.hasPrefix("/"), !destination.isEmpty, !destination.hasPrefix("-"), !destination.contains(where: { $0.isWhitespace }),
              portText.isEmpty || (port ?? 0) > 0 else { return nil }
        return .ssh(binary: binary, destination: destination, port: portText.isEmpty ? nil : port)
    }
    /// `nil` when invalid; the inner value is what is saved.
    private var remoteResourcesExecution: String?? {
        guard remote else { return .some(nil) }
        let path = remoteResources.trimmingCharacters(in: .whitespacesAndNewlines)
        if path.isEmpty { return descriptor?.remoteResources == true ? nil : .some(nil) }
        return path.hasPrefix("/") && !path.contains("\0") ? .some(path) : nil
    }
    /// A managed HOME holds the runtime directory, so that field goes away.
    private var fields: [SettingField] { (descriptor?.fields ?? []).filter { !(home == .managed && $0.key == "agentDir") } }
    private var homeField: SettingField {
        SettingField(key: "home", label: "HOME 目录", kind: remote ? .text : .directoryPath, required: true, value: "", options: [],
                     help: remote ? "执行机器上的绝对目录。" : nil)
    }
    private var remoteRootField: SettingField {
        SettingField(key: "remoteRoot", label: "远端根目录", kind: .text, required: true, value: "", options: [],
                     help: "执行机器上的绝对目录；Velune 在其下为此实例建立 HOME。")
    }
    private var homeExecution: BindingRuntimeHome? {
        let absolute = { (value: String) in value.hasPrefix("/") && !value.contains("\0") }
        switch home {
        case .inherit: return .inherit
        case .existing:
            let path = remote ? homePath.trimmingCharacters(in: .whitespacesAndNewlines) : MacPath.normalized(homePath, field: homeField)
            return absolute(path) ? .existing(path: path) : nil
        case .managed:
            guard remote else { return .managed(remoteRoot: nil) }
            let root = remoteRoot.trimmingCharacters(in: .whitespacesAndNewlines)
            return absolute(root) ? .managed(remoteRoot: root) : nil
        }
    }
    /// Blank rows are dropped; a variable needs a name without "=".
    private var launch: BindingAcpLaunch? {
        var variables: [String: String] = [:]
        for entry in environment {
            let name = entry.name.trimmingCharacters(in: .whitespacesAndNewlines)
            if name.isEmpty && entry.value.isEmpty { continue }
            guard !name.isEmpty, !name.contains("="), variables[name] == nil else { return nil }
            variables[name] = entry.value
        }
        return BindingAcpLaunch(args: launchArguments.map(\.value).filter { !$0.isEmpty }, environment: variables)
    }
    private var valid: Bool { !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && descriptor != nil && homeExecution != nil && transportExecution != nil && remoteResourcesExecution != nil && (descriptor?.launchOptions != true || launch != nil) && (!enabled || fields.allSatisfy { field in
        let value = settings[field.key] ?? field.value
        return (!field.required || !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) && MacPath.isValid(value, field: field, remote: remote)
    }) }
    private var pathValidationMessage: String? {
        guard enabled, let field = fields.first(where: { !MacPath.isValid(settings[$0.key] ?? $0.value, field: $0, remote: remote) }) else { return nil }
        return remote ? "「\(field.label)」必须是执行机器上的绝对路径。" : "「\(field.label)」必须是绝对路径；可使用 ~/ 开头，保存时会展开。"
    }
    var body: some View {
        VStack(spacing: 0) {
            Text(instance == nil ? "添加 Agent 运行时" : "编辑 Agent 运行时").font(.headline).frame(maxWidth: .infinity, alignment: .leading).padding([.horizontal, .top], 20)
            Form {
                Section {
                    TextField("实例名称", text: $name)
                    Toggle("启用此运行时", isOn: $enabled).help("停用后不读取其历史，也不创建或执行新任务；原生会话数据保留。")
                    Picker("类型", selection: $typeID) { ForEach(types) { type in Text(type.name).tag(type.id) } }.disabled(instance != nil)
                }
                Section("运行环境") { homeSection }
                if descriptor != nil {
                    Section("实例配置") {
                        ForEach(fields) { field in
                            SettingFieldView(
                                field: field,
                                value: Binding(get: { settings[field.key] ?? field.value }, set: { settings[field.key] = $0 }),
                                remote: remote,
                                discoveryTitle: discoveryTitle(for: field),
                                discoveryAction: discoveryAction(for: field)
                            )
                        }
                        if let pathValidationMessage { Text(pathValidationMessage).font(.caption).foregroundStyle(.red) }
                        if let executableDiscoveryMessage {
                            Text(executableDiscoveryMessage).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
                if descriptor?.launchOptions == true, case .ready = execution { launchSections }
                if descriptor?.agentInfo == true, let instance { agentSection(instance) }
            }.formStyle(.grouped)
            HStack {
                if let instance {
                    Button("在资源库中查看…") { dismiss(); ResourceLibraryNavigator.shared.show(instance.id, openWindow) }
                        .help("在“运行时资源”窗口管理此运行时的指令等原生资源")
                }
                Spacer(); Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Button("保存") { commit() }.keyboardShortcut(.defaultAction).disabled(!valid || isSaving)
            }.padding(20)

        }.frame(width: 590, height: 600)
        .onAppear {
            name = instance?.name ?? ""
            enabled = instance?.enabled ?? true
            typeID = instance?.typeID ?? types.first?.id ?? ""
            if instance == nil { execution = .ready } else { loadExecution() }
            settings = instance?.settings ?? [:]
            if let descriptor {
                for field in descriptor.fields where field.kind == .filePath || field.kind == .directoryPath {
                    if let value = settings[field.key] { settings[field.key] = MacPath.expanded(value) }
                }
            }
            DispatchQueue.main.async { discoverExecutableIfNeeded() }
        }
        .onChange(of: typeID) { old, new in
            if instance == nil { typeDrafts[old] = settings; settings = typeDrafts[new] ?? [:] }
            executableDiscoveryMessage = nil
            DispatchQueue.main.async { discoverExecutableIfNeeded() }
        }
    }

    @ViewBuilder private var homeSection: some View {
        switch execution {
        case .loading:
            HStack { ProgressView().controlSize(.small); Text("正在读取运行环境…").foregroundStyle(.secondary) }
        case .failed:
            HStack { Text("无法读取此实例的运行环境。").foregroundStyle(.secondary); Spacer(); Button("重试") { loadExecution() } }
        case .ready:
            Picker("运行位置", selection: $location) {
                Text("本机").tag(LocationChoice.local)
                Text("SSH").tag(LocationChoice.ssh)
            }
            if remote {
                SettingFieldView(field: sshBinaryField, value: $sshBinary)
                SettingFieldView(field: sshDestinationField, value: $sshDestination)
                SettingFieldView(field: sshPortField, value: $sshPort)
                if transportExecution == nil { Text("需要 SSH 程序的绝对路径、不含空白且不以“-”开头的主机，端口为 1–65535。").font(.caption).foregroundStyle(.red) }
                SettingFieldView(field: remoteResourcesField, value: $remoteResources, remote: true)
                Text("执行机器需要 POSIX 环境与 Python 3，运行时自身也须安装在那里；下方的路径均指执行机器上的路径。").font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            Picker("HOME", selection: $home) {
                Text("继承当前用户").tag(HomeChoice.inherit)
                Text("指定目录").tag(HomeChoice.existing)
                Text("由 Velune 管理").tag(HomeChoice.managed)
            }
            switch home {
            case .inherit:
                Text("以\(remote ? "执行机器上用户" : "你")的 HOME 启动，与在终端中直接运行相同。").font(.caption).foregroundStyle(.secondary)
            case .existing:
                SettingFieldView(field: homeField, value: $homePath)
                Text("以此目录作为 HOME 启动；运行时目录仍按下方设置。").font(.caption).foregroundStyle(.secondary)
            case .managed:
                if remote { SettingFieldView(field: remoteRootField, value: $remoteRoot) }
                Text("Velune 为此实例单独建立 HOME，运行时目录位于其中，首次使用时创建。不会复制现有的登录、配置或会话；HOME 也不是安全沙箱。")
                    .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                if let managedHome, !remote {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("位置")
                        HStack {
                            Text(managedHome).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle).textSelection(.enabled)
                            Spacer()
                            Button("在 Finder 中显示") { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: managedHome)]) }
                                .disabled(!FileManager.default.fileExists(atPath: managedHome))
                        }
                    }
                }
            }
        }
    }
    @ViewBuilder private var launchSections: some View {
        Section {
            ForEach($launchArguments) { $argument in
                HStack {
                    TextField("参数", text: $argument.value, prompt: Text("参数")).labelsHidden()
                    Button { launchArguments.removeAll { $0.id == argument.id } } label: { Image(systemName: "minus.circle") }
                        .buttonStyle(.borderless).help("移除此参数").accessibilityLabel("移除参数")
                }
            }
            Button("添加参数") { launchArguments.append(LaunchArgument()) }
        } header: { Text("启动参数") } footer: {
            Text("按顺序逐个传给运行时入口，不经过 shell，因此空格与引号原样保留。").font(.caption).foregroundStyle(.secondary)
        }
        Section {
            ForEach($environment) { $entry in
                HStack {
                    TextField("名称", text: $entry.name, prompt: Text("名称")).labelsHidden().frame(width: 170)
                    TextField("值", text: $entry.value, prompt: Text("值")).labelsHidden()
                    Button { environment.removeAll { $0.id == entry.id } } label: { Image(systemName: "minus.circle") }
                        .buttonStyle(.borderless).help("移除此变量").accessibilityLabel("移除变量 \(entry.name)")
                }
            }
            Button("添加变量") { environment.append(EnvironmentEntry()) }
            if launch == nil { Text("变量需要名称，名称不能包含“=”或重复。").font(.caption).foregroundStyle(.red) }
        } header: { Text("环境变量") } footer: {
            Text("在继承的环境之上设置，只用于此实例。值以明文保存在本机应用配置中。").font(.caption).foregroundStyle(.secondary)
        }
    }

    /// Reading starts the saved Agent once; it reports the saved settings,
    /// not unsaved edits in this form.
    @ViewBuilder private func agentSection(_ instance: RuntimeInstance) -> some View {
        Section {
            switch agent {
            case nil:
                Text("启动一次已保存的 Agent，读取它报告的名称、版本、会话能力与登录方式；不会发送消息。").font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            case .loading:
                HStack { ProgressView().controlSize(.small); Text("正在读取 Agent 信息…").foregroundStyle(.secondary) }
            case .failed(let detail):
                Text(detail).foregroundStyle(.secondary).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
            case .ready(let info):
                if let agent = info.agentInfo {
                    LabeledContent("名称", value: agent.title ?? agent.name)
                    LabeledContent("版本", value: agent.version)
                } else {
                    LabeledContent("名称", value: "未报告")
                }
                let capabilities = [("加载历史", info.capabilities.loadSession), ("列出会话", info.capabilities.listSessions), ("恢复会话", info.capabilities.resumeSession), ("关闭会话", info.capabilities.closeSession), ("删除会话", info.capabilities.deleteSession)]
                LabeledContent("会话能力") {
                    VStack(alignment: .trailing, spacing: 2) {
                        ForEach(capabilities, id: \.0) { name, supported in
                            Label(name, systemImage: supported ? "checkmark" : "minus").foregroundStyle(supported ? .primary : .secondary)
                                .accessibilityElement(children: .ignore).accessibilityLabel(name).accessibilityValue(supported ? "支持" : "不支持")
                        }
                    }
                }
                if info.authMethods.isEmpty {
                    LabeledContent("登录方式", value: "无")
                } else {
                    LabeledContent("登录方式") {
                        VStack(alignment: .trailing, spacing: 2) {
                            ForEach(info.authMethods, id: \.id) { method in
                                Text(method.name).help(method.description ?? method.name)
                            }
                        }
                    }
                    Text("登录在 Agent 自己的流程中完成，Velune 不代为认证。").font(.caption).foregroundStyle(.secondary)
                }
            }
            HStack {
                Spacer()
                Button(agentLoaded ? "重新读取" : "读取 Agent 信息") { readAgent(instance) }
                    .disabled(!instance.enabled || isLoadingAgent)
            }
        } header: { Text("Agent") }
    }
    private var agentLoaded: Bool { if case .ready = agent { return true }; return false }
    private var isLoadingAgent: Bool { if case .loading = agent { return true }; return false }
    private func readAgent(_ instance: RuntimeInstance) {
        agent = .loading
        store.agentInfo(runtimeID: instance.id) { result in
            switch result {
            case .success(let info): agent = .ready(info)
            case .failure(let error): agent = .failed(AppProblem.failure(error, source: "读取 Agent 信息").detail)
            }
        }
    }

    private func loadExecution() {
        guard let instance else { return }
        execution = .loading
        store.runtimeExecution(id: instance.id, launch: types.first { $0.id == instance.typeID }?.launchOptions == true, onFailure: { execution = .failed }) { saved, directory, launch in
            if let launch {
                launchArguments = launch.args.map { LaunchArgument(value: $0) }
                environment = launch.environment.sorted { $0.key < $1.key }.map { EnvironmentEntry(name: $0.key, value: $0.value) }
            }
            switch saved.home {
            case .inherit: home = .inherit
            case .existing(let path): home = .existing; homePath = path
            case .managed(let root): home = .managed; remoteRoot = root ?? ""; managedHome = directory
            }
            switch saved.transport {
            case .local: location = .local
            case .ssh(let binary, let destination, let port):
                location = .ssh; sshBinary = binary; sshDestination = destination; sshPort = port.map(String.init) ?? ""
            }
            remoteResources = saved.remoteResourcesDirectory ?? ""
            execution = .ready
        }
    }

    private func discoveryValue(for field: SettingField) -> String { (settings[field.key] ?? field.value).trimmingCharacters(in: .whitespacesAndNewlines) }
    private func discoveryTitle(for field: SettingField) -> String? {
        guard !remote, field.executableDiscovery != nil, discoveryValue(for: field).isEmpty else { return nil }
        return executableDiscoveryRunning ? "探测中…" : (executableDiscoveryMessage == nil ? "自动发现" : "重新发现")
    }
    private func discoveryAction(for field: SettingField) -> (() -> Void)? {
        guard !remote, field.executableDiscovery != nil, discoveryValue(for: field).isEmpty, !executableDiscoveryRunning else { return nil }
        return { discoverExecutables([field], type: typeID) }
    }
    private func discoverExecutableIfNeeded() {
        discoverExecutables(descriptor?.fields.filter { $0.executableDiscovery != nil && discoveryValue(for: $0).isEmpty } ?? [], type: typeID)
    }
    private func discoverExecutables(_ fields: [SettingField], type: String) {
        // Discovery looks on this Mac; a remote instance's executables are elsewhere.
        guard !remote, !executableDiscoveryRunning, type == typeID, let field = fields.first, let discovery = field.executableDiscovery else { return }
        executableDiscoveryRunning = true
        executableDiscoveryMessage = "正在查找\(field.label)…"
        MacExecutableDiscovery.discover(discovery) { result in
            executableDiscoveryRunning = false
            guard type == typeID else { discoverExecutableIfNeeded(); return }
            switch result {
            case .success(let path):
                if discoveryValue(for: field).isEmpty { settings[field.key] = path; executableDiscoveryMessage = "已发现：\(path)" }
            case .failure(let error): executableDiscoveryMessage = nil; store.recordProblem(error, source: "自动发现可执行文件")
            }
            discoverExecutables(Array(fields.dropFirst()), type: type)
        }
    }
    private func commit() {
        guard valid, let descriptor, let homeExecution, let transport = transportExecution, let remoteResources = remoteResourcesExecution, case .ready = execution else { return }
        // Settings this form does not edit, such as an ACP launch, stay as saved.
        var values = instance?.settings ?? [:]
        for field in descriptor.fields {
            values[field.key] = MacPath.normalized(settings[field.key] ?? field.value, field: field, remote: remote)
        }
        store.saveRuntimeInstance(RuntimeInstance(enabled: enabled, id: instance?.id ?? draftID, name: name.trimmingCharacters(in: .whitespacesAndNewlines), typeID: typeID, gatewayID: instance?.gatewayID ?? gatewayID, settings: values),
                                  execution: BindingRuntimeExecution(transport: transport, home: homeExecution, remoteResourcesDirectory: remoteResources),
                                  launch: descriptor.launchOptions ? launch : nil) { dismiss() }
    }
}

private enum MacPath {
    static func expanded(_ value: String) -> String {
        NSString(string: value.trimmingCharacters(in: .whitespacesAndNewlines)).expandingTildeInPath
    }

    /// A remote path is the other machine's: kept as typed, with no `~` expansion.
    static func normalized(_ value: String, field: SettingField, remote: Bool = false) -> String {
        guard !remote, field.kind == .filePath || field.kind == .directoryPath else {
            return value.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        let path = expanded(value)
        guard !path.isEmpty else { return "" }
        return URL(fileURLWithPath: path).standardizedFileURL.path
    }

    static func isValid(_ value: String, field: SettingField, remote: Bool = false) -> Bool {
        guard field.kind == .filePath || field.kind == .directoryPath else { return true }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return !field.required }
        if remote { return trimmed.hasPrefix("/") && !trimmed.contains("\0") }
        let path = expanded(trimmed)
        return path.hasPrefix("/") && !path.contains("\0")
    }
}

struct SettingFieldView: View {
    let field: SettingField
    @Binding var value: String
    /// The path is on a remote machine, so there is nothing here to choose.
    var remote = false
    var discoveryTitle: String? = nil
    var discoveryAction: (() -> Void)? = nil
    var body: some View {
        if field.kind == .choice {
            Picker(field.label, selection: $value) { ForEach(field.options) { option in Text(option.label).tag(option.id) } }.help(field.help ?? field.label)
        } else {
            // Paths are long: the value gets the row's full width, with the
            // label above and the explanation below instead of beside it.
            VStack(alignment: .leading, spacing: 6) {
                Text(field.label)
                HStack {
                    TextField(field.label, text: $value, prompt: Text(field.required ? "必填" : "可选"))
                        .labelsHidden().textFieldStyle(.roundedBorder).multilineTextAlignment(.leading)
                    if !remote && (field.kind == .filePath || field.kind == .directoryPath) { Button("选择…", action: choosePath) }
                    if let discoveryTitle, let discoveryAction { Button(discoveryTitle, action: discoveryAction).disabled(discoveryTitle == "探测中…") }
                }
                if let help = field.help { Text(help).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
            }
        }
    }
    private func choosePath() {
        let panel = NSOpenPanel(); panel.canChooseDirectories = field.kind == .directoryPath; panel.canChooseFiles = field.kind == .filePath; panel.allowsMultipleSelection = false
        if panel.runModal() == .OK, let url = panel.url { value = url.path }
    }
}

struct AuthenticationView: View {
    @ObservedObject var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @State private var answer = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("认证来源登录").font(.headline).frame(maxWidth: .infinity, alignment: .leading)
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    ForEach(Array(store.authenticationNotifications.enumerated()), id: \.offset) { _, notice in
                        if let text = notice.text { Text(text).textSelection(.enabled) }
                        if let instructions = notice.instructions { Text(instructions) }
                        if let code = notice.userCode { LabeledContent("验证码", value: code).textSelection(.enabled) }
                        if let address = notice.url ?? notice.verificationUri, let url = URL(string: address), url.scheme == "https" {
                            Link("在浏览器中继续", destination: url)
                        }
                    }
                    if let prompt = store.authenticationPrompt {
                        Text(prompt.text)
                        if let options = prompt.options {
                            Picker("选择", selection: $answer) { Text("请选择").tag(""); ForEach(options) { Text($0.label).tag($0.id) } }
                        } else if prompt.kind == "secret" { SecureField("输入", text: $answer) }
                        else { TextField("输入", text: $answer) }
                        Button("继续") { store.answerAuthentication(id: prompt.id, value: answer); answer = "" }.disabled(answer.isEmpty || store.isLoading)
                    }
                    if let result = store.authenticationResult { Text(result) }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }

            HStack {
                if store.authenticationRunning { ProgressView().controlSize(.small) }
                Spacer()
                if store.authenticationRunning { Button("取消登录") { store.cancelAuthentication() }.disabled(store.isLoading) }
                else { Button("完成") { dismiss() }.keyboardShortcut(.defaultAction) }
            }
        }.padding(24).frame(width: 560, height: 450)
        .interactiveDismissDisabled(store.authenticationRunning)
        .onChange(of: store.authenticationPrompt?.id) { _, _ in answer = "" }
    }
}

struct RuntimeInteractionView: View {
    @ObservedObject var store: AppStore
    let interaction: RuntimeInteraction
    @State private var values: [String:String] = [:]
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            switch interaction.kind {
            case .approval(let title, let detail, let options):
                Text(title).font(.headline).frame(maxWidth: .infinity, alignment: .leading)
                ScrollView { Text(detail).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }.frame(maxHeight: 220)
                HStack { Button("取消请求", role: .cancel) { reply(.cancel) }; Spacer(); ForEach(options) { option in Button(option.label) { reply(.decision(option.id)) } } }.disabled(store.isLoading)
            case .userInput(let questions):
                Text("运行时需要输入").font(.headline).frame(maxWidth: .infinity, alignment: .leading)
                Form {
                    ForEach(questions) { question in
                        Section {
                            Text(question.text).textSelection(.enabled)
                            if !question.options.isEmpty { Picker("选择", selection: value(question.id)) { Text("请选择").tag(""); ForEach(question.options) { Text($0.label).tag($0.id) } } }
                            else if question.secret { SecureField("回答", text: value(question.id)).privacySensitive() }
                            else { TextField("回答", text: value(question.id)) }
                        }
                    }
                }.formStyle(.grouped)
                HStack { Button("取消请求", role: .cancel) { reply(.cancel) }; Spacer(); Button("提交") { reply(.answers(questions.map { InteractionAnswer(questionID: $0.id, values: [values[$0.id] ?? ""]) })) }.disabled(questions.contains { (values[$0.id] ?? "").isEmpty }) }.disabled(store.isLoading)
            }

        }.padding(20).frame(width: 500).frame(minHeight: 180, maxHeight: 480)
        .interactiveDismissDisabled()
        .onChange(of: interaction.id) { _, _ in values = [:] }
    }
    private func value(_ id: String) -> Binding<String> { Binding(get: { values[id] ?? "" }, set: { values[id] = $0 }) }
    private func reply(_ value: RuntimeInteractionReply) { store.replyInteraction(interaction, reply: value) }
}
