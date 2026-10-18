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
/// beside them; a double click on one shares the room equally again. A
/// divider's drag is this view's until it ends: published to the window on
/// every step, it would have every view of the window evaluated again.
private struct PaneSplitView: View {
    @ObservedObject var window: ConversationWindowModel
    let split: PaneSplit
    let path: [Int]
    let previewEmpty: Bool
    /// The fractions when the divider being dragged was picked up, and as dragged.
    @State private var dragStart: [Double]?
    @State private var dragged: [Double]?
    private static let minimumLength: CGFloat = 320

    var body: some View {
        GeometryReader { geometry in
            let horizontal = split.axis == .horizontal
            let length = (horizontal ? geometry.size.width : geometry.size.height) - CGFloat(split.children.count - 1)
            let fractions = dragged.flatMap { $0.count == split.fractions.count ? $0 : nil } ?? split.fractions
            let stack = horizontal ? AnyLayout(HStackLayout(spacing: 0)) : AnyLayout(VStackLayout(spacing: 0))
            stack {
                ForEach(Array(split.children.enumerated()), id: \.element.paneIDs) { index, child in
                    if index > 0 { Divider() }
                    // Recursion through a type-erased view: a split can hold splits.
                    AnyView(PaneLayoutView(window: window, layout: child, path: path + [index], previewEmpty: previewEmpty))
                        .frame(width: horizontal ? max(0, length * fractions[index]) : geometry.size.width,
                               height: horizontal ? geometry.size.height : max(0, length * fractions[index]))
                        .clipped()
                }
            }
            .overlay {
                SashOverlay(axis: horizontal ? .horizontal : .vertical, lengths: fractions.map { max(0, length * $0) },
                            drag: { index, distance in drag(before: index, distance: distance, length: length) },
                            end: {
                                if let dragged { window.resize(at: path, fractions: dragged) }
                                dragged = nil
                                dragStart = nil
                            },
                            reset: { window.resize(at: path, fractions: split.fractions.map { _ in 1 / Double(split.fractions.count) }) })
            }
        }
    }

    private func drag(before index: Int, distance: CGFloat, length: CGFloat) {
        let start = dragStart ?? split.fractions
        if dragStart == nil { dragStart = start }
        guard length > 0 else { return }
        let pair = start[index - 1] + start[index]
        let minimum = min(pair / 2, Double(Self.minimumLength / length))
        var fractions = start
        fractions[index - 1] = min(max(start[index - 1] + Double(distance / length), minimum), pair - minimum)
        fractions[index] = pair - fractions[index - 1]
        dragged = fractions
    }
}

/// A pane: its tab bar, always there as a handle to drag and a menu to
/// open, above the shown tab. Only the shown tab has views: a tab behind it
/// keeps its transcript's native list (see `TranscriptReading`), so resizing
/// the pane lays out one tab and switching back keeps the scroll position.
struct PaneView: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var pane: Pane
    let previewEmpty: Bool
    @State private var size: CGSize = .zero
    @State private var dropZone: PaneDropZone?

    var body: some View {
        VStack(spacing: 0) {
            PaneTabBar(window: window, pane: pane)
            let tab = pane.selected
            TabContentView(window: window, tab: tab, store: tab.store, previewEmpty: previewEmpty)
                .id(tab.id)
                // Exactly the room the pane has, however wide a tab's content would like to be.
                .frame(minWidth: 0, maxWidth: .infinity, minHeight: 0, maxHeight: .infinity)
        }
        .background(PaneAnchor { [window, id = pane.id] in window.register(id, anchor: $0) })
        .onGeometryChange(for: CGSize.self) { $0.size } action: { size = $0 }
        .onDrop(of: ConversationLink.dropTypes, delegate: PaneDropDelegate(window: window, pane: pane.id, size: size, zone: $dropZone))
        .overlay { if let dropZone { PaneDropHighlight(zone: dropZone) } }
    }
}

