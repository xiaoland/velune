import AppKit
import SwiftUI

/// The editor area with the bottom dock area under it, in one native split.
/// The panes are the same views whether the bottom area is open or not,
/// and the area keeps its own height as the window resizes: the panes take
/// what the window gains or loses. Its divider writes the height once it is
/// let go; a double click puts the default height back.
struct EditorColumn: NSViewRepresentable {
    @ObservedObject var window: ConversationWindowModel
    let previewEmpty: Bool

    func makeNSView(context: Context) -> EditorColumnView { EditorColumnView(window: window, previewEmpty: previewEmpty) }

    func updateNSView(_ view: EditorColumnView, context: Context) {
        view.grid.apply(window.layout, groups: Set(window.panes.keys))
        let bottomTools = window.tools.tools(in: .bottom)
        let bottom = !bottomTools.isEmpty
        let baseMinimum: CGFloat = bottomTools.contains(.composer) ? max(window.composerMinimumHeight, 100) : 100
        let minimum = baseMinimum + (bottomTools.count > 1 ? PaneChrome.barHeight : 0)
        view.apply(bottom: bottom, height: window.tools.bottomHeight, minimum: minimum)
    }
}

/// See `EditorColumn`.
@MainActor final class EditorColumnView: NSView {
    let grid: PaneGridView
    private let split = BottomSplitView()
    private weak var model: ConversationWindowModel?
    private var bottomView: NSView?

    init(window: ConversationWindowModel, previewEmpty: Bool) {
        model = window
        grid = PaneGridView(window: window, previewEmpty: previewEmpty)
        super.init(frame: .zero)
        split.frame = bounds
        split.autoresizingMask = [.width, .height]
        addSubview(split)
        split.addSubview(grid)
        split.resized = { [weak self] height in self?.model?.tools.bottomHeight = height }
        split.reset = { [weak self] in self?.model?.tools.bottomHeight = ToolWindowLayout().bottomHeight }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    /// The bottom area's view while it is open, for checks.
    var bottomArea: NSView? { bottomView?.superview === split ? bottomView : nil }

    func apply(bottom: Bool, height: Double, minimum: CGFloat) {
        split.bottomMinimum = minimum
        if bottom {
            let view = bottomView ?? makeBottom()
            if view.superview !== split { split.addSubview(view) }
        } else if let bottomView, bottomView.superview === split {
            // Kept, so the area's tools come back as they were.
            bottomView.removeFromSuperview()
        }
        if split.bottomHeight != CGFloat(height) || split.needsShares {
            split.bottomHeight = CGFloat(height)
            split.applyHeight()
        }
    }

    private func makeBottom() -> NSView {
        guard let model else { return NSView() }
        let view = NSHostingView(rootView: DockAreaView(window: model, side: .bottom).applicationAppearance())
        view.sizingOptions = []
        bottomView = view
        return view
    }
}

/// The editor area above, the bottom dock area below at a height of its own.
@MainActor final class BottomSplitView: NSSplitView {
    var bottomHeight: CGFloat = 220
    var bottomMinimum: CGFloat = 0
    var resized: ((Double) -> Void)?
    var reset: (() -> Void)?
    /// The editor area keeps at least this much.
    private static let editorMinimum: CGFloat = 200
    private var laidOutCount = 0
    /// Not the split view itself: see `SplitViewRules`.
    private lazy var rules = SplitViewRules(
        minimum: { [unowned self] _ in min(Self.editorMinimum, max(0, bounds.height - dividerThickness - bottomMinimum)) },
        maximum: { [unowned self] _ in max(0, bounds.height - dividerThickness - bottomMinimum) },
        effectiveRect: { $0.insetBy(dx: 0, dy: -PaneSplitView.reach) },
        adjusts: { [unowned self] view in view === subviews.first })

