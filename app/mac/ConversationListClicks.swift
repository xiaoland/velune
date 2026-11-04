import AppKit
import SwiftUI

/// SwiftUI's selection binding omits a click on an already selected row.
/// Observe the native mouse stream without consuming it or replacing the table's
/// selection, primary action, drag delegate or accessibility behavior. Row-local
/// markers provide identity; no table traversal or title matching is needed.
@MainActor final class ConversationListClicks: ObservableObject {
    private final class Row { weak var view: NSView?; init(_ view: NSView) { self.view = view } }
    private var rows: [String: Row] = [:]
    private weak var anchor: NSView?
    private var monitor: Any?
    var single: ((String, NSEvent.ModifierFlags) -> Void)?
    var cancel: (() -> Void)?

    func register(_ view: NSView, id: String) { rows[id] = Row(view) }
    func unregister(_ view: NSView, id: String) { if rows[id]?.view === view { rows[id] = nil } }
    func attach(_ view: NSView) {
        guard view.window != nil else { detach(view); return }
        anchor = view
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .leftMouseDragged]) { [weak self] event in
            self?.observe(event)
            return event
        }
    }
    func detach(_ view: NSView) {
        guard anchor === view else { return }
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil; anchor = nil; single = nil; cancel = nil
    }
    private func row(at event: NSEvent) -> String? {
        guard event.window === anchor?.window else { return nil }
        return rows.first { _, row in
            guard let view = row.view, view.window === event.window, !view.isHiddenOrHasHiddenAncestor else { return false }
            return view.visibleRect.contains(view.convert(event.locationInWindow, from: nil))
        }?.key
    }
    private func observe(_ event: NSEvent) {
        switch event.type {
        case .leftMouseDown:
            guard let id = row(at: event), event.clickCount == 1 else { cancel?(); return }
            let flags = event.modifierFlags
            // NSTableView consumes mouse-up inside its tracking loop. Observe
            // the press, then read native selection on the next main-loop turn.
            // The opener still waits for release and the system double-click
            // interval; native drag and double-click paths cancel that work.
            DispatchQueue.main.async { [weak self] in self?.single?(id, flags) }
        case .leftMouseDragged: cancel?()
        default: break
        }
    }
}

struct ConversationRowClickMarker: NSViewRepresentable {
    let id: String
    let owner: ConversationListClicks
    final class Marker: NSView {
        weak var owner: ConversationListClicks?
        var id = ""
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }
    func makeNSView(context: Context) -> Marker { Marker() }
    func updateNSView(_ view: Marker, context: Context) {
        if view.id != id { view.owner?.unregister(view, id: view.id) }
        view.owner = owner; view.id = id; owner.register(view, id: id)
    }
    static func dismantleNSView(_ view: Marker, coordinator: ()) { view.owner?.unregister(view, id: view.id) }
}

struct ConversationListClickObserver: NSViewRepresentable {
    let owner: ConversationListClicks
    let single: (String, NSEvent.ModifierFlags) -> Void
    let cancel: () -> Void
    final class Anchor: NSView {
        weak var owner: ConversationListClicks?
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); owner?.attach(self) }
    }
    func makeNSView(context: Context) -> Anchor { let view = Anchor(); view.owner = owner; return view }
    func updateNSView(_ view: Anchor, context: Context) { owner.single = single; owner.cancel = cancel; owner.attach(view) }
    static func dismantleNSView(_ view: Anchor, coordinator: ()) { view.owner?.detach(view) }
}