/// A tab's conversation: its transcript above its composer.
struct TabContentView: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var tab: ConversationTab
    @ObservedObject var store: AppStore
    let previewEmpty: Bool
    /// Over the window, where a composer the window shares does not reach.
    var inSheet = false

    var body: some View {
        VStack(spacing: 0) {
            Group {
                if store.isOpeningConversation { ProgressView(store.conversationManagementStatus ?? "正在加载会话…") }
                else if previewEmpty || store.transcript.rows.isEmpty { emptyState }
                else { transcript }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            footer
        }
        .sheet(item: Binding(get: { store.pendingInteractions.first }, set: { _ in })) { RuntimeInteractionView(store: store, interaction: $0) }
        .sheet(item: $tab.editing) { request in
            MessageEditorSheet(store: store, request: request, forked: window.openInNewTab(adopting:))
        }
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
                    Button("打开父会话") { window.openLinked(parent, from: tab) }
                }
                .padding(.horizontal, 16).padding(.vertical, 10)
            }
            .background(.bar)
        } else if inSheet || window.tools.panesHaveComposer {
            ComposerView(store: store, model: tab.composer, layout: window, send: tab.send)
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
        TranscriptView(model: store.transcript, reading: tab.reading, conversationID: store.loadedConversationID, scrollRequest: tab.scrollRequest,
                       cwd: store.loadedConversationWorkingDirectory, remoteHost: store.loadedConversationLocation.host, outlineCollapsed: $tab.outlineCollapsed, navigation: tab.outlineNavigation,
                       activity: store.activity, loadPayloads: { [store] ids in store.loadPayloads(messageIDs: ids) },
                       openConversation: { [window, tab] id in window.openLinked(id, from: tab) },
                       editMessage: { [store, tab] id in store.editability(of: id) == nil ? nil : { tab.edit(messageID: id) } },
                       describeSettings: { [store] in store.describe($0) })
    }
}

