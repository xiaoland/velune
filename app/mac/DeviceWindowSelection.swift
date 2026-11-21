import Combine
import Foundation

/// Device selection shared by independent management windows.
@MainActor final class DeviceWindowSelectionModel: ObservableObject {
    @Published var selectedPeerID: String? = nil
    @Published private(set) var peerDevices: [(id: String, name: String)] = []
    private let local: DeviceStore
    private let hub: DeviceHub?
    private var watches: Set<AnyCancellable> = []
    init(local: DeviceStore, hub: DeviceHub?) {
        self.local = local
        self.hub = hub
        hub?.$state.receive(on: RunLoop.main).sink { [weak self] _ in self?.updatePeers() }.store(in: &watches)
        updatePeers()
    }
    var activeStore: DeviceStore {
        guard let pid = selectedPeerID, let hub else { return local }
        return hub.store(for: pid)
    }
    var localDeviceName: String { DeviceHub.deviceName }
    private func updatePeers() {
        let peers = (hub?.state?.contacts ?? [])
            .filter { $0.controlAuthorized }
            .map { (id: $0.peerId, name: hub?.name(of: $0.peerId) ?? $0.peerId) }
        peerDevices = peers
        if let pid = selectedPeerID, !peers.contains(where: { $0.id == pid }) { selectedPeerID = nil }
    }
}
