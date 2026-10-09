import Foundation
import OSLog
import Combine
import VeluneBindings

enum TransportError: LocalizedError {
    indirect case operation(String, TransportError)
    case unavailable
    case invalidHome
    case incompatibleCore
    case closed
    case rejected(String)
    case diagnostic(kind: String, detail: String, code: String, phase: String, operationID: String)

    var errorDescription: String? {
        switch self {
        case .operation(_, let cause): return cause.errorDescription
        case .unavailable: return "本地核心无法打开，请检查应用安装和配置目录后重试。"
        case .invalidHome: return "VELUNE_HOME 必须使用绝对目录路径；留空可使用默认目录。"
        case .incompatibleCore: return "应用与内嵌核心版本不匹配，请重新安装完整应用。"
        case .closed: return "本地核心已关闭，请重新打开应用。"
        case .rejected(let message): return message
        case .diagnostic(_, let detail, let code, let phase, let operationID): return "操作失败（诊断编号：\(operationID)，阶段：\(phase)，代码：\(code)）：\(detail)"
        }
    }
}

/// Typed UniFFI application boundary. The UI never constructs an action or
/// payload dictionary; generated records and methods are the ABI contract.
final class Transport: @unchecked Sendable {
    let configurationChanged = PassthroughSubject<Void, Never>()
    let stateDirectory: URL
    private let resourcesDirectory: URL
    private let lock = NSLock()
    private static let logger = Logger(subsystem: "local.velune", category: "transport")
    private var application: VeluneApplication?
    private var isClosed = false

    init(stateDirectory: URL, resourcesDirectory: URL) {
        self.stateDirectory = stateDirectory
        self.resourcesDirectory = resourcesDirectory
    }

    static func applicationDefault() throws -> Transport {
        let configuredHome = getenv("VELUNE_HOME").map { String(cString: $0) }
        let home = try applicationHome(configuredHome, userHome: FileManager.default.homeDirectoryForCurrentUser)
        guard let resources = Bundle.main.resourceURL else { throw TransportError.unavailable }
        return Transport(stateDirectory: home, resourcesDirectory: resources)
    }

