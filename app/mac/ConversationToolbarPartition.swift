import AppKit
import Combine
import SwiftUI

/// SwiftUI supplies the toolbar controls and their settings. AppKit owns their
/// native order. Keep that order aligned with the actual sidebar split when
/// the system restores or rebuilds the toolbar.
struct ConversationToolbarPartition: NSViewRepresentable {
    let hasList: Bool
    private static let itemAttached = Notification.Name("VeluneConversationToolbarItemAttached")

    static func itemAnchor(_ id: String) -> some View {
        PaneAnchor { anchor in
            guard let view = anchor(), let window = view.window else { return }
            view.identifier = NSUserInterfaceItemIdentifier(id)
            NotificationCenter.default.post(name: itemAttached, object: window)
        }
    }

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
        private var attachmentObservation: AnyCancellable?
        private var checkScheduled = false

        init() {
            super.init(frame: .zero)
            identifier = NSUserInterfaceItemIdentifier("conversation-sidebar-column")
        }

        required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            attachmentObservation = window == nil ? nil : NotificationCenter.default.publisher(for: itemAttached)
                .sink { [weak self] note in
                    guard let self, let window = self.window, note.object as? NSWindow === window else { return }
                    self.scheduleCheck()
                }
            windowObservation = window?.observe(\.toolbar) { [weak self] _, _ in
                DispatchQueue.main.async { self?.scheduleCheck() }
            }
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
            // The macOS 26 customizable toolbar owns its saved arrangement.
            let toolbar = hasList && window?.toolbar?.allowsUserCustomization == false ? window?.toolbar : nil
            if toolbar !== observedToolbar {
                observedToolbar = toolbar
                itemObservation = toolbar?.observe(\.itemIdentifiers) { [weak self] _, _ in
                    DispatchQueue.main.async { self?.scheduleCheck() }
                }
            }
            guard let toolbar, !isHiddenOrHasHiddenAncestor else { return }
            var column: NSView = self
            while let parent = column.superview {
                if let split = parent as? NSSplitView, split.isVertical {
                    if split.arrangedSubviews.first === column {
                        Self.alignItems(in: toolbar, sidebarSplit: split)
                    }
                    return
                }
                column = parent
            }
        }

        private static func alignItems(in toolbar: NSToolbar, sidebarSplit: NSSplitView) {
            let items = toolbar.items
            guard let divider = items.firstIndex(where: { item in
                guard let tracking = item as? NSTrackingSeparatorToolbarItem else { return false }
                return tracking.splitView === sidebarSplit && tracking.dividerIndex == 0
            }) else { return }
            let listIDs: Set<String> = ["conversation-list-filter", "conversation-new-conversation"]
            let detailIDs: Set<String> = ["conversation-back", "conversation-forward"]
            let list = items.indices.filter { containsIdentifier(in: items[$0].view, matching: listIDs) }
            let detail = items.firstIndex { containsIdentifier(in: $0.view, matching: detailIDs) }
            guard !list.isEmpty else { return }
            let boundary = min(divider, detail ?? divider)
            guard boundary != divider || list.contains(where: { $0 > divider }) else { return }

            let moved = Set(list + [divider])
            let prefix = items.indices.prefix(boundary).filter { !moved.contains($0) }
            let suffix = items.indices.dropFirst(boundary).filter { !moved.contains($0) }
            // Preserve both groups' current order and any hidden items.
            let ordered = (prefix + list + [divider] + suffix).map { items[$0].itemIdentifier }
            if ordered != toolbar.itemIdentifiers { toolbar.itemIdentifiers = ordered }
        }

        private static func containsIdentifier(in view: NSView?, matching identifiers: Set<String>) -> Bool {
            guard let view else { return false }
            if let identifier = view.identifier, identifiers.contains(identifier.rawValue) { return true }
            return view.subviews.contains { containsIdentifier(in: $0, matching: identifiers) }
        }
    }
}