/// A pane's tabs, drawn as the system's window tab bar: equal widths sharing
/// the bar (narrower, their titles shortened, as tabs are added), titles
/// centred, the shown tab in the window's colour and the others a shade
/// darker, a close button on hover at the leading end and a new-tab button
/// at the trailing end. Panes other than the focused one look as an inactive
/// window's bar does. As in the system's bar, a tab dragged along the bar
/// slides there, the others making way, and leaves it dragged off the bar.
private struct PaneTabBar: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var pane: Pane
    /// Where a tab dragged over the bar would go.
    @State private var insertion: Int?
    /// The tab sliding along the bar, and how far from its place.
    @State private var sliding: (id: UUID, distance: CGFloat)?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    static let height: CGFloat = 28

    var body: some View {
        let active = !window.isSplit || window.focusedPaneID == pane.id
        HStack(spacing: 0) {
            // No scroll view: clicks in one go to its clip view, not to the tabs' AppKit views.
            GeometryReader { geometry in
                let width = geometry.size.width / CGFloat(max(pane.tabs.count, 1))
                let slide = sliding.flatMap { sliding in pane.tabs.firstIndex { $0.id == sliding.id }.map { (from: $0, distance: sliding.distance) } }
                let target = slide.map { slideTarget(from: $0.from, distance: $0.distance, width: width) }
                HStack(spacing: 0) {
                    ForEach(Array(pane.tabs.enumerated()), id: \.element.id) { index, tab in
                        let slides = tab.id == sliding?.id
                        PaneTabItem(window: window, pane: pane, tab: tab, store: tab.store,
                                    selected: tab.id == pane.selectedID, active: active, place: index,
                                    slide: { slideChanged(tab.id, $0, width: width) })
                            .frame(width: width)
                            .overlay(alignment: .leading) { if insertion == index { insertionMark } }
                            .overlay(alignment: .trailing) { if insertion == index + 1 && index == pane.tabs.count - 1 { insertionMark } }
                            .offset(x: slide.flatMap { slide in target.map { slideOffset(index, from: slide.from, distance: slide.distance, to: $0, width: width) } } ?? 0)
                            // The sliding tab follows the pointer; the others move aside.
                            .animation(slides ? nil : motion, value: target)
                            .zIndex(slides ? 1 : 0)
                            .onDrop(of: ConversationLink.dropTypes, delegate: TabDropDelegate(window: window, pane: pane.id, index: index, width: width, insertion: $insertion))
                    }
                }
                .animation(motion, value: pane.tabs.map(\.id))
            }
            NewTabButton { window.focus(pane.id); window.openInNewTab() }
                .frame(width: Self.height, height: Self.height)
                .onDrop(of: ConversationLink.dropTypes, delegate: TabDropDelegate(window: window, pane: pane.id, index: pane.tabs.count, width: 0, insertion: $insertion))
        }
        .frame(height: Self.height)
        .background(Color(nsColor: .windowBackgroundColor))
        .overlay(alignment: .bottom) { TabShade.line.frame(height: 1) }
        .padding(.bottom, 1)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(window.focusedPaneID == pane.id ? "聚焦面板的标签页" : "标签页")
    }

    private var insertionMark: some View {
        Rectangle().fill(Color.accentColor).frame(width: 2).allowsHitTesting(false)
    }

    private var motion: Animation? { reduceMotion ? nil : .easeOut(duration: 0.18) }

    /// The place a tab slid `distance` from place `from` would take: the one its middle is over.
    private func slideTarget(from: Int, distance: CGFloat, width: CGFloat) -> Int {
        guard width > 0 else { return from }
        return min(max(Int(((CGFloat(from) * width + distance) / width).rounded()), 0), pane.tabs.count - 1)
    }

    /// The sliding tab stays within the bar; those between its place and
    /// where it would go move one place towards its place.
    private func slideOffset(_ index: Int, from: Int, distance: CGFloat, to target: Int, width: CGFloat) -> CGFloat {
        if index == from { return min(max(distance, -CGFloat(from) * width), CGFloat(pane.tabs.count - 1 - from) * width) }
        if from < target, index > from, index <= target { return -width }
        if target < from, index >= target, index < from { return width }
        return 0
    }

    private func slideChanged(_ id: UUID, _ change: TabSlide, width: CGFloat) {
        switch change {
        case .moved(let distance): sliding = (id, distance)
        case .left: sliding = nil
        case .dropped:
            guard let slide = sliding, slide.id == id, let from = pane.tabs.firstIndex(where: { $0.id == id }) else { sliding = nil; return }
            let target = slideTarget(from: from, distance: slide.distance, width: width)
            withAnimation(motion) {
                sliding = nil
                if target != from { window.moveTab(id, toPlace: target) }
            }
        }
    }
}

/// The new-tab button: the whole square at the bar's end, shaded as the
/// tabs behind it are, darker under the pointer and while pressed.
private struct NewTabButton: View {
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: "plus").font(.system(size: 11, weight: .medium))
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .contentShape(Rectangle())
        }
        .buttonStyle(Style(hovering: hovering))
        .onHover { hovering = $0 }
        .help(KeyboardShortcutSettings.shared.help("新建标签页", .newTab)).accessibilityLabel("新建标签页")
    }

    private struct Style: ButtonStyle {
        let hovering: Bool
        func makeBody(configuration: Configuration) -> some View {
            configuration.label
                .foregroundStyle(.secondary)
                .background {
                    TabShade.unselected.overlay { Color.black.opacity(configuration.isPressed ? 0.08 : hovering ? 0.04 : 0) }
                }
        }
    }
}

/// A tab dragged along its bar.
enum TabSlide {
    /// The pointer is this far along the bar from where it was pressed.
    case moved(CGFloat)
    /// Let go on the bar: the tab takes the place it is over.
    case dropped
    /// Dragged off the bar, where a drag carries it on.
    case left
}

/// The system tab bar's shades over the window's colour: tabs not shown are
/// darker with a soft shadow under the bar's top edge, and lines part them.
private enum TabShade {
    static let line = Color.black.opacity(0.16)

