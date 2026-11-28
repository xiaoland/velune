import AppKit
import Combine
import SwiftUI

/// A pane's tab bar, drawn and driven by AppKit. Tabs are as wide as their
/// titles within limits and, when they do not fit, the bar scrolls
/// sideways (a vertical wheel scrolls it too), as editors' tab bars do;
/// the shown tab is kept in view. Pressing a tab shows it; a double click
/// renames its conversation in place; the middle button, or the close
/// button shown on hover, closes it. Dragged along the bar a tab slides
/// there while the others move aside; dragged off it, the tab is carried
/// to another place, pane or window, or out of every window to become one.
struct PaneTabBar: NSViewRepresentable {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var pane: Pane
    @ObservedObject private var appearance = AppearancePreferences.shared

    init(window: ConversationWindowModel, pane: Pane) {
        self.window = window
        self.pane = pane
    }

    func makeNSView(context: Context) -> TabBarView { TabBarView(window: window, pane: pane) }

    func updateNSView(_ view: TabBarView, context: Context) { view.update() }
}

/// See `PaneTabBar`.
@MainActor final class TabBarView: NSView {
    fileprivate weak var model: ConversationWindowModel?
    fileprivate let pane: Pane
    private let scrollView = TabScrollView()
    fileprivate let strip = TabStripView()
    private let newTab = NewTabButtonView()
    private let leadingFade = TabEdgeFadeView(edge: .leading)
    private let trailingFade = TabEdgeFadeView(edge: .trailing)
    private let insertionMark = NSView()
    private var items: [UUID: TabItemView] = [:]
    private var order: [UUID] = []
    private var shownSelection: UUID?
    private var lastLayoutWidth: CGFloat = 0
    private weak var observedWindow: NSWindow?
    /// The tab sliding along the bar, where it was pressed in the strip, and how far it has gone.
    private var sliding: (item: TabItemView, distance: CGFloat)?
    /// Where a drop over the bar would put the tab: before the tab at this place.
    private var insertion: Int? { didSet { if insertion != oldValue { needsLayout = true } } }
    static let minimumTabWidth: CGFloat = 96
    static let maximumTabWidth: CGFloat = 240
    /// How far off the bar a sliding tab leaves it.
    static let leaveDistance: CGFloat = 12
    private static let fadeWidth: CGFloat = 12

