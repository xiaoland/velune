import Foundation

import VeluneBindings

/// Converts generated UniFFI records at the application boundary. The Swift UI
/// models remain platform-friendly; no JSON action envelope crosses this file.
enum BindingMapping {
    static func conversation(_ value: BindingConversationSummary) -> Conversation {
        Conversation(id: value.id, title: value.title, updatedAtUnixMs: value.updatedAtUnixMs,
                     runtimeID: value.runtimeId, cwd: value.cwd)
    }

    static func snapshot(_ value: BindingConversationSnapshot) -> ConversationSnapshot {
        ConversationSnapshot(revision: value.revision, conversation: conversation(value.conversation),
                             modelRecordKey: value.modelRecordKey, runState: runState(value.runState),
                             messages: value.messages.map(message), pendingInteractions: value.pendingInteractions.map(interaction), actions: actions(value.actions))
    }

    static func runState(_ value: BindingRunState) -> RunState {
        switch value { case .idle: return .idle; case .running: return .running; case .stopping: return .stopping; case .failed: return .failed }
    }

    static func actions(_ value: BindingConversationActions) -> ConversationActions {
        ConversationActions(canSend: value.canSend, canCancel: value.canCancel, canSwitch: value.canSwitch)
    }

    static func message(_ value: BindingMessage) -> Message {
        Message(id: value.id, role: messageRole(value.role), timestampUnixMs: value.timestampUnixMs, blocks: value.blocks.map(block))
    }

    static func messageRole(_ value: BindingMessageRole) -> MessageRole {
        switch value { case .user: return .user; case .assistant: return .assistant; case .tool: return .tool; case .system: return .system }
    }
    static func toolState(_ value: BindingToolState) -> ToolState {
        switch value { case .pending: return .pending; case .running: return .running; case .completed: return .completed; case .failed: return .failed }
    }
    static func block(_ value: BindingMessageBlock) -> MessageBlock {
        switch value {
        case .text(let text): return .text(text)
        case .reasoning(let text): return .reasoning(text)
        case .tool(let id, let title, let state, let output): return .tool(id: id, title: title, state: toolState(state), output: output)
        case .notice(let text): return .notice(text)
        }
    }

    static func settingField(_ value: BindingSettingField) -> SettingField {
        return SettingField(key: value.key, label: value.label, kind: settingKind(value.kind),
                     required: value.required, value: value.value,
                     options: value.options.map { SettingOption(id: $0.id, label: $0.label) }, help: value.help,
                     executableDiscovery: value.executableDiscovery.map { ExecutableDiscoverySpec(command: $0.command, minimumVersion: $0.minimumVersion) })
    }

    static func settingKind(_ value: BindingSettingKind) -> SettingKind {
        switch value {
        case .text: return .text
        case .filePath: return .filePath
        case .directoryPath: return .directoryPath
        case .choice: return .choice
        }
    }

    static func settingAction(_ value: BindingSettingAction) -> SettingAction {
        SettingAction(id: value.id, label: value.label)
    }

