import Foundation

import VeluneBindings

/// Converts generated UniFFI records at the application boundary. The Swift UI
/// models remain platform-friendly; no JSON action envelope crosses this file.
enum BindingMapping {
    static func selection(_ value: VeluneBindings.ModelSelection) -> ModelSelection {
        switch value {
        case .managed(let key): .managed(recordKey: key)
        case .native(let provider, let model): .native(providerID: provider, modelID: model)
        }
    }
    static func location(_ value: RuntimeTransport) -> RuntimeLocation {
        switch value {
        case .local: .local
        case .ssh(_, let destination, _): .ssh(destination: destination)
        }
    }
    static func sessionOption(_ value: AcpConfigOption) -> SessionOption {
        SessionOption(id: value.id, name: value.name, description: value.description, category: value.category, value: sessionOptionValue(value.value),
                      choices: value.choices.map { SessionOptionChoice(value: $0.value, name: $0.name, description: $0.description, group: $0.groupName ?? $0.group) })
    }
    static func sessionOptionValue(_ value: AcpConfigValue) -> SessionOptionValue {
        switch value {
        case .select(let choice): .select(choice)
        case .boolean(let on): .boolean(on)
        }
    }
    static func bindingSessionOptionValue(_ value: SessionOptionValue) -> AcpConfigValue {
        switch value {
        case .select(let choice): .select(choice)
        case .boolean(let on): .boolean(on)
        }
    }
    static func modelChoice(_ value: RuntimeModel) -> ModelChoice {
        ModelChoice(selection: selection(value.selection), displayName: value.name, reasoningEfforts: value.reasoningEfforts)
    }
    static func bindingSelection(_ value: ModelSelection) -> VeluneBindings.ModelSelection {
        switch value {
        case .managed(let key): .managed(modelRecordKey: key)
        case .native(let provider, let model): .native(providerId: provider, modelId: model)
        }
    }
    static func choice(_ value: VeluneBindings.TurnChoice) -> TurnChoice {
        TurnChoice(model: value.model.map(selection), effort: value.effort, permissionMode: value.permissionMode)
    }
    static func bindingChoice(_ value: TurnChoice) -> VeluneBindings.TurnChoice {
        VeluneBindings.TurnChoice(model: value.model.map(bindingSelection), effort: value.effort, permissionMode: value.permissionMode)
    }
    static func newConversation(_ value: VeluneBindings.NewConversationChoice) -> NewConversationChoice {
        switch value {
        case .lastChosen: .lastChosen
        case .chosen(let value): .chosen(choice(value))
        }
    }
    static func bindingNewConversation(_ value: NewConversationChoice) -> VeluneBindings.NewConversationChoice {
        switch value {
        case .lastChosen: .lastChosen
        case .chosen(let value): .chosen(choice: bindingChoice(value))
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
        ConversationSnapshot(cursor: cursor(value.cursor), revision: value.revision, conversation: conversation(value.conversation), contextRuntimeID: value.contextRuntimeId,
                             modelRecordKey: value.modelRecordKey, runState: runState(value.runState),
                             messages: [], transcriptTurns: value.transcriptTurns.map { TranscriptTurn(id: $0.id, userMessageID: $0.userMessageId, workMessageIDs: $0.workMessageIds, lastMessageID: $0.lastMessageId, replyStartBlock: Int($0.replyStartBlock), durationMs: $0.durationMs, isRunning: $0.isRunning, settings: $0.settings.map { TurnSettings(runtimeID: $0.runtimeId, model: $0.model, effort: $0.effort, permissionMode: $0.permissionMode, modelSelection: $0.modelSelection.map(selection)) }, usage: $0.usage.map { TurnUsage(inputTokens: $0.inputTokens, cacheReadTokens: $0.cacheReadTokens, cacheWriteTokens: $0.cacheWriteTokens, outputTokens: $0.outputTokens, reasoningTokens: $0.reasoningTokens, contextTokens: $0.contextTokens, contextWindow: $0.contextWindow, sessionCostMicroUSD: $0.sessionCostMicroUsd, rateLimits: $0.rateLimits.map { RateLimit(windowMinutes: $0.windowMinutes, usedPercent: $0.usedPercent, resetsAtUnixMs: $0.resetsAtUnixMs) }) }) }, transcriptItems: value.transcriptItems.map { switch $0 { case .message(let id, let messageID): return .message(id: id, messageID: messageID); case .work(let turnID): return .work(turnID: turnID) } }, transcriptOutline: value.transcriptOutline.map { TranscriptOutlineEntry(id: $0.id, messageID: $0.messageId) }, transcriptMessageIdentities: value.transcriptMessageIdentities.map { TranscriptMessageIdentity(id: $0.id, messageID: $0.messageId) }, messageIdentityConfirmations: value.messageIdentityConfirmations.map { MessageIdentityConfirmation(previousID: $0.previousId, currentID: $0.currentId) }, pendingInteractions: value.pendingInteractions.map(interaction), actions: actions(value.actions), followUp: followUp(value.followUp),
                             subAgents: value.subAgents.map { SubAgent(id: $0.conversationId, title: $0.title, failed: $0.failed) },
                             collaborators: value.collaborators.map { Collaborator(id: $0.conversationId, title: $0.title, runtimeName: $0.runtimeName, contacted: $0.contacted, contactedBy: $0.contactedBy) })
    }

    static func followUp(_ value: VeluneBindings.FollowUp) -> FollowUp {
        FollowUp(queue: value.queue.map { QueuedMessage(id: $0.id, text: $0.text, attachments: $0.attachments) },
                 paused: value.paused.map { switch $0 { case .turnStopped: .turnStopped; case .turnFailed: .turnFailed; case .sendFailed: .sendFailed } },
                 waitingForSubAgents: value.waiting.map { switch $0 { case .subAgents: true } } ?? false,
                 canStopSubAgents: value.canStopSubAgents,
                 canSteer: value.canSteer, canContinue: value.canContinue, canResume: value.canResume, canCompact: value.canCompact, isCompacting: value.isCompacting,
                 editable: value.editable.map { EditableMessage(messageID: $0.messageId, inPlace: $0.inPlace) })
    }

    static func runState(_ value: VeluneBindings.RunState) -> RunState {
        switch value { case .idle: return .idle; case .running: return .running; case .stopping: return .stopping; case .failed: return .failed; case .elsewhere: return .elsewhere }
    }

    static func actions(_ value: VeluneBindings.ConversationActions) -> ConversationActions {
        ConversationActions(canSend: value.canSend, canCancel: value.canCancel, canSwitch: value.canSwitch)
    }

    static func cursor(_ value: VeluneBindings.TranscriptCursor) -> TranscriptCursor { TranscriptCursor(generation: value.generation, sequence: value.sequence) }
    static func bindingCursor(_ value: TranscriptCursor) -> VeluneBindings.TranscriptCursor { VeluneBindings.TranscriptCursor(generation: value.generation, sequence: value.sequence) }

    static func message(_ value: DeliveredMessage) -> Message {
        Message(id: value.id, role: messageRole(value.role), completed: value.completed, timestampUnixMs: value.timestampUnixMs, blocks: value.blocks.map(block), stamp: value.stamp)
    }

    static func messageRole(_ value: VeluneBindings.MessageRole) -> MessageRole {
        switch value { case .user: return .user; case .assistant: return .assistant; case .tool: return .tool; case .system: return .system }
    }
    static func toolState(_ value: VeluneBindings.ToolState) -> ToolState {
        switch value { case .pending: return .pending; case .running: return .running; case .completed: return .completed; case .failed: return .failed }
    }
    static func block(_ value: DeliveredBlock) -> MessageBlock {
        switch value {
        case .text(let text): return .text(text)
        case .reasoning(let summary, let text): return .reasoning(summary: summary, text: payload(text))
        case .tool(let id, let title, let state, let output, let conversation): return .tool(id: id, title: title, state: toolState(state), output: output.map(payload), conversation: conversation)
        case .notice(let text), .contextCompaction(let text): return .notice(text)
        case .image(let mediaType, .path(let path)): return .image(mediaType: mediaType, source: .path(path))
        case .image(let mediaType, .data(let data)): return .image(mediaType: mediaType, source: .data(payload(data)))
        case .file(let path): return .file(path: path)
        case .agentSender(let conversationID, let runtimeName, let title): return .agentSender(conversationID: conversationID, runtimeName: runtimeName, title: title)
        }
    }

    static func payload(_ value: VeluneBindings.MessagePayload) -> MessagePayload {
        switch value {
        case .inline(let text): return .inline(text)
        case .deferred(let bytes): return .deferred(bytes: bytes)
        }
    }

    static func settingField(_ value: VeluneBindings.SettingField) -> SettingField {
        return SettingField(key: value.key, label: value.label, kind: settingKind(value.kind),
                     required: value.required, value: value.value,
                     options: value.options.map { SettingOption(id: $0.id, label: $0.label) }, help: value.help,
                     executableDiscovery: value.executableDiscovery.map { ExecutableDiscoverySpec(command: $0.command, minimumVersion: $0.minimumVersion) })
    }

    static func settingKind(_ value: VeluneBindings.SettingKind) -> SettingKind {
        switch value {
        case .text: return .text
        case .filePath: return .filePath
        case .directoryPath: return .directoryPath
        case .choice: return .choice
        }
    }

    static func settingAction(_ value: VeluneBindings.SettingAction) -> SettingAction {
        SettingAction(id: value.id, label: value.label)
    }

    static func protocolID(_ value: ApiProtocol) -> ProviderProtocol {
        switch value { case .chatCompletions: return .chatCompletions; case .responses: return .responses; case .messages: return .messages }
    }
    static func model(_ value: VeluneBindings.ProviderModel) -> ProviderModel {
        ProviderModel(recordKey: value.recordKey, providerModelID: value.providerModelId, nickname: value.nickname, icon: value.icon, contextWindow: value.contextWindow, maxOutputTokens: value.maxOutputTokens, reasoningLevels: value.reasoningLevels)
    }
    static func bindingModel(_ value: ProviderModel) -> VeluneBindings.ProviderModel {
        VeluneBindings.ProviderModel(recordKey: value.recordKey, providerModelId: value.providerModelID, nickname: value.nickname, icon: value.icon, contextWindow: value.contextWindow, maxOutputTokens: value.maxOutputTokens, reasoningLevels: value.reasoningLevels)
    }
    static func template(_ value: VeluneBindings.ModelTemplate) -> ModelTemplate {
        ModelTemplate(id: value.id, name: value.name, suggestedProviderModelID: value.suggestedProviderModelId, nickname: value.nickname, icon: value.icon, contextWindow: value.contextWindow, maxOutputTokens: value.maxOutputTokens, reasoningLevels: value.reasoningLevels)
    }
    static func bindingTemplate(_ value: ModelTemplate) -> VeluneBindings.ModelTemplate {
        VeluneBindings.ModelTemplate(id: value.id, name: value.name, suggestedProviderModelId: value.suggestedProviderModelID, nickname: value.nickname, icon: value.icon, contextWindow: value.contextWindow, maxOutputTokens: value.maxOutputTokens, reasoningLevels: value.reasoningLevels)
    }
    static func provider(_ value: ProviderSummary) -> AIProvider {
        let method: AuthenticationMethod
        switch value.authentication.method { case .apiKey: method = .apiKey; case .oAuth: method = .oauth; case .unconfigured: method = .unconfigured }
        let authentication = ProviderAuthentication(method: method, configured: value.authentication.configured, accountLabel: value.authentication.accountLabel, provenance: value.authentication.provenance?.displayName,
                                                    disclosure: value.authentication.disclosure.map(disclosure), actions: value.authentication.actions.map(settingAction),
                                                    linkedLogin: value.authentication.linkedLogin.map(linkedLogin))
        return AIProvider(id: value.id, name: value.name, protocolID: protocolID(value.`protocol`), endpoint: value.endpoint, authentication: authentication, models: value.models.map(model),
                          quotaLimits: value.quotaLimits.map { QuotaLimit(windowID: $0.windowId, maxUsedPercent: $0.maxUsedPercent) },
                          composition: ProviderComposition(executionBackend: value.executionBackend, requestProfile: value.requestProfile, authenticationSource: value.authenticationSource, quotaSource: value.quotaSource), quotaAuthenticationConfigured: value.quotaAuthentication.configured, quotaBindingConfirmed: value.quotaAuthentication.bindingConfirmed)
    }
    static func disclosure(_ value: VeluneBindings.SubscriptionDisclosure) -> SubscriptionDisclosure {
        SubscriptionDisclosure(title: value.title, detail: value.detail)
    }
    static func providerTemplate(_ value: VeluneBindings.ProviderTemplate) -> ProviderTemplate {
        let kind: ProviderTemplate.Kind
        switch value.kind { case .subscription: kind = .subscription; case .api: kind = .api }
        return ProviderTemplate(id: value.id, kind: kind, name: value.name, summary: value.summary, protocolID: protocolID(value.`protocol`), endpoint: value.endpoint,
                                composition: ProviderComposition(executionBackend: value.executionBackend, requestProfile: value.requestProfile, authenticationSource: value.authenticationSource, quotaSource: value.quotaSource),
                                executableCommand: value.executableCommand, disclosure: value.disclosure.map(disclosure))
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
                                    limits: value.limits.map { QuotaLimit(windowID: $0.windowId, maxUsedPercent: $0.maxUsedPercent) })
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
        GatewayConfig(id: value.id, name: value.name, providers: value.providers.map(provider), routes: value.routes.map(route), failover: FailoverPolicy(mode: .disabled))
    }
    static func route(_ value: VeluneBindings.ModelRoute) -> ModelRoute {
        ModelRoute(recordKey: value.recordKey, name: value.name, members: value.members, strategy: routingStrategy(value.strategy), weights: value.weights, sticky: value.sticky)
    }
    static func bindingRoute(_ value: ModelRoute) -> VeluneBindings.ModelRoute {
        VeluneBindings.ModelRoute(recordKey: value.recordKey, name: value.name, members: value.members, strategy: bindingRoutingStrategy(value.strategy), weights: value.weights, sticky: value.sticky)
    }
    static func routingStrategy(_ value: VeluneBindings.RoutingStrategy) -> RoutingStrategy {
        switch value { case .smart: return .smart; case .order: return .order; case .rotate: return .rotate; case .weight: return .weight }
    }
    static func bindingRoutingStrategy(_ value: RoutingStrategy) -> VeluneBindings.RoutingStrategy {
        switch value { case .smart: return .smart; case .order: return .order; case .rotate: return .rotate; case .weight: return .weight }
    }
    static func bindingProvider(_ value: AIProvider) -> ProviderDraft {
        ProviderDraft(id: value.id, name: value.name, protocol: bindingProtocol(value.protocolID), endpoint: value.endpoint,
                             executionBackend: value.composition.executionBackend, requestProfile: value.composition.requestProfile, authenticationSource: value.composition.authenticationSource, quotaSource: value.composition.quotaSource, models: value.models.map(bindingModel),
                             quotaLimits: value.quotaLimits.map { VeluneBindings.QuotaLimit(windowId: $0.windowID, maxUsedPercent: $0.maxUsedPercent) })
    }
    static func bindingAuthenticationEdit(_ value: AuthenticationEdit) -> VeluneBindings.AuthenticationEdit {
        switch value {
        case .keep: return .keep
        case .setAPIKey(let value): return .setApiKey(value: value)
        case .clear: return .clear
        case .useClaudeCodeLogin(let login): return .useClaudeCodeLogin(login: bindingLinkedLogin(login))
        }
    }
    static func linkedLogin(_ value: VeluneBindings.LinkedClaudeLogin) -> LinkedClaudeLogin {
        switch value { case .userDefault: return .userDefault; case .runtime(let id): return .runtime(instanceID: id) }
    }
    static func bindingLinkedLogin(_ value: LinkedClaudeLogin) -> VeluneBindings.LinkedClaudeLogin {
        switch value { case .userDefault: return .userDefault; case .runtime(let id): return .runtime(instanceId: id) }
    }
    static func bindingProtocol(_ value: ProviderProtocol) -> ApiProtocol {
        switch value { case .chatCompletions: return .chatCompletions; case .responses: return .responses; case .messages: return .messages }
    }

