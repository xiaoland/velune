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
    /// A read refused because the application is quitting.
    case quitting
    case rejected(String)
    case diagnostic(kind: String, detail: String, code: String, phase: String, operationID: String)

    var errorDescription: String? {
        switch self {
        case .operation(_, let cause): return cause.errorDescription
        case .unavailable: return "本地核心无法打开，请检查应用安装和配置目录后重试。"
        case .invalidHome: return "VELUNE_HOME 必须使用绝对目录路径；留空可使用默认目录。"
        case .incompatibleCore: return "应用与内嵌核心版本不匹配，请重新安装完整应用。"
        case .closed: return "本地核心已关闭，请重新打开应用。"
        case .quitting: return "应用正在退出。"
        case .rejected(let message): return message
        case .diagnostic(_, let detail, let code, let phase, let operationID): return "操作失败（诊断编号：\(operationID)，阶段：\(phase)，代码：\(code)）：\(detail)"
        }
    }
}

/// What an operation does: changed configuration is read again, and a read
/// not yet begun when quitting starts gives way, so the core closes after
/// the operation under way and the changes queued before it.
enum OperationEffect {
    case reads
    case writes
    case changesConfiguration
}

/// Typed UniFFI application boundary. The UI never constructs an action or
/// payload dictionary; generated records and methods are the ABI contract.
final class Transport: @unchecked Sendable {
    let configurationChanged = PassthroughSubject<Void, Never>()
    /// This Mac's Velune home; a remote device's transport keeps its interface state here too.
    let stateDirectory: URL
    /// Opens the application this transport talks to: this Mac's core, or a paired device.
    private let open: () throws -> VeluneApplication
    let telemetryResult: Result<ClientTelemetry, Error>
    private let ownsTelemetry: Bool
    private let lock = NSLock()
    private static let logger = Logger(subsystem: "local.velune", category: "transport")
    private var application: VeluneApplication?
    private var isClosed = false
    /// From the moment quitting starts until the core closes or refuses to; apart from `lock`, which an operation holds throughout.
    private let closingLock = NSLock()
    private var closingRequested = false
    private var isClosing: Bool { closingLock.withLock { closingRequested } }

    init(stateDirectory: URL, resourcesDirectory: URL) {
        self.stateDirectory = stateDirectory
        let result = Result { try ClientTelemetry(home: stateDirectory, platform: "macos", localLogs: true,
            developmentProfile: ProcessInfo.processInfo.environment["VELUNE_TELEMETRY_PROFILE"], runID: ProcessInfo.processInfo.environment["VELUNE_RUN_ID"]) }
        telemetryResult = result
        ownsTelemetry = true
        open = {
            let telemetry = try result.get(); try telemetry.start()
            return try VeluneApplication.openWithTelemetry(options: BindingOptions(homeDirectory: stateDirectory.path, resourcesDirectory: resourcesDirectory.path), telemetry: telemetry.binding)
        }
    }

    /// A paired device's application; the same named methods reach its core.
    init(stateDirectory: URL, telemetry: Result<ClientTelemetry, Error>, remote: @escaping () throws -> VeluneApplication) {
        self.stateDirectory = stateDirectory
        telemetryResult = telemetry; ownsTelemetry = false
        open = remote
    }

    func telemetryCoordinator() throws -> ClientTelemetry {
        let telemetry = try telemetryResult.get(); try telemetry.start(); return telemetry
    }