    static func protocolID(_ value: BindingGatewayProtocol) -> ProviderProtocol {
        switch value { case .chatCompletionsV1: return .chatCompletionsV1; case .responsesV1: return .responsesV1; case .messagesV1: return .messagesV1 }
    }
    static func model(_ value: BindingProviderModel) -> ProviderModel {
        ProviderModel(recordKey: value.recordKey, providerModelID: value.providerModelId, nickname: value.nickname, icon: value.icon, contextWindow: value.contextWindow, maxOutputTokens: value.maxOutputTokens, reasoningLevels: value.reasoningLevels, adapterMetadataJSON: value.adapterMetadataJson)
    }
    static func bindingModel(_ value: ProviderModel) -> BindingProviderModel {
        BindingProviderModel(recordKey: value.recordKey, providerModelId: value.providerModelID, nickname: value.nickname, icon: value.icon, contextWindow: value.contextWindow, maxOutputTokens: value.maxOutputTokens, reasoningLevels: value.reasoningLevels, adapterMetadataJson: value.adapterMetadataJSON)
    }
    static func template(_ value: BindingModelTemplate) -> ModelTemplate {
        ModelTemplate(id: value.id, name: value.name, suggestedProviderModelID: value.suggestedProviderModelId, nickname: value.nickname, icon: value.icon, contextWindow: value.contextWindow, maxOutputTokens: value.maxOutputTokens, reasoningLevels: value.reasoningLevels)
    }
    static func bindingTemplate(_ value: ModelTemplate) -> BindingModelTemplate {
        BindingModelTemplate(id: value.id, name: value.name, suggestedProviderModelId: value.suggestedProviderModelID, nickname: value.nickname, icon: value.icon, contextWindow: value.contextWindow, maxOutputTokens: value.maxOutputTokens, reasoningLevels: value.reasoningLevels)
    }
    static func provider(_ value: BindingProviderDefinition) -> AIProvider {
        let method: AuthenticationMethod
        switch value.authentication.method { case .apiKey: method = .apiKey; case .oAuth: method = .oauth; case .unconfigured: method = .unconfigured }
        return AIProvider(id: value.id, name: value.name, protocolID: protocolID(value.`protocol`), endpoint: value.endpoint, authentication: ProviderAuthentication(method: method, configured: value.authentication.configured, provenance: value.authentication.provenance?.displayName, actions: value.authentication.actions.map(settingAction)), models: value.models.map(model))
    }
    static func gateway(_ value: BindingGatewayConfig) -> GatewayConfig {
        GatewayConfig(id: value.id, name: value.name, providers: value.providers.map(provider), failover: FailoverPolicy(mode: .disabled))
    }
    static func bindingProvider(_ value: AIProvider) -> BindingProviderDraft {
        BindingProviderDraft(id: value.id, name: value.name, protocol: bindingProtocol(value.protocolID), endpoint: value.endpoint, models: value.models.map(bindingModel))
    }
    static func bindingAuthenticationEdit(_ value: AuthenticationEdit) -> BindingAuthenticationEdit {
        switch value { case .keep: return .keep; case .setAPIKey(let value): return .setApiKey(value: value); case .clear: return .clear }
    }
    static func bindingProtocol(_ value: ProviderProtocol) -> BindingGatewayProtocol {
        switch value { case .chatCompletionsV1: return .chatCompletionsV1; case .responsesV1: return .responsesV1; case .messagesV1: return .messagesV1 }
    }

    static func bindingRuntime(_ value: RuntimeInstance) -> BindingRuntimeInstance {
        BindingRuntimeInstance(id: value.id, name: value.name, typeId: value.typeID, gatewayId: value.gatewayID, settings: value.settings)
    }

    static func bindingSelection(_ value: ProviderImportSelection) -> BindingImportSelection {
        BindingImportSelection(providerId: value.providerId, candidateKeys: value.candidateKeys)
    }

    static func runtime(_ value: BindingRuntimeInstance) -> RuntimeInstance {
        RuntimeInstance(id: value.id, name: value.name, typeID: value.typeId,
                        gatewayID: value.gatewayId, settings: value.settings)
    }

    static func runtimeType(_ value: BindingRuntimeTypeDescriptor) -> RuntimeTypeDescriptor {
        RuntimeTypeDescriptor(id: value.id, familyID: value.familyId, versionRegex: value.versionRegex, supportedProtocols: value.supportedProtocols.map(protocolID), name: value.name, fields: value.fields.map(settingField))
    }

    static func configuration(_ value: BindingConfigurationSnapshot) -> (conversations: [Conversation], historyFailures: [HistoryFailure], gateways: [GatewayConfig], runtimes: [RuntimeInstance], runtimeTypes: [RuntimeTypeDescriptor], modelTemplates: [ModelTemplate], importTypes: [RuntimeTypeDescriptor], protocols: [ProtocolDescriptor], selectedRuntimeID: String?) {
        (value.conversations.map(conversation), value.historyFailures.map { HistoryFailure(runtimeID: $0.runtimeId, detail: $0.detail) }, value.gateways.map(gateway), value.runtimeInstances.map(runtime), value.runtimeTypes.map(runtimeType), value.modelTemplates.map(template), value.providerImportTypes.map(runtimeType), value.protocols.map { ProtocolDescriptor(id: protocolID($0.id), name: $0.name, supported: $0.supported) }, value.selectedRuntimeInstanceId)
    }