    init(window: ConversationWindowModel, pane: Pane) {
        model = window
        self.pane = pane
        super.init(frame: .zero)
        scrollView.drawsBackground = false
        scrollView.hasHorizontalScroller = false
        scrollView.hasVerticalScroller = false
        scrollView.verticalScrollElasticity = .none
        scrollView.horizontalScrollElasticity = .allowed
        scrollView.contentView.postsBoundsChangedNotifications = true
        scrollView.documentView = strip
        addSubview(scrollView)
        addSubview(leadingFade)
        addSubview(trailingFade)
        NotificationCenter.default.addObserver(self, selector: #selector(clipBoundsChanged),
                                               name: NSView.boundsDidChangeNotification, object: scrollView.contentView)
        newTab.action = { [weak self] in
            guard let self, let model = self.model else { return }
            model.focus(self.pane.id)
            model.openInNewTab()
        }
        addSubview(newTab)
        insertionMark.wantsLayer = true
        insertionMark.layer?.backgroundColor = NSColor.controlAccentColor.cgColor
        insertionMark.isHidden = true
        strip.addSubview(insertionMark)
        registerForDraggedTypes(TabDrop.types)
        setAccessibilityElement(true)
        setAccessibilityRole(.tabGroup)
        updateEdgeFades()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    private var reduceMotion: Bool { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if let observedWindow {
            NotificationCenter.default.removeObserver(self, name: NSWindow.didBecomeKeyNotification, object: observedWindow)
            NotificationCenter.default.removeObserver(self, name: NSWindow.didResignKeyNotification, object: observedWindow)
        }
        observedWindow = window
        if let window {
            NotificationCenter.default.addObserver(self, selector: #selector(windowKeyChanged), name: NSWindow.didBecomeKeyNotification, object: window)
            NotificationCenter.default.addObserver(self, selector: #selector(windowKeyChanged), name: NSWindow.didResignKeyNotification, object: window)
        }
    }

    @objc private func clipBoundsChanged(_ notification: Notification) {
        updateEdgeFades()
    }

    @objc private func windowKeyChanged(_ notification: Notification) {
        for item in items.values { item.refresh() }
    }

    /// Tabs added, removed, reordered, renamed or reselected; focus moved.
    func update() {
        guard let model else { return }
        newTab.refreshAppearance()
        let tabs = pane.tabs
        let ids = tabs.map(\.id)
        for (id, item) in items where !ids.contains(id) {
            item.removeFromSuperview()
            items[id] = nil
        }
        let active = !model.isSplit || model.focusedPaneID == pane.id
        for tab in tabs {
            let item = items[tab.id] ?? {
                let item = TabItemView(tab: tab, bar: self, model: model)
                items[tab.id] = item
                strip.addSubview(item, positioned: .below, relativeTo: insertionMark)
                item.alphaValue = order.isEmpty || reduceMotion ? 1 : 0
                return item
            }()
            item.selected = tab.id == pane.selectedID
            item.active = active
            item.refresh()
        }
        let reordered = ids != order
        order = ids
        setAccessibilityLabel(model.focusedPaneID == pane.id ? "聚焦面板的标签页" : "标签页")
        layoutTabs(animated: reordered && !reduceMotion)
        if shownSelection != pane.selectedID, let item = items[pane.selectedID] {
            shownSelection = pane.selectedID
            // After this layout, so the frame it scrolls to is the tab's own.
            DispatchQueue.main.async { [weak self, weak item] in
                guard let self, let item else { return }
                item.scrollToVisible(item.bounds)
                self.updateEdgeFades()
            }
        }
    }

    override func layout() {
        super.layout()
        let height = bounds.height
        newTab.frame = NSRect(x: bounds.width - height, y: 0, width: height, height: height)
        scrollView.frame = NSRect(x: 0, y: 0, width: max(0, bounds.width - height), height: height)
        let fadeWidth = min(Self.fadeWidth, scrollView.frame.width / 2)
        leadingFade.frame = NSRect(x: scrollView.frame.minX, y: 0, width: fadeWidth, height: height)
        trailingFade.frame = NSRect(x: scrollView.frame.maxX - fadeWidth, y: 0, width: fadeWidth, height: height)
        layoutTabs(animated: false)
        if abs(bounds.width - lastLayoutWidth) > 0.5 {
            lastLayoutWidth = bounds.width
            if sliding == nil { scrollSelectedIntoView() }
        }
    }

    /// The tabs side by side: the sliding one where the pointer has it, the
    /// others moved aside from the place it would take.
    fileprivate func layoutTabs(animated: Bool) {
        let height = scrollView.contentSize.height > 0 ? scrollView.contentSize.height : scrollView.frame.height
        let widths = order.map { items[$0].map { $0.preferredWidth } ?? Self.minimumTabWidth }
        var x: CGFloat = 0
        var frames: [CGRect] = []
        for width in widths {
            frames.append(CGRect(x: x, y: 0, width: width, height: height))
            x += width
        }
        strip.frame = NSRect(x: 0, y: 0, width: max(x, scrollView.contentSize.width), height: height)
        let clip = scrollView.contentView
        let maxX = max(0, strip.frame.width - clip.bounds.width)
        if clip.bounds.origin.x > maxX + 0.5 || clip.bounds.origin.x < 0 {
            clip.scroll(to: NSPoint(x: min(max(clip.bounds.origin.x, 0), maxX), y: 0))
            scrollView.reflectScrolledClipView(clip)
        }
        if let sliding, let from = order.firstIndex(of: sliding.item.tab.id) {
            let target = slideTarget(from: from, distance: sliding.distance, frames: frames)
            for (index, id) in order.enumerated() {
                guard let item = items[id], item !== sliding.item else { continue }
                var frame = frames[index]
                if from < target, index > from, index <= target { frame.origin.x -= frames[from].width }
                if target < from, index >= target, index < from { frame.origin.x += frames[from].width }
                place(item, frame, animated: !reduceMotion)
            }
            var frame = frames[from]
            frame.origin.x = min(max(frame.minX + sliding.distance, 0), max(0, x - frame.width))
            sliding.item.frame = frame
        } else {
            for (index, id) in order.enumerated() {
                guard let item = items[id] else { continue }
                place(item, frames[index], animated: animated)
            }
        }
        if let insertion {
            let edge = insertion < frames.count ? frames[insertion].minX : x
            insertionMark.frame = NSRect(x: max(0, edge - 1), y: 0, width: 2, height: height)
            insertionMark.isHidden = false
        } else {
            insertionMark.isHidden = true
        }
        updateEdgeFades()
    }

    private func updateEdgeFades() {
        let clip = scrollView.contentView.bounds
        let contentWidth = strip.frame.width
        let overflows = contentWidth > clip.width + 0.5
        leadingFade.isHidden = !overflows || clip.minX <= 0.5
        trailingFade.isHidden = !overflows || clip.maxX >= contentWidth - 0.5
    }

    private func scrollSelectedIntoView() {
        guard let item = items[pane.selectedID] else { return }
        item.scrollToVisible(item.bounds)
        updateEdgeFades()
    }

    private func place(_ item: TabItemView, _ frame: CGRect, animated: Bool) {
        let appearing = item.alphaValue == 0
        if appearing { item.frame = frame }
        guard animated || appearing, !reduceMotion else {
            item.frame = frame
            item.alphaValue = 1
            return
        }
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.18
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            item.animator().frame = frame
            item.animator().alphaValue = 1
        }
    }

    /// The place a tab slid `distance` from place `from` would take: the one its middle is over.
    private func slideTarget(from: Int, distance: CGFloat, frames: [CGRect]) -> Int {
        let middle = frames[from].midX + distance
        if let index = frames.firstIndex(where: { middle < $0.maxX }) { return index }
        return frames.count - 1
    }

    // MARK: Sliding, as the tab being dragged reports it

    /// The tabs' part of the bar in the window: a tab dragged within it slides.
    fileprivate var slidingArea: NSRect {
        scrollView.convert(scrollView.bounds, to: nil).insetBy(dx: -Self.leaveDistance, dy: -Self.leaveDistance)
    }

    fileprivate func slide(_ item: TabItemView, by distance: CGFloat) {
        sliding = (item, distance)
        layoutTabs(animated: false)
    }

    fileprivate func endSlide(_ item: TabItemView, dropped: Bool) {
        guard let sliding, sliding.item === item, let from = order.firstIndex(of: item.tab.id) else { self.sliding = nil; return }
        let widths = order.map { items[$0]?.preferredWidth ?? Self.minimumTabWidth }
        var x: CGFloat = 0
        let frames = widths.map { width -> CGRect in defer { x += width }; return CGRect(x: x, y: 0, width: width, height: 1) }
        let target = slideTarget(from: from, distance: sliding.distance, frames: frames)
        self.sliding = nil
        if dropped, target != from, let model {
            model.moveTab(item.tab.id, toPlace: target)
        } else {
            layoutTabs(animated: true)
        }
    }

    // MARK: Drops on the bar

    private func insertionIndex(for info: NSDraggingInfo) -> Int {
        let x = strip.convert(info.draggingLocation, from: nil).x
        for (index, id) in order.enumerated() {
            guard let frame = items[id]?.frame else { continue }
            if x < frame.midX { return index }
        }
        return order.count
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation { draggingUpdated(sender) }

    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard TabDrop.url(from: sender.draggingPasteboard) != nil || TabDrop.hasLink(sender.draggingPasteboard) else { insertion = nil; return [] }
        insertion = insertionIndex(for: sender)
        strip.autoscroll(with: NSApp.currentEvent ?? NSEvent())
        return .move
    }

    override func draggingExited(_ sender: NSDraggingInfo?) { insertion = nil }

    override func draggingEnded(_ sender: NSDraggingInfo) { insertion = nil }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        let at = insertion ?? order.count
        insertion = nil
        guard let model, let url = TabDrop.url(from: sender.draggingPasteboard) else { return false }
        let source = ListDragWatch.shared.dragged
        ListDragWatch.shared.accepted()
        model.drop(url, on: pane.id, zone: .center, at: at, from: source)
        return true
    }
}

/// What a drop on a tab bar reads: a conversation dragged from the list
/// (Velune's own type) or a tab's link.
enum TabDrop {
    static let linkType = NSPasteboard.PasteboardType(ConversationLink.dragType.identifier)
    static var types: [NSPasteboard.PasteboardType] { [linkType, .URL] }

