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

    func setConversationBrowserPreferences(_ preferences: BindingConversationBrowserPreferences) throws -> BindingConfigurationSnapshot { try withApplication("setConversationBrowserPreferences", changesConfiguration: true) { try $0.setConversationBrowserPreferences(preferences: preferences) } }
    func setStatusLine(_ items: [BindingStatusLineItem]) throws -> [BindingStatusLineItem] { try withApplication("setStatusLine", changesConfiguration: true) { try $0.setStatusLine(items: items) } }
    func setOutlinePresentation(_ presentation: BindingOutlinePresentation) throws -> BindingOutlinePresentation { try withApplication("setOutlinePresentation", changesConfiguration: true) { try $0.setOutlinePresentation(presentation: presentation) } }
    func composerDrafts() throws -> [BindingComposerDraft] { try withApplication("composerDrafts") { try $0.composerDrafts() } }
    func setComposerDraft(conversationID: String, text: String) throws { try withApplication("setComposerDraft") { try $0.setComposerDraft(conversationId: conversationID, text: text) } }
    func setConversationBrowserLimits(_ limits: BindingConversationBrowserLimits) throws -> BindingConversationBrowserLimits { try withApplication("setConversationBrowserLimits", changesConfiguration: true) { try $0.setConversationBrowserLimits(limits: limits) } }
    func organizeConversations(_ conversations: [BindingConversationReference], _ change: BindingConversationOrganizationChange) throws -> BindingConfigurationSnapshot { try withApplication("organizeConversations") { try $0.organizeConversations(conversations: conversations, change: change) } }
    func setProject(path: String, primaryFolder: String, name: String?, folders: [String]) throws -> BindingConfigurationSnapshot { try withApplication("setProject") { try $0.setProject(path: path, primaryFolder: primaryFolder, name: name, folders: folders) } }
    func loadMoreConversations(group: BindingConversationGroupKey) throws -> BindingConfigurationSnapshot { try withApplication("loadMoreConversations") { try $0.loadMoreConversations(group: group) } }
    func showArchivedConversations(_ archived: Bool) throws -> BindingConfigurationSnapshot { try withApplication("showArchivedConversations") { try $0.showArchivedConversations(archived: archived) } }
    func searchConversations(_ text: String) throws -> BindingConversationSearch { try withApplication("searchConversations") { try $0.searchConversations(text: text) } }
    func findConversation(_ id: String) throws -> BindingListedConversation? { try withApplication("findConversation") { try $0.findConversation(conversationId: id) } }
    func runtimeDiscoveryHints(userHome: String, overrides: [String: String]) throws -> [BindingRuntimeDiscoveryHint] { try withApplication("runtimeDiscoveryHints") { try $0.runtimeDiscoveryHints(userHome: userHome, overrides: overrides) } }
    func discoverRuntimes(_ probes: [BindingRuntimeDiscoveryProbe]) throws -> [BindingRuntimeDiscoveryCandidate] { try withApplication("discoverRuntimes") { try $0.discoverRuntimes(probes: probes) } }
    func analyticsQuery(_ query: BindingAnalyticsQuery) throws -> BindingAnalyticsReport { try withApplication("analyticsQuery") { try $0.analyticsQuery(query: query) } }
    func listResourceTemplates() throws -> [BindingResourceTemplateSummary] { try withApplication("listResourceTemplates") { try $0.listResourceTemplates() } }
    func readResourceTemplate(_ id: String) throws -> BindingResourceTemplate { try withApplication("readResourceTemplate") { try $0.readResourceTemplate(templateId: id) } }
    func saveResourceTemplate(_ template: BindingResourceTemplate) throws -> BindingResourceSaveResult { try withApplication("saveResourceTemplate") { try $0.saveResourceTemplate(template: template) } }
    func deleteResourceTemplate(_ id: String) throws { try withApplication("deleteResourceTemplate") { try $0.deleteResourceTemplate(templateId: id) } }
    func importRuntimeResource(runtimeID: String, kind: BindingRuntimeResourceKind, path: String) throws -> BindingResourceTemplateImport { try withApplication("importRuntimeResource") { try $0.importRuntimeResource(runtimeId: runtimeID, kind: kind, path: path) } }
    func adoptRuntimeResource(runtimeID: String, kind: BindingRuntimeResourceKind, path: String) throws -> BindingResourceTemplateSummary { try withApplication("adoptRuntimeResource") { try $0.adoptRuntimeResource(runtimeId: runtimeID, kind: kind, path: path) } }
    func resourceSyncStatus() throws -> [BindingResourceSyncStatus] { try withApplication("resourceSyncStatus") { try $0.resourceSyncStatus() } }
    func resourceDrift(runtimeID: String, templateID: String) throws -> [BindingResourceDriftFile] { try withApplication("resourceDrift") { try $0.resourceDrift(runtimeId: runtimeID, templateId: templateID) } }
    func unmanagedRuntimeResources(_ kind: BindingRuntimeResourceKind) throws -> [BindingUnmanagedRuntimeResource] { try withApplication("unmanagedRuntimeResources") { try $0.unmanagedRuntimeResources(kind: kind) } }
    func applyResourceTemplate(_ id: String, targets: [BindingTemplateApplyTarget]) throws -> [BindingTemplateApplyReport] { try withApplication("applyResourceTemplate") { try $0.applyResourceTemplate(templateId: id, targets: targets) } }
    func resourceNativeImport(runtimeID: String, templateID: String) throws -> BindingResourceTemplateImport { try withApplication("resourceNativeImport") { try $0.resourceNativeImport(runtimeId: runtimeID, templateId: templateID) } }
    func resolveResourceBinding(runtimeID: String, templateID: String, resolution: BindingResolution) throws { try withApplication("resolveResourceBinding") { try $0.resolveResourceBinding(runtimeId: runtimeID, templateId: templateID, resolution: resolution) } }
    func list() throws -> BindingConfigurationSnapshot { try withApplication("list") { try $0.list() } }
    func saveProvider(gatewayID: String, provider: AIProvider, authenticationEdit: AuthenticationEdit) throws -> BindingGatewayUpdate {
        try withApplication("saveProvider", changesConfiguration: true) { try $0.saveProvider(gatewayId: gatewayID, provider: BindingMapping.bindingProvider(provider), authenticationEdit: BindingMapping.bindingAuthenticationEdit(authenticationEdit)) }
    }
    func deleteProvider(gatewayID: String, providerID: String) throws -> BindingGatewayUpdate { try withApplication("deleteProvider", changesConfiguration: true) { try $0.deleteProvider(gatewayId: gatewayID, providerId: providerID) } }
    func saveModelRoute(gatewayID: String, route: ModelRoute) throws -> BindingGatewayUpdate { try withApplication("saveModelRoute", changesConfiguration: true) { try $0.saveModelRoute(gatewayId: gatewayID, route: BindingMapping.bindingRoute(route)) } }
    func deleteModelRoute(gatewayID: String, recordKey: String) throws -> BindingGatewayUpdate { try withApplication("deleteModelRoute", changesConfiguration: true) { try $0.deleteModelRoute(gatewayId: gatewayID, routeRecordKey: recordKey) } }
    func providerQuota(gatewayID: String, providerID: String, refresh: Bool) throws -> ProviderQuota {
        try withApplication(refresh ? "refreshProviderQuota" : "providerQuota") { application in
            BindingMapping.quota(refresh ? try application.refreshProviderQuota(gatewayId: gatewayID, providerId: providerID) : try application.providerQuota(gatewayId: gatewayID, providerId: providerID))
        }
    }
    func readProviderAPIKey(gatewayID: String, providerID: String) throws -> String { try withApplication("readProviderAPIKey") { try $0.readProviderApiKey(gatewayId: gatewayID, providerId: providerID) } }
    func saveTemplate(_ value: ModelTemplate) throws -> [BindingModelTemplate] { try withApplication("saveModelTemplate", changesConfiguration: true) { try $0.saveModelTemplate(template: BindingMapping.bindingTemplate(value)) } }
    func publicModelCatalog() throws -> [BindingCatalogModel] { try withApplication("fetchPublicModelCatalog") { try $0.fetchPublicModelCatalog() } }
    func deleteTemplate(id: String) throws -> [BindingModelTemplate] { try withApplication("deleteModelTemplate", changesConfiguration: true) { try $0.deleteModelTemplate(id: id) } }
    func upsertRuntime(_ runtime: BindingRuntimeInstance) throws -> BindingRuntimeUpdate { try withApplication("upsertRuntime", changesConfiguration: true) { try $0.upsertRuntime(runtime: runtime) } }
    func upsertRuntime(_ runtime: BindingRuntimeInstance, execution: BindingRuntimeExecution) throws -> BindingRuntimeUpdate { try withApplication("upsertRuntime", changesConfiguration: true) { try $0.upsertRuntimeWithExecution(runtime: runtime, execution: execution) } }
    /// The saved execution settings and the HOME they resolve to, if not inherited.
    func runtimeExecution(id: String) throws -> (BindingRuntimeExecution, String?) { try withApplication("runtimeExecution") { (try $0.runtimeExecution(runtimeId: id), try $0.runtimeHomeDirectory(runtimeId: id)) } }
    /// Saves an instance whose type takes launch options, with its arguments and environment.
    func upsertRuntime(_ runtime: BindingRuntimeInstance, execution: BindingRuntimeExecution, launch: BindingAcpLaunch) throws -> BindingRuntimeUpdate { try withApplication("upsertRuntime", changesConfiguration: true) { try $0.upsertAcpRuntime(runtime: runtime, execution: execution, launch: launch) } }
    func runtimeLaunch(id: String) throws -> BindingAcpLaunch { try withApplication("runtimeLaunch") { try $0.acpLaunch(runtimeId: id) } }
    /// Starts the Agent to read what it reports about itself unless a session of it is live.
    func agentInfo(runtimeID: String) throws -> BindingAcpRuntimeInfo { try withApplication("agentInfo") { try $0.acpRuntimeInfo(runtimeId: runtimeID) } }
    func sessionOptions(runtimeID: String, conversationID: String) throws -> [BindingAcpConfigOption] { try withApplication("sessionOptions") { try $0.acpSessionConfig(runtimeId: runtimeID, conversationId: conversationID) } }
    func setSessionOption(runtimeID: String, conversationID: String, optionID: String, value: BindingAcpConfigValue) throws -> [BindingAcpConfigOption] { try withApplication("setSessionOption") { try $0.setAcpSessionConfig(runtimeId: runtimeID, conversationId: conversationID, configId: optionID, value: value) } }
    /// Where each instance runs; an instance whose saved execution cannot be read is left out.
    func runtimeLocations(ids: [String]) throws -> [String: BindingRuntimeTransport] {
        try withApplication("runtimeLocations") { application in
            ids.reduce(into: [:]) { result, id in result[id] = try? application.runtimeExecution(runtimeId: id).transport }
        }
    }
    func deleteRuntime(id: String) throws -> BindingRuntimeUpdate { try withApplication("deleteRuntime", changesConfiguration: true) { try $0.deleteRuntime(id: id) } }
    func selectRuntime(id: String) throws -> BindingConfigurationSnapshot { try withApplication("selectRuntime") { try $0.selectRuntime(id: id) } }
    /// Keep first creation and dispatch under the same shared application lock.
    /// Report the native identity before dispatch so failure never recreates a draft.
    func createAndSend(runtimeID: String, cwd: String?, selection: ModelSelection, turn: BindingComposedTurn, onCreated: (BindingSnapshotResult) -> Void) throws -> BindingSnapshotResult {
        let selection = BindingMapping.bindingSelection(selection)
        return try withApplication("sendTurn") { application in
            let created = try application.createConversationWithSelection(runtimeId: runtimeID, cwd: cwd, selection: selection)
            onCreated(created)
            guard let native = created.snapshot else { throw TransportError.rejected("运行时未返回新会话") }
            return try application.sendTurnWithSelection(sourceRuntimeId: native.conversation.runtimeId, conversationId: native.conversation.id, targetRuntimeId: runtimeID, selection: selection, turn: turn)
        }
    }
    func openConversation(runtimeID: String, conversationID: String) throws -> BindingSnapshotResult { try withApplication("openConversation") { try $0.readConversation(runtimeId: runtimeID, conversationId: conversationID) } }
    func renameConversation(runtimeID: String, conversationID: String, title: String) throws -> BindingConfigurationSnapshot { try withApplication("renameConversation") { try $0.renameConversation(runtimeId: runtimeID, conversationId: conversationID, title: title) } }
    func deleteConversation(runtimeID: String, conversationID: String) throws -> BindingConfigurationSnapshot { try withApplication("deleteConversation") { try $0.deleteConversation(runtimeId: runtimeID, conversationId: conversationID) } }
    func snapshot(runtimeID: String, conversationID: String) throws -> BindingSnapshotResult { try withApplication("snapshot") { try $0.snapshot(runtimeId: runtimeID, conversationId: conversationID) } }
    func snapshotUpdate(runtimeID: String, conversationID: String, since: TranscriptCursor) throws -> BindingSnapshotUpdate { try withApplication("snapshotUpdate") { try $0.snapshotUpdate(runtimeId: runtimeID, conversationId: conversationID, since: BindingMapping.bindingCursor(since)) } }
    func messagePayloads(runtimeID: String, conversationID: String, messageIDs: [String]) throws -> [BindingMessage] { try withApplication("messagePayloads") { try $0.messagePayloads(runtimeId: runtimeID, conversationId: conversationID, messageIds: messageIDs) } }
    func sendTurn(sourceRuntimeID: String, conversationID: String, targetRuntimeID: String, selection: ModelSelection, turn: BindingComposedTurn) throws -> BindingSnapshotResult { try withApplication("sendTurn") { try $0.sendTurnWithSelection(sourceRuntimeId: sourceRuntimeID, conversationId: conversationID, targetRuntimeId: targetRuntimeID, selection: BindingMapping.bindingSelection(selection), turn: turn) } }
    /// The runtime's own models first, then the managed models; every entry
    /// carries the levels the runtime accepts for it.
    func runtimeModels(runtimeID: String) throws -> [BindingRuntimeModel] { try withApplication("runtimeModels") { try $0.runtimeModels(runtimeId: runtimeID) } }
    func steerTurn(runtimeID: String, conversationID: String, turn: BindingComposedTurn) throws -> BindingSnapshotResult { try withApplication("steerTurn") { try $0.steerTurn(runtimeId: runtimeID, conversationId: conversationID, turn: turn) } }
    func sendQueue(runtimeID: String, conversationID: String, operation: BindingSendQueueOperation) throws -> BindingSnapshotResult { try withApplication("sendQueue") { try $0.sendQueue(runtimeId: runtimeID, conversationId: conversationID, operation: operation) } }
    func editMessage(runtimeID: String, conversationID: String, messageID: String, placement: BindingEditPlacement, selection: ModelSelection, turn: BindingComposedTurn) throws -> BindingSnapshotResult { try withApplication("editMessage") { try $0.editMessage(runtimeId: runtimeID, conversationId: conversationID, messageId: messageID, placement: placement, selection: BindingMapping.bindingSelection(selection), turn: turn) } }
    func cancel(runtimeID: String, conversationID: String) throws -> BindingSnapshotResult { try withApplication("cancel") { try $0.cancel(runtimeId: runtimeID, conversationId: conversationID) } }
    func stopSubAgents(runtimeID: String, conversationID: String) throws -> BindingSnapshotResult { try withApplication("stopSubAgents") { try $0.stopSubAgents(runtimeId: runtimeID, conversationId: conversationID) } }
    func workingBranch(runtimeID: String, conversationID: String) throws -> String? { try withApplication("workingBranch") { try $0.workingBranch(runtimeId: runtimeID, conversationId: conversationID) } }
    func workingChanges(runtimeID: String, conversationID: String) throws -> BindingWorkingChanges { try withApplication("workingChanges") { try $0.workingChanges(runtimeId: runtimeID, conversationId: conversationID) } }
    func fileDiff(runtimeID: String, conversationID: String, path: String) throws -> BindingFileDiff { try withApplication("fileDiff") { try $0.fileDiff(runtimeId: runtimeID, conversationId: conversationID, path: path) } }
    func conversationActivity(since: UInt64) throws -> BindingConversationActivity { try withApplication("conversationActivity") { try $0.conversationActivity(since: since) } }
    func replyRuntimeInteraction(runtimeID: String, conversationID: String, interactionID: String, reply: RuntimeInteractionReply) throws -> BindingSnapshotResult { try withApplication("replyRuntimeInteraction") { try $0.replyRuntimeInteraction(runtimeId: runtimeID, conversationId: conversationID, interactionId: interactionID, reply: BindingMapping.bindingInteractionReply(reply)) } }
    func providerImportPreview(gatewayID: String, source: BindingProviderImportSource) throws -> BindingProviderImportPreview { try withApplication("providerImportPreview") { try $0.previewProviderImport(gatewayId: gatewayID, source: source) } }
    func providerImportApply(gatewayID: String, source: BindingProviderImportSource, previewToken: String, selections: [BindingImportSelection], replaceExisting: Bool) throws -> BindingImportResult { try withApplication("providerImportApply", changesConfiguration: true) { try $0.applyProviderImport(gatewayId: gatewayID, source: source, previewToken: previewToken, selections: selections, replaceExisting: replaceExisting) } }
    func authenticationInspect(gatewayID: String, providerID: String) throws -> BindingAuthenticationMetadata { try withApplication("authenticationInspect") { try $0.authenticationInspect(gatewayId: gatewayID, providerId: providerID) } }
    func authenticationStart(gatewayID: String, providerID: String) throws -> BindingAuthenticationProgress { try withApplication("authenticationStart") { try $0.authenticationStart(gatewayId: gatewayID, providerId: providerID) } }
    func authenticationPoll() throws -> BindingAuthenticationProgress { try withApplication("authenticationPoll") { try $0.authenticationPoll() } }
    func authenticationReply(promptID: String, value: String) throws -> BindingAuthenticationProgress { try withApplication("authenticationReply") { try $0.authenticationReply(promptId: promptID, value: value) } }
    func authenticationCancel() throws -> BindingAuthenticationProgress { try withApplication("authenticationCancel") { try $0.authenticationCancel() } }
    func authenticationLogout(gatewayID: String, providerID: String) throws -> BindingGatewayUpdate { try withApplication("authenticationLogout", changesConfiguration: true) { try $0.authenticationLogout(gatewayId: gatewayID, providerId: providerID) } }

    /// `stoppingWork` ends running turns and sub-agents; otherwise they refuse it.
    func close(stoppingWork: Bool = false) throws {
        lock.lock(); defer { lock.unlock() }
        guard let application else { isClosed = true; return }
        do {
            if stoppingWork { try application.shutdownStoppingWork() } else { try application.shutdown() }
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
            case .RemoteRequest(let kind, _, _): return Self.kindName(kind)
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
        case .RemoteRequest(let kind, let detail, let requestID):
            let operationID = requestID.map { "\($0.incarnation)/\($0.client)/\($0.sequence)" } ?? ""
            return .diagnostic(kind: kindName(kind), detail: detail, code: kindName(kind), phase: "remote", operationID: operationID)
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
        case .runtimeManagement: return "runtime_management"
        case .conflict: return "conflict"
        case .unauthorized: return "unauthorized"
        case .connection: return "connection"
        case .outcomeUnknown: return "outcome_unknown"
        }
    }

    private static func fullFailureDetail(_ error: Error) -> String {
        if let error = error as? BindingError {
            switch error {
            case .Diagnostic(_, let detail, _, _, _), .Invalid(let detail), .Unsupported(let detail), .Io(let detail), .Contract(let detail): return detail
            case .RemoteRequest(_, let detail, _): return detail
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
