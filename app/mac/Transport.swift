import Foundation
import VeluneBindings

enum TransportError: LocalizedError {
    case unavailable
    case invalidHome
    case incompatibleCore
    case closed
    case rejected(String)

    var errorDescription: String? {
        switch self {
        case .unavailable: return "本地核心无法打开，请检查应用安装和配置目录后重试。"
        case .invalidHome: return "VELUNE_HOME 必须使用绝对目录路径；留空可使用默认目录。"
        case .incompatibleCore: return "应用与内嵌核心版本不匹配，请重新安装完整应用。"
        case .closed: return "本地核心已关闭，请重新打开应用。"
        case .rejected(let message): return message
        }
    }
}

/// Typed UniFFI application boundary. The UI never constructs an action or
/// payload dictionary; generated records and methods are the ABI contract.
final class Transport: @unchecked Sendable {
    let stateDirectory: URL
    private let resourcesDirectory: URL
    private let credentialResolver: URL?
    private let lock = NSLock()
    private var application: VeluneApplication?
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

    func list() throws -> BindingConfigurationSnapshot { try withApplication { try $0.list() } }
    func upsertGateway(_ gateway: BindingGatewayConfig) throws -> BindingGatewayUpdate { try withApplication { try $0.upsertGateway(gateway: gateway) } }
    func deleteGateway(id: String) throws -> BindingGatewayUpdate { try withApplication { try $0.deleteGateway(id: id) } }
    func upsertRuntime(_ runtime: BindingRuntimeInstance) throws -> BindingRuntimeUpdate { try withApplication { try $0.upsertRuntime(runtime: runtime) } }
    func deleteRuntime(id: String) throws -> BindingRuntimeUpdate { try withApplication { try $0.deleteRuntime(id: id) } }
    func connectRuntime(id: String) throws -> BindingConnectionResult { try withApplication { try $0.connectRuntime(id: id) } }
    func createConversation(runtimeID: String, cwd: String) throws -> BindingSnapshotResult { try withApplication { try $0.createConversation(runtimeId: runtimeID, cwd: cwd) } }
    func openConversation(runtimeID: String, conversationID: String) throws -> BindingSnapshotResult { try withApplication { try $0.openConversation(runtimeId: runtimeID, conversationId: conversationID) } }
    func snapshot(runtimeID: String) throws -> BindingSnapshotResult { try withApplication { try $0.snapshot(runtimeId: runtimeID) } }
    func send(runtimeID: String, text: String) throws -> BindingSnapshotResult { try withApplication { try $0.send(runtimeId: runtimeID, text: text) } }
    func cancel(runtimeID: String) throws -> BindingSnapshotResult { try withApplication { try $0.cancel(runtimeId: runtimeID) } }
    func selectModel(runtimeID: String, modelID: String) throws -> BindingSnapshotResult { try withApplication { try $0.selectModel(runtimeId: runtimeID, modelId: modelID) } }
    func providerImportPreview(gatewayID: String, source: BindingProviderImportSource) throws -> BindingProviderImportPreview { try withApplication { try $0.previewProviderImport(gatewayId: gatewayID, source: source) } }
    func providerImportApply(gatewayID: String, source: BindingProviderImportSource, previewToken: String, selections: [BindingImportSelection], replaceExisting: Bool) throws -> BindingImportResult { try withApplication { try $0.applyProviderImport(gatewayId: gatewayID, source: source, previewToken: previewToken, selections: selections, replaceExisting: replaceExisting) } }
    func authenticationInspect(source: BindingCredentialSource) throws -> BindingAuthenticationMetadata { try withApplication { try $0.authenticationInspect(source: source) } }
    func authenticationStart(source: BindingCredentialSource) throws -> BindingAuthenticationProgress { try withApplication { try $0.authenticationStart(source: source) } }
    func authenticationPoll() throws -> BindingAuthenticationProgress { try withApplication { try $0.authenticationPoll() } }
    func authenticationReply(promptID: String, value: String) throws -> BindingAuthenticationProgress { try withApplication { try $0.authenticationReply(promptId: promptID, value: value) } }
    func authenticationCancel() throws -> BindingAuthenticationProgress { try withApplication { try $0.authenticationCancel() } }

    func close() throws {
        lock.lock(); defer { lock.unlock() }
        guard let application else { isClosed = true; return }
        do {
            try application.shutdown()
            self.application = nil
            isClosed = true
        } catch { throw map(error) }
    }

    private func withApplication<T>(_ body: (VeluneApplication) throws -> T) throws -> T {
        lock.lock(); defer { lock.unlock() }
        guard !isClosed else { throw TransportError.closed }
        if application == nil {
            let options = BindingOptions(homeDirectory: stateDirectory.path,
                                         resourcesDirectory: resourcesDirectory.path,
                                         credentialResolver: credentialResolver?.path)
            do { application = try VeluneApplication.open(options: options) }
            catch { throw map(error) }
        }
        do { return try body(application!) }
        catch { throw map(error) }
    }

    private func map(_ error: Error) -> TransportError {
        if let localized = error as? LocalizedError, let description = localized.errorDescription {
            return .rejected(description)
        }
        return .rejected(error.localizedDescription)
    }
}