    static func applicationHome(_ configured: String?, userHome: URL) throws -> URL {
        guard let configured, !configured.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return userHome.appendingPathComponent(".velune", isDirectory: true)
        }
        guard configured.hasPrefix("/"), !configured.contains("\u{0}") else { throw TransportError.invalidHome }
        return URL(fileURLWithPath: configured, isDirectory: true).standardizedFileURL
    }

    func setConversationBrowserPreferences(_ preferences: BindingConversationBrowserPreferences) throws -> BindingConversationBrowserPreferences { try withApplication("setConversationBrowserPreferences", changesConfiguration: true) { try $0.setConversationBrowserPreferences(preferences: preferences) } }
    func setTranscriptPresentation(_ presentation: BindingTranscriptPresentation) throws -> BindingTranscriptPresentation { try withApplication("setTranscriptPresentation", changesConfiguration: true) { try $0.setTranscriptPresentation(presentation: presentation) } }
    func setConversationBrowserGroupLimit(_ limit: UInt32) throws -> UInt32 { try withApplication("setConversationBrowserGroupLimit", changesConfiguration: true) { try $0.setConversationBrowserGroupLimit(limit: limit) } }
    func runtimeDiscoveryHints(userHome: String, overrides: [String: String]) throws -> [BindingRuntimeDiscoveryHint] { try withApplication("runtimeDiscoveryHints") { try $0.runtimeDiscoveryHints(userHome: userHome, overrides: overrides) } }
    func discoverRuntimes(_ probes: [BindingRuntimeDiscoveryProbe]) throws -> [BindingRuntimeDiscoveryCandidate] { try withApplication("discoverRuntimes") { try $0.discoverRuntimes(probes: probes) } }
    func analyticsQuery(_ query: BindingAnalyticsQuery) throws -> BindingAnalyticsReport { try withApplication("analyticsQuery") { try $0.analyticsQuery(query: query) } }
    func list() throws -> BindingConfigurationSnapshot { try withApplication("list") { try $0.list() } }
    func saveProvider(gatewayID: String, provider: AIProvider, authenticationEdit: AuthenticationEdit) throws -> BindingGatewayUpdate {
        try withApplication("saveProvider", changesConfiguration: true) { try $0.saveProvider(gatewayId: gatewayID, provider: BindingMapping.bindingProvider(provider), authenticationEdit: BindingMapping.bindingAuthenticationEdit(authenticationEdit)) }
    }
    func deleteProvider(gatewayID: String, providerID: String) throws -> BindingGatewayUpdate { try withApplication("deleteProvider", changesConfiguration: true) { try $0.deleteProvider(gatewayId: gatewayID, providerId: providerID) } }
    func readProviderAPIKey(gatewayID: String, providerID: String) throws -> String { try withApplication("readProviderAPIKey") { try $0.readProviderApiKey(gatewayId: gatewayID, providerId: providerID) } }
    func saveTemplate(_ value: ModelTemplate) throws -> [BindingModelTemplate] { try withApplication("saveModelTemplate", changesConfiguration: true) { try $0.saveModelTemplate(template: BindingMapping.bindingTemplate(value)) } }
    func publicModelCatalog() throws -> [BindingCatalogModel] { try withApplication("fetchPublicModelCatalog") { try $0.fetchPublicModelCatalog() } }
    func deleteTemplate(id: String) throws -> [BindingModelTemplate] { try withApplication("deleteModelTemplate", changesConfiguration: true) { try $0.deleteModelTemplate(id: id) } }
    func upsertRuntime(_ runtime: BindingRuntimeInstance) throws -> BindingRuntimeUpdate { try withApplication("upsertRuntime", changesConfiguration: true) { try $0.upsertRuntime(runtime: runtime) } }
    func deleteRuntime(id: String) throws -> BindingRuntimeUpdate { try withApplication("deleteRuntime", changesConfiguration: true) { try $0.deleteRuntime(id: id) } }
    func selectRuntime(id: String) throws -> BindingConfigurationSnapshot { try withApplication("selectRuntime") { try $0.selectRuntime(id: id) } }
    func createConversation(runtimeID: String, cwd: String?, modelRecordKey: String) throws -> BindingSnapshotResult { try withApplication("createConversation") { try $0.createConversation(runtimeId: runtimeID, cwd: cwd, modelRecordKey: modelRecordKey) } }
    /// Keep first creation and dispatch under the same shared application lock.
    /// Report the native identity before dispatch so failure never recreates a draft.
    func createAndSend(runtimeID: String, cwd: String?, modelRecordKey: String, text: String, onCreated: (BindingSnapshotResult) -> Void) throws -> BindingSnapshotResult {
        try withApplication("sendTurn") { application in
            let created = try application.createConversation(runtimeId: runtimeID, cwd: cwd, modelRecordKey: modelRecordKey)
            onCreated(created)
            guard let native = created.snapshot else { throw TransportError.rejected("运行时未返回新会话") }
            return try application.sendTurn(sourceRuntimeId: native.conversation.runtimeId, conversationId: native.conversation.id, targetRuntimeId: runtimeID, modelRecordKey: modelRecordKey, text: text)
        }
    }
    func openConversation(runtimeID: String, conversationID: String) throws -> BindingSnapshotResult { try withApplication("openConversation") { try $0.readConversation(runtimeId: runtimeID, conversationId: conversationID) } }
    func renameConversation(runtimeID: String, conversationID: String, title: String) throws -> BindingConfigurationSnapshot { try withApplication("renameConversation") { try $0.renameConversation(runtimeId: runtimeID, conversationId: conversationID, title: title) } }
    func deleteConversation(runtimeID: String, conversationID: String) throws -> BindingConfigurationSnapshot { try withApplication("deleteConversation") { try $0.deleteConversation(runtimeId: runtimeID, conversationId: conversationID) } }
    func snapshot(runtimeID: String, conversationID: String) throws -> BindingSnapshotResult { try withApplication("snapshot") { try $0.snapshot(runtimeId: runtimeID, conversationId: conversationID) } }
    func sendTurn(sourceRuntimeID: String, conversationID: String, targetRuntimeID: String, modelRecordKey: String, text: String) throws -> BindingSnapshotResult { try withApplication("sendTurn") { try $0.sendTurn(sourceRuntimeId: sourceRuntimeID, conversationId: conversationID, targetRuntimeId: targetRuntimeID, modelRecordKey: modelRecordKey, text: text) } }
    func cancel(runtimeID: String) throws -> BindingSnapshotResult { try withApplication("cancel") { try $0.cancel(runtimeId: runtimeID) } }
    func replyRuntimeInteraction(runtimeID: String, interactionID: String, reply: RuntimeInteractionReply) throws -> BindingSnapshotResult { try withApplication("replyRuntimeInteraction") { try $0.replyRuntimeInteraction(runtimeId: runtimeID, interactionId: interactionID, reply: BindingMapping.bindingInteractionReply(reply)) } }
    func providerImportPreview(gatewayID: String, source: BindingProviderImportSource) throws -> BindingProviderImportPreview { try withApplication("providerImportPreview") { try $0.previewProviderImport(gatewayId: gatewayID, source: source) } }
    func providerImportApply(gatewayID: String, source: BindingProviderImportSource, previewToken: String, selections: [BindingImportSelection], replaceExisting: Bool) throws -> BindingImportResult { try withApplication("providerImportApply", changesConfiguration: true) { try $0.applyProviderImport(gatewayId: gatewayID, source: source, previewToken: previewToken, selections: selections, replaceExisting: replaceExisting) } }
    func authenticationInspect(gatewayID: String, providerID: String) throws -> BindingAuthenticationMetadata { try withApplication("authenticationInspect") { try $0.authenticationInspect(gatewayId: gatewayID, providerId: providerID) } }
    func authenticationStart(gatewayID: String, providerID: String) throws -> BindingAuthenticationProgress { try withApplication("authenticationStart") { try $0.authenticationStart(gatewayId: gatewayID, providerId: providerID) } }
    func authenticationPoll() throws -> BindingAuthenticationProgress { try withApplication("authenticationPoll") { try $0.authenticationPoll() } }
    func authenticationReply(promptID: String, value: String) throws -> BindingAuthenticationProgress { try withApplication("authenticationReply") { try $0.authenticationReply(promptId: promptID, value: value) } }
    func authenticationCancel() throws -> BindingAuthenticationProgress { try withApplication("authenticationCancel") { try $0.authenticationCancel() } }

    func close() throws {
        lock.lock(); defer { lock.unlock() }
        guard let application else { isClosed = true; return }
        do {
            try application.shutdown()
            self.application = nil
            isClosed = true
        } catch { logFailure(operation: "shutdown", localCorrelation: nil, error: error); throw TransportError.operation("shutdown", map(error)) }
    }

    private func withApplication<T>(_ operation: String, changesConfiguration: Bool = false, _ body: (VeluneApplication) throws -> T) throws -> T {
        let correlation = UUID().uuidString
        Self.logger.debug("operation=\(operation, privacy: .public) localCorrelation=\(correlation, privacy: .public) started")
        lock.lock(); defer { lock.unlock() }
        guard !isClosed else { throw TransportError.operation(operation, .closed) }
        if application == nil {
            let options = BindingOptions(homeDirectory: stateDirectory.path,
                                         resourcesDirectory: resourcesDirectory.path)
            do { application = try VeluneApplication.open(options: options) }
            catch { logFailure(operation: operation, localCorrelation: correlation, error: error, categoryOverride: "open_failed"); throw TransportError.operation(operation, map(error)) }
        }
        do {
            let value = try body(application!)
            if changesConfiguration { configurationChanged.send() }
            return value
        }
        catch { logFailure(operation: operation, localCorrelation: correlation, error: error); throw TransportError.operation(operation, map(error)) }
    }

    private func map(_ error: Error) -> TransportError {
        if let error = error as? TransportError { return error }
        if let error = error as? BindingError { return Self.transportError(for: error) }
        return .rejected(Self.fullFailureDetail(error))
    }

    private func category(for error: Error) -> String {
        if let error = error as? BindingError {
            switch error {
            case .Diagnostic(let kind, _, _, _, _): return Self.kindName(kind)
            case .Invalid: return "invalid"
            case .Unsupported: return "unsupported"
            case .Io: return "io"
            case .Contract: return "contract"
            case .Closed: return "closed"
            case .Unavailable: return "unavailable"
            }
        }
        return "unknown"
    }

    private static func transportError(for error: BindingError) -> TransportError {
        switch error {
        case .Diagnostic(let kind, let detail, let code, let phase, let operationID):
            return .diagnostic(kind: kindName(kind), detail: detail, code: code, phase: phase, operationID: operationID)
        case .Invalid(let detail): return .rejected("请求无效：\(detail)")
        case .Unsupported(let detail): return .rejected("当前版本不支持此操作：\(detail)")
        case .Io(let detail): return .rejected("本地读写失败：\(detail)")
        case .Contract(let detail): return .rejected("应用与核心契约不匹配：\(detail)")
        case .Closed: return .closed
        case .Unavailable: return .unavailable
        }
    }

    private static func kindName(_ kind: BindingFailureKind) -> String {
        switch kind {
        case .invalid: return "invalid"
        case .history: return "history"
        case .unsupported: return "unsupported"
        case .io: return "io"
        case .contract: return "contract"
        case .closed: return "closed"
        case .unavailable: return "unavailable"
        case .providerImport: return "provider_import"
        }
    }

    private static func fullFailureDetail(_ error: Error) -> String {
        if let error = error as? BindingError {
            switch error {
            case .Diagnostic(_, let detail, _, _, _), .Invalid(let detail), .Unsupported(let detail), .Io(let detail), .Contract(let detail): return detail
            case .Closed: return "本地核心已关闭"
            case .Unavailable: return "本地核心不可用"
            }
        }
        var details: [String] = []
        var visited: Set<ObjectIdentifier> = []
        var retainedCauses: [NSError] = []
        var current: Error? = error
        var depth = 0
        while let cause = current {
            guard depth < 16 else { details.append("原因链超过 16 层，已停止展开。"); break }
            let native = cause as NSError
            guard visited.insert(ObjectIdentifier(native)).inserted else { details.append("原因链出现循环，已停止展开。"); break }
            retainedCauses.append(native)
            details.append("\(String(reflecting: type(of: cause))) · \(native.domain)（\(native.code)）")
            if Mirror(reflecting: cause).displayStyle != .class { details.append(String(reflecting: cause)) }
            let localized = cause as? LocalizedError
            let description = localized?.errorDescription ?? native.localizedDescription
            details.append(description)
            if let reason = localized?.failureReason ?? native.localizedFailureReason { details.append("原因：" + reason) }
            if let recovery = localized?.recoverySuggestion ?? native.localizedRecoverySuggestion { details.append("建议：" + recovery) }
            current = native.userInfo[NSUnderlyingErrorKey] as? Error
            depth += 1
        }
        return withExtendedLifetime(retainedCauses) { details.joined(separator: "\n") }
    }

    private func logFailure(operation: String, localCorrelation: String?, error: Error, categoryOverride: String? = nil) {
        let detail = Self.fullFailureDetail(error)
        let local = localCorrelation.map { " localCorrelation=\($0)" } ?? ""
        if let diagnostic = error as? BindingError {
            switch diagnostic {
            case .Diagnostic(let kind, _, let code, let phase, let operationID):
                Self.logger.error("operation=\(operation, privacy: .public)\(local, privacy: .public) kind=\(Self.kindName(kind), privacy: .public) code=\(code, privacy: .public) phase=\(phase, privacy: .public) operationId=\(operationID, privacy: .public) detail=\(detail, privacy: .public)")
            default:
                Self.logger.error("operation=\(operation, privacy: .public)\(local, privacy: .public) category=\(categoryOverride ?? self.category(for: error), privacy: .public) detail=\(detail, privacy: .public)")
            }
        } else {
            Self.logger.error("operation=\(operation, privacy: .public)\(local, privacy: .public) category=\(categoryOverride ?? self.category(for: error), privacy: .public) detail=\(detail, privacy: .public)")
        }
    }

}
