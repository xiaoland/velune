import AppKit
import Combine
import SwiftUI

/// Workspace and conversation windows: AppKit windows whose content is the
/// SwiftUI window view. Velune names each window, and the name is also its
/// recovery point in the workspace journal. The system's window restoration
/// only reopens windows by name, kind and device; the journal restores what
/// they show. Whether windows reopen after quitting stays the system's
/// choice (General › Close windows when quitting an application).
@MainActor final class WorkspaceWindows: NSObject, ObservableObject {
    static let shared = WorkspaceWindows()

    /// What every window is made with, set once as the application starts.
    struct Context {
        let store: DeviceStore
        let previewEmpty: Bool
        let previewSettings: Bool
    }
    private var context: Context?
    private(set) var controllers: [WorkspaceWindowController] = []
    /// The window commands act on: the key workspace or conversation window,
    /// or the one whose tool window is key. Nil while another window is key.
    @Published private(set) var commandTarget: ConversationWindowModel?
    private var observers: [NSObjectProtocol] = []
    private var launched = false

    func start(_ context: Context) {
        self.context = context
        context.store.openConversationWindow = { [weak self] seed in self?.open(.workspace, seed: seed) }
        DeviceHub.shared?.openWorkspaceWindow = { [weak self] seed in self?.open(.workspace, seed: seed) }
        let center = NotificationCenter.default
        observers = [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification, NSWindow.willCloseNotification].map { name in
            center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                // After the key window has changed, not while it changes.
                DispatchQueue.main.async { MainActor.assumeIsolated { self?.updateCommandTarget() } }
            }
        }
    }

    private func updateCommandTarget() {
        let key = NSApp.keyWindow
        let target = controllers.first { $0.window === key }?.model ?? ConversationWindowModel.owning(detached: key)
        if commandTarget !== target { commandTarget = target }
    }

    /// A new window of `kind` opening `seed`, named `id` (the seed's id, else a new one).
    @discardableResult
    func open(_ kind: ConversationWindowKind, seed: WindowSeed? = nil) -> WorkspaceWindowController? {
        guard let controller = make(kind, id: seed?.id.uuidString ?? UUID().uuidString,
                                    device: kind == .single ? seed?.device : nil, seed: seed, restoring: false) else { return nil }
        controller.present(cascadingFrom: NSApp.keyWindow ?? NSApp.mainWindow)
        return controller
    }

    /// A window the system reopens; AppKit gives it back its frame and shows it.
    func restore(_ kind: ConversationWindowKind, id: String, device: String?) -> NSWindow? {
        make(kind, id: id, device: device, seed: nil, restoring: true)?.window
    }

    private func make(_ kind: ConversationWindowKind, id: String, device: String?, seed: WindowSeed?, restoring: Bool) -> WorkspaceWindowController? {
        guard let context else { return nil }
        // A window already showing under that name comes forward instead of a second copy.
        if let existing = controllers.first(where: { $0.id == id }) { return existing }
        let controller = WorkspaceWindowController(kind: kind, id: id, device: device, seed: seed, restoring: restoring, context: context)
        controllers.append(controller)
        return controller
    }

    func closed(_ controller: WorkspaceWindowController) {
        controllers.removeAll { $0 === controller }
        if commandTarget === controller.model { commandTarget = nil }
    }

    /// After the system has reopened what it keeps: with no workspace window,
    /// one opens on the latest workspace. Once, the first time windows are
    /// AppKit's: SwiftUI's restoration state names scenes that no longer
    /// exist, so when the system keeps windows the previous session's are
    /// reopened from the journal instead.
    func finishLaunching() {
        guard !launched, let context else { return }
        launched = true
        guard !controllers.contains(where: { $0.kind == .workspace }) else { return }
        let migrationKey = "velune.appKitWorkspaceWindows"
        let defaults = UserDefaults.standard
        if !defaults.bool(forKey: migrationKey) {
            defaults.set(true, forKey: migrationKey)
            if defaults.bool(forKey: "NSQuitAlwaysKeepsWindows"), !context.store.isPreview {
                // Each off the one before, so they do not open on top of each other.
                var shown: NSWindow?
                for window in context.store.windowWorkspaceJournal.lastSession() {
                    guard let controller = make(window.kind, id: window.id, device: window.device, seed: nil, restoring: true) else { continue }
                    controller.present(cascadingFrom: shown)
                    shown = controller.window
                }
                if controllers.contains(where: { $0.kind == .workspace }) { return }
            }
        }
        open(.workspace)
    }

    /// The Dock icon clicked with no window showing.
    func reopen() {
        if controllers.isEmpty { open(.workspace) }
        else { controllers.last?.showWindow(nil) }
    }

    /// A link to a conversation, or an `agents:` URI, opens in the window
    /// commands act on, else in the latest workspace window, else a new one.
    func handle(_ url: URL) {
        let window = commandTarget ?? controllers.last(where: { $0.kind == .workspace })?.model
        if !MacExecutionContext.admitted.isDevelopment, url.scheme == "agents" {
            if let window { window.openAgentURI(url.absoluteString) }
            else if let controller = open(.workspace) {
                // Its list starts as the window appears.
                DispatchQueue.main.async { controller.model.openAgentURI(url.absoluteString) }
            }
            return
        }
        guard let link = ConversationLink.conversation(in: url) else { return }
        if let window { window.open(link.conversationID); window.window?.makeKeyAndOrderFront(nil) }
        else { open(.workspace, seed: WindowSeed(conversationID: link.conversationID)) }
    }
}

