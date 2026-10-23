import Foundation
import Network
import UIKit

/// Synchronous UniFFI calls run on this actor, never on the UI executor.
actor RemoteClient {
    private var endpoint: VeluneRemoteEndpoint?
    private var applications: [String: VeluneApplication] = [:]
    private let viewID = "ios-\(UUID().uuidString)"
    private var selected: String?
    private var networkSequence: UInt64 = 0

    func start(name: String) throws {
        guard endpoint == nil else { return }
        var directory = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true).appendingPathComponent("remote")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        // Restoring this directory to another device would clone its trusted identity.
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try directory.setResourceValues(values)
        endpoint = try VeluneRemoteEndpoint.open(options: BindingRemoteOptions(
            identityFile: directory.appendingPathComponent("device-identity").path,
            stateFile: directory.appendingPathComponent("remote-state.json").path,
            deviceName: name, relay: .developmentPublic, discovery: .developmentPublic,
            pathPolicy: .automatic, bindAddresses: [], requestTimeoutMs: 30_000), application: nil)
    }

    func service<T>(_ body: (VeluneRemoteEndpoint) throws -> T) throws -> T {
        guard let endpoint else { throw MobileError.unavailable }
        return try body(endpoint)
    }

    func application<T>(_ peer: String, _ body: (VeluneApplication) throws -> T) throws -> T {
        guard let endpoint else { throw MobileError.unavailable }
        let application: VeluneApplication
        if let cached = applications[peer] { application = cached }
        else {
            application = try endpoint.connect(peerId: peer, viewId: viewID)
            applications[peer] = application
        }
        return try body(application)
    }

    func activity(peer: String?, foreground: Bool) throws {
        guard let endpoint else { return }
        if let previous = selected, previous != peer {
            try endpoint.setControllerActivity(peerId: previous, selected: false, foreground: false)
        }
        selected = peer
        if let peer { try endpoint.setControllerActivity(peerId: peer, selected: true, foreground: foreground) }
    }

    func reportNetwork(_ path: NWPath) throws {
        networkSequence += 1
        let interfaces = path.availableInterfaces
        let networks = interfaces.map { interface in
            BindingPlatformNetwork(id: interface.name, wifi: interface.type == .wifi,
                cellular: interface.type == .cellular, ethernet: interface.type == .wiredEthernet,
                vpn: interface.type == .other, internet: path.status == .satisfied,
                validated: path.status == .satisfied, interfaceName: interface.name, addresses: [], dnsServers: [])
        }
        let snapshot = BindingPlatformNetworkSnapshot(sequence: networkSequence,
            elapsedRealtimeMs: UInt64(ProcessInfo.processInfo.systemUptime * 1000),
            reason: path.status == .satisfied ? .available : .lost,
            defaultNetworkId: path.status == .satisfied ? interfaces.first(where: { path.usesInterfaceType($0.type) })?.name : nil, networks: networks)
        _ = try service { try $0.reportNetworkChange(snapshot: snapshot) }
    }

    func forget(_ peer: String) throws {
        try service { try $0.forgetDevice(peerId: peer) }
        applications[peer] = nil
    }
}

enum MobileError: LocalizedError {
    case unavailable, missingConversation
    var errorDescription: String? {
        switch self {
        case .unavailable: return "远控服务尚未就绪"
        case .missingConversation: return "设备没有返回会话"
        }
    }
}

@MainActor
final class DeviceStore: ObservableObject {
    let client = RemoteClient()
    @Published var state: BindingRemoteServiceSnapshot?
    @Published var failure: String?
    @Published var peer: String?
    private let monitor = NWPathMonitor()
    private var monitoring = false

    func refresh() async {
        do {
            try await client.start(name: UIDevice.current.name)
            if !monitoring { startNetworkMonitor() }
            state = try await client.service { try $0.readState() }
            failure = nil
        } catch { failure = mobileError(error) }
    }

    private func startNetworkMonitor() {
        monitoring = true
        let client = client
        monitor.pathUpdateHandler = { path in
            Task {
                do { try await client.reportNetwork(path) }
                catch { await MainActor.run { self.failure = mobileError(error) } }
            }
        }
        monitor.start(queue: DispatchQueue(label: "app.velune.ios.network"))
    }

    func activity(foreground: Bool) async {
        do { try await client.activity(peer: peer, foreground: foreground) }
        catch { failure = mobileError(error) }
    }

    func perform(_ action: @escaping (RemoteClient) async throws -> Void) {
        Task {
            do { try await action(client); await refresh() }
            catch { failure = mobileError(error) }
        }
    }

    func status(_ peer: String) -> String {
        guard let controller = state?.controllers.first(where: { $0.peerId == peer }) else { return "未连接" }
        switch controller.phase {
        case .connected: return "已连接"
        case .connecting, .discovering, .recovering: return "连接中"
        case .blocked: return "连接失败"
        case .disconnected, .suspended: return "未连接"
        }
    }
}

func mobileError(_ error: Error) -> String {
    guard let binding = error as? BindingError else { return error.localizedDescription }
    switch binding {
    case .Diagnostic(_, let detail, _, _, _), .RemoteRequest(_, let detail, _, _): return detail
    case .Invalid(let detail), .Unsupported(let detail), .Io(let detail), .Contract(let detail): return detail
    case .Closed: return "连接已关闭"
    case .Unavailable: return "远控服务不可用"
    }
}
