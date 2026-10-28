import Foundation

import VeluneBindings

/// Converts generated UniFFI records at the application boundary. The Swift UI
/// models remain platform-friendly; no JSON action envelope crosses this file.
enum BindingMapping {
    static func location(_ value: RuntimeTransport) -> RuntimeLocation {
        switch value {
        case .local: .local
        case .ssh(_, let destination, _): .ssh(destination: destination)
        }
    }
    static func conversation(_ value: ConversationSummary) -> Conversation {
        Conversation(id: value.id, title: value.title.displayText(), updatedAtUnixMs: value.updatedAtUnixMs, createdAtUnixMs: value.createdAtUnixMs,
                     runtimeID: value.runtimeId, cwd: value.cwd, canRename: value.canRename, canDelete: value.canDelete,
                     parentConversationID: value.parentConversationId)
    }

    static func listed(_ value: ListedConversation) -> Conversation {
        var mapped = conversation(value.conversation)
        mapped.nativeID = value.nativeId; mapped.pinned = value.pinned; mapped.unread = value.unread
        mapped.agentsURI = value.agentsUri
        mapped.archived = value.archived; mapped.project = value.project; mapped.moved = value.moved; mapped.failed = value.failed; mapped.group = value.group
        return mapped
    }

    /// A complete delivery. An incremental one (non-nil `base`) carries only
    /// changed messages; see `snapshotUpdate`.
    static func snapshot(_ value: DeliveredSnapshot) -> ConversationSnapshot {
        var mapped = snapshotHeader(value)
        mapped.messages = value.messages.map(message)
        return mapped
    }

    /// Every field except messages, which an incremental update merges by identity.
    static func snapshotHeader(_ value: DeliveredSnapshot) -> ConversationSnapshot {
        ConversationSnapshot(cursor: value.cursor, revision: value.revision, conversation: conversation(value.conversation), contextRuntimeID: value.contextRuntimeId,
                             modelRecordKey: value.modelRecordKey, runState: value.runState,
                             messages: [], transcriptTurns: value.transcriptTurns, transcriptItems: value.transcriptItems, transcriptOutline: value.transcriptOutline,
                             transcriptMessageIdentities: value.transcriptMessageIdentities, messageIdentityConfirmations: value.messageIdentityConfirmations,
                             pendingInteractions: value.pendingInteractions, actions: value.actions, followUp: value.followUp,
                             subAgents: value.subAgents.map { SubAgent(id: $0.conversationId, title: $0.title, failed: $0.failed) },
                             collaborators: value.collaborators.map { Collaborator(id: $0.conversationId, title: $0.title, runtimeName: $0.runtimeName, contacted: $0.contacted, contactedBy: $0.contactedBy) })
    }

    static func message(_ value: DeliveredMessage) -> Message {
        Message(id: value.id, role: value.role, completed: value.completed, timestampUnixMs: value.timestampUnixMs, blocks: value.blocks.map(block), stamp: value.stamp)
    }

    static func block(_ value: DeliveredBlock) -> MessageBlock {
        switch value {
        case .text(let text): return .text(text)
        case .reasoning(let summary, let text): return .reasoning(summary: summary, text: text)
        case .tool(let id, let title, let state, let output, let conversation): return .tool(id: id, title: title, state: state, output: output, conversation: conversation)
        case .notice(let text), .contextCompaction(let text): return .notice(text)
        case .image(let mediaType, .path(let path)): return .image(mediaType: mediaType, source: .path(path))
        case .image(let mediaType, .data(let data)): return .image(mediaType: mediaType, source: .data(data))
        case .file(let path): return .file(path: path)
        case .agentSender(let conversationID, let runtimeName, let title): return .agentSender(conversationID: conversationID, runtimeName: runtimeName, title: title)
        }
    }