    static func bindingRuntime(_ value: RuntimeInstance) -> VeluneBindings.RuntimeInstance {
        VeluneBindings.RuntimeInstance(enabled: value.enabled, id: value.id, name: value.name, typeId: value.typeID, gatewayId: value.gatewayID, settings: value.settings)
    }

    static func bindingSelection(_ value: ProviderImportSelection) -> ImportSelection {
        ImportSelection(providerId: value.providerId, candidateKeys: value.candidateKeys)
    }

    static func runtime(_ value: VeluneBindings.RuntimeInstance) -> RuntimeInstance {
        RuntimeInstance(enabled: value.enabled, id: value.id, name: value.name, typeID: value.typeId,
                        gatewayID: value.gatewayId, settings: value.settings)
    }

    static func runtimeType(_ value: VeluneBindings.RuntimeTypeDescriptor) -> RuntimeTypeDescriptor {
        RuntimeTypeDescriptor(id: value.id, familyID: value.familyId, canRenameConversations: value.canRenameConversations, canDeleteConversations: value.canDeleteConversations, launchOptions: value.launchOptions, agentInfo: value.agentInfo, sessionOptions: value.sessionOptions, nativeAccountQuota: value.nativeAccountQuota, remoteResources: value.remoteResources, supportedProtocols: value.supportedProtocols.map(protocolID), supportedProviderProtocols: value.supportedProviderProtocols.map(protocolID), name: value.name, fields: value.fields.map(settingField),
                              permissionModes: value.permissionModes.map { PermissionMode(id: $0.id, label: $0.label, detail: $0.detail) })
    }