/// One workspace or conversation window. Its content is SwiftUI; the
/// window, its life and its restoration are AppKit's.
@MainActor final class WorkspaceWindowController: NSWindowController, NSWindowDelegate {
    let id: String
    let kind: ConversationWindowKind
    /// The paired device a conversation window was opened for.
    let device: String?
    let model: ConversationWindowModel
    private var shell: WorkspaceShell!

    init(kind: ConversationWindowKind, id: String, device: String?, seed: WindowSeed?, restoring: Bool, context: WorkspaceWindows.Context) {
        self.id = id
        self.kind = kind
        self.device = device
        // A conversation window belongs to the device it opened for; a
        // workspace window aggregates every device this Mac controls.
        let application = kind == .single ? (device.flatMap { DeviceHub.shared?.store(for: $0) } ?? context.store) : context.store
        model = ConversationWindowModel(application: application, kind: kind, device: device, hub: DeviceHub.shared)
        let size = kind == .workspace ? NSSize(width: 1060, height: 760) : NSSize(width: 720, height: 760)
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: size),
                              styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                              backing: .buffered, defer: true)
        window.title = "Velune"
        window.tabbingMode = .disallowed
        window.collectionBehavior.insert(.fullScreenPrimary)
        window.isReleasedWhenClosed = false
        window.identifier = NSUserInterfaceItemIdentifier("velune.window.\(id)")
        window.isRestorable = true
        window.restorationClass = WorkspaceWindowRestoration.self
        super.init(window: window)
        window.delegate = self
        model.openWindow = { kind, seed in WorkspaceWindows.shared.open(kind, seed: seed) }
        let content = ConversationWindowContent(window: model, seed: seed, persistenceID: id, restoring: restoring,
                                                previewEmpty: context.previewEmpty, previewSettings: context.previewSettings)
        shell = WorkspaceShell(window: window, model: model, content: content, previewEmpty: context.previewEmpty)
        window.setContentSize(size)
        window.contentMinSize = NSSize(width: 520, height: 420)
        window.invalidateRestorableState()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    /// New windows of a kind open with the frame the last one was moved or
    /// resized to, the way a document window remembers its size.
    private var frameName: NSWindow.FrameAutosaveName { kind == .workspace ? "velune.workspace" : "velune.conversation" }

    /// The remembered frame, shown off the window it was opened from, or
    /// centred when nothing is remembered and it is the first.
    func present(cascadingFrom source: NSWindow?) {
        guard let window else { return }
        let remembered = window.setFrameUsingName(frameName) || adoptSceneFrame(window)
        if let source, source !== window, !source.styleMask.contains(.fullScreen) {
            window.setFrameTopLeftPoint(window.cascadeTopLeft(from: NSPoint(x: source.frame.minX, y: source.frame.maxY)))
        } else if !remembered {
            window.center()
        }
        showWindow(nil)
    }

    /// Once, the frame SwiftUI kept for this kind of window before windows were AppKit's.
    private func adoptSceneFrame(_ window: NSWindow) -> Bool {
        let scene = kind == .workspace ? "conversation" : "single-conversation"
        guard let frame = UserDefaults.standard.string(forKey: "NSWindow Frame \(scene)-AppWindow-1") else { return false }
        window.setFrame(from: frame)
        window.saveFrame(usingName: frameName)
        return true
    }

    private func rememberFrame() {
        guard let window, window.isVisible, !window.styleMask.contains(.fullScreen) else { return }
        window.saveFrame(usingName: frameName)
    }

    func windowDidEndLiveResize(_ notification: Notification) { rememberFrame() }
    func windowDidMove(_ notification: Notification) { rememberFrame() }

    func window(_ window: NSWindow, willEncodeRestorableState state: NSCoder) {
        state.encode(id as NSString, forKey: WorkspaceWindowRestoration.idKey)
        state.encode(kind.rawValue as NSString, forKey: WorkspaceWindowRestoration.kindKey)
        if let device { state.encode(device as NSString, forKey: WorkspaceWindowRestoration.deviceKey) }
    }

    func windowWillClose(_ notification: Notification) {
        model.persistWorkspace()
        model.noteWindowClosed()
        model.detached.closeAll()
        WorkspaceWindows.shared.closed(self)
    }
}

/// Reopens a window the system kept, by the name it was saved with.
final class WorkspaceWindowRestoration: NSObject, NSWindowRestoration {
    static let idKey = "veluneWindowID", kindKey = "veluneWindowKind", deviceKey = "veluneWindowDevice"

    static func restoreWindow(withIdentifier identifier: NSUserInterfaceItemIdentifier, state: NSCoder,
                              completionHandler: @escaping (NSWindow?, Error?) -> Void) {
        let id = state.decodeObject(of: NSString.self, forKey: idKey) as String?
        let kind = (state.decodeObject(of: NSString.self, forKey: kindKey) as String?).flatMap(ConversationWindowKind.init(rawValue:))
        let device = state.decodeObject(of: NSString.self, forKey: deviceKey) as String?
        MainActor.assumeIsolated {
            guard let id, UUID(uuidString: id) != nil, let kind,
                  let window = WorkspaceWindows.shared.restore(kind, id: id, device: device) else {
                return completionHandler(nil, CocoaError(.coderReadCorrupt))
            }
            completionHandler(window, nil)
        }
    }
}