    override init(frame: NSRect) {
        super.init(frame: frame)
        isVertical = false
        dividerStyle = .thin
        delegate = rules
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    /// The area was just opened or closed, so its height is to be laid out again.
    var needsShares: Bool { laidOutCount != subviews.count }

    /// The height the area gets here, within what the window has room for.
    private var effectiveHeight: CGFloat {
        let room = bounds.height - dividerThickness
        return max(0, min(max(bottomHeight, bottomMinimum), max(bottomMinimum, room - Self.editorMinimum)))
    }

    func applyHeight() { resizeSubviews(withOldSize: bounds.size) }

    override func resizeSubviews(withOldSize oldSize: NSSize) {
        laidOutCount = subviews.count
        guard let editor = subviews.first else { return }
        guard subviews.count > 1 else {
            editor.frame = bounds
            return
        }
        // The views and the divider always fill the bounds exactly, however short the column is.
        let bottom = min(effectiveHeight, max(0, bounds.height - dividerThickness))
        let top = max(0, bounds.height - bottom - dividerThickness)
        editor.frame = NSRect(x: 0, y: 0, width: bounds.width, height: top)
        subviews[1].frame = NSRect(x: 0, y: top + dividerThickness, width: bounds.width, height: bottom)
    }

    override func mouseDown(with event: NSEvent) {
        let onDivider = subviews.count > 1 && isOnDivider(convert(event.locationInWindow, from: nil))
        if event.clickCount == 2, onDivider {
            reset?()
            return
        }
        super.mouseDown(with: event)
        guard onDivider, subviews.count > 1 else { return }
        bottomHeight = subviews[1].frame.height
        resized?(Double(bottomHeight))
    }

    private func isOnDivider(_ point: NSPoint) -> Bool {
        let edge = subviews[0].frame.maxY
        return point.y >= edge - PaneSplitView.reach && point.y <= edge + dividerThickness + PaneSplitView.reach
    }
}

/// Several tools of one dock area in a native split: side by side in the
/// bottom area, stacked in a side one. Each tool keeps its view as tools
/// join and leave the area.
struct ToolSplit: NSViewRepresentable {
    @ObservedObject var window: ConversationWindowModel
    let side: DockSide
    let kinds: [ToolWindowKind]

    func makeNSView(context: Context) -> ToolSplitView { ToolSplitView(window: window, side: side) }

    func updateNSView(_ view: ToolSplitView, context: Context) {
        view.apply(kinds, weights: kinds.map { window.tools[$0].weight })
    }
}

/// See `ToolSplit`.
@MainActor final class ToolSplitView: NSView {
    private weak var model: ConversationWindowModel?
    private let side: DockSide
    private var tools: [ToolWindowKind: NSView] = [:]
    private var applied: [ToolWindowKind] = []
    private var split: PaneSplitView?

    init(window: ConversationWindowModel, side: DockSide) {
        model = window
        self.side = side
        super.init(frame: .zero)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var isFlipped: Bool { true }

    /// The split holding the tools, for checks.
    var splitView: NSSplitView? { split }

    func apply(_ kinds: [ToolWindowKind], weights: [Double]) {
        let total = weights.reduce(0, +)
        let fractions = total > 0 ? weights.map { $0 / total } : weights.map { _ in 1 / Double(max(weights.count, 1)) }
        if kinds == applied, let split {
            if split.fractions != fractions {
                split.fractions = fractions
                split.applyShares()
            }
            return
        }
        applied = kinds
        let next = PaneSplitView(axis: side == .bottom ? .horizontal : .vertical, fractions: fractions, minimumLength: 120)
        next.resized = { [weak self] _, shares in
            guard let self, let model = self.model else { return }
            model.tools.setWeights(Dictionary(uniqueKeysWithValues: zip(self.applied, shares)))
        }
        next.frame = bounds
        next.autoresizingMask = [.width, .height]
        addSubview(next)
        for kind in kinds { next.addSubview(tool(kind)) }
        next.applyShares()
        split?.removeFromSuperview()
        split = next
        for kind in tools.keys where !kinds.contains(kind) { tools.removeValue(forKey: kind) }
    }

    private func tool(_ kind: ToolWindowKind) -> NSView {
        if let view = tools[kind] { return view }
        guard let model else { return NSView() }
        let view = NSHostingView(rootView: DockedTool(window: model, kind: kind, side: side).applicationAppearance())
        view.sizingOptions = []
        tools[kind] = view
        return view
    }
}