    static func hasLink(_ board: NSPasteboard) -> Bool { board.availableType(from: [linkType]) != nil }

    static func url(from board: NSPasteboard) -> URL? {
        if let data = board.data(forType: linkType), let url = URL(dataRepresentation: data, relativeTo: nil) { return url }
        return (board.readObjects(forClasses: [NSURL.self]) as? [URL])?.first
    }
}

/// Scrolls sideways only; handles both horizontal gestures and vertical wheel without a scrollbar.
final class TabScrollView: NSScrollView {
    override func scrollWheel(with event: NSEvent) {
        guard let documentView else { return }
        let maxX = max(0, documentView.frame.width - contentView.bounds.width)
        guard maxX > 0 else { return }
        let raw = abs(event.scrollingDeltaY) > abs(event.scrollingDeltaX) ? event.scrollingDeltaY : event.scrollingDeltaX
        guard raw != 0 else { return }
        let delta = event.hasPreciseScrollingDeltas ? raw : raw * 16
        scrollHorizontally(by: -delta)
    }

    @discardableResult
    func scrollHorizontally(by deltaX: CGFloat) -> Bool {
        guard let documentView else { return false }
        let clip = contentView
        let maxX = max(0, documentView.frame.width - clip.bounds.width)
        var origin = clip.bounds.origin
        let nextX = min(max(origin.x + deltaX, 0), maxX)
        guard abs(nextX - origin.x) > 0.01 else { return false }
        origin.x = nextX
        clip.scroll(to: origin)
        reflectScrolledClipView(clip)
        return true
    }
}

/// Subtle edge fade cue at the scrolled ends of the tab strip; passes pointer events through.
@MainActor final class TabEdgeFadeView: NSView {
    enum Edge { case leading, trailing }
    private let edge: Edge

