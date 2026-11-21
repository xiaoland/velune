import Foundation
import OSLog
import Combine
import VeluneBindings

@MainActor enum MacNativePersistenceGate {
    private(set) static var isEnabled = false
    static func close() { isEnabled = false }
    static func open() { isEnabled = true }
}

enum TransportError: LocalizedError {
    indirect case operation(String, TransportError)
    case unavailable
    case invalidHome
    case incompatibleCore
    case closed
    /// A read refused because the application is quitting.
    case quitting
    case submissionCancelled
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
        case .submissionCancelled: return "发送已取消。"
        case .rejected(let message): return message
        case .diagnostic(_, let detail, let code, let phase, let operationID):
            let id = operationID.isEmpty ? "" : "诊断编号：\(operationID)，"
            return "操作失败（\(id)阶段：\(phase)，代码：\(code)）：\(detail)"
        }
    }

    var isSubmissionCancelled: Bool {
        switch self {
        case .submissionCancelled: return true
        case .operation(_, let cause): return cause.isSubmissionCancelled
        case .diagnostic(_, _, let code, _, _): return code == "submission_cancelled"
        default: return false
        }
    }
}

/// Business calls capture a shared backend handle under the lifecycle lock,
/// then wait outside it. The core owns command and shutdown ordering; a long
/// write receipt must not block another conversation's reads or control calls.
/// Closing refuses new admissions, but does not retract previously captured
/// calls or turn an unknown mutation result into a safe-to-retry rejection.
enum OperationEffect {
    case reads
    /// Browsing waits independently of other conversations' operations.
    case browsesConversation
    /// Preparation can wait for native history while independent conversations
    /// continue. Its persistent target initialization uses the core's CAS.
    case preparesComposer
    /// Handoff checks and owned summary jobs admit independent handles. Their
    /// source CAS and job state remain owned by the application; cancellation
    /// must not queue behind an unrelated long-running frontend write lock.
    case preparesContinuation
    case writes
    case changesConfiguration
}

/// Typed UniFFI application boundary. The UI never constructs an action or
/// payload dictionary; generated records and methods are the ABI contract.
final class Transport: @unchecked Sendable {
    let configurationChanged = PassthroughSubject<Void, Never>()
    /// This Mac's Velune home; a remote device's transport keeps its interface state here too.
    let stateDirectory: URL
    private let remoteAttachments: Bool
    /// Opens the application this transport talks to: this Mac's core, or a paired device.
    private let open: () throws -> VeluneApplication
    let telemetryResult: Result<ClientTelemetry, Error>
    private let ownsTelemetry: Bool
    private let lock = NSLock()
    private static let logger = Logger(subsystem: "local.velune", category: "transport")
    private var application: VeluneApplication?
    private let observationStateLock = NSLock()
    private let observationSubmissionLock = NSLock()
    private var desiredObservations: [ObservedConversation] = []
    private var submittedObservations: [ObservedConversation]?
    private var observationPending = false
    private var observationsDirty = true
    private var observationFailureLogged = false
    /// A failed open is retried at most once per interval; operations in between
    /// fail with the recorded reason instead of each opening and logging anew.
    private static let openRetryInterval: TimeInterval = 15
    private var openFailure: (error: Error, at: Date)?
    private struct CachedOpenFailure: Error { let error: Error }
    private var isClosed = false
    /// From the moment quitting starts until the core closes or refuses to.
    private let closingLock = NSLock()
    private var closingRequested = false
    private var isClosing: Bool { closingLock.withLock { closingRequested } }

