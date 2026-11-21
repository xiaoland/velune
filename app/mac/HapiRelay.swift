import Darwin
import Foundation

/// Owns only the public HTTPS transport. The application owns the listener,
/// session grants and the separate token shown in HAPI's pairing QR.
actor HapiRelay {
    private let directory: URL
    private var operation = UUID()
    private var child: HapiRelayChild?
    private var retiring: HapiRelayChild?

    init(stateDirectory: URL) {
        directory = stateDirectory.appendingPathComponent("remote/hapi-relay", isDirectory: true)
    }

    func start(port: UInt16) async throws -> URL {
        stop()
        let request = operation
        do {
            try await waitForRetiring(request)
            guard port != 0,
                  let resources = Bundle.main.resourceURL?.appendingPathComponent("HapiRelay", isDirectory: true)
            else { throw HapiRelayError.missingTool }
            let supervisor = resources.appendingPathComponent("supervisor")
            let tunnel = resources.appendingPathComponent("tunwg")
            guard FileManager.default.isExecutableFile(atPath: supervisor.path),
                  FileManager.default.isExecutableFile(atPath: tunnel.path)
            else { throw HapiRelayError.missingTool }

            let cache = directory.appendingPathComponent("tunnel", isDirectory: true)
            try Self.preparePrivateDirectory(directory)
            try Self.preparePrivateDirectory(cache)
            let keyFile = directory.appendingPathComponent("relay-key")
            let existingKey = try Self.readKey(at: keyFile)
            var key: String
            if let existingKey {
                key = existingKey
            } else {
                key = try await Self.issueKey()
                try check(request)
                try Self.writeKey(key, to: keyFile)
            }

            let first = try HapiRelayChild(supervisor: supervisor, tunnel: tunnel, cache: cache, key: key, port: port)
            child = first
            do {
                let url = try await first.ready()
                try check(request)
                return url
            } catch HapiRelayError.rejected where existingKey != nil {
                try check(request)
                retireChild()
                try await waitForRetiring(request)
                key = try await Self.issueKey()
                try check(request)
                try Self.writeKey(key, to: keyFile)
                let replacement = try HapiRelayChild(supervisor: supervisor, tunnel: tunnel, cache: cache, key: key, port: port)
                child = replacement
                let url = try await replacement.ready()
                try check(request)
                return url
            }
        } catch {
            if operation == request { stop() }
            throw error
        }
    }

    func isRunning() -> Bool {
        child?.isRunning == true
    }

    func stop() {
        operation = UUID()
        retireChild()
    }

    private func retireChild() {
        guard let child else { return }
        child.stop()
        retiring = child
        self.child = nil
    }

    private func waitForRetiring(_ request: UUID) async throws {
        guard let retiring else { try check(request); return }
        let deadline = ContinuousClock.now.advanced(by: .seconds(3))
        while retiring.isRunning {
            guard ContinuousClock.now < deadline else { throw HapiRelayError.stopping }
            try await Task.sleep(for: .milliseconds(50))
            try check(request)
        }
        try check(request)
        if self.retiring === retiring { self.retiring = nil }
    }

    private func check(_ request: UUID) throws {
        try Task.checkCancellation()
        guard operation == request else { throw CancellationError() }
    }

    private static func preparePrivateDirectory(_ url: URL) throws {
        do {
            try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            var info = stat()
            guard lstat(url.path, &info) == 0, info.st_uid == geteuid(),
                  info.st_mode & S_IFMT == S_IFDIR, info.st_mode & 0o077 == 0
            else { throw HapiRelayError.storage }
        } catch { throw HapiRelayError.storage }
    }

    private static func readKey(at url: URL) throws -> String? {
        var info = stat()
        if lstat(url.path, &info) != 0 {
            if errno == ENOENT { return nil }
            throw HapiRelayError.storage
        }
        guard info.st_uid == geteuid(), info.st_mode & S_IFMT == S_IFREG,
              info.st_mode & 0o077 == 0, (1...256).contains(info.st_size),
              let key = try? String(contentsOf: url, encoding: .utf8), validKey(key)
        else { throw HapiRelayError.storage }
        return key
    }

    private static func writeKey(_ key: String, to url: URL) throws {
        do {
            try Data(key.utf8).write(to: url, options: .atomic)
            guard chmod(url.path, 0o600) == 0 else { throw HapiRelayError.storage }
        } catch { throw HapiRelayError.storage }
    }

    private static func validKey(_ key: String) -> Bool {
        (1...256).contains(key.utf8.count) && key.utf8.allSatisfy {
            (65...90).contains($0) || (97...122).contains($0) || (48...57).contains($0) || [45, 46, 95].contains($0)
        }
    }

    private static func issueKey() async throws -> String {
        struct IssuedKey: Decodable { let key: String }

        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 10
        configuration.timeoutIntervalForResource = 12
        configuration.httpShouldSetCookies = false
        configuration.httpCookieStorage = nil
        configuration.urlCredentialStorage = nil
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        var request = URLRequest(url: URL(string: "https://relay.hapi.run/issue")!)
        request.httpMethod = "POST"

        do {
            let (bytes, response) = try await session.bytes(for: request, delegate: HapiRelayIssueDelegate())
            guard let http = response as? HTTPURLResponse else { throw HapiRelayError.issuance }
            if http.statusCode == 429 { throw HapiRelayError.rateLimited }
            guard http.statusCode == 200 else { throw HapiRelayError.issuance }
            var data = Data()
            for try await byte in bytes {
                guard data.count < 1024 else { throw HapiRelayError.issuance }
                data.append(byte)
            }
            guard let issued = try? JSONDecoder().decode(IssuedKey.self, from: data), validKey(issued.key)
            else { throw HapiRelayError.issuance }
            return issued.key
        } catch let error as HapiRelayError {
            throw error
        } catch {
            try Task.checkCancellation()
            throw HapiRelayError.issuance
        }
    }
}