    static func provider(_ value: ProviderSummary) -> AIProvider {
        let method: AuthenticationMethod
        switch value.authentication.method { case .apiKey: method = .apiKey; case .oAuth: method = .oauth; case .unconfigured: method = .unconfigured }
        let authentication = ProviderAuthentication(method: method, configured: value.authentication.configured, accountLabel: value.authentication.accountLabel, provenance: value.authentication.provenance?.displayName,
                                                    disclosure: value.authentication.disclosure, actions: value.authentication.actions,
                                                    linkedLogin: value.authentication.linkedLogin)
        return AIProvider(id: value.id, name: value.name, protocolID: value.`protocol`, endpoint: value.endpoint, authentication: authentication, models: value.models,
                          quotaLimits: value.quotaLimits.map { QuotaLimit(windowId: $0.windowId, maxUsedPercent: $0.maxUsedPercent) },
                          composition: ProviderComposition(executionBackend: value.executionBackend, requestProfile: value.requestProfile, authenticationSource: value.authenticationSource, quotaSource: value.quotaSource), quotaAuthenticationConfigured: value.quotaAuthentication.configured, quotaBindingConfirmed: value.quotaAuthentication.bindingConfirmed)
    }
    static func providerTemplate(_ value: VeluneBindings.ProviderTemplate) -> ProviderTemplate {
        let kind: ProviderTemplate.Kind
        switch value.kind { case .subscription: kind = .subscription; case .api: kind = .api }
        return ProviderTemplate(id: value.id, kind: kind, name: value.name, summary: value.summary, protocolID: value.`protocol`, endpoint: value.endpoint,
                                composition: ProviderComposition(executionBackend: value.executionBackend, requestProfile: value.requestProfile, authenticationSource: value.authenticationSource, quotaSource: value.quotaSource),
                                executableCommand: value.executableCommand, disclosure: value.disclosure)
    }
    static func quota(_ value: ProviderQuotaStatus) -> AccountQuota {
        var result = quota(freshness: value.freshness, windows: value.windows, refreshing: value.refreshing, problem: value.problem)
        result.balances = value.balances.map { QuotaBalance(id: $0.id, amount: $0.amount, unit: $0.unit, observedAt: Date(timeIntervalSince1970: Double($0.observedAtMs) / 1000)) }
        result.available = value.available
        return result
    }
    static func composerQuota(_ value: ComposerQuotaEntry) -> ComposerAccountQuota {
        let observation = value.observation
        var result = quota(freshness: observation.freshness, windows: observation.windows, refreshing: observation.refreshing, problem: observation.problem)
        result.balances = observation.balances.map { QuotaBalance(id: $0.id, amount: $0.amount, unit: $0.unit, observedAt: Date(timeIntervalSince1970: Double($0.observedAtMs) / 1000)) }
        result.available = observation.available
        return ComposerAccountQuota(id: value.id, name: value.name, origin: value.origin, quota: result,
                                    limits: value.limits.map { QuotaLimit(windowId: $0.windowId, maxUsedPercent: $0.maxUsedPercent) })
    }
    static func quota(_ value: RuntimeQuotaStatus) -> AccountQuota {
        quota(freshness: value.freshness, windows: value.windows, refreshing: value.refreshing, problem: value.problem)
    }
    private static func quota(freshness sourceFreshness: VeluneBindings.QuotaFreshness, windows: [QuotaWindowObservation], refreshing: Bool, problem: String?) -> AccountQuota {
        let freshness: QuotaFreshness
        switch sourceFreshness { case .fresh: freshness = .fresh; case .stale: freshness = .stale; case .unknown: freshness = .unknown; case .unsupported: freshness = .unsupported }
        func date(_ ms: Int64) -> Date { Date(timeIntervalSince1970: Double(ms) / 1000) }
        func pace(_ value: QuotaPaceObservation) -> QuotaPace {
            let status: QuotaPace.Status
            switch value.status { case .onPace: status = .onPace; case .runsOutBeforeReset: status = .runsOutBeforeReset; case .exhausted: status = .exhausted }
            return QuotaPace(expectedUsedPercent: value.expectedUsedPercent, runsOutAt: value.runsOutAtMs.map(date), status: status)
        }
        return AccountQuota(freshness: freshness,
                             windows: windows.map { QuotaWindow(id: $0.id, modelIDs: $0.modelIds, usedPercent: $0.usedPercent, resetsAt: $0.resetAtMs.map(date), period: $0.windowMs.map { Double($0) / 1000 }, observedAt: $0.observedAtMs.map(date), pace: $0.pace.map(pace)) },
                             refreshing: refreshing, problem: problem)
    }
    static func quotaObservation(_ value: QuotaHistoryPoint) -> QuotaObservation {
        var values: [QuotaMetric: Double] = [:]
        for window in value.windows { values[.window(id: window.id, modelIDs: window.modelIds)] = window.usedPercent }
        for balance in value.balances { values[.balance(id: balance.id, unit: balance.unit)] = balance.amount }
        return QuotaObservation(observedAt: Date(timeIntervalSince1970: Double(value.observedAtMs) / 1000), available: value.available, values: values)
    }
    static func gateway(_ value: GatewaySummary) -> GatewayConfig {
        GatewayConfig(id: value.id, name: value.name, providers: value.providers.map(provider), routes: value.routes)
    }
    static func bindingProvider(_ value: AIProvider) -> ProviderDraft {
        ProviderDraft(id: value.id, name: value.name, protocol: value.protocolID, endpoint: value.endpoint,
                             executionBackend: value.composition.executionBackend, requestProfile: value.composition.requestProfile, authenticationSource: value.composition.authenticationSource, quotaSource: value.composition.quotaSource, models: value.models,
                             quotaLimits: value.quotaLimits.map { VeluneBindings.QuotaLimit(windowId: $0.windowId, maxUsedPercent: $0.maxUsedPercent) })
    }




