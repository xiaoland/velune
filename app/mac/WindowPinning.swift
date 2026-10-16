import AppKit
import SwiftUI

/// Keeping a window in front of other apps' windows ("pin to top"). A
/// conversation window and a tool window keep it with their layout; any
/// other window for as long as it is open. Floating panels float already.
@MainActor final class WindowPinning: ObservableObject {
    static let shared = WindowPinning()
    /// The key window, as the Window menu's item shows it.
    @Published private(set) var keyWindowPinned = false
    @Published private(set) var canPin = false
    private var observers: [NSObjectProtocol] = []

    private init() {
        observers = [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification].map { name in
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { _ in
                MainActor.assumeIsolated { WindowPinning.shared.refresh() }
            }
        }
    }

    /// A titled window of its own: not a panel, not a sheet.
    private static func pinnable(_ window: NSWindow?) -> NSWindow? {
        guard let window, !(window is NSPanel), window.sheetParent == nil, window.styleMask.contains(.titled) else { return nil }
        return window
    }

    func refresh() {
        let window = Self.pinnable(NSApp.keyWindow)
        let pinned = window?.level == .floating
        if canPin != (window != nil) { canPin = window != nil }
        if keyWindowPinned != pinned { keyWindowPinned = pinned }
    }

    func toggle() {
        guard let window = Self.pinnable(NSApp.keyWindow) else { return }
        if let model = ConversationWindowModel.owning(window: window) { model.pinned.toggle() }
        else if let tool = window as? ToolWindow, let model = tool.model { model.tools[tool.kind].pinned.toggle() }
        else { Self.apply(window.level != .floating, to: window) }
    }

    static func apply(_ pinned: Bool, to window: NSWindow) {
        let level: NSWindow.Level = pinned ? .floating : .normal
        if window.level != level { window.level = level }
        shared.refresh()
    }
}

/// Puts the window showing it at the level `pinned` asks for.
struct WindowLevelSetter: NSViewRepresentable {
    let pinned: Bool

    func makeNSView(context: Context) -> LevelView { LevelView() }

    func updateNSView(_ view: LevelView, context: Context) {
        view.pinned = pinned
        view.apply()
    }

    final class LevelView: NSView {
        var pinned = false

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            apply()
        }

        func apply() {
            if let window { WindowPinning.apply(pinned, to: window) }
        }

        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }
}
