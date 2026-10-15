import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// A window's panes as arranged: a pane, or a split laid out by its fractions.
struct PaneLayoutView: View {
    @ObservedObject var window: ConversationWindowModel
    let layout: PaneLayout
    /// Child indices from the root to this part of the layout.
    let path: [Int]
    let previewEmpty: Bool

    var body: some View {
        switch layout {
        case .pane(let id):
            if let pane = window.panes[id] {
                PaneView(window: window, pane: pane, previewEmpty: previewEmpty).id(id)
            }
        case .split(let split):
            PaneSplitView(window: window, split: split, path: path, previewEmpty: previewEmpty)
        }
    }
}

/// Panes side by side or stacked, with dividers that resize the two panes
/// beside them.
private struct PaneSplitView: View {
    @ObservedObject var window: ConversationWindowModel
    let split: PaneSplit
    let path: [Int]
    let previewEmpty: Bool
    /// The fractions when the divider being dragged was picked up.
    @State private var dragStart: [Double]?
    private static let minimumLength: CGFloat = 320

    var body: some View {
        GeometryReader { geometry in
            let horizontal = split.axis == .horizontal
            let length = (horizontal ? geometry.size.width : geometry.size.height) - CGFloat(split.children.count - 1)
            let stack = horizontal ? AnyLayout(HStackLayout(spacing: 0)) : AnyLayout(VStackLayout(spacing: 0))
            stack {
                ForEach(Array(split.children.enumerated()), id: \.element.paneIDs) { index, child in
                    if index > 0 { divider(before: index, length: length) }
                    // Recursion through a type-erased view: a split can hold splits.
                    AnyView(PaneLayoutView(window: window, layout: child, path: path + [index], previewEmpty: previewEmpty))
                        .frame(width: horizontal ? max(0, length * split.fractions[index]) : geometry.size.width,
                               height: horizontal ? geometry.size.height : max(0, length * split.fractions[index]))
                        .clipped()
                }
            }
        }
    }

    private func divider(before index: Int, length: CGFloat) -> some View {
        let horizontal = split.axis == .horizontal
        return Divider()
            .overlay {
                // A wider grip than the line it moves.
                Color.clear
                    .frame(width: horizontal ? 7 : nil, height: horizontal ? nil : 7)
                    .contentShape(Rectangle())
                    .pointerStyle(horizontal ? .columnResize : .rowResize)
                    .gesture(DragGesture(minimumDistance: 1, coordinateSpace: .global)
                        .onChanged { value in
                            let start = dragStart ?? split.fractions
                            if dragStart == nil { dragStart = start }
                            guard length > 0 else { return }
                            let pair = start[index - 1] + start[index]
                            let minimum = min(pair / 2, Double(Self.minimumLength / length))
                            let moved = Double((horizontal ? value.translation.width : value.translation.height) / length)
                            var fractions = start
                            fractions[index - 1] = min(max(start[index - 1] + moved, minimum), pair - minimum)
                            fractions[index] = pair - fractions[index - 1]
                            window.resize(at: path, fractions: fractions)
                        }
                        .onEnded { _ in dragStart = nil })
            }
            .zIndex(1)
    }
}