    /// The remote service of this Mac's own core, opened with it.
    func openRemoteEndpoint(_ options: BindingRemoteOptions) throws -> VeluneRemoteEndpoint {
        try withApplication("openRemoteEndpoint", .reads, .write) { try VeluneRemoteEndpoint.openWithTelemetry(options: options, application: $0, telemetry: telemetryCoordinator().binding) }
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

    func setConversationBrowserPreferences(_ preferences: ConversationBrowserPreferences) throws -> ConfigurationSnapshot { try withApplication("setConversationBrowserPreferences", .writes, .configurationWrite) { try $0.setConversationBrowserPreferences(preferences: preferences) } }
    func setStatusLine(_ items: [StatusLineItem]) throws -> [StatusLineItem] { try withApplication("setStatusLine", .changesConfiguration, .configurationWrite) { try $0.setStatusLine(items: items) } }
    func setOutlinePresentation(_ presentation: OutlinePresentation) throws -> OutlinePresentation { try withApplication("setOutlinePresentation", .changesConfiguration, .configurationWrite) { try $0.setOutlinePresentation(presentation: presentation) } }
    func setLinkedConversationOpening(_ opening: LinkedConversationOpening) throws -> LinkedConversationOpening { try withApplication("setLinkedConversationOpening", .changesConfiguration, .configurationWrite) { try $0.setLinkedConversationOpening(opening: opening) } }
    func setUsageDisplay(_ display: UsageDisplay) throws -> UsageDisplay { try withApplication("setUsageDisplay", .changesConfiguration, .configurationWrite) { try $0.setUsageDisplay(display: display) } }
    func setContextUsageDisplay(_ display: UsageDisplay) throws -> UsageDisplay { try withApplication("setContextUsageDisplay", .changesConfiguration, .configurationWrite) { try $0.setContextUsageDisplay(display: display) } }
    func composerDrafts() throws -> [ComposerDraft] { try withApplication("composerDrafts", .reads, .conversationRead) { try $0.composerDrafts() } }
    func setComposerDraft(conversationID: String, text: String) throws { try withApplication("setComposerDraft", .writes, .conversationWrite) { try $0.setComposerDraft(conversationId: conversationID, text: text) } }
    func composerChoices() throws -> ComposerChoices { try withApplication("composerChoices", .reads, .configurationRead) { try $0.composerChoices() } }
    func setNextTurnIntent(_ intent: VeluneBindings.NextTurnIntent) throws { try withApplication("setNextTurnIntent", .writes, .configurationWrite) { try $0.setNextTurnIntent(intent: intent) } }
    func setRuntimeComposerChoices(_ choices: VeluneBindings.RuntimeComposerChoices) throws { try withApplication("setRuntimeComposerChoices", .writes, .configurationWrite) { try $0.setRuntimeComposerChoices(choices: choices) } }
    func setConversationBrowserLimits(_ limits: ConversationBrowserLimits) throws -> ConversationBrowserLimits { try withApplication("setConversationBrowserLimits", .changesConfiguration, .configurationWrite) { try $0.setConversationBrowserLimits(limits: limits) } }
    func organizeConversations(_ conversations: [ConversationReference], _ change: ConversationOrganizationChange) throws -> ConfigurationSnapshot { try withApplication("organizeConversations", .writes, .conversationWrite) { try $0.organizeConversations(conversations: conversations, change: change) } }
    func setProject(path: String, primaryFolder: String, name: String?, folders: [String]) throws -> ConfigurationSnapshot { try withApplication("setProject", .writes, .configurationWrite) { try $0.setProject(path: path, primaryFolder: primaryFolder, name: name, folders: folders) } }
    func loadMoreConversations(group: ConversationGroupKey) throws -> ConfigurationSnapshot { try withApplication("loadMoreConversations", .reads, .conversationRead) { try $0.loadMoreConversations(group: group) } }
    func showArchivedConversations(_ archived: Bool) throws -> ConfigurationSnapshot { try withApplication("showArchivedConversations", .writes, .conversationWrite) { try $0.showArchivedConversations(archived: archived) } }
    func searchConversations(_ text: String) throws -> ConversationSearch { try withApplication("searchConversations", .reads, .conversationRead) { try $0.searchConversations(text: text) } }
    func resolveAgentURI(_ uri: String, runtimeID: String? = nil) throws -> AgentUriResolution {
        try withApplication("resolveAgentUri", .reads, .conversationRead) { try $0.resolveAgentUri(uri: uri, runtimeId: runtimeID) }
    }
    func findConversation(_ id: String) throws -> ListedConversation? { try withApplication("findConversation", .reads, .conversationRead) { try $0.findConversation(conversationId: id) } }
    func runtimeDiscoveryHints(userHome: String, overrides: [String: String]) throws -> [RuntimeDiscoveryHint] { try withApplication("runtimeDiscoveryHints", .reads, .read) { try $0.runtimeDiscoveryHints(userHome: userHome, overrides: overrides) } }
    func discoverRuntimes(_ probes: [RuntimeDiscoveryProbe]) throws -> [RuntimeDiscoveryCandidate] { try withApplication("discoverRuntimes", .reads, .read) { try $0.discoverRuntimes(probes: probes) } }
    func analyticsQuery(_ query: AnalyticsQuery) throws -> AnalyticsReport { try withApplication("analyticsQuery", .reads, .read) { try $0.analyticsQuery(query: query) } }
    func listResourceTemplates() throws -> [ResourceTemplateSummary] { try withApplication("listResourceTemplates", .reads, .read) { try $0.listResourceTemplates() } }
    func readResourceTemplate(_ id: String) throws -> ResourceTemplate { try withApplication("readResourceTemplate", .reads, .read) { try $0.readResourceTemplate(templateId: id) } }
    func saveResourceTemplate(_ template: ResourceTemplate) throws -> ResourceSaveResult { try withApplication("saveResourceTemplate", .writes, .write) { try $0.saveResourceTemplate(template: template) } }
    func deleteResourceTemplate(_ id: String) throws { try withApplication("deleteResourceTemplate", .writes, .write) { try $0.deleteResourceTemplate(templateId: id) } }
    func importRuntimeResource(runtimeID: String, kind: RuntimeResourceKind, path: String) throws -> ResourceTemplateImport { try withApplication("importRuntimeResource", .writes, .write) { try $0.importRuntimeResource(runtimeId: runtimeID, kind: kind, path: path) } }
    func adoptRuntimeResource(runtimeID: String, kind: RuntimeResourceKind, path: String) throws -> ResourceTemplateSummary { try withApplication("adoptRuntimeResource", .writes, .write) { try $0.adoptRuntimeResource(runtimeId: runtimeID, kind: kind, path: path) } }
    func resourceSyncStatus() throws -> [ResourceSyncStatus] { try withApplication("resourceSyncStatus", .reads, .read) { try $0.resourceSyncStatus() } }
    func resourceDrift(runtimeID: String, templateID: String) throws -> [ResourceDriftFile] { try withApplication("resourceDrift", .reads, .read) { try $0.resourceDrift(runtimeId: runtimeID, templateId: templateID) } }
    func unmanagedRuntimeResources(_ kind: RuntimeResourceKind) throws -> [UnmanagedRuntimeResource] { try withApplication("unmanagedRuntimeResources", .reads, .read) { try $0.unmanagedRuntimeResources(kind: kind) } }
    func applyResourceTemplate(_ id: String, targets: [TemplateApplyTarget]) throws -> [TemplateApplyReport] { try withApplication("applyResourceTemplate", .writes, .write) { try $0.applyResourceTemplate(templateId: id, targets: targets) } }
    func resourceNativeImport(runtimeID: String, templateID: String) throws -> ResourceTemplateImport { try withApplication("resourceNativeImport", .writes, .write) { try $0.resourceNativeImport(runtimeId: runtimeID, templateId: templateID) } }
    func resolveResourceBinding(runtimeID: String, templateID: String, resolution: ResourceBindingResolution) throws { try withApplication("resolveResourceBinding", .writes, .write) { try $0.resolveResourceBinding(runtimeId: runtimeID, templateId: templateID, resolution: resolution) } }
    func list() throws -> ConfigurationSnapshot { try withApplication("list", .reads, .configurationRead) { try $0.list() } }
    func saveProvider(gatewayID: String, provider: ProviderDraft, authenticationEdit: AuthenticationEdit, quotaAuthenticationEdit: QuotaAuthenticationEdit? = nil) throws -> GatewayUpdate {
        try withApplication("saveProvider", .changesConfiguration, .configurationWrite) { try $0.saveProvider(gatewayId: gatewayID, provider: provider, authenticationEdit: authenticationEdit, quotaAuthenticationEdit: quotaAuthenticationEdit) }
    }
    func deleteProvider(gatewayID: String, providerID: String) throws -> GatewayUpdate { try withApplication("deleteProvider", .changesConfiguration, .configurationWrite) { try $0.deleteProvider(gatewayId: gatewayID, providerId: providerID) } }
    func saveModelRoute(gatewayID: String, route: ModelRoute) throws -> GatewayUpdate { try withApplication("saveModelRoute", .changesConfiguration, .configurationWrite) { try $0.saveModelRoute(gatewayId: gatewayID, route: route) } }
    func deleteModelRoute(gatewayID: String, recordKey: String) throws -> GatewayUpdate { try withApplication("deleteModelRoute", .changesConfiguration, .configurationWrite) { try $0.deleteModelRoute(gatewayId: gatewayID, routeRecordKey: recordKey) } }
    func composerQuota(_ request: ComposerQuotaRequest) throws -> [ComposerQuotaEntry] {
        try withApplication("composerQuota", .reads, .read) { application in
            try application.composerQuota(context: ComposerQuotaContext(runtimeId: request.runtimeID,
                selection: request.selection, conversationId: request.conversationID), refresh: false)
        }
    }
    func providerQuota(gatewayID: String, providerID: String, refresh: Bool) throws -> QuotaObservation {
        try withApplication(refresh ? "refreshProviderQuota" : "providerQuota", .reads, .read) { application in
            (refresh ? try application.refreshProviderQuota(gatewayId: gatewayID, providerId: providerID) : try application.providerQuota(gatewayId: gatewayID, providerId: providerID)).observation
        }
    }
    func providerQuotaHistory(gatewayID: String, providerID: String, since: Date) throws -> [QuotaHistorySample] {
        try withApplication("providerQuotaHistory", .reads, .read) { application in
            try application.providerQuotaHistory(gatewayId: gatewayID, providerId: providerID, sinceMs: Int64(since.timeIntervalSince1970 * 1000)).points.map(BindingMapping.quotaObservation)
        }
    }
    func clearProviderQuotaHistory(gatewayID: String, providerID: String) throws {
        _ = try withApplication("clearProviderQuotaHistory", .writes, .write) { try $0.clearProviderQuotaHistory(gatewayId: gatewayID, providerId: providerID) }
    }
    func quotaAlerts(since: UInt64) throws -> QuotaAlerts { try withApplication("quotaAlerts", .reads, .read) { try $0.quotaAlerts(since: since) } }
    func readProviderAPIKey(gatewayID: String, providerID: String) throws -> String { try withApplication("readProviderAPIKey", .reads, .read) { try $0.readProviderApiKey(gatewayId: gatewayID, providerId: providerID) } }
    func saveTemplate(_ value: ModelTemplate) throws -> [VeluneBindings.ModelTemplate] { try withApplication("saveModelTemplate", .changesConfiguration, .configurationWrite) { try $0.saveModelTemplate(template: value) } }
    func publicModelCatalog() throws -> [VeluneBindings.CatalogModel] { try withApplication("fetchPublicModelCatalog", .reads, .read) { try $0.fetchPublicModelCatalog() } }
    func deleteTemplate(id: String) throws -> [VeluneBindings.ModelTemplate] { try withApplication("deleteModelTemplate", .changesConfiguration, .configurationWrite) { try $0.deleteModelTemplate(id: id) } }
    func upsertRuntime(_ runtime: VeluneBindings.RuntimeInstance) throws -> RuntimeUpdate { try withApplication("upsertRuntime", .changesConfiguration, .configurationWrite) { try $0.upsertRuntime(runtime: runtime) } }
    func upsertRuntime(_ runtime: VeluneBindings.RuntimeInstance, execution: RuntimeExecution) throws -> RuntimeUpdate { try withApplication("upsertRuntime", .changesConfiguration, .configurationWrite) { try $0.upsertRuntimeWithExecution(runtime: runtime, execution: execution) } }
    /// The saved execution settings and the HOME they resolve to, if not inherited.
    func runtimeExecution(id: String) throws -> (RuntimeExecution, String?) { try withApplication("runtimeExecution", .reads, .configurationRead) { (try $0.runtimeExecution(runtimeId: id), try $0.runtimeHomeDirectory(runtimeId: id)) } }
    /// Saves an instance whose type takes launch options, with its arguments and environment.
    func upsertRuntime(_ runtime: VeluneBindings.RuntimeInstance, execution: RuntimeExecution, launch: AcpLaunch) throws -> RuntimeUpdate { try withApplication("upsertRuntime", .changesConfiguration, .configurationWrite) { try $0.upsertAcpRuntime(runtime: runtime, execution: execution, launch: launch) } }
    func runtimeLaunch(id: String) throws -> AcpLaunch { try withApplication("runtimeLaunch", .writes, .configurationRead) { try $0.acpLaunch(runtimeId: id) } }
    /// Starts the Agent to read what it reports about itself unless a session of it is live.
    func agentInfo(runtimeID: String) throws -> AcpRuntimeInfo { try withApplication("agentInfo", .reads, .configurationRead) { try $0.acpRuntimeInfo(runtimeId: runtimeID) } }
    func sessionOptions(runtimeID: String, conversationID: String) throws -> [AcpConfigOption] { try withApplication("sessionOptions", .reads, .configurationRead) { try $0.acpSessionConfig(runtimeId: runtimeID, conversationId: conversationID) } }
    func setSessionOption(runtimeID: String, conversationID: String, optionID: String, value: AcpConfigValue) throws -> [AcpConfigOption] { try withApplication("setSessionOption", .writes, .configurationWrite) { try $0.setAcpSessionConfig(runtimeId: runtimeID, conversationId: conversationID, configId: optionID, value: value) } }
    /// Where each instance runs; an instance whose saved execution cannot be read is left out.
    func runtimeLocations(ids: [String]) throws -> [String: RuntimeTransport] {
        try withApplication("runtimeLocations", .reads, .configurationRead) { application in
            ids.reduce(into: [:]) { result, id in result[id] = try? application.runtimeExecution(runtimeId: id).transport }
        }
    }
    func deleteRuntime(id: String) throws -> RuntimeUpdate { try withApplication("deleteRuntime", .changesConfiguration, .configurationWrite) { try $0.deleteRuntime(id: id) } }
    func selectRuntime(id: String) throws -> ConfigurationSnapshot { try withApplication("selectRuntime", .writes, .configurationWrite) { try $0.selectRuntime(id: id) } }
    /// Keep first creation and dispatch under the same shared application lock.
    /// Report the native identity before dispatch so failure never recreates a draft.
    func createAndSend(runtimeID: String, cwd: String?, selection: ModelSelection?, turn: ComposedTurn, onCreated: (SnapshotResult) -> Void) throws -> SnapshotResult {
        let selection = selection
        return try withApplication("sendTurn", .writes, .send) { application in
            let created = try application.createConversation(runtimeId: runtimeID, cwd: cwd, selection: selection)
            onCreated(created)
            guard let native = created.snapshot else { throw TransportError.rejected("运行时未返回新会话") }
            return try application.sendTurn(sourceRuntimeId: native.conversation.runtimeId, conversationId: native.conversation.id, targetRuntimeId: runtimeID, selection: selection, turn: turn)
        }
    }
    func openConversation(runtimeID: String, conversationID: String) throws -> SnapshotResult { try withApplication("openConversation", .reads, .conversationRead) { try $0.readConversation(runtimeId: runtimeID, conversationId: conversationID) } }
    func renameConversation(runtimeID: String, conversationID: String, title: String) throws -> ConfigurationSnapshot { try withApplication("renameConversation", .writes, .conversationWrite) { try $0.renameConversation(runtimeId: runtimeID, conversationId: conversationID, title: title) } }
    func deleteConversation(runtimeID: String, conversationID: String) throws -> ConfigurationSnapshot { try withApplication("deleteConversation", .writes, .conversationWrite) { try $0.deleteConversation(runtimeId: runtimeID, conversationId: conversationID) } }
    func snapshot(runtimeID: String, conversationID: String) throws -> SnapshotResult { try withApplication("snapshot", .reads, .conversationRead) { try $0.snapshot(runtimeId: runtimeID, conversationId: conversationID) } }
    func snapshotUpdate(runtimeID: String, conversationID: String, since: TranscriptCursor) throws -> SnapshotUpdate { try withApplication("snapshotUpdate", .reads, .conversationRead) { try $0.snapshotUpdate(runtimeId: runtimeID, conversationId: conversationID, since: since) } }
    func messagePayloads(runtimeID: String, conversationID: String, messageIDs: [String]) throws -> [DeliveredMessage] { try withApplication("messagePayloads", .reads, .conversationRead) { try $0.messagePayloads(runtimeId: runtimeID, conversationId: conversationID, messageIds: messageIDs) } }
    func sendTurn(sourceRuntimeID: String, conversationID: String, targetRuntimeID: String, selection: ModelSelection?, turn: ComposedTurn) throws -> SnapshotResult { try withApplication("sendTurn", .writes, .send) { try $0.sendTurn(sourceRuntimeId: sourceRuntimeID, conversationId: conversationID, targetRuntimeId: targetRuntimeID, selection: selection, turn: turn) } }
    /// The runtime's own models first, then the managed models; every entry
    /// carries the levels the runtime accepts for it.
    func runtimeModels(runtimeID: String) throws -> [RuntimeModel] { try withApplication("runtimeModels", .reads, .configurationRead) { try $0.runtimeModels(runtimeId: runtimeID) } }
    func steerTurn(runtimeID: String, conversationID: String, turn: ComposedTurn) throws -> SnapshotResult { try withApplication("steerTurn", .writes, .send) { try $0.steerTurn(runtimeId: runtimeID, conversationId: conversationID, turn: turn) } }
    func compactConversation(runtimeID: String, conversationID: String, selection: ModelSelection?) throws -> SnapshotResult {
        try withApplication("compactConversation", .writes, .compact) { try $0.compactConversation(runtimeId: runtimeID, conversationId: conversationID, selection: selection) }
    }
    func resumeTurn(runtimeID: String, conversationID: String, selection: ModelSelection?, effort: String?, permissionMode: String?) throws -> SnapshotResult {
        try withApplication("resumeTurn", .writes, .send) { try $0.resumeTurn(runtimeId: runtimeID, conversationId: conversationID, selection: selection, effort: effort, permissionMode: permissionMode) }
    }
    func sendQueue(runtimeID: String, conversationID: String, operation: SendQueueOperation) throws -> SnapshotResult { try withApplication("sendQueue", .writes, .conversationWrite) { try $0.sendQueue(runtimeId: runtimeID, conversationId: conversationID, operation: operation) } }
    func editMessage(runtimeID: String, conversationID: String, messageID: String, placement: EditPlacement, selection: ModelSelection?, turn: ComposedTurn) throws -> SnapshotResult { try withApplication("editMessage", .writes, .conversationWrite) { try $0.editMessage(runtimeId: runtimeID, conversationId: conversationID, messageId: messageID, placement: placement, selection: selection, turn: turn) } }
    func cancel(runtimeID: String, conversationID: String) throws -> SnapshotResult { try withApplication("cancel", .writes, .cancel) { try $0.cancel(runtimeId: runtimeID, conversationId: conversationID) } }
    func stopSubAgents(runtimeID: String, conversationID: String) throws -> SnapshotResult { try withApplication("stopSubAgents", .writes, .cancel) { try $0.stopSubAgents(runtimeId: runtimeID, conversationId: conversationID) } }
    func workingBranch(runtimeID: String, conversationID: String) throws -> String? { try withApplication("workingBranch", .reads, .conversationRead) { try $0.workingBranch(runtimeId: runtimeID, conversationId: conversationID) } }
    func workingChanges(runtimeID: String, conversationID: String) throws -> WorkingChanges { try withApplication("workingChanges", .reads, .conversationRead) { try $0.workingChanges(runtimeId: runtimeID, conversationId: conversationID) } }
    func fileDiff(runtimeID: String, conversationID: String, path: String) throws -> FileDiff { try withApplication("fileDiff", .reads, .conversationRead) { try $0.fileDiff(runtimeId: runtimeID, conversationId: conversationID, path: path) } }
    func conversationActivity(since: UInt64) throws -> ConversationActivity { try withApplication("conversationActivity", .reads, .conversationRead) { try $0.conversationActivity(since: since) } }
    func replyRuntimeInteraction(runtimeID: String, conversationID: String, interactionID: String, reply: RuntimeInteractionReply) throws -> SnapshotResult { try withApplication("replyRuntimeInteraction", .writes, .reply) { try $0.replyRuntimeInteraction(runtimeId: runtimeID, conversationId: conversationID, interactionId: interactionID, reply: reply) } }
    func providerImportPreview(gatewayID: String, source: VeluneBindings.ProviderImportSource) throws -> VeluneBindings.ProviderImportPreview { try withApplication("providerImportPreview", .reads, .configurationRead) { try $0.previewProviderImport(gatewayId: gatewayID, source: source) } }
    func providerImportApply(gatewayID: String, source: VeluneBindings.ProviderImportSource, previewToken: String, selections: [ImportSelection], replaceExisting: Bool) throws -> ImportResult { try withApplication("providerImportApply", .changesConfiguration, .configurationWrite) { try $0.applyProviderImport(gatewayId: gatewayID, source: source, previewToken: previewToken, selections: selections, replaceExisting: replaceExisting) } }
    func authenticationInspect(gatewayID: String, providerID: String) throws -> VeluneBindings.AuthenticationMetadata { try withApplication("authenticationInspect", .reads, .read) { try $0.authenticationInspect(gatewayId: gatewayID, providerId: providerID) } }
    func authenticationStart(gatewayID: String, providerID: String) throws -> AuthenticationProgress { try withApplication("authenticationStart", .writes, .write) { try $0.authenticationStart(gatewayId: gatewayID, providerId: providerID) } }
    func authenticationPoll() throws -> AuthenticationProgress { try withApplication("authenticationPoll", .reads, .read) { try $0.authenticationPoll() } }
    func authenticationReply(promptID: String, value: String) throws -> AuthenticationProgress { try withApplication("authenticationReply", .writes, .write) { try $0.authenticationReply(promptId: promptID, value: value) } }
    func authenticationCancel() throws -> AuthenticationProgress { try withApplication("authenticationCancel", .writes, .write) { try $0.authenticationCancel() } }
    func authenticationLogout(gatewayID: String, providerID: String) throws -> GatewayUpdate { try withApplication("authenticationLogout", .changesConfiguration, .configurationWrite) { try $0.authenticationLogout(gatewayId: gatewayID, providerId: providerID) } }

    /// Quitting starts: reads not yet begun are refused until `close` ends.
    func beginClosing() { closingLock.withLock { closingRequested = true } }

    /// `stoppingWork` ends running turns and sub-agents; otherwise they refuse it.
    func close(stoppingWork: Bool = false) throws {
        lock.lock(); defer { lock.unlock() }
        guard let application else {
            isClosed = true
            if ownsTelemetry, let telemetry = try? telemetryResult.get() { try telemetry.shutdown() }
            return
        }
        do {
            if stoppingWork { try application.shutdownStoppingWork() } else { try application.shutdown() }
            self.application = nil
            isClosed = true
            if ownsTelemetry { try telemetryResult.get().shutdown() }
        } catch {
            closingLock.withLock { closingRequested = false }
            logFailure(operation: "shutdown", localCorrelation: nil, error: error); throw TransportError.operation("shutdown", map(error))
        }
    }

    private func withApplication<T>(_ operation: String, _ effect: OperationEffect, _ category: NativeOperation, _ body: (VeluneApplication) throws -> T) throws -> T {
        let correlation = UUID().uuidString
        Self.logger.debug("operation=\(operation, privacy: .public) localCorrelation=\(correlation, privacy: .public) started")
        do {
            let value = try telemetryCoordinator().operation(category) { carrier in
                lock.lock(); defer { lock.unlock() }
                guard !isClosed else { throw TransportError.operation(operation, .closed) }
                guard effect != .reads || !isClosing else { throw TransportError.operation(operation, .quitting) }
                if application == nil { application = try open() }
                return try body(application!.withTraceContext(carrier: carrier))
            }
            if effect == .changesConfiguration { configurationChanged.send() }
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
            case .Diagnostic(let kind, _, _, _, _), .RemoteRequest(let kind, _, _, _): return Self.kindName(kind)
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
        case .RemoteRequest(let kind, let detail, let code, _):
            return .diagnostic(kind: kindName(kind), detail: detail, code: code, phase: "remote", operationID: "")
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
            case .Diagnostic(_, let detail, _, _, _), .RemoteRequest(_, let detail, _, _), .Invalid(let detail), .Unsupported(let detail), .Io(let detail), .Contract(let detail): return detail
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