    init(edge: Edge) {
        self.edge = edge
        super.init(frame: .zero)
        setAccessibilityElement(false)
        identifier = NSUserInterfaceItemIdentifier(edge == .leading ? "tab-bar-leading-fade" : "tab-bar-trailing-fade")
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func draw(_ dirtyRect: NSRect) {
        let strength: CGFloat = TabPaint.isDark ? 0.32 : 0.14
        let gradient = NSGradient(starting: NSColor.black.withAlphaComponent(strength), ending: .clear)
        gradient?.draw(in: bounds, angle: edge == .leading ? 0 : 180)
    }
}

/// The tabs, side by side, on the shade of the tabs not shown.
final class TabStripView: NSView {
    override var isFlipped: Bool { true }
    override func draw(_ dirtyRect: NSRect) {
        MainActor.assumeIsolated { TabPaint.unselected(in: bounds, hover: 0, flipped: true) }
    }
}

/// The new-tab button: the whole square at the bar's end, shaded as the
/// tabs behind it are, darker under the pointer and while pressed.
@MainActor final class NewTabButtonView: NSView {
    var action: (() -> Void)?
    private var hovering = false { didSet { needsDisplay = true } }
    private var pressed = false { didSet { needsDisplay = true } }
    private let image = NSImageView()
    private var appliedPointSize: CGFloat = 0

    init() {
        super.init(frame: .zero)
        image.image = NSImage(systemSymbolName: "plus", accessibilityDescription: nil)
        image.contentTintColor = .secondaryLabelColor
        refreshAppearance()
        addSubview(image)
        toolTip = KeyboardShortcutSettings.shared.help("新建标签页", .newTab)
        setAccessibilityElement(true)
        setAccessibilityRole(.button)
        setAccessibilityLabel("新建标签页")
    }