/// A pane: its tab bar, shown once the window is split or the pane has more
/// than one tab, above the shown tab. Tabs shown before stay in place, hidden,
/// so they keep their scroll position.
struct PaneView: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var pane: Pane
    let previewEmpty: Bool
    @State private var size: CGSize = .zero
    @State private var dropZone: PaneDropZone?

    var body: some View {
        VStack(spacing: 0) {
            if window.isSplit || pane.tabs.count > 1 { PaneTabBar(window: window, pane: pane) }
            ZStack {
                ForEach(pane.tabs) { tab in
                    ActiveTab(tab: tab) {
                        TabContentView(window: window, tab: tab, store: tab.store, shown: tab.id == pane.selectedID, previewEmpty: previewEmpty)
                    }
                    .hidden(tab.id != pane.selectedID)
                }
            }
            // Exactly the room the pane has, however wide a tab's content would like to be.
            .frame(minWidth: 0, maxWidth: .infinity, minHeight: 0, maxHeight: .infinity)
        }
        .background(PaneAnchor { [window, id = pane.id] in window.register(id, anchor: $0) })
        .onGeometryChange(for: CGSize.self) { $0.size } action: { size = $0 }
        .onDrop(of: [.url], delegate: PaneDropDelegate(window: window, pane: pane.id, size: size, zone: $dropZone))
        .overlay { if let dropZone { PaneDropHighlight(zone: dropZone) } }
        .onChange(of: pane.selectedID) { old, new in
            // Typing that went to the tab now hidden goes to the one shown.
            guard pane.tabs.first(where: { $0.id == old })?.composer.isFocused == true, let tab = pane.tabs.first(where: { $0.id == new }) else { return }
            DispatchQueue.main.async { tab.composer.focus() }
        }
    }
}

/// A tab's views once it has been shown.
private struct ActiveTab<Content: View>: View {
    @ObservedObject var tab: ConversationTab
    @ViewBuilder let content: () -> Content

    var body: some View {
        if tab.isActive { content() }
    }
}

private extension View {
    @ViewBuilder func hidden(_ hidden: Bool) -> some View {
        if hidden { self.hidden().accessibilityHidden(true) } else { self }
    }
}

/// A tab's conversation: its transcript above its composer.
private struct TabContentView: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var tab: ConversationTab
    @ObservedObject var store: AppStore
    /// The tab its pane shows; only it presents runtime questions.
    let shown: Bool
    let previewEmpty: Bool

    var body: some View {
        VStack(spacing: 0) {
            Group {
                if store.pendingConversationID != nil { ProgressView(store.conversationManagementStatus ?? "正在加载会话…") }
                else if previewEmpty || store.transcript.rows.isEmpty { emptyState }
                else { transcript }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            footer
        }
        .sheet(item: Binding(get: { shown ? store.pendingInteractions.first : nil }, set: { _ in })) { RuntimeInteractionView(store: store, interaction: $0) }
        .onChange(of: store.loadedConversationID) { _, _ in tab.outlineCollapsed = false }
        .onChange(of: store.outlinePresentation) { _, _ in tab.outlineCollapsed = false }
    }

    /// A native sub-agent is continued by the agent that started it, so its
    /// conversation has no composer.
    @ViewBuilder private var footer: some View {
        if let parent = store.loadedParentConversationID, store.pendingConversationID == nil {
            VStack(spacing: 0) {
                Divider()
                HStack(spacing: 8) {
                    Text("这是原生 sub-agent 的会话，由启动它的会话继续。").foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                    Button("打开父会话") { store.openLinkedConversation(id: parent) }
                }
                .padding(.horizontal, 16).padding(.vertical, 10)
            }
            .background(.bar)
        } else if !window.tools.sharesComposer {
            ComposerView(store: store, model: tab.composer, send: tab.send)
        }
    }

    private var emptyState: some View {
        ContentUnavailableView {
            Label { Text("新会话") } icon: { VeluneLogo(size: 48) }
        } description: {
            Text(store.needsModelSelection ? "选择下一轮模型后即可发送。" : store.canSend ? "输入消息，开始工作。" : "先在设置中配置AI提供商、模型与Agent 运行时。")
        } actions: {
            if !store.canSend && !store.needsModelSelection { SettingsTabLink(tab: store.enabledRuntimeInstances.isEmpty ? .runtimes : .providers, title: "打开设置") }
        }
    }

    private var transcript: some View {
        TranscriptView(model: store.transcript, conversationID: store.loadedConversationID, scrollRequest: tab.scrollRequest,
                       cwd: store.loadedConversationWorkingDirectory, remoteHost: store.loadedConversationLocation.host, outlineCollapsed: $tab.outlineCollapsed, navigation: tab.outlineNavigation,
                       activity: store.activity, loadPayloads: { [store] ids in store.loadPayloads(messageIDs: ids) },
                       openConversation: { [store] id in store.openLinkedConversation(id: id) },
                       describeSettings: { [store] in store.describe($0) })
    }
}

