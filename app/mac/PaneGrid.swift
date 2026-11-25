import AppKit
import SwiftUI

/// The editor area's panes as the window's layout arranges them: nested
/// native split views. Each group's pane is one hosting view kept for as
/// long as the group exists, so splitting, merging or moving a tab moves the
/// panes that stay instead of building them again: their transcripts keep
/// their scroll positions and nothing in them starts over.
struct PaneGrid: NSViewRepresentable {
    @ObservedObject var window: ConversationWindowModel
    let previewEmpty: Bool

    func makeNSView(context: Context) -> PaneGridView { PaneGridView(window: window, previewEmpty: previewEmpty) }

    func updateNSView(_ view: PaneGridView, context: Context) {
        view.apply(window.layout, groups: Set(window.panes.keys))
    }
}

/// See `PaneGrid`.
@MainActor final class PaneGridView: NSView {
    private weak var model: ConversationWindowModel?
    private let previewEmpty: Bool
    private var panes: [UUID: NSView] = [:]
    private var applied: PaneLayout?
    private var root: NSView?

    init(window: ConversationWindowModel, previewEmpty: Bool) {
        model = window
        self.previewEmpty = previewEmpty
        super.init(frame: .zero)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var isFlipped: Bool { true }

    /// The view showing group `id`, for checks.
    func paneView(_ id: UUID) -> NSView? { panes[id] }

    func apply(_ layout: PaneLayout, groups: Set<UUID>) {
        guard layout != applied || panes.keys.contains(where: { !groups.contains($0) }) else { return }
        defer { applied = layout }
        if let applied, let root, Self.sameShape(applied, layout) {
            // Only the shares changed: the split views take them as they are.
            update(root, to: layout)
            return
        }
        // The new arrangement goes in beside the old one, so panes moving
        // from one to the other never leave the window on the way.
        let previous = root
        let next = makeNode(for: layout, path: [])
        next.frame = bounds
        next.autoresizingMask = [.width, .height]
        addSubview(next)
        fill(next, with: layout)
        // A pane that was the whole grid has already moved into the new arrangement.
        if let previous, previous !== next, previous.superview === self { previous.removeFromSuperview() }
        root = next
        for id in panes.keys where !layout.paneIDs.contains(id) {
            panes.removeValue(forKey: id)?.removeFromSuperview()
        }
    }

    /// The same splits, axes and panes in the same places; shares may differ.
    private static func sameShape(_ a: PaneLayout, _ b: PaneLayout) -> Bool {
        switch (a, b) {
        case (.pane(let x), .pane(let y)): x == y
        case (.split(let x), .split(let y)):
            x.axis == y.axis && x.children.count == y.children.count && zip(x.children, y.children).allSatisfy(sameShape)
        default: false
        }
    }

    private func update(_ view: NSView, to layout: PaneLayout) {
        guard case .split(let split) = layout, let splitView = view as? PaneSplitView else { return }
        if splitView.fractions != split.fractions {
            splitView.fractions = split.fractions
            splitView.applyShares()
        }
        for (child, node) in zip(splitView.subviews, split.children) { update(child, to: node) }
    }

    /// An empty split view for a split, or the pane's own view.
    private func makeNode(for layout: PaneLayout, path: [Int]) -> NSView {
        switch layout {
        case .pane(let id): return pane(id)
        case .split(let split):
            let view = PaneSplitView(axis: split.axis, fractions: split.fractions)
            view.path = path
            view.resized = { [weak self] path, fractions in self?.model?.resize(at: path, fractions: fractions) }
            return view
        }
    }

    private func fill(_ view: NSView, with layout: PaneLayout) {
        guard case .split(let split) = layout, let splitView = view as? PaneSplitView else { return }
        for (index, child) in split.children.enumerated() {
            let node = makeNode(for: child, path: splitView.path + [index])
            splitView.addSubview(node)
            fill(node, with: child)
        }
        splitView.applyShares()
    }

    private func pane(_ id: UUID) -> NSView {
        if let view = panes[id] { return view }
        guard let model, let pane = model.panes[id] else { return NSView() }
        // A hosting view of its own takes nothing from the window's root view's environment.
        let view = NSHostingView(rootView: PaneView(window: model, pane: pane, previewEmpty: previewEmpty).applicationAppearance())
        view.sizingOptions = []
        panes[id] = view
        return view
    }
}

/// One split of the editor area, or of a dock area's tools. Its views keep
/// their shares of its length as it resizes; a divider dragged writes the
/// new shares once it is let go, and a double click on one shares the room
/// equally again.
@MainActor final class PaneSplitView: NSSplitView {
    var fractions: [Double]
    var path: [Int] = []
    var resized: (([Int], [Double]) -> Void)?
    /// The least a view keeps while a divider is dragged, room permitting.
    private let minimumLength: CGFloat
    /// The divider takes the pointer this far either side of its line.
    static let reach: CGFloat = 4
    private lazy var rules = SplitViewRules(
        minimum: { [unowned self] in minimumCoordinate(ofSubviewAt: $0) },
        maximum: { [unowned self] in maximumCoordinate(ofSubviewAt: $0) },
        effectiveRect: { [unowned self] rect in isVertical ? rect.insetBy(dx: -Self.reach, dy: 0) : rect.insetBy(dx: 0, dy: -Self.reach) },
        adjusts: { _ in true })