    func refreshAppearance() {
        let size = 11 * AppearancePreferences.shared.values.uiScale
        guard abs(size - appliedPointSize) > 0.01 else { return }
        appliedPointSize = size
        image.symbolConfiguration = .init(pointSize: size, weight: .medium)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func layout() {
        super.layout()
        image.frame = bounds
    }

    private var hoverArea: NSTrackingArea?

    // Only its own: the tooltip manager keeps areas of its own on this view.
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let hoverArea { removeTrackingArea(hoverArea) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect], owner: self)
        hoverArea = area
        addTrackingArea(area)
    }

    override func mouseEntered(with event: NSEvent) { hovering = true }
    override func mouseExited(with event: NSEvent) { hovering = false }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func mouseDown(with event: NSEvent) { pressed = true }
    override func mouseUp(with event: NSEvent) {
        pressed = false
        if bounds.contains(convert(event.locationInWindow, from: nil)) { action?() }
    }
    override func accessibilityPerformPress() -> Bool { action?(); return true }

    override func draw(_ dirtyRect: NSRect) {
        TabPaint.unselected(in: bounds, hover: pressed ? 0.08 : hovering ? 0.04 : 0, flipped: isFlipped)
        TabPaint.line.setFill()
        NSRect(x: 0, y: 0, width: 1, height: bounds.height).fill()
    }
}

/// The system tab bar's shades over the window's colour: tabs not shown are
/// darker with a soft shadow under the bar's top edge, and lines part them.
@MainActor enum TabPaint {
    static var line: NSColor { .separatorColor }
    static var isDark: Bool { NSAppearance.currentDrawing().bestMatch(from: [.darkAqua, .aqua]) == .darkAqua }

    static func selected(in rect: NSRect) {
        NSColor.windowBackgroundColor.setFill()
        rect.fill()
    }

    static func unselected(in rect: NSRect, hover: CGFloat, flipped: Bool) {
        NSColor.windowBackgroundColor.setFill()
        rect.fill()
        if isDark {
            NSColor.black.withAlphaComponent(0.24).setFill()
            rect.fill(using: .sourceOver)
            if hover > 0 {
                NSColor.white.withAlphaComponent(hover).setFill()
                rect.fill(using: .sourceOver)
            }
        } else {
            NSColor.black.withAlphaComponent(0.065 + hover).setFill()
            rect.fill(using: .sourceOver)
        }
        let shadow = NSGradient(starting: NSColor.black.withAlphaComponent(isDark ? 0.32 : 0.12), ending: .clear)
        let top = flipped ? NSRect(x: rect.minX, y: rect.minY, width: rect.width, height: 3) : NSRect(x: rect.minX, y: rect.maxY - 3, width: rect.width, height: 3)
        shadow?.draw(in: top, angle: flipped ? 90 : 270)
    }
}

/// One tab in a bar. See `PaneTabBar`.
@MainActor final class TabItemView: NSView, NSDraggingSource, NSTextFieldDelegate {
    let tab: EditorTab
    /// Nil for a tab shown alone, as the artifact sheet's handle: it does not slide.
    private weak var bar: TabBarView?
    private weak var model: ConversationWindowModel?
    var selected = false
    var active = true
    private let label = NSTextField(labelWithString: "")
    private let icon = NSImageView()
    private let spinner = NSProgressIndicator()
    private let dot = NSView()
    private var editor: NSTextField?
    private var hovering = false { didSet { if hovering != oldValue { refresh() } } }
    private enum ClosePointer { case outside, over, pressed }
    private var closePointer: ClosePointer = .outside { didSet { if closePointer != oldValue { needsDisplay = true } } }
    private var pressed: NSEvent?
    private var pressedStripX: CGFloat = 0
    private var pressedClose = false
    private var sliding = false
    private var watching: Set<AnyCancellable> = []
    private var refreshScheduled = false
    static let closeInset: CGFloat = 6
    static let closeSize: CGFloat = 16