/// A pane's tabs, after the system tab bar: equal widths sharing the bar
/// (narrower, their titles shortened, as tabs are added), titles centred, a close button on hover at the leading end and a new-tab
/// button at the trailing end. The focused pane's shown tab is marked once
/// the window is split.
private struct PaneTabBar: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var pane: Pane
    /// Where a tab dragged over the bar would go.
    @State private var insertion: Int?
    private static let height: CGFloat = 28

    var body: some View {
        HStack(spacing: 0) {
            // No scroll view: clicks in one go to its clip view, not to the tabs' AppKit views.
            GeometryReader { geometry in
                let width = geometry.size.width / CGFloat(max(pane.tabs.count, 1))
                HStack(spacing: 0) {
                    ForEach(Array(pane.tabs.enumerated()), id: \.element.id) { index, tab in
                        PaneTabItem(window: window, pane: pane, tab: tab, store: tab.store,
                                    selected: tab.id == pane.selectedID, marked: window.isSplit && window.focusedPaneID == pane.id && tab.id == pane.selectedID)
                            .frame(width: width)
                            .overlay(alignment: .leading) { if insertion == index { insertionMark } }
                            .overlay(alignment: .trailing) { if insertion == index + 1 && index == pane.tabs.count - 1 { insertionMark } }
                            .onDrop(of: [.url], delegate: TabDropDelegate(window: window, pane: pane.id, index: index, width: width, insertion: $insertion))
                    }
                }
            }
            Divider()
            Button { window.focus(pane.id); window.openInNewTab() } label: { Image(systemName: "plus") }
                .buttonStyle(.borderless).frame(width: Self.height, height: Self.height)
                .help("新建标签页（⌘ T）").accessibilityLabel("新建标签页")
                .onDrop(of: [.url], delegate: TabDropDelegate(window: window, pane: pane.id, index: pane.tabs.count, width: 0, insertion: $insertion))
        }
        .frame(height: Self.height)
        .background(Color.primary.opacity(0.06))
        .overlay(alignment: .bottom) { Divider() }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(window.focusedPaneID == pane.id ? "聚焦面板的标签页" : "标签页")
    }

    private var insertionMark: some View {
        Rectangle().fill(Color.accentColor).frame(width: 2).allowsHitTesting(false)
    }
}