private final class HapiRelayIssueDelegate: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest,
        completionHandler: @escaping @Sendable (URLRequest?) -> Void
    ) {
        completionHandler(nil)
    }
}

private enum HapiRelayError: LocalizedError {
    case missingTool, storage, issuance, rateLimited, rejected, launch, exited, timedOut, stopping

    var errorDescription: String? {
        switch self {
        case .missingTool: "当前安装缺少 HAPI 中继组件"
        case .storage: "无法读取或保护 HAPI 中继凭据"
        case .issuance: "公共中继暂时无法签发凭据"
        case .rateLimited: "公共中继暂时限制凭据签发，请稍后重试"
        case .rejected: "公共中继拒绝了凭据"
        case .launch: "无法启动 HAPI 中继组件"
        case .exited: "HAPI 中继在就绪前退出"
        case .timedOut: "等待 HAPI 中继启动超时"
        case .stopping: "上一个 HAPI 中继仍在停止"
        }
    }
}

/// The supervisor closes its exact child when the app's stdin pipe reaches
/// EOF. This covers ordinary stop and the application's unexpected exit.
private final class HapiRelayChild: @unchecked Sendable {
    private enum Event: Sendable {
        case ready(URL), rejected
    }

    private let process: Process
    private let lifetime: FileHandle
    private let events: AsyncStream<Event>

    var isRunning: Bool { process.isRunning }

    init(supervisor: URL, tunnel: URL, cache: URL, key: String, port: UInt16) throws {
        let process = Process()
        let lifetime = Pipe()
        let logs = Pipe()
        process.executableURL = supervisor
        process.arguments = [tunnel.path, "--json", "--log_level=0", "--forward=http://127.0.0.1:\(port)"]
        process.environment = ["TUNWG_API": "relay.hapi.run", "TUNWG_AUTH": key, "TUNWG_PATH": cache.path]
        process.standardInput = lifetime
        process.standardOutput = FileHandle.nullDevice
        process.standardError = logs
        do { try process.run() } catch { throw HapiRelayError.launch }
        try? lifetime.fileHandleForReading.close()
        try? logs.fileHandleForWriting.close()
        self.process = process
        self.lifetime = lifetime.fileHandleForWriting
        let (events, continuation) = AsyncStream<Event>.makeStream(bufferingPolicy: .bufferingOldest(2))
        self.events = events
        let input = logs.fileHandleForReading
        DispatchQueue.global(qos: .utility).async {
            Self.drain(input, continuation: continuation)
        }
    }

    deinit { stop() }

    func stop() { try? lifetime.close() }

    func ready() async throws -> URL {
        try await withThrowingTaskGroup(of: URL.self) { group in
            group.addTask { [events] in
                for await event in events {
                    switch event {
                    case let .ready(url): return url
                    case .rejected: throw HapiRelayError.rejected
                    }
                }
                try Task.checkCancellation()
                throw HapiRelayError.exited
            }
            group.addTask {
                try await Task.sleep(for: .seconds(20))
                throw HapiRelayError.timedOut
            }
            defer { group.cancelAll() }
            guard let url = try await group.next() else { throw HapiRelayError.exited }
            return url
        }
    }

    private static func drain(_ input: FileHandle, continuation: AsyncStream<Event>.Continuation) {
        defer { continuation.finish(); try? input.close() }
        var line = Data()
        var oversized = false
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let count = buffer.withUnsafeMutableBytes { read(input.fileDescriptor, $0.baseAddress, $0.count) }
            if count == -1 && errno == EINTR { continue }
            guard count > 0 else { return }
            for byte in buffer.prefix(count) {
                if byte == 10 {
                    if !oversized, let event = parse(line) { continuation.yield(event) }
                    line.removeAll(keepingCapacity: true)
                    oversized = false
                } else if line.count < 4096 {
                    line.append(byte)
                } else {
                    oversized = true
                }
            }
        }
    }

    private static func parse(_ line: Data) -> Event? {
        guard let row = (try? JSONSerialization.jsonObject(with: line)) as? [String: Any] else { return nil }
        if row["event"] as? String == "peer_registration_rejected", row["status"] as? Int == 403 { return .rejected }
        guard row["msg"] as? String == "listener started", let value = row["url"] as? String,
              let components = URLComponents(string: value), components.scheme == "https",
              components.port == nil || components.port == 443,
              components.user == nil, components.password == nil, components.query == nil, components.fragment == nil,
              components.path.isEmpty || components.path == "/", let host = components.host,
              host.hasSuffix(".relay.hapi.run")
        else { return nil }
        let label = host.dropLast(".relay.hapi.run".count)
        guard (1...63).contains(label.utf8.count),
              label.utf8.allSatisfy({ (97...122).contains($0) || (48...57).contains($0) || $0 == 45 }),
              label.first != "-", label.last != "-", let url = URL(string: "https://\(host)")
        else { return nil }
        return .ready(url)
    }
}