    init(stateDirectory: URL, resourcesDirectory: URL) {
        self.stateDirectory = stateDirectory
        remoteAttachments = false
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
    init(stateDirectory: URL, telemetry: Result<ClientTelemetry, Error>, composerAuthority: Transport? = nil, remote: @escaping () throws -> VeluneApplication) {
        self.stateDirectory = stateDirectory
        remoteAttachments = true
        telemetryResult = telemetry; ownsTelemetry = false
        open = {
            let application = try remote()
            guard let composerAuthority else { return application }
            let authority = try composerAuthority.withApplication("composerAuthority", .reads, .conversationRead) { $0 }
            return try application.withComposerAuthority(authority: authority)
        }
    }

    /// An independent browser over the same device and application owner.
    /// Closing it releases only this presentation view and never the runtime.
    private init(source: Transport, viewID: String) {
        stateDirectory = source.stateDirectory
        remoteAttachments = source.remoteAttachments
        telemetryResult = source.telemetryResult
        ownsTelemetry = false
        open = {
            try source.withApplication("openClientView", .reads, .configurationRead) {
                try $0.withClientView(viewId: viewID)
            }
        }
    }

    func clientView(_ id: String) -> Transport { Transport(source: self, viewID: id) }

    func telemetryCoordinator() throws -> ClientTelemetry {
        let telemetry = try telemetryResult.get(); try telemetry.start(); return telemetry
    }

    /// The remote service of this Mac's own core, opened with it.
    func openRemoteEndpoint(_ options: BindingRemoteOptions) throws -> VeluneRemoteEndpoint {
        try withApplication("openRemoteEndpoint", .reads, .write) { try VeluneRemoteEndpoint.openWithTelemetry(options: options, application: $0, telemetry: telemetryCoordinator().binding) }
    }

    static func applicationDefault() throws -> Transport {
        let context = try MacExecutionContext.application()
        guard let resources = Bundle.main.resourceURL else { throw TransportError.unavailable }
        return Transport(stateDirectory: context.stateDirectory, resourcesDirectory: resources)
    }

    /// Open the local core before native preference and scene restoration reads.
    /// Application::open acquires and retains the same home lock used by later calls.
    func prepareForLaunch() throws {
        try withApplication("prepareForLaunch", .reads, .read) { _ in () }
    }

    func setConversationBrowserPreferences(_ preferences: ConversationBrowserPreferences) throws -> ConfigurationSnapshot { try withApplication("setConversationBrowserPreferences", .writes, .configurationWrite) { try $0.setConversationBrowserPreferences(preferences: preferences) } }
    func setStatusLine(_ items: [StatusLineItem]) throws -> [StatusLineItem] { try withApplication("setStatusLine", .changesConfiguration, .configurationWrite) { try $0.setStatusLine(items: items) } }
    func setOutlinePresentation(_ presentation: OutlinePresentation) throws -> OutlinePresentation { try withApplication("setOutlinePresentation", .changesConfiguration, .configurationWrite) { try $0.setOutlinePresentation(presentation: presentation) } }
    func setLinkedConversationOpening(_ opening: LinkedConversationOpening) throws -> LinkedConversationOpening { try withApplication("setLinkedConversationOpening", .changesConfiguration, .configurationWrite) { try $0.setLinkedConversationOpening(opening: opening) } }
    func setUsageDisplay(_ display: UsageDisplay) throws -> UsageDisplay { try withApplication("setUsageDisplay", .changesConfiguration, .configurationWrite) { try $0.setUsageDisplay(display: display) } }
    func setContextUsageDisplay(_ display: UsageDisplay) throws -> UsageDisplay { try withApplication("setContextUsageDisplay", .changesConfiguration, .configurationWrite) { try $0.setContextUsageDisplay(display: display) } }
    func composerInputs(_ operation: ComposerInputOperation) throws -> ComposerInputUpdate {
        let reading: Bool = if case .read = operation { true } else { false }
        return try withApplication("composerInputs", reading ? .reads : .writes, reading ? .conversationRead : .conversationWrite) { try $0.manageComposerInputs(operation: operation) }
    }
    func prepareComposerTarget(_ request: ComposerTargetRequest, overlayChoices: ComposerTargetChoices?) throws -> PreparedComposerTarget {
        try withApplication("prepareComposerTarget", .preparesComposer, .conversationRead) { try $0.prepareComposerTarget(request: request, overlayChoices: overlayChoices) }
    }
    func composerOwnerID() throws -> String { try withApplication("composerOwnerID", .reads, .conversationRead) { try $0.composerOwnerId() } }
    func uploadComposerAttachment(name: String, bytes: Data) throws -> ComposerAttachment { try withApplication("uploadComposerAttachment", .writes, .conversationWrite) { try $0.uploadComposerAttachment(name: name, bytes: bytes) } }
    func readComposerAttachment(_ id: String) throws -> Data { try withApplication("readComposerAttachment", .reads, .conversationRead) { try $0.readComposerAttachment(attachmentId: id).bytes } }
    func composerChoices() throws -> ComposerChoices { try withApplication("composerChoices", .reads, .configurationRead) { try $0.composerChoices() } }
    func setRuntimeComposerChoices(_ choices: VeluneBindings.RuntimeComposerChoices) throws { try withApplication("setRuntimeComposerChoices", .writes, .configurationWrite) { try $0.setRuntimeComposerChoices(choices: choices) } }
    func setConversationBrowserLimits(_ limits: ConversationBrowserLimits) throws -> ConversationBrowserLimits { try withApplication("setConversationBrowserLimits", .changesConfiguration, .configurationWrite) { try $0.setConversationBrowserLimits(limits: limits) } }
    func organizeConversations(_ conversations: [ConversationReference], _ change: ConversationOrganizationChange) throws -> ConfigurationSnapshot { try withApplication("organizeConversations", .writes, .conversationWrite) { try $0.organizeConversations(conversations: conversations, change: change) } }
    func setProject(path: String, primaryFolder: String, name: String?, folders: [String], basis: ConversationProject? = nil) throws -> ConfigurationSnapshot {
        try withApplication("setProject", .writes, .configurationWrite) { application in
            // A fresh remote proxy carries the revision of this read. The
            // shared browsing proxy may predate intervening conversation work.
            let editor = remoteAttachments ? try open() : application
            defer { if remoteAttachments { try? editor.shutdown() } }
            let current = try editor.list()
            if let basis, current.conversationProjects.first(where: { $0.path == path }) != basis {
                throw TransportError.rejected("项目已在其他窗口或设备中修改。请保留当前输入，重新打开项目后核对再存储。")
            }
            return try editor.setProject(path: path, primaryFolder: primaryFolder, name: name, folders: folders)
        }
    }
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
    func providerAccount(gatewayID: String, providerID: String, refresh: Bool) throws -> ProviderAccountStatus {
        try withApplication("providerAccount", .reads, .read) { try $0.providerAccount(gatewayId: gatewayID, providerId: providerID, refresh: refresh) }
    }
    func providerAnalytics(refresh: Bool) throws -> ProviderAnalyticsReport {
        try withApplication("providerAnalytics", .reads, .read) { try $0.providerAnalytics(refresh: refresh) }
    }
    func providerBilling(gatewayID: String, providerID: String, refresh: Bool) throws -> ProviderBillingStatus {
        try withApplication("providerBilling", .reads, .read) { try $0.providerBilling(gatewayId: gatewayID, providerId: providerID, refresh: refresh) }
    }
    func saveProviderBilling(gatewayID: String, providerID: String, configuration: BillingConfiguration?, authenticationEdit: BillingAuthenticationEdit, confirmed: Bool) throws -> ProviderBillingStatus {
        try withApplication("saveProviderBilling", .changesConfiguration, .configurationWrite) { try $0.saveProviderBilling(gatewayId: gatewayID, providerId: providerID, configuration: configuration, authenticationEdit: authenticationEdit, confirmed: confirmed) }
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
    /// Keep dependent creation and dispatch on the same captured application.
    /// Publish the native identity first: closing or a failed dispatch must not
    /// erase successful creation or make a retry create another conversation.
    func createAndSend(runtimeID: String, cwd: String?, project: String? = nil, selection: ModelSelection?, turn: ComposedTurn, cancellation: VeluneSendCancellation? = nil, submissionId: String? = nil, onCreated: (SnapshotResult) -> Void) throws -> SnapshotResult {
        let selection = selection
        return try withApplication("sendTurn", .writes, .send) { application in
            try checkCancellation(cancellation)
            let created: SnapshotResult
            if let project { created = try application.createConversationInProject(runtimeId: runtimeID, project: project, selection: selection) }
            else { created = try application.createConversation(runtimeId: runtimeID, cwd: cwd, selection: selection) }
            onCreated(created)
            guard let native = created.snapshot else { throw TransportError.rejected("运行时未返回新会话") }
            return try dispatchTurn(application, sourceRuntimeID: native.conversation.runtimeId, conversationID: native.conversation.id, targetRuntimeID: runtimeID, selection: selection, turn: turn, cancellation: cancellation, submissionId: submissionId)
        }
    }
    func openConversation(runtimeID: String, conversationID: String) throws -> SnapshotResult { try withApplication("openConversation", .browsesConversation, .conversationRead) { try $0.readConversation(runtimeId: runtimeID, conversationId: conversationID) } }
    func refreshConversation(runtimeID: String, conversationID: String) throws -> SnapshotResult { try withApplication("refreshConversation", .browsesConversation, .conversationRead) { try $0.openConversation(runtimeId: runtimeID, conversationId: conversationID) } }
    func renameConversation(runtimeID: String, conversationID: String, title: String) throws -> ConfigurationSnapshot { try withApplication("renameConversation", .writes, .conversationWrite) { try $0.renameConversation(runtimeId: runtimeID, conversationId: conversationID, title: title) } }
    func deleteConversation(runtimeID: String, conversationID: String) throws -> ConfigurationSnapshot { try withApplication("deleteConversation", .writes, .conversationWrite) { try $0.deleteConversation(runtimeId: runtimeID, conversationId: conversationID) } }
    func scheduledTasks(_ operation: ScheduledTasksOperation) throws -> ScheduledTasksState {
        switch operation {
        case .list, .previewImport, .previewSchedule:
            return try withApplication("scheduledTasks", .reads, .read) { try $0.scheduledTasks(operation: operation) }
        default:
            return try withApplication("scheduledTasks", .writes, .write) { try $0.scheduledTasks(operation: operation) }
        }
    }
    /// Visibility is aggregated on the main actor. Only this complete set may
    /// replace the client view; snapshot readers never submit one pane's subset.
    func observeConversations(_ conversations: [ObservedConversation]) {
        let shouldSubmit = observationStateLock.withLock {
            if desiredObservations != conversations { desiredObservations = conversations; observationsDirty = true }
            guard observationsDirty, !observationPending else { return false }
            observationPending = true; return true
        }
        guard shouldSubmit else { return }
        DispatchQueue.global(qos: .utility).async { [self] in
            while true {
                do {
                    let published = try withApplication("observedConversations", .reads, .conversationRead) { synchronizeObservations($0) }
                    if !published {
                        observationStateLock.withLock { observationPending = false }
                        return
                    }
                } catch {
                    // The next visibility/poll tick retries; a failed core must
                    // not turn this into an unbounded immediate retry loop.
                    observationStateLock.withLock { observationPending = false }
                    return
                }
                let again = observationStateLock.withLock {
                    if observationsDirty { return true }
                    observationPending = false; return false
                }
                if !again { return }
            }
        }
    }
    @discardableResult
    private func synchronizeObservations(_ application: VeluneApplication) -> Bool {
        observationSubmissionLock.lock(); defer { observationSubmissionLock.unlock() }
        let desired = observationStateLock.withLock { desiredObservations }
        if submittedObservations != desired {
            do {
                try application.setObservedConversations(conversations: desired)
                submittedObservations = desired
                observationFailureLogged = false
            } catch {
                // Cache hints are optional across mixed-version remote peers.
                // Keep the desired set dirty, but never prevent the real read.
                observationStateLock.withLock { observationsDirty = true }
                if !observationFailureLogged {
                    observationFailureLogged = true
                    logFailure(operation: "observedConversations", localCorrelation: nil, error: error)
                }
                return false
            }
        }
        observationStateLock.withLock { observationsDirty = desiredObservations != desired }
        return true
    }
    func snapshot(runtimeID: String, conversationID: String) throws -> SnapshotResult {
        try withApplication("snapshot", .reads, .conversationRead) {
            synchronizeObservations($0)
            return try $0.snapshot(runtimeId: runtimeID, conversationId: conversationID)
        }
    }
    func snapshotUpdate(runtimeID: String, conversationID: String, since: TranscriptCursor) throws -> SnapshotUpdate {
        try withApplication("snapshotUpdate", .reads, .conversationRead) {
            synchronizeObservations($0)
            return try $0.snapshotUpdate(runtimeId: runtimeID, conversationId: conversationID, since: since)
        }
    }
    func messagePayloads(runtimeID: String, conversationID: String, messageIDs: [String]) throws -> [DeliveredMessage] { try withApplication("messagePayloads", .reads, .conversationRead) { try $0.messagePayloads(runtimeId: runtimeID, conversationId: conversationID, messageIds: messageIDs) } }
    func readAttachment(runtimeID: String, conversationID: String, messageID: String, block: UInt32) throws -> AttachmentContent { try withApplication("readAttachment", .reads, .conversationRead) { try $0.readAttachment(runtimeId: runtimeID, conversationId: conversationID, messageId: messageID, blockIndex: block) } }
    func sendTurn(sourceRuntimeID: String, conversationID: String, targetRuntimeID: String, selection: ModelSelection?, turn: ComposedTurn, cancellation: VeluneSendCancellation? = nil, submissionId: String? = nil) throws -> SnapshotResult {
        try withApplication("sendTurn", .writes, .send) {
            try dispatchTurn($0, sourceRuntimeID: sourceRuntimeID, conversationID: conversationID, targetRuntimeID: targetRuntimeID, selection: selection, turn: turn, cancellation: cancellation, submissionId: submissionId)
        }
    }
    func handoffRequirements(sourceRuntimeID: String, conversationID: String, targetRuntimeID: String, requestID: String?) throws -> HandoffRequirements {
        try withApplication("handoffRequirements", .preparesContinuation, .conversationRead) {
            try $0.readHandoffRequirements(sourceRuntimeId: sourceRuntimeID, conversationId: conversationID,
                                           targetRuntimeId: targetRuntimeID, requestId: requestID)
        }
    }
    func beginHandoffSummary(sourceRuntimeID: String, conversationID: String, targetRuntimeID: String, model: ModelSelection?, effort: String?) throws -> HandoffSummaryStart {
        try withApplication("beginHandoffSummary", .preparesContinuation, .conversationWrite) {
            try $0.beginHandoffSummary(sourceRuntimeId: sourceRuntimeID, conversationId: conversationID,
                                      targetRuntimeId: targetRuntimeID, summaryModel: model, effort: effort)
        }
    }
    func queueHandoffSummary(sourceRuntimeID: String, conversationID: String, targetRuntimeID: String,
                             selection: ModelSelection?, turn: ComposedTurn, model: ModelSelection?, effort: String?, submissionId: String? = nil) throws -> QueuedHandoffSummaryStart {
        try withApplication("queueHandoffSummary", .preparesContinuation, .send) { application in
            let delivered = try deliver(turn, using: application)
            let request = BindingQueuedHandoffSummary(sourceRuntimeId: sourceRuntimeID,
                conversationId: conversationID, targetRuntimeId: targetRuntimeID, selection: selection,
                turn: delivered, summaryModel: model, effort: effort)
            return try application.queueHandoffSummary(request: request, submissionId: submissionId)
        }
    }
    func retryQueuedHandoffSummary(sourceRuntimeID: String, conversationID: String, queuedMessageID: String,
                                   model: ModelSelection?, effort: String?) throws -> QueuedHandoffSummaryStart {
        try withApplication("retryQueuedHandoffSummary", .preparesContinuation, .send) {
            try $0.retryQueuedHandoffSummary(sourceRuntimeId: sourceRuntimeID, conversationId: conversationID,
                queuedMessageId: queuedMessageID, summaryModel: model, effort: effort)
        }
    }
    func handoffSummary(workID: String) throws -> HandoffSummaryStatus {
        try withApplication("handoffSummary", .preparesContinuation, .conversationRead) { try $0.readHandoffSummary(workId: workID) }
    }
    func cancelHandoffSummary(workID: String) throws -> HandoffSummaryStatus {
        try withApplication("cancelHandoffSummary", .preparesContinuation, .cancel) { try $0.cancelHandoffSummary(workId: workID) }
    }
    func sendTurnWithPreparedHandoff(sourceRuntimeID: String, conversationID: String, targetRuntimeID: String,
                                     selection: ModelSelection?, turn: ComposedTurn, preparedToken: String,
                                     cancellation: VeluneSendCancellation) throws -> SnapshotResult {
        try withApplication("sendTurnWithPreparedHandoff", .writes, .send) { application in
            try checkCancellation(cancellation)
            let delivered = try deliver(turn, using: application)
            try checkCancellation(cancellation)
            return try application.sendPreparedHandoffCancellable(sourceRuntimeId: sourceRuntimeID, conversationId: conversationID,
                                                                   targetRuntimeId: targetRuntimeID, selection: selection, turn: delivered,
                                                                   preparedToken: preparedToken, cancellation: cancellation)
        }
    }
    private func checkCancellation(_ cancellation: VeluneSendCancellation?) throws {
        if cancellation?.isCancelled() == true { throw TransportError.submissionCancelled }
    }
    private func dispatchTurn(_ application: VeluneApplication, sourceRuntimeID: String, conversationID: String, targetRuntimeID: String, selection: ModelSelection?, turn: ComposedTurn, cancellation: VeluneSendCancellation?, submissionId: String? = nil) throws -> SnapshotResult {
        try checkCancellation(cancellation)
        let delivered = try deliver(turn, using: application)
        try checkCancellation(cancellation)
        let result: SnapshotResult
        if let cancellation {
            result = try application.sendTurnCancellable(sourceRuntimeId: sourceRuntimeID, conversationId: conversationID, targetRuntimeId: targetRuntimeID, selection: selection, turn: delivered, cancellation: cancellation, submissionId: submissionId)
        } else {
            result = try application.sendTurn(sourceRuntimeId: sourceRuntimeID, conversationId: conversationID, targetRuntimeId: targetRuntimeID, selection: selection, turn: delivered, submissionId: submissionId)
        }
        return result
    }
    /// The runtime's own models first, then the managed models; every entry
    /// carries the levels the runtime accepts for it.
    func runtimeModels(runtimeID: String) throws -> [RuntimeModel] { try withApplication("runtimeModels", .reads, .configurationRead) { try $0.runtimeModels(runtimeId: runtimeID) } }
    func steerTurn(runtimeID: String, conversationID: String, turn: ComposedTurn) throws -> SnapshotResult { try withApplication("steerTurn", .writes, .send) { try $0.steerTurn(runtimeId: runtimeID, conversationId: conversationID, turn: try deliver(turn, using: $0)) } }
    func compactConversation(runtimeID: String, conversationID: String, selection: ModelSelection?) throws -> SnapshotResult {
        try withApplication("compactConversation", .writes, .compact) { try $0.compactConversation(runtimeId: runtimeID, conversationId: conversationID, selection: selection) }
    }
    func resumeTurn(runtimeID: String, conversationID: String, selection: ModelSelection?, effort: String?, permissionMode: String?) throws -> SnapshotResult {
        try withApplication("resumeTurn", .writes, .send) { try $0.resumeTurn(runtimeId: runtimeID, conversationId: conversationID, selection: selection, effort: effort, permissionMode: permissionMode) }
    }
    private func deliver(_ turn: ComposedTurn, using application: VeluneApplication) throws -> ComposedTurn {
        guard remoteAttachments else { return turn }
        var delivered = turn
        delivered.attachments = try turn.attachments.map { path in
            guard path.hasPrefix("/") else { return path }
            let url = URL(fileURLWithPath: path)
            return try application.uploadAttachment(name: url.lastPathComponent, bytes: Data(contentsOf: url)).reference
        }
        return delivered
    }
    func sendQueue(runtimeID: String, conversationID: String, operation: SendQueueOperation, submissionId: String? = nil) throws -> SnapshotResult {
        try withApplication("sendQueue", .writes, .conversationWrite) { application in
            let delivered: SendQueueOperation
            switch operation {
            case let .add(source, target, selection, turn):
                delivered = .add(sourceRuntimeId: source, targetRuntimeId: target, selection: selection, turn: try deliver(turn, using: application))
            case let .replace(id, revision, target, selection, retained, turn):
                delivered = .replace(id: id, expectedRevision: revision, targetRuntimeId: target, selection: selection,
                                     retainedAttachments: retained, turn: try deliver(turn, using: application))
            default: delivered = operation
            }
            return try application.sendQueue(runtimeId: runtimeID, conversationId: conversationID, operation: delivered, submissionId: submissionId)
        }
    }
    func deleteUserMessage(runtimeID: String, conversationID: String, messageID: String, truncate: Bool, expectedRevision: String) throws -> SnapshotResult {
        try withApplication("deleteUserMessage", .writes, .conversationWrite) { try $0.deleteUserMessage(runtimeId: runtimeID, conversationId: conversationID, messageId: messageID, truncate: truncate, expectedRevision: expectedRevision) }
    }
    func editMessage(runtimeID: String, conversationID: String, messageID: String, placement: EditPlacement, targetRuntimeID: String? = nil, selection: ModelSelection?, turn: ComposedTurn, expectedRevision: String? = nil) throws -> SnapshotResult {
        try withApplication("editMessage", .writes, .conversationWrite) { application in
            let message = BindingMessageReference(runtimeId: runtimeID, conversationId: conversationID, messageId: messageID)
            return try application.editMessageWithRuntime(message: message, placement: placement,
                targetRuntimeId: targetRuntimeID, selection: selection, turn: try deliver(turn, using: application),
                expectedRevision: expectedRevision)
        }
    }
    func forkConversation(runtimeID: String, conversationID: String, messageID: String) throws -> SnapshotResult { try withApplication("forkConversation", .writes, .conversationWrite) { try $0.forkConversation(runtimeId: runtimeID, conversationId: conversationID, messageId: messageID) } }
    func cancel(runtimeID: String, conversationID: String) throws -> SnapshotResult { try withApplication("cancel", .writes, .cancel) { try $0.cancel(runtimeId: runtimeID, conversationId: conversationID) } }
    func openSideChat(runtimeID: String, conversationID: String, selection: ModelSelection?, effort: String?, permissionMode: String?) throws -> SnapshotResult {
        try withApplication("openSideChat", .writes, .send) { try $0.openSideChat(runtimeId: runtimeID, conversationId: conversationID, selection: selection, effort: effort, permissionMode: permissionMode) }
    }
    func sendSideChat(sideChatID: String, runtimeID: String, selection: ModelSelection?, turn: ComposedTurn, cancellation: VeluneSendCancellation? = nil) throws -> SnapshotResult {
        try withApplication("sendSideChat", .writes, .send) { application in
            try checkCancellation(cancellation)
            let delivered = try deliver(turn, using: application)
            try checkCancellation(cancellation)
            if let cancellation {
                return try application.sendSideChatCancellable(sideChatId: sideChatID, targetRuntimeId: runtimeID, selection: selection, turn: delivered, cancellation: cancellation)
            }
            return try application.sendSideChat(sideChatId: sideChatID, targetRuntimeId: runtimeID, selection: selection, turn: delivered)
        }
    }
    func closeSideChat(sideChatID: String) throws { try withApplication("closeSideChat", .writes, .cancel) { try $0.closeSideChat(sideChatId: sideChatID) } }
    func backgroundTasks(runtimeID: String, conversationID: String) throws -> BackgroundTaskList { try withApplication("backgroundTasks", .reads, .conversationRead) { try $0.backgroundTasks(runtimeId: runtimeID, conversationId: conversationID) } }
    func backgroundTaskOutput(runtimeID: String, conversationID: String, taskID: String, requestID: String?) throws -> BackgroundTaskOutput { try withApplication("backgroundTaskOutput", .reads, .conversationRead) { try $0.backgroundTaskOutput(runtimeId: runtimeID, conversationId: conversationID, taskId: taskID, requestId: requestID) } }
    func stopBackgroundTask(runtimeID: String, conversationID: String, taskID: String) throws -> BackgroundTaskList { try withApplication("stopBackgroundTask", .writes, .cancel) { try $0.stopBackgroundTask(runtimeId: runtimeID, conversationId: conversationID, taskId: taskID) } }
    func stopSubAgents(runtimeID: String, conversationID: String) throws -> SnapshotResult { try withApplication("stopSubAgents", .writes, .cancel) { try $0.stopSubAgents(runtimeId: runtimeID, conversationId: conversationID) } }
    func setConversationHandoff(runtimeID: String, conversationID: String, release: Bool) throws -> SnapshotResult {
        try withApplication("setConversationHandoff", .writes, .conversationWrite) {
            try $0.setConversationHandoff(runtimeId: runtimeID, conversationId: conversationID, release: release)
        }
    }
    func workingBranch(runtimeID: String, conversationID: String) throws -> String? { try withApplication("workingBranch", .reads, .conversationRead) { try $0.workingBranch(runtimeId: runtimeID, conversationId: conversationID) } }
    func workingChanges(runtimeID: String, conversationID: String) throws -> WorkingChanges { try withApplication("workingChanges", .reads, .conversationRead) { try $0.workingChanges(runtimeId: runtimeID, conversationId: conversationID) } }
    func fileDiff(runtimeID: String, conversationID: String, path: String) throws -> FileDiff { try withApplication("fileDiff", .reads, .conversationRead) { try $0.fileDiff(runtimeId: runtimeID, conversationId: conversationID, path: path) } }
    func conversationActivity(since: UInt64) throws -> ConversationActivity { try withApplication("conversationActivity", .reads, .conversationRead) { try $0.conversationActivity(since: since) } }
    func replyRuntimeInteraction(runtimeID: String, conversationID: String, interactionID: String, reply: RuntimeInteractionReply) throws -> SnapshotResult { try withApplication("replyRuntimeInteraction", .writes, .reply) { try $0.replyRuntimeInteraction(runtimeId: runtimeID, conversationId: conversationID, interactionId: interactionID, reply: reply) } }
    func providerImportPreview(gatewayID: String, runtimeID: String) throws -> VeluneBindings.ProviderImportPreview { try withApplication("providerImportPreview", .reads, .configurationRead) { try $0.previewProviderImport(gatewayId: gatewayID, runtimeId: runtimeID) } }
    func providerImportApply(gatewayID: String, runtimeID: String, previewToken: String, selections: [ImportSelection], replaceExisting: Bool) throws -> ImportResult { try withApplication("providerImportApply", .changesConfiguration, .configurationWrite) { try $0.applyProviderImport(gatewayId: gatewayID, runtimeId: runtimeID, previewToken: previewToken, selections: selections, replaceExisting: replaceExisting) } }
    func authenticationInspect(gatewayID: String, providerID: String) throws -> VeluneBindings.AuthenticationMetadata { try withApplication("authenticationInspect", .reads, .read) { try $0.authenticationInspect(gatewayId: gatewayID, providerId: providerID) } }
    func authenticationStart(gatewayID: String, providerID: String) throws -> AuthenticationProgress { try withApplication("authenticationStart", .writes, .write) { try $0.authenticationStart(gatewayId: gatewayID, providerId: providerID) } }
    func authenticationPoll() throws -> AuthenticationProgress { try withApplication("authenticationPoll", .reads, .read) { try $0.authenticationPoll() } }
    func authenticationReply(promptID: String, value: String) throws -> AuthenticationProgress { try withApplication("authenticationReply", .writes, .write) { try $0.authenticationReply(promptId: promptID, value: value) } }
    func authenticationCancel() throws -> AuthenticationProgress { try withApplication("authenticationCancel", .writes, .write) { try $0.authenticationCancel() } }
    func authenticationLogout(gatewayID: String, providerID: String) throws -> GatewayUpdate { try withApplication("authenticationLogout", .changesConfiguration, .configurationWrite) { try $0.authenticationLogout(gatewayId: gatewayID, providerId: providerID) } }

    /// A retry the user asked for forgets the cached open failure, so the
    /// next operation opens the connection again at once.
    func retryOpen() {
        lock.lock(); defer { lock.unlock() }
        openFailure = nil
    }

    /// Quitting stops new admissions; a busy refusal restores the same owner.
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
                let application = try lock.withLock { try admittedApplication(operation) }
                return try body(application.withTraceContext(carrier: carrier))
            }
            if effect == .changesConfiguration { configurationChanged.send() }
            return value
        }
        catch {
            if let error = error as? TransportError, error.isSubmissionCancelled { throw error }
            if case .Diagnostic(_, _, let code, _, _)? = error as? BindingError, code == "submission_cancelled" { throw TransportError.submissionCancelled }
            // A cached open failure was already logged when the open itself failed.
            if let cached = error as? CachedOpenFailure { throw TransportError.operation(operation, map(cached.error)) }
            logFailure(operation: operation, localCorrelation: correlation, error: error); throw TransportError.operation(operation, map(error))
        }
    }

    /// Call under the lifecycle lock. The handle shares the Rust backend
    /// state, including closure, with every admitted call.
    private func admittedApplication(_ operation: String) throws -> VeluneApplication {
        guard !isClosed else { throw TransportError.operation(operation, .closed) }
        if isClosing {
            throw TransportError.operation(operation, .quitting)
        }
        if let application { return application }
        if let failure = openFailure, Date().timeIntervalSince(failure.at) < Self.openRetryInterval {
            throw CachedOpenFailure(error: failure.error)
        }
        do {
            let application = try open()
            self.application = application
            openFailure = nil
            return application
        } catch {
            openFailure = (error, Date())
            throw error
        }
    }

    private func map(_ error: Error) -> TransportError {
        if let error = error as? TransportError { return error }
        if let error = error as? BindingError { return Self.transportError(for: error) }
        return .rejected(Self.fullFailureDetail(error))
    }

    private func category(for error: Error) -> String {
        if let error = error as? BindingError {
            switch error {
            case .Diagnostic(let kind, _, _, _, _), .RemoteRequest(let kind, _, _, _, _): return Self.kindName(kind)
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

    static func transportError(for error: BindingError) -> TransportError {
        switch error {
        case .Diagnostic(let kind, let detail, let code, let phase, let operationID):
            return .diagnostic(kind: kindName(kind), detail: detail, code: code, phase: phase, operationID: operationID)
        case .RemoteRequest(let kind, let detail, let code, _, let operationID):
            return .diagnostic(kind: kindName(kind), detail: detail, code: code, phase: "remote", operationID: operationID)
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
            case .Diagnostic(_, let detail, _, _, _), .RemoteRequest(_, let detail, _, _, _), .Invalid(let detail), .Unsupported(let detail), .Io(let detail), .Contract(let detail): return detail
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
            details.append("\(String(reflecting: type(of: cause)))，错误域：\(native.domain)（\(native.code)）")
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
        if let binding = error as? BindingError,
           case .diagnostic(let kind, _, let code, let phase, let operationID) = Self.transportError(for: binding) {
            Self.logger.error("operation=\(operation, privacy: .public)\(local, privacy: .public) kind=\(kind, privacy: .public) code=\(code, privacy: .public) phase=\(phase, privacy: .public) operationId=\(operationID, privacy: .public) detail=\(detail, privacy: .public)")
        } else {
            Self.logger.error("operation=\(operation, privacy: .public)\(local, privacy: .public) category=\(categoryOverride ?? self.category(for: error), privacy: .public) detail=\(detail, privacy: .public)")
        }
    }

}