    init(tab: EditorTab, bar: TabBarView?, model: ConversationWindowModel?) {
        self.tab = tab
        self.bar = bar
        self.model = model
        super.init(frame: .zero)
        label.alignment = .center
        label.maximumNumberOfLines = 1
        addSubview(label)
        icon.symbolConfiguration = .init(pointSize: 9, weight: .semibold)
        addSubview(icon)
        spinner.style = .spinning
        spinner.controlSize = .mini
        spinner.isDisplayedWhenStopped = false
        addSubview(spinner)
        dot.wantsLayer = true
        dot.layer?.cornerRadius = 3.5
        addSubview(dot)
        setAccessibilityElement(true)
        setAccessibilityRole(.radioButton)
        // Changes to the tab or its conversation are drawn once, after they settle.
        tab.objectWillChange.sink { [weak self] _ in self?.scheduleRefresh() }.store(in: &watching)
        if let conversation = tab as? ConversationTab {
            conversation.store.objectWillChange.sink { [weak self] _ in self?.scheduleRefresh() }.store(in: &watching)
        }
        AppearancePreferences.shared.objectWillChange.sink { [weak self] _ in self?.scheduleRefresh() }.store(in: &watching)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private var store: ConversationStore? { (tab as? ConversationTab)?.store }
    private var conversation: Conversation? {
        guard let store, !store.isUnsentConversation, let id = store.loadedConversationID else { return nil }
        return store.conversation(id: id)
    }
    private var scale: CGFloat { AppearancePreferences.shared.values.uiScale }
    private var font: NSFont { .systemFont(ofSize: 11 * scale) }

    private func scheduleRefresh() {
        guard !refreshScheduled else { return }
        refreshScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            refreshScheduled = false
            let width = preferredWidth
            refresh()
            if preferredWidth != width { bar?.needsLayout = true }
        }
    }

    /// As wide as its title with room for the indicator on either side, within the bar's limits.
    var preferredWidth: CGFloat {
        // Measured once per title and size: the bar lays its tabs out again
        // at every frame of a resize.
        let font = font
        if let measured, measured.title == label.stringValue, measured.size == font.pointSize { return measured.width }
        let width = Self.width(of: label.stringValue, font: font)
        measured = (label.stringValue, font.pointSize, width)
        return width
    }
    private var measured: (title: String, size: CGFloat, width: CGFloat)?

    private static func width(of text: String, font: NSFont) -> CGFloat {
        let title = (text as NSString).size(withAttributes: [.font: font]).width
        // The label's cell pads its text a little on either side.
        return min(max((title + 2 * Self.titleInset + 8).rounded(.up), TabBarView.minimumTabWidth), TabBarView.maximumTabWidth)
    }

    /// The room either side of the title: the indicator's place and a little more.
    static var titleInset: CGFloat { closeInset + closeSize + 4 }

    func refresh() {
        let title = tab.title
        let keepSuffix = store != nil && Conversation.hasNumberedTitleSuffix(title)
        label.lineBreakMode = keepSuffix ? .byTruncatingMiddle : .byTruncatingTail
        label.cell?.truncatesLastVisibleLine = !keepSuffix
        label.stringValue = title
        label.font = font
        label.textColor = selected && (active || tab is ArtifactTab) ? .labelColor : .secondaryLabelColor
        toolTip = title
        setAccessibilityLabel(title)
        setAccessibilityValue(selected)
        icon.isHidden = true
        dot.isHidden = true
        spinner.stopAnimation(nil)
        if editor == nil, hovering {
            show(icon: "xmark", tint: .secondaryLabelColor, label: "关闭标签页")
        } else if tab is ArtifactTab {
            show(icon: "flowchart", tint: .secondaryLabelColor, label: nil)
        } else if let store, store.tabIsWaiting {
            show(icon: "exclamationmark.circle.fill", tint: .systemOrange, label: "等待你的回应")
        } else if let store, store.tabIsRunning {
            spinner.startAnimation(nil)
        } else if let conversation, conversation.unread, !(selected && active && window?.isKeyWindow == true) {
            if conversation.failed { show(icon: "exclamationmark.circle.fill", tint: .systemRed, label: "未读，上一轮失败") }
            else {
                dot.layer?.backgroundColor = NSColor.controlAccentColor.cgColor
                dot.isHidden = false
                dot.toolTip = "未读"
            }
        }
        needsLayout = true
        needsDisplay = true
    }

