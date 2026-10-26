import AppKit

/// Key presses Velune takes ahead of the menus and the focused view.
/// Escape does one thing, in this order: it ends an input method's
/// composition, closes the list, palette or field in front, and only then
/// reaches the menu's Stop. Sheets and alerts keep it for their cancel button.
@MainActor enum KeyRouter {
    /// Closers of what is in front, the latest first; each returns whether it took the press.
    private static var escapeHandlers: [(id: UUID, handle: () -> Bool)] = []
    private static var monitor: Any?

    static func install() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            MainActor.assumeIsolated { route(event) } ? nil : event
        }
    }

    /// Adds a closer for Escape; remove it with the token.
    static func addEscapeHandler(_ handle: @escaping () -> Bool) -> UUID {
        let id = UUID()
        escapeHandlers.append((id, handle))
        return id
    }

    static func removeEscapeHandler(_ id: UUID) { escapeHandlers.removeAll { $0.id == id } }

    private static func route(_ event: NSEvent) -> Bool {
        guard event.keyCode == 53, event.modifierFlags.intersection(.deviceIndependentFlagsMask).isDisjoint(with: [.command, .control, .option, .shift]),
              let window = event.window ?? NSApp.keyWindow, window.sheetParent == nil, NSApp.modalWindow == nil else { return false }
        // Composition and single-line fields (search, find, renaming) end with Escape themselves.
        if let text = window.firstResponder as? NSTextView, text.hasMarkedText() || text.isFieldEditor {
            text.keyDown(with: event)
            return true
        }
        for handler in escapeHandlers.reversed() where handler.handle() { return true }
        return false
    }
}