    init(axis: PaneSplit.Axis, fractions: [Double], minimumLength: CGFloat = 320) {
        self.fractions = fractions
        self.minimumLength = minimumLength
        super.init(frame: .zero)
        isVertical = axis == .horizontal
        dividerStyle = .thin
        delegate = rules
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private var length: CGFloat { isVertical ? bounds.width : bounds.height }
    private var available: CGFloat { max(0, length - dividerThickness * CGFloat(max(0, subviews.count - 1))) }

    /// Lays the panes out by `fractions` now (`adjustSubviews` would keep their current proportions).
    func applyShares() { resizeSubviews(withOldSize: bounds.size) }

    /// Each pane its share, whatever the old size: the shares are the layout's.
    override func resizeSubviews(withOldSize oldSize: NSSize) {
        let count = subviews.count
        guard count > 0 else { return }
        let shares = fractions.count == count ? fractions : Array(repeating: 1 / Double(count), count: count)
        var offset: CGFloat = 0
        for (index, view) in subviews.enumerated() {
            let size = index == count - 1 ? max(0, length - offset) : (available * CGFloat(shares[index])).rounded()
            view.frame = isVertical ? NSRect(x: offset, y: 0, width: size, height: bounds.height)
                : NSRect(x: 0, y: offset, width: bounds.width, height: size)
            offset += size + dividerThickness
        }
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        let divider = dividerIndex(at: point)
        if event.clickCount == 2, divider != nil {
            resized?(path, Array(repeating: 1, count: subviews.count))
            return
        }
        // The divider's drag is the split view's own, tracked here until it is let go.
        super.mouseDown(with: event)
        guard divider != nil, available > 0 else { return }
        let lengths = subviews.map { isVertical ? $0.frame.width : $0.frame.height }
        let total = lengths.reduce(0, +)
        guard total > 0 else { return }
        fractions = lengths.map { Double($0 / total) }
        resized?(path, fractions)
    }

    private func dividerIndex(at point: NSPoint) -> Int? {
        for index in 0..<max(0, subviews.count - 1) {
            let frame = subviews[index].frame
            let edge = isVertical ? frame.maxX : frame.maxY
            let position = isVertical ? point.x : point.y
            if position >= edge - Self.reach, position <= edge + dividerThickness + Self.reach { return index }
        }
        return nil
    }

    // MARK: Divider limits

    private func minimumCoordinate(ofSubviewAt dividerIndex: Int) -> CGFloat {
        let frame = subviews[dividerIndex].frame
        return (isVertical ? frame.minX : frame.minY) + min(minimumLength, pairLength(dividerIndex) / 2)
    }

    private func maximumCoordinate(ofSubviewAt dividerIndex: Int) -> CGFloat {
        let frame = subviews[dividerIndex + 1].frame
        return (isVertical ? frame.maxX : frame.maxY) - dividerThickness - min(minimumLength, pairLength(dividerIndex) / 2)
    }

    private func pairLength(_ dividerIndex: Int) -> CGFloat {
        let a = subviews[dividerIndex].frame, b = subviews[dividerIndex + 1].frame
        return isVertical ? a.width + b.width : a.height + b.height
    }
}

/// A split view's delegate, kept apart from the split view. AppKit's split
/// view passes responder-chain questions (`respondsToSelector:`, asked of
/// every responder as a command or a menu item is validated) on to its
/// delegate, so a split view that is its own delegate asks itself again
/// without end and the application crashes.
@MainActor final class SplitViewRules: NSObject, NSSplitViewDelegate {
    private let minimum: (Int) -> CGFloat
    private let maximum: (Int) -> CGFloat
    private let effectiveRect: (NSRect) -> NSRect
    private let adjusts: (NSView) -> Bool

    init(minimum: @escaping (Int) -> CGFloat, maximum: @escaping (Int) -> CGFloat,
         effectiveRect: @escaping (NSRect) -> NSRect, adjusts: @escaping (NSView) -> Bool) {
        self.minimum = minimum
        self.maximum = maximum
        self.effectiveRect = effectiveRect
        self.adjusts = adjusts
    }

    func splitView(_ splitView: NSSplitView, constrainMinCoordinate proposedMinimumPosition: CGFloat, ofSubviewAt dividerIndex: Int) -> CGFloat {
        minimum(dividerIndex)
    }

    func splitView(_ splitView: NSSplitView, constrainMaxCoordinate proposedMaximumPosition: CGFloat, ofSubviewAt dividerIndex: Int) -> CGFloat {
        maximum(dividerIndex)
    }

    func splitView(_ splitView: NSSplitView, effectiveRect proposedEffectiveRect: NSRect, forDrawnRect drawnRect: NSRect, ofDividerAt dividerIndex: Int) -> NSRect {
        effectiveRect(drawnRect)
    }

    func splitView(_ splitView: NSSplitView, shouldAdjustSizeOfSubview view: NSView) -> Bool { adjusts(view) }
}