/// One tab: a click shows it, a double click renames its conversation in
/// place, a drag carries it, the middle button or the close button shown on
/// hover closes it.
private struct PaneTabItem: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var pane: Pane
    @ObservedObject var tab: ConversationTab
    @ObservedObject var store: AppStore
    let selected: Bool
    /// The focused pane's shown tab, in a split window.
    let marked: Bool
    @State private var hovering = false
    @State private var renaming: String?
    @FocusState private var renameFocused: Bool

    var body: some View {
        ZStack {
            TabHandle(window: window, tab: tab, title: tab.title, actions: TabHandle.Actions(
                select: { window.selectTab(tab.id) },
                rename: { if canRename { renaming = tab.title } },
                close: { window.closeTab(tab.id) },
                hover: { hovering = $0 },
                menu: menu))
            if let name = renaming {
                TextField("会话名称", text: Binding(get: { name }, set: { renaming = $0 }))
                    .textFieldStyle(.plain).multilineTextAlignment(.center).font(.system(size: 12))
                    .focused($renameFocused)
                    .onSubmit(commitRename)
                    .onExitCommand { renaming = nil }
                    .onAppear { renameFocused = true }
                    .onChange(of: renameFocused) { _, focused in if !focused { commitRename() } }
                    .padding(.horizontal, 26)
            } else {
                Text(tab.title)
                    .font(.system(size: 12)).foregroundStyle(selected ? .primary : .secondary)
                    .lineLimit(1).truncationMode(.tail)
                    .padding(.horizontal, 26)
                    .allowsHitTesting(false)
            }
            HStack(spacing: 0) {
                leading.frame(width: TabHandle.closeSize, height: TabHandle.closeSize).padding(.leading, TabHandle.closeInset)
                Spacer(minLength: 0)
            }
            // The handle takes every click, the close button's included.
            .allowsHitTesting(false)
        }
        .frame(maxHeight: .infinity)
        .background(selected ? Color(nsColor: .windowBackgroundColor) : Color.clear)
        .overlay(alignment: .top) { if marked { Rectangle().fill(Color.accentColor).frame(height: 2) } }
        .overlay(alignment: .trailing) { Rectangle().fill(Color.primary.opacity(0.12)).frame(width: 1) }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(tab.title)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private var running: Bool {
        guard let id = store.selectedConversationID, let conversation = store.conversation(id: id) else { return false }
        return store.isRunning(conversation)
    }

    /// Close on hover; otherwise what the conversation is doing.
    @ViewBuilder private var leading: some View {
        if hovering && renaming == nil {
            Image(systemName: "xmark").font(.system(size: 9, weight: .semibold)).foregroundStyle(.secondary)
                .accessibilityLabel("关闭标签页")
        } else if !store.pendingInteractions.isEmpty {
            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.orange).imageScale(.small)
                .accessibilityLabel("等待你的回应")
        } else if running {
            ProgressView().controlSize(.mini).accessibilityLabel("正在运行")
        }
    }

    private var conversation: Conversation? {
        guard !store.isUnsentConversation, let id = store.loadedConversationID else { return nil }
        return store.conversation(id: id)
    }

    private var canRename: Bool {
        guard let conversation else { return false }
        return store.canManageConversations && store.canRenameConversation(conversation)
    }

    private func commitRename() {
        guard let name = renaming else { return }
        renaming = nil
        guard let conversation, name.trimmingCharacters(in: .whitespacesAndNewlines) != conversation.title else { return }
        store.renameConversation(conversation, title: name) {}
    }

    private func menu() -> NSMenu {
        let menu = NSMenu()
        if canRename { menu.addItem(ClosureMenuItem("重命名") { renaming = tab.title }) }
        menu.addItem(ClosureMenuItem("关闭标签页") { window.closeTab(tab.id) })
        if pane.tabs.count > 1 {
            menu.addItem(ClosureMenuItem("关闭其他标签页") { window.closeOtherTabs(tab.id) })
            menu.addItem(.separator())
            menu.addItem(ClosureMenuItem("在右侧拆分") { window.splitTab(tab.id, edge: .trailing) })
            menu.addItem(ClosureMenuItem("在下方拆分") { window.splitTab(tab.id, edge: .bottom) })
        }
        if window.tabCount > 1 {
            if pane.tabs.count == 1 { menu.addItem(.separator()) }
            menu.addItem(ClosureMenuItem("移到新窗口") { window.moveToNewWindow(tab.id) })
        }
        return menu
    }
}

/// A menu item that runs a closure.
final class ClosureMenuItem: NSMenuItem {
    private let run: () -> Void