    static var unselected: some View {
        Color.black.opacity(0.065)
            .overlay(alignment: .top) {
                LinearGradient(colors: [Color.black.opacity(0.12), .clear], startPoint: .top, endPoint: .bottom).frame(height: 3)
            }
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
    /// In the focused pane, or the window's only one.
    let active: Bool
    /// Its place among the pane's tabs.
    let place: Int
    let slide: (TabSlide) -> Void
    @State private var hovering = false
    @State private var closePointer: TabHandle.ClosePointer = .outside
    @State private var renaming: String?
    @FocusState private var renameFocused: Bool

    var body: some View {
        ZStack {
            TabHandle(window: window, tab: tab, title: tab.title, place: place, count: pane.tabs.count, actions: TabHandle.Actions(
                select: { window.selectTab(tab.id) },
                rename: { if canRename { renaming = tab.title } },
                close: { window.closeTab(tab.id) },
                hover: { hovering = $0 },
                closePointer: { closePointer = $0 },
                slide: slide,
                menu: menu))
            if let name = renaming {
                TextField("会话名称", text: Binding(get: { name }, set: { renaming = $0 }))
                    .textFieldStyle(.plain).multilineTextAlignment(.center).font(.system(size: 11))
                    .focused($renameFocused)
                    .onSubmit(commitRename)
                    .onExitCommand { renaming = nil }
                    .onAppear { renameFocused = true }
                    .onChange(of: renameFocused) { _, focused in if !focused { commitRename() } }
                    .padding(.horizontal, 26)
            } else {
                Text(tab.title)
                    .font(.system(size: 11)).foregroundStyle(selected && active ? .primary : .secondary)
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
        .background {
            // Opaque, so a tab slid over another hides it.
            if selected { Color(nsColor: .windowBackgroundColor) }
            else { Color(nsColor: .windowBackgroundColor).overlay { TabShade.unselected.overlay { if hovering { Color.black.opacity(0.04) } } } }
        }
        .overlay(alignment: .trailing) { TabShade.line.frame(width: 1) }
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
                .frame(width: TabHandle.closeSize, height: TabHandle.closeSize)
                .background {
                    // As the system's tab close button: a rounded square under the pointer, darker pressed.
                    RoundedRectangle(cornerRadius: 3, style: .continuous)
                        .fill(Color.primary.opacity(closePointer == .pressed ? 0.18 : closePointer == .over ? 0.09 : 0))
                }
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

    /// Splitting moves the tab beside its pane; a pane's only tab stays, and
    /// a new conversation opens beside it.
    private func menu() -> NSMenu {
        let menu = NSMenu()
        if canRename { menu.addItem(ClosureMenuItem("重命名") { renaming = tab.title }) }
        menu.addItem(ClosureMenuItem("关闭标签页") { window.closeTab(tab.id) })
        if pane.tabs.count > 1 { menu.addItem(ClosureMenuItem("关闭其他标签页") { window.closeOtherTabs(tab.id) }) }
        menu.addItem(.separator())
        if pane.tabs.count > 1 {
            menu.addItem(ClosureMenuItem("移到右侧拆分") { window.splitTab(tab.id, edge: .trailing) })
            menu.addItem(ClosureMenuItem("移到下方拆分") { window.splitTab(tab.id, edge: .bottom) })
        } else {
            menu.addItem(ClosureMenuItem("向右拆分") { window.focus(pane.id); window.split(.horizontal) })
            menu.addItem(ClosureMenuItem("向下拆分") { window.focus(pane.id); window.split(.vertical) })
        }
        if window.canMoveTabToNewWindow { menu.addItem(ClosureMenuItem("移到新窗口") { window.moveToNewWindow(tab.id) }) }
        menu.addItem(.separator())
        menu.addItem(window.layoutMenuItem())
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
        let closePointer: (ClosePointer) -> Void
        let slide: (TabSlide) -> Void
        let menu: () -> NSMenu
    }
    enum ClosePointer { case outside, over, pressed }
    /// How far off the bar a sliding tab leaves it.
    static let leaveDistance: CGFloat = 12
    /// Where the close button shows, from the leading edge.
    static let closeInset: CGFloat = 6
    static let closeSize: CGFloat = 16
    let window: ConversationWindowModel
    let tab: ConversationTab
    let title: String
    /// Its place among `count` tabs sharing the bar equally.
    let place: Int
    let count: Int
    let actions: Actions

    func makeNSView(context: Context) -> HandleView { HandleView() }

    func updateNSView(_ view: HandleView, context: Context) {
        view.model = window
        view.tab = tab
        view.actions = actions
        view.place = (place, count)
        view.toolTip = title
    }

    final class HandleView: NSView, NSDraggingSource {
        weak var model: ConversationWindowModel?
        weak var tab: ConversationTab?
        var actions: Actions?
        var place = (index: 0, count: 1)
        private var pressed: NSEvent?
        /// Where in the tab it was pressed, and the tabs' part of the bar
        /// then, in the window.
        private var pressedAt = NSPoint.zero
        private var bar = NSRect.zero
        private var pressedClose = false
        private var hovering = false
        private var sliding = false
        private var closePointer: ClosePointer = .outside {
            didSet { if closePointer != oldValue { actions?.closePointer(closePointer) } }
        }

        override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

        // SwiftUI's hover does not reach through an AppKit view; the handle tracks it.
        override func updateTrackingAreas() {
            super.updateTrackingAreas()
            trackingAreas.forEach(removeTrackingArea)
            addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .mouseMoved, .activeInActiveApp, .inVisibleRect], owner: self))
        }

        override func mouseEntered(with event: NSEvent) { setHovering(true); trackClose(event) }
        override func mouseMoved(with event: NSEvent) { trackClose(event) }
        override func mouseExited(with event: NSEvent) { setHovering(false); closePointer = .outside }

        private func setHovering(_ value: Bool) {
            guard hovering != value else { return }
            hovering = value
            actions?.hover(value)
        }

        private func overClose(_ event: NSEvent) -> Bool {
            hovering && closeRect.contains(convert(event.locationInWindow, from: nil))
        }

        private func trackClose(_ event: NSEvent) {
            guard !sliding else { return }
            closePointer = overClose(event) ? (pressedClose ? .pressed : .over) : .outside
        }

        private var closeRect: NSRect {
            let size = TabHandle.closeSize
            return NSRect(x: TabHandle.closeInset, y: (bounds.height - size) / 2, width: size, height: size).insetBy(dx: -3, dy: -3)
        }

        override func mouseDown(with event: NSEvent) {
            if overClose(event) {
                pressedClose = true
                closePointer = .pressed
                return
            }
            pressed = event
            pressedAt = convert(event.locationInWindow, from: nil)
            let frame = convert(bounds, to: nil)
            bar = NSRect(x: frame.minX - CGFloat(place.index) * frame.width, y: frame.minY, width: CGFloat(place.count) * frame.width, height: frame.height)
            actions?.select()
            if event.clickCount == 2 { actions?.rename() }
        }

        override func mouseUp(with event: NSEvent) {
            pressed = nil
            if sliding {
                sliding = false
                actions?.slide(.dropped)
                return
            }
            guard pressedClose else { return }
            pressedClose = false
            let inside = overClose(event)
            closePointer = inside ? .over : .outside
            if inside { actions?.close() }
        }

        override func otherMouseUp(with event: NSEvent) {
            if event.buttonNumber == 2, bounds.contains(convert(event.locationInWindow, from: nil)) { actions?.close() }
        }

        override func menu(for event: NSEvent) -> NSMenu? { actions?.menu() }

        /// Along the bar the tab slides there; dragged off it, it is carried
        /// on to another place, pane or window, or out to become a window.
        override func mouseDragged(with event: NSEvent) {
            if pressedClose { trackClose(event); return }
            guard let start = pressed else { return }
            let from = start.locationInWindow, to = event.locationInWindow
            guard sliding || hypot(to.x - from.x, to.y - from.y) > 4 else { return }
            // Along the tabs' part of the bar it slides; off it, it leaves.
            if bar.insetBy(dx: -TabHandle.leaveDistance, dy: -TabHandle.leaveDistance).contains(to) {
                if !sliding { sliding = true; closePointer = .outside }
                actions?.slide(.moved(to.x - from.x))
                return
            }
            if sliding { sliding = false; actions?.slide(.left) }
            carry(at: event)
        }

        private func carry(at event: NSEvent) {
            guard let tab else { return }
            pressed = nil
            let link = TabDragSource.link(for: tab.id, conversation: tab.link)
            TabDragSource.shared.begin(tab: tab.id, link: link)
            let board = NSPasteboardItem()
            board.setString(link.absoluteString, forType: .URL)
            board.setString(link.absoluteString, forType: .string)
            let item = NSDraggingItem(pasteboardWriter: board)
            // Held where it was pressed, wherever the tab has slid to.
            let now = convert(event.locationInWindow, from: nil)
            item.setDraggingFrame(bounds.offsetBy(dx: now.x - pressedAt.x, dy: now.y - pressedAt.y), contents: snapshot())
            let session = beginDraggingSession(with: [item], event: event, source: self)
            // Let go where nothing takes it, it does not slide back: it opens there.
            session.animatesToStartingPositionsOnCancelOrFail = false
        }

        /// Outside the app nothing may take the tab: the desktop or another
        /// app would keep its link as a file or text, and the tab would not
        /// become a window.
        func draggingSession(_ session: NSDraggingSession, sourceOperationMaskFor context: NSDraggingContext) -> NSDragOperation {
            context == .withinApplication ? .move : []
        }

        /// Let go outside every window, the tab becomes a window of its own
        /// there; a window's only tab that cannot leave it takes its window there.
        func draggingSession(_ session: NSDraggingSession, endedAt screenPoint: NSPoint, operation: NSDragOperation) {
            guard operation == [], let model, let tab,
                  !NSApp.windows.contains(where: { $0.isVisible && $0.isOnActiveSpace && $0.styleMask.contains(.titled) && $0.frame.contains(screenPoint) }) else { return }
            if model.canMoveTabToNewWindow { model.moveToNewWindow(tab.id, at: screenPoint) }
            else { model.window?.setFrameTopLeftPoint(screenPoint) }
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

    func validateDrop(info: DropInfo) -> Bool { info.hasItemsConforming(to: ConversationLink.dropTypes) }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        let at = width > 0 && info.location.x > width / 2 ? index + 1 : index
        if insertion != at { insertion = at }
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) { insertion = nil }

    func performDrop(info: DropInfo) -> Bool {
        guard let at = insertion, let provider = info.itemProviders(for: ConversationLink.dropTypes).first else { insertion = nil; return false }
        insertion = nil
        ConversationLink.load(provider) { [window, pane] url in
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

    func validateDrop(info: DropInfo) -> Bool { info.hasItemsConforming(to: ConversationLink.dropTypes) }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        let x = info.location.x / max(size.width, 1), y = info.location.y / max(size.height, 1)
        let nearest = [(PaneEdge.leading, x), (.trailing, 1 - x), (.top, y), (.bottom, 1 - y)].min { $0.1 < $1.1 }!
        let zone: PaneDropZone = nearest.1 < 0.25 ? .edge(nearest.0) : .center
        if self.zone != zone { self.zone = zone }
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) { zone = nil }

    func performDrop(info: DropInfo) -> Bool {
        guard let zone, let provider = info.itemProviders(for: ConversationLink.dropTypes).first else { zone = nil; return false }
        self.zone = nil
        ConversationLink.load(provider) { [window, pane] url in
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
