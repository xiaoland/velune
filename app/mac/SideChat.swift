import AppKit
import Combine
import SwiftUI
import VeluneBindings

/// A side chat inherits these choices once; it has no persisted next-turn intent.
struct SideChatTarget: Sendable {
    let runtimeID: String
    let conversationID: String
    let title: String
    let nextTurn: NextTurn
    var selection: ModelSelection? { nextTurn.model }
    var effort: String? { nextTurn.effort }
    var permissionMode: String? { nextTurn.permissionMode?.id }
}

enum SideChatPresentation: String, Codable, CaseIterable {
    case split, sheet
    var title: String { self == .split ? "拆分显示" : "在 sheet 中显示" }
}

/// Closes the core's side chat once released or no longer held, however its
/// tab goes away.
final class SideChatLease: @unchecked Sendable {
    private let transport: Transport
    private let lock = NSLock()
    private var id: String?
    private var released = false

    init(transport: Transport) { self.transport = transport }

    func hold(_ id: String) {
        lock.withLock { self.id = id }
        if lock.withLock({ released }) { release() }
    }

    func release() {
        let id: String? = lock.withLock {
            released = true
            defer { self.id = nil }
            return self.id
        }
        guard let id else { return }
        let transport = transport
        DispatchQueue.global(qos: .utility).async { try? transport.closeSideChat(sideChatID: id) }
    }

    deinit { release() }
}

/// Resolves the list target without borrowing a visible editor.
@MainActor final class BackgroundSideChat {
    let tab: ConversationTab
    var watch: AnyCancellable?
    var timeout: Timer?
    init(tab: ConversationTab) { self.tab = tab }
    func close() { watch = nil; timeout?.invalidate(); timeout = nil; tab.close() }
}

extension ConversationWindowModel {
    var sideChatPresentation: SideChatPresentation {
        get { tools.sideChatPresentation ?? .split }
        set {
            guard sideChatPresentation != newValue else { return }
            let side = commandTab.store.isSideChat ? commandTab : conversationTabs.first { $0.store.isSideChat }
            tools.sideChatPresentation = newValue
            if let side { presentSideChat(side, as: newValue) }
            application.objectWillChange.send()
        }
    }

    static func setSideChatPresentation(_ value: SideChatPresentation, application: DeviceStore) {
        var remembered = application.rememberedToolLayout.layout
        remembered.sideChatPresentation = value
        application.rememberedToolLayout.remember(remembered)
        for window in openWindows where window.application === application { window.sideChatPresentation = value }
    }
}
