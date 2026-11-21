import AppKit
import SwiftUI

/// SwiftUI supplies the toolbar controls and their settings. AppKit owns their
/// native order. Keep that order aligned with the actual sidebar split when
/// the system restores or rebuilds the toolbar.
struct ConversationToolbarPartition: NSViewRepresentable {
    let hasList: Bool

    func makeNSView(context: Context) -> Anchor { Anchor() }

    func updateNSView(_ view: Anchor, context: Context) {
        guard view.hasList != hasList else { return }
        view.hasList = hasList
        view.scheduleCheck()
    }

    final class Anchor: NSView {
        var hasList = false
        private weak var observedToolbar: NSToolbar?
        private var windowObservation: NSKeyValueObservation?
        private var itemObservation: NSKeyValueObservation?
        private var customizationObservation: NSKeyValueObservation?
        private var checkScheduled = false

        init() {
            super.init(frame: .zero)
            identifier = NSUserInterfaceItemIdentifier("conversation-sidebar-column")
        }

        required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            windowObservation = window?.observe(\.toolbar) { [weak self] _, _ in
                DispatchQueue.main.async { self?.scheduleCheck() }
            }
            scheduleCheck()
        }

        override func layout() {
            super.layout()
            // The split controller may attach its divider after the first
            // toolbar update, particularly while restoring a window.
            scheduleCheck()
        }

        override func viewDidMoveToSuperview() {
            super.viewDidMoveToSuperview()
            scheduleCheck()
        }

        func scheduleCheck() {
            guard !checkScheduled else { return }
            checkScheduled = true
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.checkScheduled = false
                self.checkPartition()
            }
        }

        private func checkPartition() {
            let toolbar = hasList ? window?.toolbar : nil
            if toolbar !== observedToolbar {
                observedToolbar = toolbar
                itemObservation = toolbar?.observe(\.itemIdentifiers) { [weak self] _, _ in
                    DispatchQueue.main.async { self?.scheduleCheck() }
                }
                customizationObservation = toolbar?.observe(\.customizationPaletteIsRunning) { [weak self] _, _ in
                    DispatchQueue.main.async { self?.scheduleCheck() }
                }
            }
            guard let toolbar, !toolbar.customizationPaletteIsRunning, !isHiddenOrHasHiddenAncestor else { return }
            let items = toolbar.items
            var column: NSView = self
            while let parent = column.superview {
                if let split = parent as? NSSplitView, split.isVertical, split.arrangedSubviews.first === column,
                   let divider = items.firstIndex(where: { item in
                       guard let tracking = item as? NSTrackingSeparatorToolbarItem else { return false }
                       return tracking.splitView === split && tracking.dividerIndex == 0
                   }) {
                    Self.alignItems(in: toolbar, divider: divider)
                    return
                }
                column = parent
            }
        }

        private static func alignItems(in toolbar: NSToolbar, divider: Int) {
            let items = toolbar.itemIdentifiers
            let list = items.indices.filter { ConversationToolbar.Item.sidebar.contains(items[$0]) }
            let detail = items.firstIndex { ConversationToolbar.Item.detailNavigation.contains($0) }
            let boundary = min(divider, detail ?? divider)
            guard boundary != divider || list.contains(where: { $0 > divider }) else { return }

            let moved = Set(list + [divider])
            let prefix = items.indices.prefix(boundary).filter { !moved.contains($0) }
            let suffix = items.indices.dropFirst(boundary).filter { !moved.contains($0) }
            // Move only existing items, preserving the user's visibility and
            // relative ordering within each side of the split.
            toolbar.itemIdentifiers = (prefix + list + [divider] + suffix).map { items[$0] }
        }
    }
}
