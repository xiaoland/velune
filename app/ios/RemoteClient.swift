import Foundation
import Network
import UIKit

final class ClientResources: @unchecked Sendable {
    private let lock = NSLock()
    private var owner: ClientTelemetry?
    private var service: VeluneRemoteEndpoint?
    func install(_ owner: ClientTelemetry, _ service: VeluneRemoteEndpoint) { lock.withLock { self.owner = owner; self.service = service } }
    func telemetry() throws -> ClientTelemetry { try lock.withLock { guard let owner else { throw MobileError.unavailable }; return owner } }
    func endpoint() throws -> VeluneRemoteEndpoint { try lock.withLock { guard let service else { throw MobileError.unavailable }; return service } }
}

/// Synchronous UniFFI calls run on this actor, never on the UI executor.
actor RemoteService {
    static let shared = RemoteService()
    nonisolated let resources = ClientResources()
    private struct ViewKey: Hashable { let peer: String; let view: String }
    private var endpoint: VeluneRemoteEndpoint?
    private var applications: [ViewKey: VeluneApplication] = [:]
    private var windows: [String: (peer: String?, foreground: Bool)] = [:]
    private let monitor = NWPathMonitor()
    private var networkSequence: UInt64 = 0

    func start(name: String) throws {
        guard endpoint == nil else { return }
        var directory = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true).appendingPathComponent("remote")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        // Restoring this directory to another device would clone its trusted identity.
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try directory.setResourceValues(values)
        var clientHome = directory.deletingLastPathComponent().appendingPathComponent("client")
        try FileManager.default.createDirectory(at: clientHome, withIntermediateDirectories: true)
        try clientHome.setResourceValues(values)
        let owner = try ClientTelemetry(home: clientHome, platform: "ios", localLogs: false,
            developmentProfile: ProcessInfo.processInfo.environment["VELUNE_TELEMETRY_PROFILE"], runID: ProcessInfo.processInfo.environment["VELUNE_RUN_ID"])
        try owner.start()
        let opened = try VeluneRemoteEndpoint.openWithTelemetry(options: BindingRemoteOptions(
            identityFile: directory.appendingPathComponent("device-identity").path,
            stateFile: directory.appendingPathComponent("remote-state.json").path,
            deviceName: name, relay: .developmentPublic, discovery: .developmentPublic,
            pathPolicy: .automatic, bindAddresses: [], requestTimeoutMs: 30_000), application: nil, telemetry: owner.binding)
        endpoint = opened; resources.install(owner, opened)
        for peer in Set(windows.values.compactMap { $0.peer }) { try updateActivity(peer) }
        monitor.pathUpdateHandler = { [weak self] path in
            Task {
                do { try await self?.reportNetwork(path) }
                catch { await ProblemStore.shared.record(error, source: "网络变化") }
            }
        }
        monitor.start(queue: DispatchQueue(label: "app.velune.ios.network"))
    }

    func service<T>(_ body: (VeluneRemoteEndpoint) throws -> T) throws -> T {
        guard let endpoint else { throw MobileError.unavailable }
        return try body(endpoint)
    }

    func application<T>(_ peer: String, viewID: String, carrier: BindingTraceCarrier?, _ body: (VeluneApplication) throws -> T) throws -> T {
        guard let endpoint else { throw MobileError.unavailable }
        let key = ViewKey(peer: peer, view: viewID)
        let application: VeluneApplication
        if let cached = applications[key] { application = cached }
        else {
            application = try endpoint.connect(peerId: peer, viewId: viewID)
            applications[key] = application
        }
        return try body(application.withTraceContext(carrier: carrier))
    }

    func activity(viewID: String, peer: String?, foreground: Bool) throws {
        let previous = windows[viewID]?.peer
        windows[viewID] = (peer, foreground)
        for affected in Set([previous, peer].compactMap { $0 }) { try updateActivity(affected) }
    }

    private func updateActivity(_ peer: String) throws {
        let views = windows.values.filter { $0.peer == peer }
        try endpoint?.setControllerActivity(peerId: peer, selected: !views.isEmpty, foreground: views.contains { $0.foreground })
    }

    func release(viewID: String) throws {
        let previous = windows.removeValue(forKey: viewID)?.peer
        if let previous { try updateActivity(previous) }
        for key in applications.keys.filter({ $0.view == viewID }) {
            let app = applications.removeValue(forKey: key)
            try app?.releaseView()
        }
    }

    func reportNetwork(_ path: NWPath) throws {
        networkSequence += 1
        // NWPath can repeat an interface; core snapshots require unique network IDs.
        var names = Set<String>()
        let interfaces = path.availableInterfaces.filter { names.insert($0.name).inserted }
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
        applications = applications.filter { $0.key.peer != peer }
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

/// A window owns a core view, while transport and device identity remain shared.
final class RemoteClient: @unchecked Sendable {
    let viewID: String
    init(viewID: String = "ios-\(UUID().uuidString)") { self.viewID = viewID }
    func start(name: String) async throws { try await RemoteService.shared.start(name: name) }
    func service<T>(_ body: (VeluneRemoteEndpoint) throws -> T) async throws -> T { try await RemoteService.shared.service(body) }
    func application<T>(_ peer: String, operation: String = "read", _ body: (VeluneApplication) throws -> T) async throws -> T {
        let telemetry = try RemoteService.shared.resources.telemetry()
        return try await telemetry.operation(operation) { carrier in
            try await RemoteService.shared.application(peer, viewID: viewID, carrier: carrier, body)
        }
    }
    func activity(peer: String?, foreground: Bool) async throws { try await RemoteService.shared.activity(viewID: viewID, peer: peer, foreground: foreground) }
    func forget(_ peer: String) async throws { try await RemoteService.shared.forget(peer) }
    deinit {
        let id = viewID
        Task {
            do { try await RemoteService.shared.release(viewID: id) }
            catch { await ProblemStore.shared.record(error, source: "关闭窗口") }
        }
    }
}

@MainActor
final class DeviceDirectory: ObservableObject {
    @Published var state: BindingRemoteServiceSnapshot?
    private var activeWindows = Set<String>()
    private var polling: Task<Void, Never>?

    func activity(window: String, active: Bool) {
        if active { activeWindows.insert(window) } else { activeWindows.remove(window) }
        if activeWindows.isEmpty { polling?.cancel(); polling = nil }
        else if polling == nil {
            polling = Task { [weak self] in
                while !Task.isCancelled {
                    await self?.refresh()
                    do { try await Task.sleep(for: .seconds(1)) } catch { return }
                }
            }
        }
    }
    func refresh() async {
        do {
            try await RemoteService.shared.start(name: UIDevice.current.name)
            state = try await RemoteService.shared.service { try $0.readState() }
            ProblemStore.shared.recovered(source: "设备连接")
        } catch { ProblemStore.shared.record(error, source: "设备连接") }
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
