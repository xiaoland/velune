import Foundation

enum TransportError: LocalizedError {
    case unavailable
    case invalidResponse
    case rejected(String)
    case invalidHome
    case incompatibleHost

    var errorDescription: String? {
        switch self {
        case .unavailable: return "本地 Host 不可用"
        case .invalidResponse: return "本地 Host 返回了无效响应"
        case .incompatibleHost: return "本地 Host 版本与当前应用不匹配。请先结束旧版任务并停止旧版 Host，再重新打开应用。"
        case .invalidHome: return "VELUNE_HOME 必须使用绝对目录路径；留空可使用默认目录。"
        case .rejected(let message): return message
        }
    }
}

private struct Request: Encodable {
    let version = 3
    let action: String
    let payload: [String: String]
}

private struct ResponseHeader: Decodable {
    let version: Int?
    let ok: Bool
    let error: String?
}

private struct Response<T: Decodable>: Decodable {
    let ok: Bool
    let error: String?
    let data: T?
}

// Requests and Host startup share one lock, including mutable process ownership.
final class Transport: @unchecked Sendable {
    let executable: URL
    let stateDirectory: URL
    private let lock = NSLock()
    private var hostProcess: Process?

    init(executable: URL, stateDirectory: URL) {
        self.executable = executable
        self.stateDirectory = stateDirectory
    }

    static func applicationDefault() throws -> Transport {
        let configuredHome = getenv("VELUNE_HOME").map { String(cString: $0) }
        let home = try applicationHome(configuredHome, userHome: FileManager.default.homeDirectoryForCurrentUser)
        return Transport(executable: Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/velune-core"), stateDirectory: home)
    }

    static func applicationHome(_ configured: String?, userHome: URL) throws -> URL {
        guard let configured, !configured.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return userHome.appendingPathComponent(".velune", isDirectory: true)
        }
        guard configured.hasPrefix("/"), !configured.contains("\u{0}") else { throw TransportError.invalidHome }
        return URL(fileURLWithPath: configured, isDirectory: true).standardizedFileURL
    }

    func request<T: Decodable>(_ action: String, payload: [String: String] = [:], as type: T.Type) throws -> T {
        lock.lock()
        defer { lock.unlock() }
        try ensureHost()
        return try perform(action, payload: payload, as: type)
    }

    private func perform<T: Decodable>(_ action: String, payload: [String: String], as type: T.Type) throws -> T {
        let request = Request(action: action, payload: payload)
        let encoded = try JSONEncoder().encode(request)
        guard let requestText = String(data: encoded, encoding: .utf8) else { throw TransportError.invalidResponse }
        let process = Process()
        process.executableURL = executable
        process.arguments = ["rpc", stateDirectory.path, requestText]
        process.standardInput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        let output = Pipe()
        process.standardOutput = output
        do { try process.run() } catch { throw TransportError.unavailable }
        let data = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { throw TransportError.unavailable }
        let header: ResponseHeader
        do { header = try JSONDecoder().decode(ResponseHeader.self, from: data) }
        catch { throw TransportError.invalidResponse }
        guard header.version == 3 else { throw TransportError.incompatibleHost }
        guard header.ok else { throw TransportError.rejected(header.error ?? "请求失败") }
        do {
            let response = try JSONDecoder().decode(Response<T>.self, from: data)
            guard let value = response.data else { throw TransportError.invalidResponse }
            return value
        } catch is DecodingError { throw TransportError.incompatibleHost }
    }

    private func ensureHost() throws {
        let socket = stateDirectory.appendingPathComponent("ipc.sock")
        if FileManager.default.fileExists(atPath: socket.path) { return }
        try FileManager.default.createDirectory(at: stateDirectory, withIntermediateDirectories: true)
        let process = Process()
        process.executableURL = executable
        process.arguments = ["host", stateDirectory.path]
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        do { try process.run() } catch { throw TransportError.unavailable }
        hostProcess = process
        for _ in 0..<40 {
            if !process.isRunning { throw TransportError.unavailable }
            if FileManager.default.fileExists(atPath: socket.path) { return }
            Thread.sleep(forTimeInterval: 0.05)
        }
        throw TransportError.unavailable
    }
}