    static func interaction(_ value: BindingRuntimeInteraction) -> RuntimeInteraction {
        let kind: InteractionKind
        switch value.kind {
        case .approval(let title, let detail, let options): kind = .approval(title: title, detail: detail, options: options.map { InteractionOption(id: $0.id, label: $0.label) })
        case .userInput(let questions): kind = .userInput(questions: questions.map { InteractionQuestion(id: $0.id, text: $0.text, options: $0.options.map { InteractionOption(id: $0.id, label: $0.label) }, secret: $0.secret) })
        }
        return RuntimeInteraction(id: value.id, kind: kind)
    }
    static func bindingInteractionReply(_ value: RuntimeInteractionReply) -> BindingRuntimeInteractionReply {
        switch value {
        case .decision(let option): return .decision(optionId: option)
        case .answers(let answers): return .answers(answers: answers.map { BindingInteractionAnswer(questionId: $0.questionID, values: $0.values) })
        case .cancel: return .cancel
        }
    }

    static func snapshotResult(_ value: BindingSnapshotResult) -> ConversationSnapshot? {
        value.snapshot.map(snapshot)
    }

    static func importSource(_ value: BindingProviderImportSource) -> ProviderImportSource {
        ProviderImportSource(kind: value.kind, harnessTypeId: value.harnessTypeId, sourceInstanceId: value.sourceInstanceId, settings: value.settings)
    }

    static func bindingImportSource(_ value: ProviderImportSource) -> BindingProviderImportSource {
        BindingProviderImportSource(kind: value.kind, harnessTypeId: value.harnessTypeId, sourceInstanceId: value.sourceInstanceId, providerId: nil, settings: value.settings)
    }

    static func importPreview(_ value: BindingProviderImportPreview) -> ProviderImportPreviewData {
        ProviderImportPreviewData(preview: ProviderImportPreview(token: value.token, sourceLabel: value.sourceLabel, warnings: value.warnings, providers: value.providers.map(importProvider)))
    }

    private static func importProvider(_ value: BindingImportProviderCandidate) -> ProviderImportCandidate {
        ProviderImportCandidate(id: value.id, name: value.name, protocol: value.`protocol`, endpoint: value.endpoint ?? "", canImport: value.canImport, alreadyImported: value.alreadyImported, credentialStatus: value.credentialStatus, issues: value.issues, models: value.models.map { ProviderImportModel(candidateKey: $0.candidateKey, providerModelID: $0.providerModelId, name: $0.name, contextWindow: $0.contextWindow, maxOutputTokens: $0.maxOutputTokens, reasoningLevels: $0.reasoningLevels, canImport: $0.canImport, issues: $0.issues) })
    }

    static func authenticationMetadata(_ value: BindingAuthenticationMetadata) -> AuthenticationMetadata {
        AuthenticationMetadata(configured: value.configured, capabilities: AuthenticationCapabilities(protocol: protocolID(value.capabilities.`protocol`), endpoint: value.capabilities.endpoint, explicitOutputCap: value.capabilities.explicitOutputCap, temperature: value.capabilities.temperature), actions: value.actions?.map(settingAction))
    }

    static func authenticationProgress(_ value: BindingAuthenticationProgress) -> AuthenticationData {
        AuthenticationData(running: value.running, events: value.events.map { AuthenticationEvent(type: $0.typeId, id: $0.id, prompt: $0.prompt.map { AuthenticationPromptBody(kind: $0.kind, text: $0.text, options: $0.options?.map { SettingOption(id: $0.id, label: $0.label) }) }, notification: $0.notification.map { AuthenticationNotification(kind: $0.kind, id: $0.id, text: $0.text, url: $0.url, instructions: $0.instructions, userCode: $0.userCode, verificationUri: $0.verificationUri) }, ok: $0.ok, error: $0.error, cancelled: $0.cancelled) }, gateways: value.gateways?.map(gateway), executionInvalidated: value.executionInvalidated)
    }
}