    private func show(icon name: String, tint: NSColor, label: String?) {
        icon.image = NSImage(systemSymbolName: name, accessibilityDescription: label)
        icon.contentTintColor = tint
        icon.symbolConfiguration = .init(pointSize: name == "xmark" ? 9 : 11, weight: name == "xmark" ? .semibold : .regular)
        icon.isHidden = false
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        refresh()
    }

    override func layout() {
        super.layout()
        let inset = Self.titleInset
        let height = label.intrinsicContentSize.height
        label.frame = NSRect(x: inset, y: (bounds.height - height) / 2, width: max(0, bounds.width - 2 * inset), height: height)
        editor?.frame = label.frame.insetBy(dx: 0, dy: -1)
        icon.frame = closeRect
        spinner.frame = closeRect.insetBy(dx: 2, dy: 2)
        dot.frame = NSRect(x: closeRect.midX - 3.5, y: closeRect.midY - 3.5, width: 7, height: 7)
    }

    private var closeRect: NSRect {
        NSRect(x: Self.closeInset, y: (bounds.height - Self.closeSize) / 2, width: Self.closeSize, height: Self.closeSize)
    }

    override func draw(_ dirtyRect: NSRect) {
        // Opaque, so a tab slid over another hides it.
        if selected { TabPaint.selected(in: bounds) }
        else { TabPaint.unselected(in: bounds, hover: hovering ? 0.04 : 0, flipped: isFlipped) }
        TabPaint.line.setFill()
        NSRect(x: bounds.maxX - 1, y: 0, width: 1, height: bounds.height).fill()
        if hovering, editor == nil, closePointer != .outside {
            // As the system's tab close button: a rounded square under the pointer, darker pressed.
            NSColor.labelColor.withAlphaComponent(closePointer == .pressed ? 0.18 : 0.09).setFill()
            NSBezierPath(roundedRect: closeRect, xRadius: 3, yRadius: 3).fill()
        }
    }

    // MARK: Pointer

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    private var hoverArea: NSTrackingArea?

