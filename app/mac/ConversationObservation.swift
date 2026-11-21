import AppKit
import Combine
import VeluneBindings

/// One complete visible set per device transport. Window selection and native
/// visibility use the same predicate as ordinary transcript polling.
@MainActor final class ConversationObservation {
    private weak var device: DeviceStore?
    private var subscriptions: Set<AnyCancellable> = []
    init(device: DeviceStore) {
        self.device = device
        let center = NotificationCenter.default
        for name in [NSWindow.didChangeOcclusionStateNotification, NSWindow.didMiniaturizeNotification,
                     NSWindow.didDeminiaturizeNotification, NSWindow.willCloseNotification,
                     NSApplication.didHideNotification, NSApplication.didUnhideNotification] {
            center.publisher(for: name).receive(on: DispatchQueue.main).sink { [weak self] _ in
                // willClose also lets the window model commit its closed state.
                DispatchQueue.main.async { self?.refresh() }
            }.store(in: &subscriptions)
        }
        NSWorkspace.shared.notificationCenter.publisher(for: NSWorkspace.activeSpaceDidChangeNotification)
            .receive(on: DispatchQueue.main).sink { [weak self] _ in self?.refresh() }.store(in: &subscriptions)
    }
    func refresh() {
        guard let device, !device.isShuttingDown, let transport = device.transport else { return }
        transport.observeConversations(Self.visibleConversations(transport: transport))
    }
    static func visibleConversations(transport: Transport) -> [ObservedConversation] {
        var result: [ObservedConversation] = []
        for model in ConversationWindowModel.openWindows {
            for tab in model.conversationTabs {
                let store = tab.store
                guard store.device.transport === transport, !store.isUnsentConversation,
                      ConversationWindowModel.isConversationStoreSelectedAndVisible(store),
                      let runtime = store.projectionRuntimeID, let conversation = store.loadedConversationID else { continue }
                let item = ObservedConversation(runtimeId: runtime, conversationId: conversation)
                if !result.contains(item) { result.append(item) }
            }
        }
        return result.sorted {
            $0.runtimeId == $1.runtimeId ? $0.conversationId < $1.conversationId : $0.runtimeId < $1.runtimeId
        }
    }
}
