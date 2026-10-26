import AppKit
import Carbon.HIToolbox
import Combine

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
        guard let window = event.window ?? NSApp.keyWindow, window.sheetParent == nil, NSApp.modalWindow == nil else { return false }
        // The conversation list's rows: ⌘⌫ deletes, typing searches.
        if let model = ConversationWindowModel.owning(window: window) ?? ConversationWindowModel.owning(detached: window),
           model.listHasKeyboard, let handle = model.listKeyHandler, handle(event) { return true }
        guard event.keyCode == 53, event.modifierFlags.intersection(.deviceIndependentFlagsMask).isDisjoint(with: [.command, .control, .option, .shift]) else { return false }
        // Composition and single-line fields (search, find, renaming) end with Escape themselves.
        if let text = window.firstResponder as? NSTextView, text.hasMarkedText() || text.isFieldEditor {
            text.keyDown(with: event)
            return true
        }
        for handler in escapeHandlers.reversed() where handler.handle() { return true }
        return false
    }
}

/// The global shortcuts ("显示 Velune", "新建会话（全局）"), registered
/// with the system while the user has set them. A key the system or another
/// app holds does not register; the settings say so.
@MainActor final class GlobalHotKeys: ObservableObject {
    static let shared = GlobalHotKeys()
    static let commands: [ShortcutCommand] = [.globalShow, .globalNewConversation]
    /// Set, but taken by the system or another app.
    @Published private(set) var failed: Set<ShortcutCommand> = []
    private var registered: [EventHotKeyRef] = []
    private var handler: EventHandlerRef?
    private var subscription: AnyCancellable?

    func start() {
        guard subscription == nil else { return }
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        InstallEventHandler(GetApplicationEventTarget(), { _, event, _ in
            var id = EventHotKeyID()
            GetEventParameter(event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID), nil, MemoryLayout<EventHotKeyID>.size, nil, &id)
            let index = Int(id.id)
            DispatchQueue.main.async { if GlobalHotKeys.commands.indices.contains(index) { GlobalHotKeys.perform(GlobalHotKeys.commands[index]) } }
            return noErr
        }, 1, &spec, nil, &handler)
        subscription = KeyboardShortcutSettings.shared.$changed.sink { [weak self] _ in
            DispatchQueue.main.async { self?.sync() }
        }
    }

    private func sync() {
        for ref in registered { UnregisterEventHotKey(ref) }
        registered = []
        var failures: Set<ShortcutCommand> = []
        for (index, command) in Self.commands.enumerated() {
            guard let shortcut = KeyboardShortcutSettings.shared.shortcut(command) else { continue }
            var ref: EventHotKeyRef?
            let id = EventHotKeyID(signature: OSType(0x564C_4E45), id: UInt32(index))
            if let code = shortcut.virtualKeyCode,
               RegisterEventHotKey(UInt32(code), shortcut.carbonModifiers, id, GetApplicationEventTarget(), 0, &ref) == noErr, let ref {
                registered.append(ref)
            } else {
                failures.insert(command)
            }
        }
        if failed != failures { failed = failures }
    }

    /// Velune in front, in its frontmost conversation window; a new
    /// conversation there takes the keyboard.
    private static func perform(_ command: ShortcutCommand) {
        NSApp.activate()
        let window = NSApp.orderedWindows.first(where: ConversationWindowModel.isConversationWindow)
        window?.makeKeyAndOrderFront(nil)
        guard command == .globalNewConversation, let model = ConversationWindowModel.owning(window: window) ?? ConversationWindowModel.openWindows.first else { return }
        model.perform(.newConversation)
        model.focused.composer.focusWhenShown()
    }
}

extension Shortcut {
    /// The key's code on the current keyboard layout, for registering with the system.
    var virtualKeyCode: UInt16? {
        if let code = Self.code(named: key) { return code }
        guard let source = TISCopyCurrentKeyboardLayoutInputSource()?.takeRetainedValue(),
              let pointer = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData) else { return nil }
        let data = Unmanaged<CFData>.fromOpaque(pointer).takeUnretainedValue() as Data
        return data.withUnsafeBytes { bytes -> UInt16? in
            guard let layout = bytes.bindMemory(to: UCKeyboardLayout.self).baseAddress else { return nil }
            for code in UInt16(0)...127 {
                var dead: UInt32 = 0
                var length = 0
                var characters = [UniChar](repeating: 0, count: 4)
                let status = UCKeyTranslate(layout, code, UInt16(kUCKeyActionDisplay), 0, UInt32(LMGetKbdType()), OptionBits(kUCKeyTranslateNoDeadKeysBit), &dead, characters.count, &length, &characters)
                if status == noErr, length > 0, String(utf16CodeUnits: characters, count: length).lowercased() == key { return code }
            }
            return nil
        }
    }

    var carbonModifiers: UInt32 {
        var value: UInt32 = 0
        if modifiers.contains(.command) { value |= UInt32(cmdKey) }
        if modifiers.contains(.shift) { value |= UInt32(shiftKey) }
        if modifiers.contains(.option) { value |= UInt32(optionKey) }
        if modifiers.contains(.control) { value |= UInt32(controlKey) }
        return value
    }
}