    static func configuration(_ value: ConfigurationSnapshot) -> (conversations: [Conversation], historyFailures: [HistoryFailure], gateways: [GatewayConfig], runtimes: [RuntimeInstance], runtimeTypes: [RuntimeTypeDescriptor], modelTemplates: [ModelTemplate], importTypes: [RuntimeTypeDescriptor], protocols: [ProtocolDescriptor], selectedRuntimeID: String?) {
        (value.conversations.map(listed), value.historyFailures.map { HistoryFailure(runtimeID: $0.runtimeId, detail: $0.detail) }, value.gateways.map(gateway), value.runtimeInstances.map(runtime), value.runtimeTypes.map(runtimeType), value.modelTemplates.map(template), value.providerImportTypes.map(runtimeType), value.protocols.map { ProtocolDescriptor(id: protocolID($0.id), name: $0.name, supported: $0.supported) }, value.selectedRuntimeInstanceId)
    }

    static func interaction(_ value: VeluneBindings.RuntimeInteraction) -> RuntimeInteraction {
        let kind: InteractionKind
        switch value.kind {
        case .approval(let title, let detail, let options): kind = .approval(title: title, detail: detail, options: options.map { InteractionOption(id: $0.id, label: $0.label) })
        case .userInput(let questions): kind = .userInput(questions: questions.map { InteractionQuestion(id: $0.id, text: $0.text, options: $0.options.map { InteractionOption(id: $0.id, label: $0.label) }, secret: $0.secret) })
        }
        return RuntimeInteraction(id: value.id, kind: kind)
    }
    static func bindingInteractionReply(_ value: RuntimeInteractionReply) -> VeluneBindings.RuntimeInteractionReply {
        switch value {
        case .decision(let option): return .decision(optionId: option)
        case .answers(let answers): return .answers(answers: answers.map { VeluneBindings.InteractionAnswer(questionId: $0.questionID, values: $0.values) })
        case .cancel: return .cancel
        }
    }