    init(_ title: String, run: @escaping () -> Void) {
        self.run = run
        super.init(title: title, action: #selector(runClosure), keyEquivalent: "")
        target = self
    }

    required init(coder: NSCoder) { fatalError("not decoded") }

    @objc private func runClosure() { run() }
}

/// A tab as an AppKit view, since only AppKit tells where a drag ended:
/// dropped where nothing takes it, outside every window, the tab becomes a
/// window of its own. It selects on mouse down, as the system's tabs do.
private struct TabHandle: NSViewRepresentable {
    struct Actions {
        let select: () -> Void
        let rename: () -> Void
        let close: () -> Void
        let hover: (Bool) -> Void
        let menu: () -> NSMenu
    }
    /// Where the close button shows, from the leading edge.
    static let closeInset: CGFloat = 6
    static let closeSize: CGFloat = 16
    let window: ConversationWindowModel
    let tab: ConversationTab
    let title: String
    let actions: Actions

    func makeNSView(context: Context) -> HandleView { HandleView() }

    func updateNSView(_ view: HandleView, context: Context) {
        view.model = window
        view.tab = tab
        view.actions = actions
        view.toolTip = title
    }

    final class HandleView: NSView, NSDraggingSource {
        weak var model: ConversationWindowModel?
        weak var tab: ConversationTab?
        var actions: Actions?
        private var pressed: NSEvent?
        private var pressedClose = false
        private var hovering = false

        override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

        // SwiftUI's hover does not reach through an AppKit view; the handle tracks it.
        override func updateTrackingAreas() {
            super.updateTrackingAreas()
            trackingAreas.forEach(removeTrackingArea)
            addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect], owner: self))
        }

        override func mouseEntered(with event: NSEvent) { setHovering(true) }
        override func mouseExited(with event: NSEvent) { setHovering(false) }

        private func setHovering(_ value: Bool) {
            guard hovering != value else { return }
            hovering = value
            actions?.hover(value)
        }

        private var closeRect: NSRect {
            let size = TabHandle.closeSize
            return NSRect(x: TabHandle.closeInset, y: (bounds.height - size) / 2, width: size, height: size).insetBy(dx: -3, dy: -3)
        }

        override func mouseDown(with event: NSEvent) {
            if hovering, closeRect.contains(convert(event.locationInWindow, from: nil)) {
                pressedClose = true
                return
            }
            pressed = event
            actions?.select()
            if event.clickCount == 2 { actions?.rename() }
        }

        override func mouseUp(with event: NSEvent) {
            pressed = nil
            guard pressedClose else { return }
            pressedClose = false
            if closeRect.contains(convert(event.locationInWindow, from: nil)) { actions?.close() }
        }

        override func otherMouseUp(with event: NSEvent) {
            if event.buttonNumber == 2, bounds.contains(convert(event.locationInWindow, from: nil)) { actions?.close() }
        }

        override func menu(for event: NSEvent) -> NSMenu? { actions?.menu() }

        override func mouseDragged(with event: NSEvent) {
            guard let start = pressed, let tab else { return }
            let from = start.locationInWindow, to = event.locationInWindow
            guard hypot(to.x - from.x, to.y - from.y) > 4 else { return }
            pressed = nil
            let link = TabDragSource.link(for: tab.id, conversation: tab.link)
            TabDragSource.shared.begin(tab: tab.id, link: link)
            let board = NSPasteboardItem()
            board.setString(link.absoluteString, forType: .URL)
            board.setString(link.absoluteString, forType: .string)
            let item = NSDraggingItem(pasteboardWriter: board)
            item.setDraggingFrame(bounds, contents: snapshot())
            beginDraggingSession(with: [item], event: start, source: self)
        }

        func draggingSession(_ session: NSDraggingSession, sourceOperationMaskFor context: NSDraggingContext) -> NSDragOperation {
            context == .withinApplication ? .move : .copy
        }

        func draggingSession(_ session: NSDraggingSession, endedAt screenPoint: NSPoint, operation: NSDragOperation) {
            guard operation == [], let model, let tab,
                  !NSApp.windows.contains(where: { $0.isVisible && $0.frame.contains(screenPoint) }) else { return }
            model.moveToNewWindow(tab.id, at: screenPoint)
        }

        /// The tab as it looks, to drag around.
        private func snapshot() -> NSImage {
            guard let content = window?.contentView else { return NSImage(size: NSSize(width: 1, height: 1)) }
            let rect = convert(bounds, to: content)
            guard rect.width > 0, rect.height > 0, let representation = content.bitmapImageRepForCachingDisplay(in: rect) else { return NSImage(size: NSSize(width: 1, height: 1)) }
            content.cacheDisplay(in: rect, to: representation)
            let image = NSImage(size: rect.size)
            image.addRepresentation(representation)
            return image
        }
    }
}