    // Only its own: the tooltip manager keeps areas of its own on this view.
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let hoverArea { removeTrackingArea(hoverArea) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .mouseMoved, .activeInActiveApp, .inVisibleRect], owner: self)
        hoverArea = area
        addTrackingArea(area)
    }

    override func mouseEntered(with event: NSEvent) { hovering = true; trackClose(event) }
    override func mouseMoved(with event: NSEvent) { trackClose(event) }
    override func mouseExited(with event: NSEvent) { hovering = false; closePointer = .outside }

    private func overClose(_ event: NSEvent) -> Bool {
        hovering && editor == nil && closeRect.insetBy(dx: -3, dy: -3).contains(convert(event.locationInWindow, from: nil))
    }

    private func trackClose(_ event: NSEvent) {
        guard !sliding else { return }
        closePointer = overClose(event) ? (pressedClose ? .pressed : .over) : .outside
    }

    override func mouseDown(with event: NSEvent) {
        // Control-click is a context menu gesture, including on an
        // unselected tab: it neither selects nor starts a drag.
        if event.modifierFlags.contains(.control) {
            if let menu = menu(for: event) { NSMenu.popUpContextMenu(menu, with: event, for: self) }
            return
        }
        if overClose(event) {
            pressedClose = true
            closePointer = .pressed
            return
        }
        pressed = event
        pressedStripX = bar?.strip.convert(event.locationInWindow, from: nil).x ?? 0
        model?.selectTab(tab.id)
        if event.clickCount == 2 { beginRename() }
    }

    override func mouseDragged(with event: NSEvent) {
        if pressedClose { trackClose(event); return }
        guard let start = pressed else { return }
        let from = start.locationInWindow, to = event.locationInWindow
        guard sliding || hypot(to.x - from.x, to.y - from.y) > 4 else { return }
        // Along the tabs' part of the bar it slides; off it, it leaves.
        if let bar, bar.slidingArea.contains(to) {
            if !sliding { sliding = true; closePointer = .outside }
            let currentStripX = bar.strip.convert(to, from: nil).x
            bar.slide(self, by: currentStripX - pressedStripX)
            autoscroll(with: event)
            return
        }
        if sliding { sliding = false; bar?.endSlide(self, dropped: false) }
        carry(at: event)
    }

    override func mouseUp(with event: NSEvent) {
        pressed = nil
        if sliding {
            sliding = false
            bar?.endSlide(self, dropped: true)
            return
        }
        guard pressedClose else { return }
        pressedClose = false
        let inside = overClose(event)
        closePointer = inside ? .over : .outside
        if inside { close() }
    }

    override func otherMouseUp(with event: NSEvent) {
        if event.buttonNumber == 2, bounds.contains(convert(event.locationInWindow, from: nil)) { close() }
    }

    override func accessibilityPerformPress() -> Bool { model?.selectTab(tab.id); return true }

    private func close() {
        guard let model else { return }
        if tab is ArtifactTab, model.artifactSheet?.id == tab.id { model.closeArtifactSheet() } else { model.closeTab(tab.id) }
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        guard let model else { return nil }
        if let store, let conversation {
            return NSHostingMenu(rootView: Group {
                ConversationManagementMenu(window: model, store: store, ids: [conversation.id])
                Divider()
                EditorTabActions(window: model, tabID: tab.id)
            })
        }
        return NSHostingMenu(rootView: EditorTabActions(window: model, tabID: tab.id))
    }

    // MARK: Renaming in place

    private var canRename: Bool {
        guard let store, let conversation else { return false }
        return store.canManageConversations && store.canRenameConversation(conversation)
    }

    private func beginRename() {
        guard canRename, editor == nil else { return }
        let field = NSTextField(string: tab.title)
        field.font = font
        field.alignment = .center
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.placeholderString = "会话名称"
        field.delegate = self
        editor = field
        label.isHidden = true
        addSubview(field)
        needsLayout = true
        refresh()
        window?.makeFirstResponder(field)
        field.currentEditor()?.selectAll(nil)
    }

    private func endRename(commit: Bool) {
        guard let field = editor else { return }
        editor = nil
        let name = field.stringValue
        field.removeFromSuperview()
        label.isHidden = false
        refresh()
        guard commit, let store, let conversation, name.trimmingCharacters(in: .whitespacesAndNewlines) != conversation.title else { return }
        store.renameConversation(conversation, title: name) {}
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        if selector == #selector(NSResponder.cancelOperation(_:)) { endRename(commit: false); return true }
        return false
    }

    func controlTextDidEndEditing(_ notification: Notification) { endRename(commit: true) }

    // MARK: Carried off the bar

    private func carry(at event: NSEvent) {
        guard let start = pressed else { return }
        pressed = nil
        let link = TabDragSource.link(for: tab.id, conversation: tab.link)
        TabDragSource.shared.begin(tab: tab.id, link: link)
        let board = NSPasteboardItem()
        board.setString(link.absoluteString, forType: .URL)
        board.setString(link.absoluteString, forType: .string)
        let item = NSDraggingItem(pasteboardWriter: board)
        // Held where it was pressed.
        let pressedAt = convert(start.locationInWindow, from: nil), now = convert(event.locationInWindow, from: nil)
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
        guard operation == [], let model,
              !NSApp.windows.contains(where: { $0.isVisible && $0.isOnActiveSpace && $0.styleMask.contains(.titled) && $0.frame.contains(screenPoint) }) else { return }
        if model.canMoveTabToNewWindow { model.moveToNewWindow(tab.id, at: screenPoint) }
        else { model.window?.setFrameTopLeftPoint(screenPoint) }
    }

    /// The tab as it looks, to drag around.
    private func snapshot() -> NSImage {
        guard let representation = bitmapImageRepForCachingDisplay(in: bounds) else { return NSImage(size: NSSize(width: 1, height: 1)) }
        cacheDisplay(in: bounds, to: representation)
        let image = NSImage(size: bounds.size)
        image.addRepresentation(representation)
        return image
    }
}

/// A tab shown alone: the artifact sheet's handle, dragged to a bar, a pane or out of the window.
struct ArtifactTabItem: NSViewRepresentable {
    let window: ConversationWindowModel
    let tab: ArtifactTab

    func makeNSView(context: Context) -> TabItemView {
        let item = TabItemView(tab: tab, bar: nil, model: window)
        item.selected = true
        return item
    }

    func updateNSView(_ view: TabItemView, context: Context) { view.refresh() }
}
