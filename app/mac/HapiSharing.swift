import Foundation
import SwiftUI
import VeluneBindings

struct HapiShareTarget: Identifiable, Sendable {
    let conversationID: String
    let runtimeID: String
    let title: String
    var id: String { conversationID }

    init(_ conversation: Conversation) {
        conversationID = conversation.id
        runtimeID = conversation.runtimeID
        title = conversation.title
    }

    @MainActor static func local(_ conversation: Conversation, device: DeviceStore) -> Self? {
        guard DeviceHub.shared?.localStore === device, conversation.parentConversationID == nil else { return nil }
        return Self(conversation)
    }
}

struct HapiPairing {
    let target: HapiShareTarget
    let origin: URL
    let grant: BindingHapiShare

    var link: String {
        var components = URLComponents()
        components.scheme = "hapicompanion"
        components.host = "bind"
        components.queryItems = [
            URLQueryItem(name: "hub", value: origin.absoluteString),
            URLQueryItem(name: "code", value: grant.accessToken),
        ]
        return components.string ?? ""
    }
}

/// One share for the whole app. Windows display it and can close without revoking it.
@MainActor final class HapiSharing: ObservableObject {
    nonisolated static let port: UInt16 = 33430

    @Published private(set) var pairing: HapiPairing?
    @Published private(set) var busy = false
    @Published private(set) var errorMessage: String?

    private let endpoint: RemoteEndpointBox
    private let relay: HapiRelay
    private var starting: Task<Void, Never>?
    private var monitor: Task<Void, Never>?

    init(endpoint: RemoteEndpointBox, stateDirectory: URL) {
        self.endpoint = endpoint
        relay = HapiRelay(stateDirectory: stateDirectory)
    }

    func start(_ target: HapiShareTarget, existingOrigin: URL?) {
        guard !busy, pairing == nil else { return }
        busy = true
        errorMessage = nil
        starting = Task {
            defer { busy = false; starting = nil }
            var grant: BindingHapiShare?
            do {
                let created = try await perform { endpoint in
                    guard try endpoint.activeHapiShareId() == nil else {
                        throw TransportError.rejected("已有 HAPI 共享。请先结束它，再开启本次共享。")
                    }
                    return try endpoint.startHapiShare(port: Self.port, runtimeId: target.runtimeID, conversationId: target.conversationID)
                }
                grant = created
                try Task.checkCancellation()
                let origin: URL
                if let existingOrigin { origin = existingOrigin }
                else { origin = try await relay.start(port: Self.port) }
                try await HapiOriginCheck.verify(origin, shareID: created.id)
                guard try await perform({ try $0.activeHapiShareId() }) == created.id else {
                    throw TransportError.rejected("会话共享已结束，请重新开启。")
                }
                try Task.checkCancellation()
                pairing = HapiPairing(target: target, origin: origin, grant: created)
                watchShare(created.id, usesRelay: existingOrigin == nil)
            } catch {
                if let grant { try? await perform { try $0.stopHapiShare(shareId: grant.id) } }
                await relay.stop()
                if !Task.isCancelled { errorMessage = error.localizedDescription }
            }
        }
    }

    func cancelStart() { starting?.cancel() }

    func stop() {
        guard !busy, let pairing else { return }
        busy = true
        errorMessage = nil
        Task {
            defer { busy = false }
            do {
                try await perform { try $0.stopHapiShare(shareId: pairing.grant.id) }
                await relay.stop()
                monitor?.cancel()
                monitor = nil
                self.pairing = nil
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    private func watchShare(_ id: String, usesRelay: Bool) {
        monitor?.cancel()
        monitor = Task { [weak self] in
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(2)) } catch { return }
                guard let self, pairing?.grant.id == id else { return }
                if busy { continue }
                let running = usesRelay ? await relay.isRunning() : true
                let active = try? await perform { try $0.activeHapiShareId() }
                guard !Task.isCancelled, pairing?.grant.id == id else { return }
                if busy { continue }
                if !running || active != id {
                    busy = true
                    defer { busy = false }
                    if active == id { try? await perform { try $0.stopHapiShare(shareId: id) } }
                    await relay.stop()
                    pairing = nil
                    errorMessage = running ? "会话共享已结束。" : "HAPI 中继已断开，共享已结束。"
                    return
                }
            }
        }
    }

    private func perform<T: Sendable>(_ operation: @escaping @Sendable (VeluneRemoteEndpoint) throws -> T) async throws -> T {
        let endpoint = endpoint
        return try await Task.detached {
            guard let opened = endpoint.value else { throw TransportError.unavailable }
            do { return try operation(opened) }
            catch let error as BindingError { throw Transport.transportError(for: error) }
        }.value
    }
}

private final class HapiOriginCheck: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    private struct Health: Decodable {
        let status: String
        let protocolVersion: Int
        let veluneShareId: String
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping @Sendable (URLRequest?) -> Void) {
        completionHandler(nil)
    }

    static func verify(_ origin: URL, shareID: String) async throws {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 5
        config.timeoutIntervalForResource = 5
        config.httpShouldSetCookies = false
        config.httpCookieStorage = nil
        config.urlCredentialStorage = nil
        let session = URLSession(configuration: config, delegate: HapiOriginCheck(), delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        // Initial ACME issuance can take longer than bringing up the tunnel.
        for attempt in 0..<5 {
            if attempt > 0 { try await Task.sleep(for: .seconds(1)) }
            let url = origin.appending(path: "health")
            guard let (bytes, response) = try? await session.bytes(from: url),
                  (response as? HTTPURLResponse)?.statusCode == 200 else { continue }
            var data = Data()
            do {
                for try await byte in bytes {
                    guard data.count < 4096 else { throw URLError(.dataLengthExceedsMaximum) }
                    data.append(byte)
                }
            } catch { continue }
            if let health = try? JSONDecoder().decode(Health.self, from: data),
               health.status == "ok", health.protocolVersion == 1, health.veluneShareId == shareID { return }
        }
        throw TransportError.rejected("HTTPS 地址未连到本次 Velune 共享。请检查中继连接或 HTTPS 转发后重试。")
    }
}