/// A tab dropped on the bar goes before or after the tab under the pointer.
private struct TabDropDelegate: DropDelegate {
    let window: ConversationWindowModel
    let pane: UUID
    let index: Int
    let width: CGFloat
    @Binding var insertion: Int?

    func validateDrop(info: DropInfo) -> Bool { info.hasItemsConforming(to: [.url]) }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        let at = width > 0 && info.location.x > width / 2 ? index + 1 : index
        if insertion != at { insertion = at }
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) { insertion = nil }

    func performDrop(info: DropInfo) -> Bool {
        guard let at = insertion, let provider = info.itemProviders(for: [.url]).first else { insertion = nil; return false }
        insertion = nil
        _ = provider.loadObject(ofClass: NSURL.self) { [window, pane] object, _ in
            guard let url = object as? URL else { return }
            Task { @MainActor in window.drop(url, on: pane, zone: .center, at: at) }
        }
        return true
    }
}

/// Where a tab dragged over a pane would go: the nearest edge within a
/// quarter of the pane splits there; the middle adds it to the pane's tabs.
private struct PaneDropDelegate: DropDelegate {
    let window: ConversationWindowModel
    let pane: UUID
    let size: CGSize
    @Binding var zone: PaneDropZone?

    func validateDrop(info: DropInfo) -> Bool { info.hasItemsConforming(to: [.url]) }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        let x = info.location.x / max(size.width, 1), y = info.location.y / max(size.height, 1)
        let nearest = [(PaneEdge.leading, x), (.trailing, 1 - x), (.top, y), (.bottom, 1 - y)].min { $0.1 < $1.1 }!
        let zone: PaneDropZone = nearest.1 < 0.25 ? .edge(nearest.0) : .center
        if self.zone != zone { self.zone = zone }
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) { zone = nil }

    func performDrop(info: DropInfo) -> Bool {
        guard let zone, let provider = info.itemProviders(for: [.url]).first else { zone = nil; return false }
        self.zone = nil
        _ = provider.loadObject(ofClass: NSURL.self) { [window, pane] object, _ in
            guard let url = object as? URL else { return }
            Task { @MainActor in window.drop(url, on: pane, zone: zone) }
        }
        return true
    }
}

/// The part of a pane a drop would take.
private struct PaneDropHighlight: View {
    let zone: PaneDropZone

    var body: some View {
        GeometryReader { geometry in
            let size = geometry.size
            let rect: CGRect = switch zone {
            case .center: CGRect(origin: .zero, size: size)
            case .edge(.leading): CGRect(x: 0, y: 0, width: size.width / 2, height: size.height)
            case .edge(.trailing): CGRect(x: size.width / 2, y: 0, width: size.width / 2, height: size.height)
            case .edge(.top): CGRect(x: 0, y: 0, width: size.width, height: size.height / 2)
            case .edge(.bottom): CGRect(x: 0, y: size.height / 2, width: size.width, height: size.height / 2)
            }
            RoundedRectangle(cornerRadius: 6)
                .fill(Color.accentColor.opacity(0.12))
                .strokeBorder(Color.accentColor.opacity(0.6), lineWidth: 2)
                .frame(width: max(0, rect.width - 8), height: max(0, rect.height - 8))
                .position(x: rect.midX, y: rect.midY)
        }
        .allowsHitTesting(false)
        .animation(.easeOut(duration: 0.12), value: zone)
    }
}