    static func snapshotResult(_ value: SnapshotResult) -> ConversationSnapshot? {
        value.snapshot.map(snapshot)
    }

    static func importSource(_ value: VeluneBindings.ProviderImportSource) -> ProviderImportSource {
        ProviderImportSource(kind: value.kind, harnessTypeId: value.harnessTypeId, sourceInstanceId: value.sourceInstanceId, settings: value.settings)
    }

    static func bindingImportSource(_ value: ProviderImportSource) -> VeluneBindings.ProviderImportSource {
        VeluneBindings.ProviderImportSource(kind: value.kind, harnessTypeId: value.harnessTypeId, sourceInstanceId: value.sourceInstanceId, providerId: nil, settings: value.settings)
    }

    static func importPreview(_ value: VeluneBindings.ProviderImportPreview) -> ProviderImportPreviewData {
        ProviderImportPreviewData(preview: ProviderImportPreview(token: value.token, sourceLabel: value.sourceLabel, warnings: value.warnings, providers: value.providers.map(importProvider)))
    }

    private static func importProvider(_ value: ImportProviderCandidate) -> ProviderImportCandidate {
        ProviderImportCandidate(id: value.id, name: value.name, protocol: value.`protocol`, endpoint: value.endpoint ?? "", canImport: value.canImport, alreadyImported: value.alreadyImported, credentialStatus: value.credentialStatus, issues: value.issues, models: value.models.map { ProviderImportModel(candidateKey: $0.candidateKey, providerModelID: $0.providerModelId, name: $0.name, contextWindow: $0.contextWindow, maxOutputTokens: $0.maxOutputTokens, reasoningLevels: $0.reasoningLevels, canImport: $0.canImport, issues: $0.issues) })
    }

    static func authenticationMetadata(_ value: VeluneBindings.AuthenticationMetadata) -> AuthenticationMetadata {
        AuthenticationMetadata(configured: value.configured, capabilities: AuthenticationCapabilities(protocol: protocolID(value.capabilities.`protocol`), endpoint: value.capabilities.endpoint, explicitOutputCap: value.capabilities.explicitOutputCap, temperature: value.capabilities.temperature), actions: value.actions?.map(settingAction))
    }

    static func authenticationProgress(_ value: AuthenticationProgress) -> AuthenticationData {
        AuthenticationData(running: value.running, events: value.events.map { AuthenticationEvent(type: $0.typeId, id: $0.id, prompt: $0.prompt.map { AuthenticationPromptBody(kind: $0.kind, text: $0.text, options: $0.options?.map { SettingOption(id: $0.id, label: $0.label) }) }, notification: $0.notification.map { AuthenticationNotification(kind: $0.kind, id: $0.id, text: $0.text, url: $0.url, instructions: $0.instructions, userCode: $0.userCode, verificationUri: $0.verificationUri) }, ok: $0.ok, error: $0.error, cancelled: $0.cancelled) }, gateways: value.gateways?.map(gateway), executionInvalidated: value.executionInvalidated)
    }
}