    static func configuration(_ value: ConfigurationSnapshot) -> (conversations: [Conversation], historyFailures: [HistoryFailure], gateways: [GatewayConfig], runtimes: [RuntimeInstance], runtimeTypes: [RuntimeTypeDescriptor], modelTemplates: [ModelTemplate], importTypes: [RuntimeTypeDescriptor], protocols: [ProtocolDescriptor], selectedRuntimeID: String?) {
        (value.conversations.map(listed), value.historyFailures.map { HistoryFailure(runtimeID: $0.runtimeId, detail: $0.detail) }, value.gateways.map(gateway), value.runtimeInstances, value.runtimeTypes, value.modelTemplates, value.providerImportTypes, value.protocols, value.selectedRuntimeInstanceId)
    }


    static func snapshotResult(_ value: SnapshotResult) -> ConversationSnapshot? {
        value.snapshot.map(snapshot)
    }





    static func authenticationMetadata(_ value: VeluneBindings.AuthenticationMetadata) -> AuthenticationMetadata {
        AuthenticationMetadata(configured: value.configured, capabilities: AuthenticationCapabilities(protocol: value.capabilities.`protocol`, endpoint: value.capabilities.endpoint, explicitOutputCap: value.capabilities.explicitOutputCap, temperature: value.capabilities.temperature), actions: value.actions)
    }

    static func authenticationProgress(_ value: AuthenticationProgress) -> AuthenticationData {
        AuthenticationData(running: value.running, events: value.events.map { AuthenticationEvent(type: $0.typeId, id: $0.id, prompt: $0.prompt.map { AuthenticationPromptBody(kind: $0.kind, text: $0.text, options: $0.options?.map { SettingOption(id: $0.id, label: $0.label) }) }, notification: $0.notification.map { AuthenticationNotification(kind: $0.kind, id: $0.id, text: $0.text, url: $0.url, instructions: $0.instructions, userCode: $0.userCode, verificationUri: $0.verificationUri) }, ok: $0.ok, error: $0.error, cancelled: $0.cancelled) }, gateways: value.gateways?.map(gateway), executionInvalidated: value.executionInvalidated)
    }
}
