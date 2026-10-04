import Foundation

enum TransportError: LocalizedError {
    case unavailable
    case invalidResponse
    case rejected(String)
    case invalidHome
    case incompatibleCore
    case responseSchema(action: String, field: String)
    case closed

    var errorDescription: String? {
        switch self {
        case .unavailable: return "本地核心无法打开，请检查应用安装和配置目录后重试。"
        case .invalidResponse: return "本地核心返回了无效响应。"
        case .incompatibleCore: return "应用与内嵌核心版本不匹配，请重新安装完整应用。"
        case .responseSchema(let action, let field): return "本地核心的响应格式有误（\(action)：\(field)）。请更新应用或报告这个字段。"
        case .invalidHome: return "VELUNE_HOME 必须使用绝对目录路径；留空可使用默认目录。"
        case .closed: return "本地核心已关闭，请重新打开应用。"
        case .rejected(let message): return message
        }
    }
}

private struct Request: Encodable {
    let version = 3
    let action: String
    let payload: [String: String]
}
private struct CoreOptions: Encodable {
    let homeDirectory: String
    let resourcesDirectory: String
    let credentialResolver: String?
}
private struct CoreFailure: Decodable { let code: String; let message: String }
private struct ResponseHeader: Decodable {
    let version: Int?
    let ok: Bool
    let error: CoreFailure?
}
private struct CoreInfo: Decodable { let abiVersion: Int; let contractVersion: Int }
private struct CloseData: Decodable { let closed: Bool }
private struct Response<T: Decodable>: Decodable { let data: T? }

// The opaque Rust handle and all ABI calls share one lock. Rust owns every
// returned string; copying its UTF-8 bytes never transfers that ownership.
final class Transport: @unchecked Sendable {
    let stateDirectory: URL
    private let resourcesDirectory: URL
    private let credentialResolver: URL?
    private let lock = NSLock()
    private var handle: OpaquePointer?
    private var isClosed = false

    init(stateDirectory: URL, resourcesDirectory: URL, credentialResolver: URL? = nil) {
        self.stateDirectory = stateDirectory
        self.resourcesDirectory = resourcesDirectory
        self.credentialResolver = credentialResolver
    }

    static func applicationDefault() throws -> Transport {
        let configuredHome = getenv("VELUNE_HOME").map { String(cString: $0) }
        let home = try applicationHome(configuredHome, userHome: FileManager.default.homeDirectoryForCurrentUser)
        guard let resources = Bundle.main.resourceURL else { throw TransportError.unavailable }
        return Transport(stateDirectory: home, resourcesDirectory: resources,
                         credentialResolver: Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/velune-credential"))
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
        guard !isClosed else { throw TransportError.closed }
        try openIfNeeded()
        let encoded = try JSONEncoder().encode(Request(action: action, payload: payload))
        let text = String(decoding: encoded, as: UTF8.self)
        let data = try text.withCString { try take(velune_core_request(handle, $0)) }
        return try decode(data, action: action, as: type, versioned: true)
    }

    func close() throws {
        lock.lock()
        defer { lock.unlock() }
        guard handle != nil else { isClosed = true; return }
        let data = try take(velune_core_close(&handle))
        let result = try decode(data, action: "close", as: CloseData.self, versioned: false)
        guard result.closed, handle == nil else { throw TransportError.invalidResponse }
        isClosed = true
    }

    private func openIfNeeded() throws {
        guard handle == nil else { return }
        guard velune_core_abi_version() == 1 else { throw TransportError.incompatibleCore }
        let options = CoreOptions(homeDirectory: stateDirectory.path, resourcesDirectory: resourcesDirectory.path,
                                  credentialResolver: credentialResolver?.path)
        let encoded = try JSONEncoder().encode(options)
        let text = String(decoding: encoded, as: UTF8.self)
        do {
            let data = try text.withCString { try take(velune_core_open($0, &handle)) }
            let info = try decode(data, action: "open", as: CoreInfo.self, versioned: false)
            guard info.abiVersion == 1, info.contractVersion == 3 else { throw TransportError.incompatibleCore }
            guard handle != nil else { throw TransportError.invalidResponse }
        } catch {
            // Opening never starts an agent. Release a returned handle if the
            // initial metadata is unusable, without retrying the operation.
            if handle != nil { if let value = velune_core_close(&handle) { velune_core_string_free(value) } }
            throw error
        }
    }

    private func take(_ pointer: UnsafeMutablePointer<CChar>?) throws -> Data {
        guard let pointer else { throw TransportError.invalidResponse }
        defer { velune_core_string_free(pointer) }
        guard let text = String(validatingUTF8: pointer) else { throw TransportError.invalidResponse }
        return Data(text.utf8)
    }

    private func decode<T: Decodable>(_ data: Data, action: String, as type: T.Type, versioned: Bool) throws -> T {
        let header: ResponseHeader
        do { header = try JSONDecoder().decode(ResponseHeader.self, from: data) }
        catch { throw TransportError.invalidResponse }
        guard header.ok else { throw TransportError.rejected(header.error?.message ?? "核心操作失败") }
        if versioned, header.version != 3 { throw TransportError.incompatibleCore }
        do {
            let response = try JSONDecoder().decode(Response<T>.self, from: data)
            guard let value = response.data else { throw TransportError.invalidResponse }
            return value
        } catch let failure as DecodingError {
            let path: [CodingKey]
            switch failure {
            case .keyNotFound(let key, let context): path = context.codingPath + [key]
            case .typeMismatch(_, let context), .valueNotFound(_, let context), .dataCorrupted(let context): path = context.codingPath
            @unknown default: path = []
            }
            let field = path.isEmpty ? "data" : path.map(\.stringValue).joined(separator: ".")
            throw TransportError.responseSchema(action: action, field: field)
        }
    }
}
